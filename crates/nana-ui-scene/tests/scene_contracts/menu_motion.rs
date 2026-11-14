//! A triggered menu's entrance and exit as the scene hands them to a painter.
//!
//! The menu's node is its trigger, and its motion layers are the hanging
//! surface's: the surface and the items it places fade together and scale
//! about the surface's pivot, while the trigger and its trigger content keep
//! their logical opacity and transform.

use std::time::Duration;

use nana_ui_runtime::view::{button, entity_ref, row, text, widget, with_refs};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MeasureTextShaper, Popover, StableNodeId, Stack,
};
use nana_ui_scene::{AffineTransform, PrimitiveId, RuntimeDocument, SceneRect};

const SURFACE_SLOT: u64 = 0xFFFF_FFE0 << 32;

fn advance(runtime: &mut RuntimeDocument, millis: u64) {
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(millis));
    runtime
        .flush(LayoutViewport::new(640.0, 480.0), &mut MeasureTextShaper)
        .unwrap();
}

fn close(a: SceneRect, b: SceneRect) -> bool {
    (a.x - b.x).abs() < 1e-3
        && (a.y - b.y).abs() < 1e-3
        && (a.width - b.width).abs() < 1e-3
        && (a.height - b.height).abs() < 1e-3
}

#[test]
fn a_popovers_trigger_stays_put_while_its_surface_fades_and_scales() {
    let id = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(id);
    let root = runtime
        .context_mut()
        .create_component(id, Stack::column(0.0))
        .unwrap();
    let (_, (popover, mark, item)) = runtime
        .context_mut()
        .mount_view(root.stable_id(), || {
            let popover = entity_ref::<Popover>();
            let mark = entity_ref::<nana_ui_runtime::Text>();
            let item = entity_ref::<nana_ui_runtime::Button>();
            let view = widget(Popover::new().trigger("收藏"))
                .entity_ref(popover)
                .trigger(row().children(text("1800").entity_ref(mark)))
                .children(button("加入收藏夹").entity_ref(item));
            with_refs(view, (popover, mark, item))
        })
        .unwrap();
    advance(&mut runtime, 1000);
    let menu = popover.stable_id();
    let rest = |runtime: &RuntimeDocument, node: StableNodeId| {
        runtime.scene().draw_node_bounds(node).unwrap()
    };
    let trigger_at_rest = rest(&runtime, menu);
    let mark_at_rest = rest(&runtime, mark.stable_id());

    let check = |runtime: &RuntimeDocument, label: &str, opening: bool| {
        let scene = runtime.scene();
        let mut moving = None;
        let mut fading = None;
        for primitive in scene.primitives() {
            let draw = scene.draw_primitive(primitive.id).unwrap();
            let groups = scene.primitive_opacity_groups(draw.primitive);
            let encode = scene.compositor_primitive_encode(
                draw.primitive,
                draw.transform,
                draw.paint_opacity,
            );
            let trigger = primitive.node == mark.stable_id()
                || (primitive.node == menu && primitive.id.slot != SURFACE_SLOT);
            if trigger {
                assert!(
                    groups.is_empty(),
                    "{label}: {:?} fades with the surface",
                    primitive.id
                );
                assert_eq!(draw.paint_opacity, 1.0, "{label}: {:?}", primitive.id);
                assert_eq!(encode.opacity, 1.0, "{label}: {:?}", primitive.id);
                assert_eq!(encode.motion_ids, (0, 0), "{label}: {:?}", primitive.id);
                assert_eq!(
                    draw.transform,
                    AffineTransform::IDENTITY,
                    "{label}: {:?} moves with the surface",
                    primitive.id
                );
            } else {
                // The surface and every item share one fade and one move.
                assert_eq!(groups.len(), 1, "{label}: {:?}", primitive.id);
                assert_eq!(groups[0].node, menu);
                let opacity = *fading.get_or_insert(groups[0].opacity);
                assert_eq!(groups[0].opacity, opacity, "{label}: {:?}", primitive.id);
                let transform = *moving.get_or_insert(draw.transform);
                assert_eq!(draw.transform, transform, "{label}: {:?}", primitive.id);
                assert_eq!(encode.motion_ids.0, 0, "{label}: {:?}", primitive.id);
            }
        }
        let opacity = fading.expect("the surface paints");
        assert!(
            opacity > 0.0 && opacity < 1.0,
            "{label}: mid-fade, not {opacity}"
        );
        let transform = moving.unwrap();
        assert!(
            transform.0[0] > 0.9 && transform.0[0] < 1.0,
            "{label}: mid-scale, not {:?}",
            transform.0
        );
        assert!(close(rest(runtime, menu), trigger_at_rest), "{label}");
        assert!(
            close(rest(runtime, mark.stable_id()), mark_at_rest),
            "{label}"
        );
        let world = runtime.context().world();
        for node in [menu, mark.stable_id(), item.stable_id()] {
            let input = world.presentation_input_bounds(node).unwrap();
            let shown = scene.draw_node_bounds(node).unwrap();
            assert!(
                close(
                    shown,
                    SceneRect {
                        x: input.x,
                        y: input.y,
                        width: input.width,
                        height: input.height,
                    }
                ),
                "{label}: paint {shown:?} and input {input:?} disagree on {node:?}"
            );
        }
        let surface = scene
            .draw_primitive(PrimitiveId {
                node: menu,
                slot: SURFACE_SLOT,
            })
            .unwrap();
        if !opening {
            return;
        }
        // Inside the surface's padding, clear of the items.
        let [px, py] = [
            surface.primitive.bounds.x + 2.0,
            surface.primitive.bounds.y + 2.0,
        ];
        let [a, b, c, d, e, f] = transform.0;
        let (x, y) = (a * px + c * py + e, b * px + d * py + f);
        assert_eq!(
            world.hit_test(id, x, y),
            Some(menu),
            "{label}: the scaled surface takes the pointer where it paints"
        );
    };

    runtime
        .context_mut()
        .update_component(popover, |popover, _| popover.open = true)
        .unwrap();
    // Before the compositor layer is promoted, and after.
    advance(&mut runtime, 1008);
    check(&runtime, "opening, pending promotion", true);
    advance(&mut runtime, 1030);
    check(&runtime, "opening, promoted", true);
    advance(&mut runtime, 2000);
    runtime
        .context_mut()
        .update_component(popover, |popover, _| popover.open = false)
        .unwrap();
    advance(&mut runtime, 2030);
    check(&runtime, "closing", false);
}
