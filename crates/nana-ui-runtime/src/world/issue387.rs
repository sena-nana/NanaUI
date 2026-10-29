//! Issue #387: a triggered menu that opens and then closes keeps its items
//! laid out while its exit plays — the surface still shows, fading — and
//! drops them from its trigger's overflow once the exit finishes, as a cold
//! layout of the closed menu has none of them.
#![cfg(test)]

use std::time::Duration;

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
use crate::{
    ActionMenu, ActionMenuItem, AppContext, DocumentId, Entity, LayoutViewport, Popover,
    StableNodeId, Stack,
};
use nana_ui_core::LengthSpec;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(700.0, 600.0)
}

/// A popover (`kind` 0) or an action menu (1) with one item, open or not,
/// in a 420px row, laid out once.
fn page(kind: u8, open: bool) -> (AppContext, StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document, Stack::row(4.0).width(LengthSpec::Px(420.0)))
        .unwrap();
    let item = cx
        .create_detached_component(document, ActionMenuItem::new("Open"))
        .unwrap();
    let menu = if kind == 0 {
        let menu = cx
            .create_detached_component(document, Popover::new().trigger("Menu").open(open))
            .unwrap();
        cx.append_child(menu, item).unwrap();
        cx.append_child(root, menu).unwrap();
        menu.stable_id()
    } else {
        let mut view = ActionMenu::new();
        view.popover = view.popover.trigger("Menu").open(open);
        let menu = cx.create_detached_component(document, view).unwrap();
        cx.append_child(menu, item).unwrap();
        cx.append_child(root, menu).unwrap();
        menu.stable_id()
    };
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, menu)
}

fn set_open(cx: &mut AppContext, kind: u8, menu: StableNodeId, open: bool) {
    if kind == 0 {
        cx.update_component(Entity::<Popover>::from_stable_id(menu), |view, _| {
            view.open = open;
        })
        .unwrap();
    } else {
        cx.update_component(Entity::<ActionMenu>::from_stable_id(menu), |view, _| {
            view.popover.open = open;
        })
        .unwrap();
    }
}

/// Open, close, open, close. While the exit plays the items keep their
/// boxes in the trigger's overflow; the frame after it finishes lays out as
/// the closed menu laid out cold, the overflow the trigger's own box. Open
/// steps match a cold open menu.
#[test]
fn issue387_a_closed_menu_drops_its_items_from_the_overflow_once_it_exits() {
    let document = DocumentId::new(1).unwrap();
    let mut shaper = bundled_face_shaper();
    for kind in 0..2u8 {
        let (mut cx, menu) = page(kind, false);
        let mut now = Duration::ZERO;
        for open in [true, false, true, false] {
            set_open(&mut cx, kind, menu, open);
            product_frame(&mut cx, document, viewport(), &mut shaper);
            if !open {
                assert!(cx.world().surface_closing(menu), "kind {kind}");
                let result = cx.world().layout_result(menu).unwrap();
                assert_ne!(result.overflow, result.bounds, "kind {kind}");
                now += Duration::from_secs(10);
                cx.advance_animations(now);
                product_frame(&mut cx, document, viewport(), &mut shaper);
                assert!(!cx.world().surface_closing(menu), "kind {kind}");
                let result = cx.world().layout_result(menu).unwrap();
                assert_eq!(result.overflow, result.bounds, "kind {kind}");
            }
            let (mut cold, cold_menu) = page(kind, open);
            assert_eq!(cold_menu, menu);
            assert_matches_cold(&mut cx, &mut cold, document);
        }
    }
}
