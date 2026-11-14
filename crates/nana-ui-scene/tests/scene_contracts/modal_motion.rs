//! A dialog's entrance as the scene hands it to a painter.
//!
//! The modal frame's node paints the scrim and the card, and its motion
//! layers are the card's: the card moves on them about its own centre,
//! while its destination group fades the card and its content together.
//! The scrim takes none of that; it fades by its own
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
    // Allocating a compositor layer may wait 16ms; applying the entrance
    // cannot. The first painted frame must already agree with Runtime.
    for millis in [1000, 1008, 1016] {
        advance(&mut runtime, millis);
        let id = dialog.stable_id();
        let scrim_opacity = runtime.context().world().scrim_presence(id);
        let scene = runtime.scene();
        let card = scene
            .draw_primitive(PrimitiveId { node: id, slot: 11 })
            .unwrap();
        let scrim = scene
            .draw_primitive(PrimitiveId { node: id, slot: 10 })
            .unwrap();
        let opacity = (millis - 1000) as f32 / 160.0;
        let groups = scene.primitive_opacity_groups(card.primitive);
        assert_eq!(groups.len(), 1, "the card and its content fade together");
        assert_eq!(groups[0].node, id);
        assert!(
            (groups[0].opacity - opacity).abs() < 1e-3,
            "{millis}: {}",
            groups[0].opacity
        );
        assert_eq!(
            scene.opacity_groups(body.stable_id()).as_ref(),
            groups.as_ref(),
            "the body shares the card's fade group"
        );
        assert!(
            scene.primitive_opacity_groups(scrim.primitive).is_empty(),
            "the scrim stays outside the card's fade group"
        );
        assert_eq!(
            card.paint_opacity, 1.0,
            "no per-primitive fade inside the group"
        );
        assert!(
            (scrim.paint_opacity - scrim_opacity).abs() < 1e-3,
            "scrim must follow its independently sampled clock, not multiply the card fade"
        );
        let encode =
            scene.compositor_primitive_encode(card.primitive, card.transform, card.paint_opacity);
        if millis < 1016 {
            assert!(scene.compositor_layer(id).is_none());
            assert_eq!(
                encode.motion_ids,
                (0, 0),
                "CPU presentation before promotion"
            );
        } else {
            assert_ne!(encode.motion_ids.0, 0, "GPU transform after promotion");
        }
        assert_eq!(encode.motion_ids.1, 0, "the group owns the fade");
        assert_eq!(encode.opacity, 1.0, "the card must not fade twice");
        let expected = runtime
            .context()
            .world()
            .presentation_input_bounds(body.stable_id())
            .unwrap();
        let shown = scene.draw_node_bounds(body.stable_id()).unwrap();
        assert!(
            (shown.x - expected.x).abs() < 1e-3 && (shown.y - expected.y).abs() < 1e-3,
            "paint and input share the card pivot before and after promotion"
        );
    }
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
    assert_ne!(card_transform, 0, "the card moves on the GPU");
    assert_eq!(card_fade, 0, "the card's destination group owns the fade");
    let groups = scene.primitive_opacity_groups(card.primitive);
    assert_eq!(groups.len(), 1);
    assert!((groups[0].opacity - 0.5).abs() < 1e-3);
    assert_eq!(card_encode.opacity, 1.0);
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

#[test]
fn reduced_motion_has_no_first_frame_gap() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    cx.set_reduced_motion(true);
    let host = cx.create_component(document, OverlayHost::new()).unwrap();
    let dialog = cx
        .create_detached_component(document, Dialog::new("Export"))
        .unwrap();
    cx.append_child(host, dialog).unwrap();
    advance(&mut runtime, 1000);
    runtime
        .context_mut()
        .activate_overlay(host, dialog)
        .unwrap();
    advance(&mut runtime, 1000);
    let scene = runtime.scene();
    for slot in [10, 11] {
        let draw = scene
            .draw_primitive(PrimitiveId {
                node: dialog.stable_id(),
                slot,
            })
            .unwrap();
        assert_eq!(draw.paint_opacity, 1.0);
        let encode =
            scene.compositor_primitive_encode(draw.primitive, draw.transform, draw.paint_opacity);
        assert_eq!(encode.opacity, 1.0);
    }
    assert_eq!(scene.compositor_layer_count(), 0);
}
