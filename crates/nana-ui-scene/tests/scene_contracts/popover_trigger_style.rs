//! A popover trigger the application styles paints what it wrote over the
//! recipe, from `Popover::trigger_style` through the Runtime to the scene.

use nana_ui_core::{Icon, LengthSpec};
use nana_ui_runtime::{DocumentId, LayoutViewport, MeasureTextShaper, NodeStyle, Popover, Stack};
use nana_ui_scene::{PrimitiveId, RuntimeDocument, ScenePrimitiveKind};

/// The corners the trigger chrome of an icon popover styled `style` paints.
fn trigger_corners(style: Option<NodeStyle>) -> [f32; 4] {
    let id = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(id);
    let root = runtime
        .context_mut()
        .create_component(id, Stack::column(0.0))
        .unwrap();
    let mut popover = Popover::new().trigger_icon(Icon::Add, "More");
    if let Some(style) = style {
        popover = popover.trigger_style(style);
    }
    let popover = runtime.context_mut().create_component(id, popover).unwrap();
    runtime.context_mut().append_child(root, popover).unwrap();
    runtime
        .flush(LayoutViewport::new(320.0, 240.0), &mut MeasureTextShaper)
        .unwrap();
    let chrome = runtime
        .scene()
        .primitive(PrimitiveId {
            node: popover.stable_id(),
            slot: 1,
        })
        .expect("trigger chrome");
    match chrome.kind {
        ScenePrimitiveKind::Quad { corner_radius, .. } => corner_radius,
        ref other => panic!("trigger chrome is a quad, got {other:?}"),
    }
}

#[test]
fn a_trigger_style_radius_is_the_radius_the_trigger_paints() {
    let compact = trigger_corners(None);
    assert_eq!(compact, [nana_ui_core::UI_METRICS.radius_sm; 4]);

    let mut round = NodeStyle::default();
    {
        let layout = round.layout_mut();
        layout.width = Some(LengthSpec::Px(36.0));
        layout.height = Some(LengthSpec::Px(36.0));
        layout.border_radius = Some(18.0);
    }
    assert_eq!(trigger_corners(Some(round)), [18.0; 4]);

    // A style that writes no radius leaves the recipe's.
    let mut wide = NodeStyle::default();
    wide.layout_mut().width = Some(LengthSpec::Px(64.0));
    assert_eq!(trigger_corners(Some(wide)), compact);
}
