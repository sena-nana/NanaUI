#![cfg(feature = "runtime-agent")]
use nana_ui::runtime::{DocumentId, NodeStyle, RuntimeDocument, Stack, Text};
use nana_ui_core::{LayoutStyle, LengthSpec, PositionSpec, SemanticColorRole};
use nana_ui_devtools::agent::RuntimeAgentSession;
use nana_ui_devtools::offscreen;
use std::sync::Arc;

fn covered_text_pixels(text: &str) -> Vec<u8> {
    let mut document = RuntimeDocument::new(DocumentId::new(1).unwrap());
    let id = document.document();
    document
        .context_mut()
        .build(id, |ui| {
            let root = ui.leaf(
                Stack::column(0.0)
                    .width(LengthSpec::Px(256.0))
                    .height(LengthSpec::Px(128.0)),
            );
            ui.nest(root, |ui| {
                let layout = Arc::new(LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_top: Some(LengthSpec::Px(0.0)),
                    width: Some(LengthSpec::Px(256.0)),
                    height: Some(LengthSpec::Px(128.0)),
                    ..Default::default()
                });
                let mut label = Text::new(text);
                label.style.layout = layout.clone();
                ui.child("under", label);
                ui.child(
                    "opaque-cover",
                    Stack::column(0.0).style(NodeStyle {
                        layout,
                        background: Some(SemanticColorRole::Accent),
                        ..Default::default()
                    }),
                );
            });
        })
        .unwrap();
    RuntimeAgentSession::new(document, 256, 128)
        .unwrap()
        .screenshot_rgba()
        .unwrap()
        .1
}

#[test]
fn later_opaque_surface_occludes_earlier_text_pixels() {
    if !offscreen::pixels_available() {
        return;
    }
    let with_text = covered_text_pixels("MMMMMMMM\nMMMMMMMM");
    let without_text = covered_text_pixels("");
    assert_eq!(
        with_text, without_text,
        "text below an opaque surface cannot remain visible"
    );
}
