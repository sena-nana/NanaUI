//! The installed theme's corner shape reaches the scene a painter paints.
//!
//! Each document carries the shape of its own theme, so two documents (two
//! windows, or an offscreen session beside a window) paint their own; a
//! shape change is a new scene identity, and an idle frame is not.

use nana_ui_core::{CornerShape, ThemeAppearance, ThemeDefinition};
use nana_ui_runtime::{
    ComputedStyle, DocumentId, LayoutViewport, StableNodeId, Stack, TextContent, TextMetrics,
    TextShapeConstraints, TextShaper,
};
use nana_ui_scene::RuntimeDocument;

struct NoText;

impl TextShaper for NoText {
    fn shape(
        &mut self,
        _id: StableNodeId,
        _text: &TextContent,
        _style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics::default()
    }
}

fn document() -> RuntimeDocument {
    let id = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(id);
    runtime
        .context_mut()
        .create_component(id, Stack::column(0.0).radius_px(8.0))
        .unwrap();
    runtime
}

fn flush(runtime: &mut RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(200.0, 100.0), &mut NoText)
        .unwrap();
}

#[test]
fn each_document_paints_the_corner_shape_of_its_own_theme() {
    let mut round = document();
    let mut squircle = document();
    flush(&mut round);
    flush(&mut squircle);
    assert_eq!(squircle.scene().corner_shape(), CornerShape::Round);

    let before = squircle.scene().instance_id();
    squircle
        .context_mut()
        .set_theme_definition(
            &ThemeDefinition::for_appearance(ThemeAppearance::Dark)
                .with_corner_shape(CornerShape::SQUIRCLE),
        )
        .unwrap();
    flush(&mut squircle);
    assert_eq!(squircle.scene().corner_shape(), CornerShape::SQUIRCLE);
    assert_ne!(squircle.scene().instance_id(), before);
    assert_eq!(round.scene().corner_shape(), CornerShape::Round);

    let settled = squircle.scene().instance_id();
    flush(&mut squircle);
    assert_eq!(squircle.scene().instance_id(), settled, "an idle frame");

    squircle
        .context_mut()
        .set_preset_theme(ThemeAppearance::Dark)
        .unwrap();
    flush(&mut squircle);
    assert_eq!(squircle.scene().corner_shape(), CornerShape::Round);
}
