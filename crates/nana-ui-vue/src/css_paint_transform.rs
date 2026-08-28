//! CSS `transform` list → [`PaintTransform`] 2×3 or planar [`PaintMat4`].
//!
//! Parse lives here (L1 adapter). Scene paints 2×3 via
//! [`PaintTransform::around_origin`] and planar 3D via the z=0 homography in
//! the existing quad pass (perspective divide, same 4 vertices).
//! `transform-origin` / `transform-box` are separate Style Model fields
//! resolved at paint/hit time (box size is not known at parse).
//! `content-box` / `fill-box` use padding + border on `LayoutStyle`;
//! `view-box` uses the border box (no SVG viewport).
//!
//! ## Planar 3D
//!
//! These compose as CSS `matrix3d` and paint when the z=0 plane is a
//! homography (`perspective()` + `rotateY` trapezoid):
//!
//! - `rotateX` / `rotateY` / `perspective()`
//! - `rotate3d`
//! - `matrix3d` (including perspective residual)
//! - `translateZ` / `translate3d` z / `scale3d` z (with perspective they scale)
//!
//! ## Still fail-closed
//!
//! - Unknown transform functions
//! - Parent CSS `perspective` property / `transform-style: preserve-3d`
//!   (parsed onto LayoutStyle flags; not a 3D rendering context)

use nana_ui_core::box_layout::{
    LengthSpec, PaintMat4, PaintTransform, TransformBox, TransformOrigin,
};

use crate::css_map::parse_css_length_px;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ParsedPaintTransform {
    Affine(PaintTransform),
    Mat4(PaintMat4),
}

/// Parse a CSS transform list. 2D stays 2×3; 3D functions stay 4×4 even when
/// the z=0 projection is affine, so a parent `perspective` / `preserve-3d`
/// can fail-close instead of painting an orthographic squash.
pub(crate) fn parse_css_transform(raw: &str) -> Option<ParsedPaintTransform> {
    let (mat, used_3d) = parse_transform_mat4(raw)?;
    if !mat.m.iter().all(|v| v.is_finite()) {
        return None;
    }
    if !used_3d {
        let affine = mat.as_affine()?;
        return [affine.a, affine.b, affine.c, affine.d, affine.e, affine.f]
            .into_iter()
            .all(f32::is_finite)
            .then_some(ParsedPaintTransform::Affine(affine));
    }
    Some(ParsedPaintTransform::Mat4(mat))
}

/// Parse a CSS transform list into one 2×3 paint matrix (function order preserved).
pub(crate) fn parse_paint_transform(raw: &str) -> Option<PaintTransform> {
    match parse_css_transform(raw)? {
        ParsedPaintTransform::Affine(transform) => Some(transform),
        ParsedPaintTransform::Mat4(mat) => mat.as_affine(),
    }
}

fn parse_transform_mat4(raw: &str) -> Option<(PaintMat4, bool)> {
    let mut rest = raw.trim();
    let mut result = PaintMat4::IDENTITY;
    let mut used_3d = false;
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        used_3d |= matches!(
            name.as_str(),
            "rotatex"
                | "rotatey"
                | "rotate3d"
                | "perspective"
                | "matrix3d"
                | "translatez"
                | "translate3d"
                | "scale3d"
                | "scalez"
        );
        let close = rest[open + 1..].find(')')? + open + 1;
        let args = rest[open + 1..close]
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let next = match name.as_str() {
            "translate" => {
                if !(1..=2).contains(&args.len()) {
                    return None;
                }
                let x = parse_transform_length(args.first().copied()?)?;
                let y = match args.get(1).copied() {
                    Some(value) => parse_transform_length(value)?,
                    None => 0.0,
                };
                PaintMat4::translation(x, y, 0.0)
            }
            "translatex" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::translation(parse_transform_length(args.first().copied()?)?, 0.0, 0.0)
            }
            "translatey" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::translation(0.0, parse_transform_length(args.first().copied()?)?, 0.0)
            }
            "translatez" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::translation(0.0, 0.0, parse_transform_length(args.first().copied()?)?)
            }
            "translate3d" if args.len() == 3 => {
                let x = parse_transform_length(args[0])?;
                let y = parse_transform_length(args[1])?;
                let z = parse_transform_length(args[2])?;
                PaintMat4::translation(x, y, z)
            }
            "scale" | "scalex" | "scaley" => {
                let valid_len = if name == "scale" {
                    (1..=2).contains(&args.len())
                } else {
                    args.len() == 1
                };
                if !valid_len {
                    return None;
                }
                let x = args.first()?.parse::<f32>().ok()?;
                let (x, y) = match name.as_str() {
                    "scalex" => (x, 1.0),
                    "scaley" => (1.0, x),
                    _ => (
                        x,
                        match args.get(1) {
                            Some(value) => value.parse::<f32>().ok()?,
                            None => x,
                        },
                    ),
                };
                PaintMat4::scaling(x, y, 1.0)
            }
            "scale3d" if args.len() == 3 => {
                let x = args[0].parse::<f32>().ok()?;
                let y = args[1].parse::<f32>().ok()?;
                let z = args[2].parse::<f32>().ok()?;
                PaintMat4::scaling(x, y, z)
            }
            "matrix" if args.len() == 6 => {
                let values = args
                    .iter()
                    .map(|value| value.parse::<f32>().ok())
                    .collect::<Option<Vec<_>>>()?;
                PaintMat4::from_affine(PaintTransform {
                    a: values[0],
                    b: values[1],
                    c: values[2],
                    d: values[3],
                    e: values[4],
                    f: values[5],
                })
            }
            "matrix3d" if args.len() == 16 => {
                let values = args
                    .iter()
                    .map(|value| value.parse::<f32>().ok())
                    .collect::<Option<Vec<_>>>()?;
                let mut m = [0.0f32; 16];
                m.copy_from_slice(&values);
                PaintMat4::from_matrix3d(m)?
            }
            "rotate" | "rotatez" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::rotate_z(parse_transform_angle(args.first().copied()?)?)
            }
            "rotatex" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::rotate_x(parse_transform_angle(args.first().copied()?)?)
            }
            "rotatey" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::rotate_y(parse_transform_angle(args.first().copied()?)?)
            }
            "rotate3d" if args.len() == 4 => {
                let x = args[0].parse::<f32>().ok()?;
                let y = args[1].parse::<f32>().ok()?;
                let z = args[2].parse::<f32>().ok()?;
                let angle = parse_transform_angle(args[3])?;
                PaintMat4::rotate3d(x, y, z, angle)?
            }
            "perspective" => {
                if args.len() != 1 {
                    return None;
                }
                let d = parse_transform_length(args.first().copied()?)?;
                PaintMat4::perspective(d)?
            }
            "skew" => {
                if !(1..=2).contains(&args.len()) {
                    return None;
                }
                let x = parse_transform_angle(args.first().copied()?)?.tan();
                let y = match args.get(1).copied() {
                    Some(value) => parse_transform_angle(value)?,
                    None => 0.0,
                }
                .tan();
                PaintMat4::from_affine(PaintTransform {
                    b: y,
                    c: x,
                    ..PaintTransform::default()
                })
            }
            "skewx" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::from_affine(PaintTransform {
                    c: parse_transform_angle(args.first().copied()?)?.tan(),
                    ..PaintTransform::default()
                })
            }
            "skewy" => {
                if args.len() != 1 {
                    return None;
                }
                PaintMat4::from_affine(PaintTransform {
                    b: parse_transform_angle(args.first().copied()?)?.tan(),
                    ..PaintTransform::default()
                })
            }
            _ => return None,
        };
        result = result.then(next);
        rest = rest[close + 1..].trim_start();
    }
    Some((result, used_3d))
}

fn parse_transform_angle(raw: &str) -> Option<f32> {
    let raw = raw.trim().to_ascii_lowercase();
    if let Some(value) = raw.strip_suffix("deg") {
        value.trim().parse::<f32>().ok().map(f32::to_radians)
    } else if let Some(value) = raw.strip_suffix("rad") {
        value.trim().parse().ok()
    } else if let Some(value) = raw.strip_suffix("turn") {
        value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|turns| turns * std::f32::consts::TAU)
    } else if let Some(value) = raw.strip_suffix("grad") {
        value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|grads| grads * std::f32::consts::PI / 200.0)
    } else if raw == "0" || raw == "+0" || raw == "-0" {
        Some(0.0)
    } else {
        None
    }
}

/// CSS `transform-origin`: keywords, `%`, `px`, 1–3 values. The optional z
/// length is accepted and dropped (2×3 has no z pivot).
pub(crate) fn parse_transform_origin(raw: &str) -> Option<TransformOrigin> {
    let tokens: Vec<&str> = raw
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect();
    match tokens.as_slice() {
        [one] => parse_origin_one(one),
        [a, b] => parse_origin_two(a, b),
        [a, b, z] => {
            parse_origin_z(z)?;
            parse_origin_two(a, b)
        }
        _ => None,
    }
}

/// CSS `transform-box`. `stroke-box` is not parsed (HTML used-value would be
/// border-box). `initial` / `unset` restore CSS initial `view-box`.
pub(crate) fn parse_transform_box(raw: &str) -> Option<TransformBox> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "border-box" => Some(TransformBox::BorderBox),
        "fill-box" => Some(TransformBox::FillBox),
        "view-box" => Some(TransformBox::ViewBox),
        "content-box" => Some(TransformBox::ContentBox),
        "initial" | "unset" => Some(TransformBox::ViewBox),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum OriginKw {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

fn origin_keyword(raw: &str) -> Option<OriginKw> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "left" => Some(OriginKw::Left),
        "right" => Some(OriginKw::Right),
        "top" => Some(OriginKw::Top),
        "bottom" => Some(OriginKw::Bottom),
        "center" => Some(OriginKw::Center),
        _ => None,
    }
}

fn parse_origin_length(raw: &str) -> Option<LengthSpec> {
    let raw = raw.trim();
    if raw == "0" || raw == "+0" || raw == "-0" {
        return Some(LengthSpec::Px(0.0));
    }
    if let Some(p) = raw.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(LengthSpec::Percent);
    }
    let px = raw.strip_suffix("px").or_else(|| raw.strip_suffix("PX"))?;
    px.trim().parse().ok().map(LengthSpec::Px)
}

fn parse_origin_z(raw: &str) -> Option<()> {
    matches!(parse_origin_length(raw), Some(LengthSpec::Px(_))).then_some(())
}

fn origin_x(kw: OriginKw) -> Option<LengthSpec> {
    match kw {
        OriginKw::Left => Some(LengthSpec::Percent(0.0)),
        OriginKw::Right => Some(LengthSpec::Percent(100.0)),
        OriginKw::Center => Some(LengthSpec::Percent(50.0)),
        OriginKw::Top | OriginKw::Bottom => None,
    }
}

fn origin_y(kw: OriginKw) -> Option<LengthSpec> {
    match kw {
        OriginKw::Top => Some(LengthSpec::Percent(0.0)),
        OriginKw::Bottom => Some(LengthSpec::Percent(100.0)),
        OriginKw::Center => Some(LengthSpec::Percent(50.0)),
        OriginKw::Left | OriginKw::Right => None,
    }
}

fn parse_origin_one(raw: &str) -> Option<TransformOrigin> {
    if let Some(kw) = origin_keyword(raw) {
        return Some(match kw {
            OriginKw::Left | OriginKw::Right | OriginKw::Center => TransformOrigin {
                x: origin_x(kw)?,
                y: LengthSpec::Percent(50.0),
            },
            OriginKw::Top | OriginKw::Bottom => TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: origin_y(kw)?,
            },
        });
    }
    Some(TransformOrigin {
        x: parse_origin_length(raw)?,
        y: LengthSpec::Percent(50.0),
    })
}

fn parse_origin_two(a: &str, b: &str) -> Option<TransformOrigin> {
    match (origin_keyword(a), origin_keyword(b)) {
        (Some(ka), Some(kb)) => {
            if let (Some(x), Some(y)) = (origin_x(ka), origin_y(kb)) {
                return Some(TransformOrigin { x, y });
            }
            if let (Some(x), Some(y)) = (origin_x(kb), origin_y(ka)) {
                return Some(TransformOrigin { x, y });
            }
            None
        }
        (Some(ka), None) => Some(TransformOrigin {
            x: origin_x(ka)?,
            y: parse_origin_length(b)?,
        }),
        (None, Some(kb)) => Some(TransformOrigin {
            x: parse_origin_length(a)?,
            y: origin_y(kb)?,
        }),
        (None, None) => Some(TransformOrigin {
            x: parse_origin_length(a)?,
            y: parse_origin_length(b)?,
        }),
    }
}

fn parse_transform_length(raw: &str) -> Option<f32> {
    if raw == "0" || raw == "+0" || raw == "-0" {
        Some(0.0)
    } else if let Some(value) = raw
        .trim()
        .strip_suffix("px")
        .or_else(|| raw.trim().strip_suffix("PX"))
    {
        value.trim().parse().ok()
    } else {
        parse_css_length_px(raw, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_interactive::parse_keyframes_at_rule;
    use crate::css_interactive_apply::keyframe_paint_at;
    use crate::css_map::LayoutStyleCss;
    use nana_ui_core::LayoutStyle;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    fn approx_matrix(got: [f32; 6], expected: [f32; 6]) {
        for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
            assert!((g - e).abs() < 1e-4, "matrix[{i}]: expected {e}, got {g}");
        }
    }

    fn assert_unsupported(css: &str) {
        assert!(
            parse_css_transform(css).is_none(),
            "expected fail-closed parse for {css}"
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(&format!("transform: {css}"), None, None);
        assert_eq!(layout.transform, None);
        assert_eq!(layout.transform_3d, None);
        assert!(
            layout.unsupported_transform.is_some(),
            "expected unsupported_transform for {css}"
        );
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        (dx * dx + dy * dy).sqrt()
    }

    #[test]
    fn rotate_y_without_perspective_is_orthographic_mat4() {
        let parsed = parse_css_transform("rotateY(60deg)").expect("rotateY");
        let ParsedPaintTransform::Mat4(mat) = parsed else {
            panic!("3D functions stay 4×4 so parent perspective can fail-close");
        };
        let t = mat.as_affine().expect("orthographic z=0");
        approx(t.a, 60_f32.to_radians().cos());
        approx(t.d, 1.0);
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("transform: rotateY(60deg)", None, None);
        assert!(layout.transform_3d.is_some());
        assert_eq!(layout.transform, None);
        assert!(parse_css_transform("rotateY(0deg)").is_some());
    }

    #[test]
    fn rotate_x_without_perspective_is_orthographic() {
        assert!(parse_css_transform("rotateX(45deg)").is_some());
    }

    #[test]
    fn perspective_rotate_y_stores_4x4_trapezoid() {
        let parsed = parse_css_transform("perspective(800px) rotateY(30deg)").expect("3d");
        let ParsedPaintTransform::Mat4(mat) = parsed else {
            panic!("perspective+rotateY must stay 4×4, not squash to 2×3");
        };
        let pivoted = mat.around_origin(0.0, 0.0, 100.0, 40.0);
        let corners = pivoted
            .projected_corners(0.0, 0.0, 200.0, 80.0)
            .expect("corners");
        let left = dist(corners[0], corners[3]);
        let right = dist(corners[1], corners[2]);
        assert!(
            (left - right).abs() > 4.0,
            "non-uniform x scale / trapezoid: left={left} right={right} corners={corners:?}"
        );
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("transform: perspective(800px) rotateY(30deg)", None, None);
        assert!(layout.transform.is_none());
        assert!(layout.transform_3d.is_some());
        assert_eq!(layout.unsupported_transform, None);
        let (_, persp) = layout
            .world_scene_transform(0.0, 0.0, 200.0, 80.0)
            .expect("scene");
        assert!(persp[0].abs() > 1e-4, "g must be nonzero, {persp:?}");
    }

    #[test]
    fn perspective_alone_is_planar_identity() {
        let parsed = parse_css_transform("perspective(800px)").expect("perspective");
        let ParsedPaintTransform::Mat4(mat) = parsed else {
            panic!("perspective() stays 4×4");
        };
        assert!(mat.as_affine().expect("z=0").is_identity());
    }

    #[test]
    fn unknown_transform_still_fails_closed() {
        assert_unsupported("rotate3d()");
        assert_unsupported("perspective(0px)");
        assert_unsupported("not-a-function(1)");
    }

    #[test]
    fn translate3d_z0_applies_xy() {
        let transform = parse_paint_transform("translate3d(10px, -4px, 0)").expect("z=0");
        approx(transform.e, 10.0);
        approx(transform.f, -4.0);
        assert!(transform.a == 1.0 && transform.d == 1.0);
    }

    #[test]
    fn translate3d_nonzero_z_keeps_xy() {
        let transform = parse_paint_transform("translate3d(10px, 20px, 100px)").expect("nonzero z");
        approx(transform.e, 10.0);
        approx(transform.f, 20.0);
        approx(transform.a, 1.0);
        approx(transform.d, 1.0);
        approx(
            parse_paint_transform("scale(2) translate3d(4px, 0, 50px)")
                .expect("must not drop the list")
                .e,
            8.0,
        );
    }

    #[test]
    fn translate3d_nonzero_z_is_supported_on_layout() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("transform: translate3d(4px, 0, 12px)", None, None);
        let mat = layout.transform_3d.expect("3D function stays 4×4");
        let transform = mat.as_affine().expect("no perspective");
        approx(transform.e, 4.0);
        approx(transform.f, 0.0);
        assert_eq!(layout.transform, None);
        assert_eq!(layout.unsupported_transform, None);
    }

    #[test]
    fn matrix3d_2d_embedding_extracts_affine() {
        let transform =
            parse_paint_transform("matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 12, -8, 0, 1)")
                .expect("2D matrix3d");
        approx(transform.a, 1.0);
        approx(transform.d, 1.0);
        approx(transform.e, 12.0);
        approx(transform.f, -8.0);
    }

    #[test]
    fn matrix3d_perspective_residual_is_planar_3d() {
        let parsed =
            parse_css_transform("matrix3d(1, 0, 0, 0.1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)")
                .expect("m14");
        assert!(matches!(parsed, ParsedPaintTransform::Mat4(_)));
        let parsed =
            parse_css_transform("matrix3d(1, 0, 0, 0, 0, 1, 0, 0.2, 0, 0, 1, 0, 0, 0, 0, 1)")
                .expect("m24");
        assert!(matches!(parsed, ParsedPaintTransform::Mat4(_)));
    }

    #[test]
    fn matrix3d_rotate_y_is_orthographic_or_3d() {
        let angle = 40_f32.to_radians();
        let (sin, cos) = angle.sin_cos();
        let raw = format!(
            "matrix3d({cos}, 0, {sin}, 0, 0, 1, 0, 0, {nsin}, 0, {cos}, 0, 0, 0, 0, 1)",
            nsin = -sin
        );
        assert!(parse_css_transform(&raw).is_some());
    }

    #[test]
    fn rotate3d_and_scale3d_are_supported() {
        assert!(parse_css_transform("rotate3d(1, 0, 0, 45deg)").is_some());
        let scale = parse_css_transform("scale3d(1, 1, 2)").expect("scale3d");
        match scale {
            ParsedPaintTransform::Affine(t) => {
                approx(t.a, 1.0);
                approx(t.d, 1.0);
            }
            ParsedPaintTransform::Mat4(_) => {}
        }
    }

    #[test]
    fn two_d_functions_unchanged() {
        let transform = parse_paint_transform("scale(2) translate(4px, -3px)").expect("2D");
        approx(transform.a, 2.0);
        approx(transform.e, 8.0);
        approx(transform.f, -6.0);
        let rotated = parse_paint_transform("rotate(90deg)").expect("rotate");
        approx(rotated.a, 0.0);
        approx(rotated.b, 1.0);
        approx(rotated.c, -1.0);
        approx(rotated.d, 0.0);
    }

    #[test]
    fn keyframes_lerp_2d_rotate_affine() {
        let (rule, _) = parse_keyframes_at_rule(
            "@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(90deg); } }",
            0,
        )
        .expect("keyframes");
        let mid = keyframe_paint_at(&rule, 0.5).expect("sample");
        let transform = mid.transform.expect("transform");
        // Motion bucket lerps the 2×3, not the angle.
        approx(transform.a, 0.5);
        approx(transform.b, 0.5);
        approx(transform.c, -0.5);
        approx(transform.d, 0.5);
    }

    fn rotate_90() -> PaintTransform {
        PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            ..PaintTransform::default()
        }
    }

    #[test]
    fn transform_origin_center_matches_50_percent_not_zero_zero() {
        let rotate = rotate_90();
        let mut center = LayoutStyle {
            transform: Some(rotate),
            ..LayoutStyle::default()
        };
        center.apply_css_text("transform-origin: center", None, None);
        let mut pct = LayoutStyle {
            transform: Some(rotate),
            ..LayoutStyle::default()
        };
        pct.apply_css_text("transform-origin: 50% 50%", None, None);
        let mut zero = LayoutStyle {
            transform: Some(rotate),
            ..LayoutStyle::default()
        };
        zero.apply_css_text("transform-origin: 0 0", None, None);

        let via_center = rotate.around_center(0.0, 0.0, 80.0, 40.0);
        assert_eq!(
            center.world_paint_transform(0.0, 0.0, 80.0, 40.0),
            Some(via_center)
        );
        assert_eq!(
            pct.world_paint_transform(0.0, 0.0, 80.0, 40.0),
            Some(via_center)
        );
        let via_zero = rotate.around_origin(0.0, 0.0, 0.0, 0.0);
        assert_eq!(
            zero.world_paint_transform(0.0, 0.0, 80.0, 40.0),
            Some(via_zero)
        );
        assert_ne!(via_center, via_zero);
    }

    #[test]
    fn transform_origin_keywords_percent_px_and_ignored_z() {
        assert_eq!(
            parse_transform_origin("left top"),
            Some(TransformOrigin {
                x: LengthSpec::Percent(0.0),
                y: LengthSpec::Percent(0.0),
            })
        );
        assert_eq!(
            parse_transform_origin("top left"),
            Some(TransformOrigin {
                x: LengthSpec::Percent(0.0),
                y: LengthSpec::Percent(0.0),
            })
        );
        assert_eq!(
            parse_transform_origin("12px 8px"),
            Some(TransformOrigin {
                x: LengthSpec::Px(12.0),
                y: LengthSpec::Px(8.0),
            })
        );
        assert_eq!(
            parse_transform_origin("0 0 24px"),
            parse_transform_origin("0 0")
        );
        assert_eq!(parse_transform_origin("0 0 50%"), None);

        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "transform: rotate(90deg); transform-origin: 0 0 16px",
            None,
            None,
        );
        assert_eq!(
            layout.transform_origin,
            Some(TransformOrigin {
                x: LengthSpec::Px(0.0),
                y: LengthSpec::Px(0.0),
            })
        );
        layout.apply_css_text("perspective-origin: 0 0", None, None);
        assert_eq!(
            layout.transform_origin,
            Some(TransformOrigin {
                x: LengthSpec::Px(0.0),
                y: LengthSpec::Px(0.0),
            }),
            "perspective-origin must not write transform-origin"
        );
    }

    #[test]
    fn transform_box_keywords_and_content_box_origin() {
        assert_eq!(
            parse_transform_box("content-box"),
            Some(TransformBox::ContentBox)
        );
        assert_eq!(parse_transform_box("fill-box"), Some(TransformBox::FillBox));
        assert_eq!(parse_transform_box("view-box"), Some(TransformBox::ViewBox));
        assert_eq!(
            parse_transform_box("border-box"),
            Some(TransformBox::BorderBox)
        );
        assert_eq!(parse_transform_box("initial"), Some(TransformBox::ViewBox));
        assert_eq!(parse_transform_box("stroke-box"), None);

        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "transform: rotate(90deg); transform-origin: 0 0; transform-box: content-box; padding: 10px; border: 5px solid black",
            None,
            None,
        );
        assert_eq!(layout.transform_box, TransformBox::ContentBox);
        let content = layout
            .world_paint_transform(0.0, 0.0, 100.0, 50.0)
            .expect("content-box origin");
        approx_matrix(content, rotate_90().around_origin(0.0, 0.0, 15.0, 15.0));
        approx(content[4], 30.0);
        layout.apply_css_text("transform-box: view-box", None, None);
        assert_eq!(layout.transform_box, TransformBox::ViewBox);
        approx_matrix(
            layout
                .world_paint_transform(0.0, 0.0, 100.0, 50.0)
                .expect("view-box origin"),
            rotate_90().around_origin(0.0, 0.0, 0.0, 0.0),
        );
        layout.apply_css_text("transform-box: fill-box", None, None);
        approx_matrix(
            layout
                .world_paint_transform(0.0, 0.0, 100.0, 50.0)
                .expect("fill-box origin"),
            rotate_90().around_origin(0.0, 0.0, 15.0, 15.0),
        );
    }

    #[test]
    fn parent_perspective_and_preserve_3d_fail_closed() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("perspective: 800px", None, None);
        assert_eq!(layout.css_perspective, Some(800.0));
        assert!(layout.fails_closed_3d_context());
        layout.apply_css_text("transform-style: preserve-3d", None, None);
        assert!(layout.preserve_3d);
        layout.apply_css_text("transform-style: flat", None, None);
        assert!(!layout.preserve_3d);
        layout.apply_css_text("perspective: none", None, None);
        assert_eq!(layout.css_perspective, None);
    }
}
