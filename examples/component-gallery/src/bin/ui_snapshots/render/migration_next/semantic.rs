//! Issue #101 §3: the Gallery state matrix as resolved style, not as pixels.
//!
//! The committed PNG tree is keyed by GPU adapter, so it can only be verified
//! on the machine that recorded it. That is the right trade for a pixel gate,
//! but it leaves the theme baseline unreadable everywhere else: a reviewer on
//! another adapter cannot tell whether light/dark, hover, pressed, focused,
//! disabled, selected, checked or invalid still resolve to the values the
//! design system intends.
//!
//! This writes the same fixtures as a text description of what the theme
//! resolved to — background, border, radius, shadow, text colour, size and
//! weight, per scene primitive, in scene order. It is adapter-independent by
//! construction (nothing is rasterised) and diffs line by line, so a token
//! change shows up as the roles it moved rather than as a pixel count.
//!
//! It does not replace the pixel suite. A semantic snapshot cannot see a
//! rasteriser bug, and the pixel suite cannot say *why* a colour moved. Phase
//! 0 keeps both.

use super::*;

/// Digits kept for a resolved length. Two is enough to catch a metrics change
/// and coarse enough that float noise cannot flip a baseline.
const PRECISION: usize = 2;

/// One fixture's resolved description, as the lines it contributes.
pub(super) fn describe_fixture(fixture: Fixture, runtime: &RuntimeEvidence) -> String {
    let world = runtime.document.context().world();
    let mut out = String::new();
    out.push_str(&format!("state: {}\n", fixture.state));
    match world.layout_box(runtime.target) {
        Some(bounds) => out.push_str(&format!(
            "  target: {}\n",
            rect(bounds.x, bounds.y, bounds.width, bounds.height)
        )),
        None => out.push_str("  target: none\n"),
    }
    match world.accessibility(runtime.target) {
        Some(state) => out.push_str(&format!(
            "  a11y: role={:?} disabled={} checked={:?} selected={:?} mixed={}\n",
            state.role, state.disabled, state.checked, state.selected, state.mixed,
        )),
        None => out.push_str("  a11y: none\n"),
    }
    for primitive in runtime.document.scene().primitives() {
        out.push_str(&format!("  {}\n", describe_primitive(primitive)));
    }
    out
}

fn describe_primitive(primitive: &nana_ui_scene::ScenePrimitive) -> String {
    let head = format!(
        "#{}/{} {} opacity={}",
        primitive.node.get(),
        primitive.id.slot,
        rect(
            primitive.bounds.x,
            primitive.bounds.y,
            primitive.bounds.width,
            primitive.bounds.height
        ),
        number(primitive.opacity),
    );
    match &primitive.kind {
        ScenePrimitiveKind::Quad {
            background,
            border_color,
            border_width,
            corner_radius,
            shadow,
            ..
        } => format!(
            "quad {head} bg={} border={} border_width={} radius={} shadow={}",
            color(*background),
            color(*border_color),
            number(*border_width),
            radius(*corner_radius),
            shadow.map_or_else(|| "none".to_owned(), |value| format!("{value:?}")),
        ),
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background,
            border_color,
            border_width,
            corner_radius,
            shadow,
            ..
        } => format!(
            "quad-batch {head} count={} bg={} border={} border_width={} radius={} shadow={}",
            bounds.len(),
            color(*background),
            color(*border_color),
            number(*border_width),
            radius(*corner_radius),
            shadow.map_or_else(|| "none".to_owned(), |value| format!("{value:?}")),
        ),
        ScenePrimitiveKind::QuadColorBatch {
            bounds,
            colors,
            border_color,
            border_width,
            corner_radius,
        } => format!(
            "quad-color-batch {head} count={} colors={} border={} border_width={} radius={}",
            bounds.len(),
            colors.len(),
            color(*border_color),
            number(*border_width),
            radius(*corner_radius),
        ),
        ScenePrimitiveKind::Text {
            color: text_color,
            size,
            weight,
            line_height,
            letter_spacing,
            italic,
            underline,
            line_through,
            spans,
            text_shadow,
            ..
        } => format!(
            "text {head} color={} size={} weight={} line_height={} tracking={} italic={} \
             underline={} line_through={} spans={} shadow={}",
            color(*text_color),
            number(*size),
            weight.map_or_else(|| "inherit".to_owned(), |value| value.to_string()),
            line_height.map_or_else(|| "normal".to_owned(), |value| format!("{value:?}")),
            number(*letter_spacing),
            italic,
            underline,
            line_through,
            spans.len(),
            text_shadow.is_some(),
        ),
        ScenePrimitiveKind::Icon {
            icon,
            color: icon_color,
        } => format!("icon {head} icon={icon:?} color={}", color(*icon_color)),
        ScenePrimitiveKind::IconBatch {
            bounds,
            icon,
            color: icon_color,
        } => format!(
            "icon-batch {head} count={} icon={icon:?} color={}",
            bounds.len(),
            color(*icon_color),
        ),
        // The phase is the animation clock, not a design value. Recording it
        // would make every re-run of a spinner fixture a baseline change.
        ScenePrimitiveKind::Spinner {
            color: spinner_color,
            ..
        } => format!("spinner {head} color={}", color(*spinner_color)),
        ScenePrimitiveKind::Stroke {
            points,
            width,
            color: stroke_color,
            cap,
            ..
        } => format!(
            "stroke {head} points={} width={} color={} cap={cap:?}",
            points.len(),
            number(*width),
            color(Some(*stroke_color)),
        ),
        // A painted node's triangles: their count and the theme colour they
        // were resolved to.
        ScenePrimitiveKind::Path { mesh, .. } => format!(
            "path {head} triangles={} color={}",
            mesh.indices.len() / 3,
            color(mesh.vertices.first().map(|vertex| vertex.color)),
        ),
        ScenePrimitiveKind::LayerBegin { opacity, blend, .. } => {
            format!(
                "layer-begin {head} opacity={} blend={blend:?}",
                number(*opacity)
            )
        }
        ScenePrimitiveKind::LayerEnd { mask } => format!(
            "layer-end {head} mask={}",
            mask.as_ref()
                .map_or_else(|| "none".to_owned(), |mask| format!("{:?}", mask.mode)),
        ),
        // A custom node's contents are the host's, not the theme's.
        ScenePrimitiveKind::Custom { .. } => format!("custom {head}"),
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> String {
    format!(
        "{},{} {}x{}",
        number(x),
        number(y),
        number(width),
        number(height)
    )
}

fn radius(corners: [f32; 4]) -> String {
    corners
        .iter()
        .map(|value| number(*value))
        .collect::<Vec<_>>()
        .join("/")
}

fn color(value: Option<[f32; 4]>) -> String {
    match value {
        None => "none".to_owned(),
        Some([r, g, b, a]) => format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            channel(r),
            channel(g),
            channel(b),
            channel(a)
        ),
    }
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `-0` and `0` are the same length; printing both would make a baseline
/// depend on which side of zero a layout arrived from.
fn number(value: f32) -> String {
    if !value.is_finite() {
        return "nan".to_owned();
    }
    let rounded = format!("{value:.PRECISION$}");
    if rounded
        .trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        format!("{:.PRECISION$}", 0.0)
    } else {
        rounded
    }
}
