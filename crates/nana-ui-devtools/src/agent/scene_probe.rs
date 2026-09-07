//! "Why is this element not on screen?" answered from Scene state.
//!
//! Clipped, occluded, transparent, zero-sized and off-viewport all look
//! identical in a PNG. The Scene already distinguishes them; this module reads
//! that out instead of leaving the caller to guess from pixels.

use nana_ui::runtime::{LayoutBox, StableNodeId};
use nana_ui_scene::{ScenePrimitive, SceneRect, UiScene};

use super::AccessibilityDumpNode;
use super::protocol::{HitDump, RectDump, SceneProbeDump};

/// Map hit-test candidates (topmost first) onto their accessibility identity.
pub fn hits(nodes: &[AccessibilityDumpNode], candidates: &[StableNodeId]) -> Vec<HitDump> {
    candidates
        .iter()
        .map(|candidate| {
            let id = candidate.get();
            match nodes.iter().find(|node| node.id == id) {
                Some(node) => HitDump {
                    node: id,
                    role: node.role.clone(),
                    label: node.label.clone(),
                    agent_id: node.agent_id.clone(),
                },
                // A node under the pointer that the accessibility projection
                // does not carry is still the reason a click missed, so it must
                // stay in the list rather than be filtered out.
                None => HitDump {
                    node: id,
                    role: "unprojected".into(),
                    label: None,
                    agent_id: None,
                },
            }
        })
        .collect()
}

pub fn probe(
    id: StableNodeId,
    scene: &UiScene,
    layout: Option<LayoutBox>,
    viewport_width: f32,
    viewport_height: f32,
    hit_at: impl Fn(f32, f32) -> Vec<HitDump>,
) -> Option<SceneProbeDump> {
    let draw = scene.node_bounds(id);
    let primitives: Vec<_> = scene
        .primitives()
        .filter(|primitive| primitive.node == id)
        .collect();
    if draw.is_none() && layout.is_none() && primitives.is_empty() {
        return None;
    }

    // Ancestor opacity groups composite the subtree as a layer, so a node can
    // be fully opaque itself and still invisible.
    let group_opacity: f32 = scene
        .opacity_groups(id)
        .iter()
        .map(|group| group.opacity)
        .product();
    let own_opacity = primitives
        .iter()
        .map(|primitive| primitive.opacity)
        .fold(0.0_f32, f32::max);
    let effective_opacity = if primitives.is_empty() {
        group_opacity
    } else {
        group_opacity * own_opacity
    };

    let clips: Vec<RectDump> = primitives
        .first()
        .map(|primitive| {
            primitive
                .clips
                .iter()
                .map(|clip| rect(clip.bounds))
                .collect()
        })
        .unwrap_or_default();

    let draw_rect = draw.map(rect);
    let in_viewport = draw_rect.is_some_and(|bounds| {
        bounds.width > 0.0
            && bounds.height > 0.0
            && bounds.x < viewport_width
            && bounds.y < viewport_height
            && bounds.x + bounds.width > 0.0
            && bounds.y + bounds.height > 0.0
    });

    let occluded_by = draw_rect
        .filter(|bounds| in_viewport && bounds.width > 0.0 && bounds.height > 0.0)
        .and_then(|bounds| {
            hit_at(
                bounds.x + bounds.width * 0.5,
                bounds.y + bounds.height * 0.5,
            )
            .into_iter()
            .next()
        })
        .filter(|hit| hit.node != id.get());

    let verdict = verdict(
        &primitives,
        draw_rect,
        in_viewport,
        effective_opacity,
        occluded_by.is_some(),
    );

    Some(SceneProbeDump {
        node: id.get(),
        layout_bounds: layout.map(|value| RectDump {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }),
        draw_bounds: draw_rect,
        in_viewport,
        primitive_count: primitives.len(),
        effective_opacity,
        clips,
        occluded_by,
        verdict,
    })
}

fn verdict(
    primitives: &[&ScenePrimitive],
    draw: Option<RectDump>,
    in_viewport: bool,
    opacity: f32,
    occluded: bool,
) -> String {
    if primitives.is_empty() {
        return "not_painted".into();
    }
    match draw {
        Some(bounds) if bounds.width <= 0.0 || bounds.height <= 0.0 => "zero_size".into(),
        None => "no_bounds".into(),
        Some(_) if !in_viewport => "off_viewport".into(),
        Some(_) if opacity <= 0.0 => "zero_opacity".into(),
        Some(_) if occluded => "occluded".into(),
        Some(_) => "painted".into(),
    }
}

fn rect(bounds: SceneRect) -> RectDump {
    RectDump {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}
