//! Path fill/stroke outline construction and dashing via `i_overlay`.

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{
    LineCap as OverlayCap, LineJoin as OverlayJoin, StrokeStyle as OverlayStroke,
};

use super::*;

pub(super) fn fill_shapes(path: &PaintPath, t: Affine) -> Shapes {
    let contours: Vec<Contour> = flatten(path, t, TOLERANCE)
        .into_iter()
        .map(|(contour, _)| contour)
        .filter(|contour| contour.len() >= 3)
        .collect();
    simplify_contours(contours, path.fill_rule())
}

/// The region a stroke covers, as clean polygons.
///
/// The stroke — width, joins, caps, dashes — is laid out in the painter's
/// coordinates before its local transform, then the outline is transformed:
/// a stroke scales with the transform, as on a Canvas.
pub(super) fn stroke_shapes(path: &PaintPath, stroke: &StrokeStyle, t: Affine) -> Shapes {
    if !stroke.width.is_finite() || stroke.width <= 0.0 || !path_is_drawable(path, t) {
        return Vec::new();
    }
    let cap = |cap: LineCap| match cap {
        LineCap::Butt => OverlayCap::Butt,
        LineCap::Round => OverlayCap::Round(0.1),
        LineCap::Square => OverlayCap::Square,
    };
    let style = OverlayStroke::new(f64::from(stroke.width.min(MAX_LENGTH)))
        .start_cap(cap(stroke.cap))
        .end_cap(cap(stroke.cap))
        .line_join(match stroke.join {
            // A miter longer than `limit` widths is where the corner's
            // interior angle drops below 2·asin(1 / limit); bevel there.
            LineJoin::Miter => {
                let limit = if stroke.miter_limit.is_finite() {
                    stroke.miter_limit.max(1.0)
                } else {
                    10.0
                };
                OverlayJoin::Miter(2.0 * (1.0 / f64::from(limit)).asin())
            }
            LineJoin::Round => OverlayJoin::Round(0.1),
            LineJoin::Bevel => OverlayJoin::Bevel,
        });
    let tolerance = TOLERANCE / affine_scale(t).max(1e-3);
    let dash = dash_pattern(&stroke.dash);
    let mut contours: Vec<Contour> = Vec::new();
    for (points, closed) in flatten(path, AFFINE_IDENTITY, tolerance) {
        if points.len() < 2 {
            continue;
        }
        match &dash {
            Some(pattern) if !too_many_dashes(&points, closed, pattern) => {
                for piece in dashes(&points, closed, pattern, stroke.dash_offset) {
                    if piece.len() >= 2 {
                        for shape in piece.stroke(style.clone(), false) {
                            contours.extend(shape);
                        }
                    }
                }
            }
            _ => {
                for shape in points.stroke(style.clone(), closed) {
                    contours.extend(shape);
                }
            }
        }
    }
    if contours.is_empty() {
        return Vec::new();
    }
    // Sub-paths and dashes overlap each other; one union keeps a translucent
    // stroke from darkening where they cross.
    let mut shapes = contours.simplify_shape(OverlayFill::NonZero);
    transform_stroke_shapes(&mut shapes, t);
    shapes
}

pub(super) fn transform_stroke_shapes(shapes: &mut Shapes, t: Affine) {
    if t != AFFINE_IDENTITY {
        let flips = t[0] * t[3] - t[1] * t[2] < 0.0;
        for shape in shapes.iter_mut() {
            for contour in shape.iter_mut() {
                for point in contour.iter_mut() {
                    let [x, y] = affine_apply(t, [point[0] as f32, point[1] as f32]);
                    *point = [f64::from(x), f64::from(y)];
                }
                // A mirroring transform reverses every contour; keep outer
                // contours and holes running the way the fill expects.
                if flips {
                    contour.reverse();
                }
            }
        }
    }
}

/// A usable dash pattern: `None` for a solid stroke.
pub(super) fn dash_pattern(dash: &[f32]) -> Option<Vec<f64>> {
    if dash.is_empty()
        || dash
            .iter()
            .any(|length| !length.is_finite() || *length < 0.0)
    {
        return None;
    }
    let mut pattern: Vec<f64> = dash.iter().map(|length| f64::from(*length)).collect();
    if pattern.len() % 2 == 1 {
        pattern.extend_from_within(..);
    }
    (pattern.iter().sum::<f64>() > 1e-6).then_some(pattern)
}

/// Past this many dashes a stroke is drawn solid: the pieces would be
/// sub-pixel, and cutting and outlining millions of them would stall the frame.
pub(super) const MAX_DASHES: f64 = 10_000.0;

pub(super) fn too_many_dashes(points: &Contour, closed: bool, pattern: &[f64]) -> bool {
    let mut length: f64 = points
        .windows(2)
        .map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt())
        .sum();
    if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
        length += ((first[0] - last[0]).powi(2) + (first[1] - last[1]).powi(2)).sqrt();
    }
    let period: f64 = pattern.iter().sum();
    let dashes = length / period;
    dashes.is_nan() || dashes > MAX_DASHES
}

/// The "on" pieces of a polyline under a dash pattern, as open polylines.
pub(super) fn dashes(points: &Contour, closed: bool, pattern: &[f64], offset: f32) -> Vec<Contour> {
    let mut path = points.clone();
    if closed {
        path.push(points[0]);
    }
    let period: f64 = pattern.iter().sum();
    // Where in the pattern the path starts.
    let mut phase = f64::from(offset).rem_euclid(period);
    let mut index = 0;
    while phase >= pattern[index] {
        phase -= pattern[index];
        index = (index + 1) % pattern.len();
    }
    let mut left = pattern[index] - phase;
    let mut on = index % 2 == 0;
    let mut pieces = Vec::new();
    let mut current: Contour = if on { vec![path[0]] } else { Vec::new() };
    for segment in path.windows(2) {
        let (mut from, to) = (segment[0], segment[1]);
        let mut length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        while length > left {
            let k = left / length;
            let cut = [
                from[0] + (to[0] - from[0]) * k,
                from[1] + (to[1] - from[1]) * k,
            ];
            if on {
                current.push(cut);
                pieces.push(std::mem::take(&mut current));
            } else {
                current = vec![cut];
            }
            on = !on;
            length -= left;
            from = cut;
            index = (index + 1) % pattern.len();
            left = pattern[index];
        }
        left -= length;
        if on {
            current.push(to);
        }
    }
    if on && current.len() >= 2 {
        pieces.push(current);
    }
    pieces
}
