//! Issue #386: a node with no box (`hidden` / `display: none`, or inside
//! such a node) publishes no `LayoutResult`, however its ancestors were laid
//! out before, as a cold layout of the same tree publishes none.
#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{DisplaySpec, FlexDirection, LayoutStyle, ThemeDefinition};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(600.0, 400.0)
}

fn row(display: DisplaySpec) -> LayoutStyle {
    LayoutStyle {
        display: Some(display),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    }
}

fn hidden(hidden: bool) -> LayoutStyle {
    LayoutStyle {
        hidden,
        ..LayoutStyle::default()
    }
}

/// A row holding a visible text, a hidden text and a hidden box with a text
/// inside, under `theme` when given, laid out once. Returns the row and the
/// hidden text, the hidden box and the text in it.
fn page(
    display: DisplaySpec,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, StableNodeId, [StableNodeId; 3]) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let (mut b, page) = Builder::page(document, 1, 400.0);
    let row = b.element(page, row(display));
    b.label(row, "Shown");
    let text = b.label(row, "Hidden");
    b.queue.set_style(text, styled(hidden(true)));
    let boxed = b.element(row, hidden(true));
    let inner = b.label(boxed, "Inside");
    cx.commit_mutations(b.queue).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, row, [text, boxed, inner])
}

fn published(cx: &AppContext, ids: [StableNodeId; 3]) -> [bool; 3] {
    ids.map(|id| cx.world().layout_result(id).is_some())
}

/// A theme switch relays the row out; the hidden nodes still have no result.
#[test]
fn issue386_hidden_nodes_publish_nothing_after_a_theme_switch() {
    let document = DocumentId::new(1).unwrap();
    let theme = ThemeDefinition::NANA_DARK;
    let (mut cx, _, ids) = page(DisplaySpec::Flex, None);
    assert_eq!(published(&cx, ids), [false; 3]);
    cx.set_theme_definition(&theme).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    assert_eq!(published(&cx, ids), [false; 3]);
    let (mut cold, ..) = page(DisplaySpec::Flex, Some(&theme));
    assert_matches_cold(&mut cx, &mut cold, document);
}

/// The row switching between Flex and Grid keeps the hidden nodes out of
/// the published results.
#[test]
fn issue386_hidden_nodes_publish_nothing_across_context_switches() {
    let document = DocumentId::new(1).unwrap();
    let (mut cx, row, ids) = page(DisplaySpec::Flex, None);
    for display in [DisplaySpec::Grid, DisplaySpec::Flex] {
        let mut style = cx.world().node_style(row).unwrap().clone();
        Arc::make_mut(&mut style.layout).display = Some(display);
        let mut q = MutationQueue::new();
        q.set_style(row, style);
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        assert_eq!(published(&cx, ids), [false; 3], "{display:?}");
        let (mut cold, ..) = page(display, None);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

/// A text hidden after it was laid out drops its result, and gets one back
/// when shown again, as a cold layout does.
#[test]
fn issue386_hiding_a_laid_out_text_retracts_its_result() {
    let document = DocumentId::new(1).unwrap();
    let (mut cx, _, [text, ..]) = page(DisplaySpec::Flex, None);
    for shown in [true, false] {
        let mut q = MutationQueue::new();
        q.set_style(text, styled(hidden(!shown)));
        cx.commit_mutations(q).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        assert_eq!(cx.world().layout_result(text).is_some(), shown);
    }
    let (mut cold, ..) = page(DisplaySpec::Flex, None);
    assert_matches_cold(&mut cx, &mut cold, document);
}

/// A closed menu's items lay out with no box although their own style
/// keeps one: after a theme switch they publish no result either.
#[test]
fn issue386_a_closed_menus_items_publish_nothing_after_a_theme_switch() {
    use crate::{ActionMenuItem, Popover, Stack};
    let document = DocumentId::new(1).unwrap();
    let theme = ThemeDefinition::NANA_DARK;
    let build = |theme: Option<&ThemeDefinition>| {
        let mut cx = AppContext::new();
        if let Some(theme) = theme {
            cx.set_theme_definition(theme).unwrap();
        }
        let root = cx.create_component(document, Stack::row(4.0)).unwrap();
        let menu = cx
            .create_detached_component(document, Popover::new().trigger("Menu"))
            .unwrap();
        let item = cx
            .create_detached_component(document, ActionMenuItem::new("Open"))
            .unwrap();
        cx.append_child(menu, item).unwrap();
        cx.append_child(root, menu).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        (cx, item.stable_id())
    };
    let (mut cx, item) = build(None);
    assert!(cx.world().layout_result(item).is_none());
    cx.set_theme_definition(&theme).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    assert!(cx.world().layout_result(item).is_none());
    let (mut cold, _) = build(Some(&theme));
    assert_matches_cold(&mut cx, &mut cold, document);
}
