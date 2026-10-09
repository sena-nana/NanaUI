//! A dialog's entrance as the scene hands it to a painter.
//!
//! The modal frame's node paints the scrim and the card, and its motion
//! layers are the card's: the card fades and moves on them, on the GPU,
//! about its own centre. The scrim takes none of that; it fades by its own
//! opacity, which carries its backdrop blur with it.

use std::time::Duration;

use nana_ui_core::motion::Easing;
use nana_ui_core::{DialogMotion, DialogRecipe, DialogTransition, LengthSpec, ThemeDefinition};
use nana_ui_runtime::{
    ComponentGeometry, ComputedStyle, Dialog, DocumentId, LayoutViewport, ModalSlots, OverlayHost,
    StableNodeId, Stack, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
};
use nana_ui_scene::{PrimitiveId, RuntimeDocument};

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

fn flush(runtime: &mut RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut NoText)
        .unwrap();
}

fn advance(runtime: &mut RuntimeDocument, millis: u64) {
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(millis));
    flush(runtime);
}

#[test]
fn a_dialogs_card_moves_on_its_layer_and_its_scrim_fades_on_its_own() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    cx.set_theme_definition(&ThemeDefinition::NANA_DARK.with_dialog(DialogRecipe {
        motion: DialogMotion {
            scrim: DialogTransition::new(160, Easing::Linear),
            card_fade: DialogTransition::new(160, Easing::Linear),
            card_move: DialogTransition::new(160, Easing::Linear),
            enter_offset_y: -8.0,
            enter_scale: 0.98,
        },
        ..DialogRecipe::DEFAULT
    }))
    .unwrap();
    let host = cx.create_component(document, OverlayHost::new()).unwrap();
    let dialog = cx
        .create_detached_component(document, Dialog::new("导出"))
        .unwrap();
    let body = cx
        .create_detached_component(document, Stack::column(0.0).height(LengthSpec::Px(40.0)))
        .unwrap();
    cx.set_modal_slots(
        dialog,
        ModalSlots {
            body: Some(body.stable_id()),
            ..Default::default()
        },
    )
    .unwrap();
    cx.append_child(host, dialog).unwrap();
    advance(&mut runtime, 1000);
    assert!(
        runtime
            .context_mut()
            .activate_overlay(host, dialog)
            .unwrap()
    );
    flush(&mut runtime);
    advance(&mut runtime, 1016);
    advance(&mut runtime, 1080);

    let id = dialog.stable_id();
    let Some(ComponentGeometry::ModalFrame {
        surface,
        scrim_opacity,
        ..
    }) = runtime.context().world().component_geometry(id)
    else {
        panic!("dialog geometry")
    };
    assert!((scrim_opacity - 0.5).abs() < 0.01, "{scrim_opacity}");
    let scene = runtime.scene();
    let draw = |slot| {
        scene
            .draw_primitive(PrimitiveId { node: id, slot })
            .unwrap_or_else(|| panic!("slot {slot}"))
    };

    let card = draw(11);
    let card_encode =
        scene.compositor_primitive_encode(card.primitive, card.transform, card.paint_opacity);
    let (card_transform, card_fade) = card_encode.motion_ids;
    assert!(
        card_transform != 0 && card_fade != 0,
        "the card fades and moves on the GPU: {:?}",
        card_encode.motion_ids
    );
    let centre = [
        surface.x + surface.width / 2.0,
        surface.y + surface.height / 2.0,
    ];
    assert!(
        (card_encode.transform_origin[0] - centre[0]).abs() < 1e-3
            && (card_encode.transform_origin[1] - centre[1]).abs() < 1e-3,
        "the card turns about its own centre {centre:?}, not {:?}",
        card_encode.transform_origin
    );

    let scrim = draw(10);
    assert!(
        (scrim.paint_opacity - scrim_opacity).abs() < 1e-3,
        "the scrim shows by its own opacity, not the card's: {}",
        scrim.paint_opacity
    );
    let scrim_encode =
        scene.compositor_primitive_encode(scrim.primitive, scrim.transform, scrim.paint_opacity);
    assert_eq!(
        scrim_encode.motion_ids.1, 0,
        "the card's fade does not reach the scrim"
    );
    assert!((scrim_encode.opacity - scrim_opacity).abs() < 1e-3);
}
