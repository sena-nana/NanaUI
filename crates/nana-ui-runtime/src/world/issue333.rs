//! Issue #333: a ListItem whose leading child is hidden lays out as if it
//! had no leading slot. The label starts at the padding and paints, and the
//! trailing slot stays at the row's end, before and after the child is shown
//! again, as a cold layout of the same tree does.
#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::LengthSpec;

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
use crate::{
    AppContext, ComponentGeometry, DocumentId, Entity, LayoutBox, LayoutPartKind, LayoutViewport,
    ListItem, ListItemSlots, Text,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 200.0)
}

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

struct Page {
    cx: AppContext,
    row: Entity<ListItem>,
    leading: Option<Entity<Text>>,
    trailing: Entity<Text>,
}

/// A 320px row with a trailing child and, when `leading` is given, a leading
/// child hidden or shown as it says, laid out once.
fn page(leading: Option<bool>) -> Page {
    let document = document();
    let mut cx = AppContext::new();
    let mut row = ListItem::new("Document");
    Arc::make_mut(&mut row.style.layout).width = Some(LengthSpec::Px(320.0));
    let row = cx.create_component(document, row).unwrap();
    let leading = leading.map(|hidden| {
        let mut text = Text::new("L");
        Arc::make_mut(&mut text.style.layout).hidden = hidden;
        cx.create_detached_component(document, text).unwrap()
    });
    let trailing = cx
        .create_detached_component(document, Text::new("T"))
        .unwrap();
    cx.set_list_item_slots(
        row,
        ListItemSlots {
            leading: leading.map(|leading| leading.stable_id()),
            content: None,
            trailing: Some(trailing.stable_id()),
        },
    )
    .unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    Page {
        cx,
        row,
        leading,
        trailing,
    }
}

fn set_hidden(page: &mut Page, hidden: bool) {
    page.cx
        .update_component(page.leading.unwrap(), |text, _| {
            Arc::make_mut(&mut text.style.layout).hidden = hidden
        })
        .unwrap();
    product_frame(
        &mut page.cx,
        document(),
        viewport(),
        &mut bundled_face_shaper(),
    );
}

fn label(page: &Page) -> LayoutBox {
    page.cx
        .world()
        .layout_result(page.row.stable_id())
        .unwrap()
        .parts
        .iter()
        .find(|part| part.kind == LayoutPartKind::TextContent)
        .unwrap()
        .bounds
}

fn trailing(page: &Page) -> LayoutBox {
    page.cx
        .world()
        .layout_box(page.trailing.stable_id())
        .unwrap()
}

/// The label region the row paints.
fn painted_label(page: &Page) -> Option<LayoutBox> {
    match page
        .cx
        .world()
        .component_geometry(page.row.stable_id())
        .unwrap()
    {
        ComponentGeometry::ListItem { content, .. } => content,
        _ => panic!("row"),
    }
}

/// The row with its leading child hidden, as it is laid out: like the same
/// row without a leading slot, the label from the padding and painted, the
/// trailing child at the end.
fn assert_lays_out_without_leading(row: &Page) {
    let reference = page(None);
    let content = row
        .cx
        .world()
        .layout_result(row.row.stable_id())
        .unwrap()
        .content_box;
    assert_eq!(label(row), label(&reference));
    assert_eq!(label(row).x, content.x);
    assert!(label(row).width > 0.0);
    assert_eq!(painted_label(row), Some(label(row)));
    assert_eq!(trailing(row), trailing(&reference));
    let end = trailing(row);
    assert!((end.x + end.width - (content.x + content.width)).abs() < 0.01);
}

#[test]
fn issue333_hidden_leading_child_lays_out_as_no_leading_slot() {
    let page = page(Some(true));
    assert_lays_out_without_leading(&page);
}

#[test]
fn issue333_hiding_and_showing_the_leading_child_matches_cold() {
    let mut page = page(Some(false));
    let shown_label = label(&page);
    let shown_trailing = trailing(&page);
    let leading = page
        .cx
        .world()
        .layout_box(page.leading.unwrap().stable_id())
        .unwrap();
    assert!(shown_label.x >= leading.x + leading.width);
    for hidden in [true, false, true] {
        set_hidden(&mut page, hidden);
        let mut cold = self::page(Some(hidden));
        assert_matches_cold(&mut page.cx, &mut cold.cx, document());
        assert_eq!(label(&page), label(&cold));
        if hidden {
            assert_lays_out_without_leading(&page);
        } else {
            assert_eq!(label(&page), shown_label);
            assert_eq!(trailing(&page), shown_trailing);
        }
    }
}
