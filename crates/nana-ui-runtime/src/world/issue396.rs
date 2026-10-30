//! Issue #396: a ListItem keeps its side slots in first-line bands exactly
//! while it puts them on its first line, whichever way the slots came: a
//! binding that moves `trailing_align` or `auto_height` after mount, a write
//! after the slots were given, and slots given through
//! `AppContext::set_list_item_slots`. A theme switch moves the band. Each
//! step lays out what a cold layout of the same row does.
#![cfg(test)]

use std::cell::Cell;
use std::sync::Arc;

use nana_ui_core::{ControlSize, LengthSpec, ThemeAppearance, ThemeDefinition};

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
use crate::view::{Signal, column, signal, widget};
use crate::{
    AppContext, DocumentId, Entity, LayoutBox, LayoutViewport, ListItem, ListItemSlots,
    StableNodeId, Stack, TrailingAlign,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 300.0)
}

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn fixed(width: Option<f32>, height: f32) -> Stack {
    Stack::column(0.0).with_layout(|l| {
        l.width = Some(width.map_or(LengthSpec::Fill, LengthSpec::Px));
        l.height = Some(LengthSpec::Px(height));
        l.flex_shrink = Some(0.0);
    })
}

fn line(cx: &AppContext) -> f32 {
    cx.world()
        .theme()
        .typography()
        .line_height(nana_ui_core::LineRole::Line)
}

fn rect(cx: &AppContext, id: StableNodeId) -> LayoutBox {
    cx.world().layout_box(id).unwrap()
}

fn middle(r: LayoutBox) -> f32 {
    r.y + r.height / 2.0
}

fn children(cx: &AppContext, id: StableNodeId) -> Vec<StableNodeId> {
    cx.world().child_ids(id).to_vec()
}

/// The row's slots: its leading, its two-line content and its trailing.
struct Row {
    row: StableNodeId,
    leading: StableNodeId,
    content: StableNodeId,
    trailing: StableNodeId,
}

/// A side slot sits in a band, a child of the row, exactly while the row
/// puts its slots on its first line; there it centres on the content's
/// first line, and otherwise on the row.
fn assert_slots_placed(cx: &AppContext, row: &Row, first_line: bool) {
    let world = cx.world();
    let name = rect(cx, children(cx, row.content)[0]);
    let whole = rect(cx, row.row);
    for slot in [row.leading, row.trailing] {
        let parent = world.parent_id(slot).unwrap();
        let banded = parent != row.row;
        assert_eq!(banded, first_line, "{slot:?} in a band");
        if banded {
            assert_eq!(world.parent_id(parent), Some(row.row));
            assert!(crate::view_components::is_list_item_band(world, parent));
        }
        let at = rect(cx, slot);
        let target = if first_line { name } else { whole };
        assert!(
            (middle(at) - middle(target)).abs() < 0.5,
            "{slot:?} {at:?} on {target:?}"
        );
    }
    assert_eq!(
        children(cx, row.row).len(),
        3,
        "leading, content and trailing"
    );
    assert!(
        children(cx, row.row).contains(&row.content),
        "the content slot stays the row's child"
    );
}

/// A page built as a view: one 320px row whose alignment and `auto_height`
/// are bound, with a 20px leading slot, two lines of content and a 28px
/// trailing slot, laid out once.
fn bound_page(
    align: Option<TrailingAlign>,
    auto: bool,
) -> (AppContext, Row, Signal<Option<TrailingAlign>>, Signal<bool>) {
    let mut cx = AppContext::new();
    let root = cx
        .create_component(document(), Stack::column(0.0))
        .unwrap()
        .stable_id();
    let line = line(&cx);
    let signals = Cell::new(None);
    let mut item = ListItem::new("");
    Arc::make_mut(&mut item.style.layout).width = Some(LengthSpec::Px(320.0));
    let view = cx
        .mount_view(root, || {
            let (align, auto) = (signal(align), signal(auto));
            signals.set(Some((align, auto)));
            column().children(
                widget(item)
                    .bind(move |item: &mut ListItem| {
                        item.trailing_align = align.get();
                        item.auto_height = auto.get();
                    })
                    .leading(widget(fixed(Some(20.0), 20.0)).key("leading"))
                    .content(
                        column()
                            .children((widget(fixed(None, line)), widget(fixed(None, line))))
                            .key("content"),
                    )
                    .trailing(widget(fixed(Some(28.0), 28.0)).key("trailing"))
                    .key("row"),
            )
        })
        .unwrap();
    let base = view.roots()[0];
    let path = |path: &str| cx.resolve_assembly_path(base, path).unwrap();
    let row = Row {
        row: path("row"),
        leading: path("row/leading"),
        content: path("row/content"),
        trailing: path("row/trailing"),
    };
    let (align, auto) = signals.get().unwrap();
    product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
    (cx, row, align, auto)
}

/// A row centred at mount and then put on its first line by its bound
/// alignment, and back, lays its slots out in their bands and out of them as
/// a row mounted that way does.
#[test]
fn issue396_a_bound_alignment_moves_the_slots_into_their_bands_like_a_cold_layout() {
    let mut shaper = bundled_face_shaper();
    let (mut cx, row, align, _) = bound_page(None, false);
    assert_slots_placed(&cx, &row, false);
    for value in [Some(TrailingAlign::FirstLine), None] {
        align.set(value);
        cx.flush_reactive().unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, cold_row, ..) = bound_page(value, false);
        assert_eq!(cold_row.row, row.row);
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_slots_placed(&cx, &row, value.is_some());
    }
}

/// `auto_height` bound after mount puts the slots on the first line and
/// takes them off it again, as an explicit alignment does, unless the row
/// names its alignment.
#[test]
fn issue396_a_bound_auto_height_moves_the_slots_into_their_bands_like_a_cold_layout() {
    let mut shaper = bundled_face_shaper();
    let (mut cx, row, align, auto) = bound_page(None, false);
    for (named, value) in [
        (None, true),
        (None, false),
        (Some(TrailingAlign::Center), true),
    ] {
        align.set(named);
        auto.set(value);
        cx.flush_reactive().unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, ..) = bound_page(named, value);
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_slots_placed(&cx, &row, named.is_none() && value);
    }
}

/// A 320px row whose slots come through `set_list_item_slots` (a 20px
/// leading slot, two lines of content and a 24px trailing slot), aligned as
/// `align` says when it is created, under `theme` when given, laid out once.
fn slots_page(
    align: Option<TrailingAlign>,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, Entity<ListItem>, Row) {
    sized_slots_page(align, None, theme)
}

/// [`slots_page`] with a row of `size` when given.
fn sized_slots_page(
    align: Option<TrailingAlign>,
    size: Option<ControlSize>,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, Entity<ListItem>, Row) {
    let document = document();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let line = line(&cx);
    let mut item = ListItem::new("");
    if let Some(size) = size {
        item = item.size(size);
    }
    item.trailing_align = align;
    Arc::make_mut(&mut item.style.layout).width = Some(LengthSpec::Px(320.0));
    let item = cx.create_component(document, item).unwrap();
    let detached =
        |cx: &mut AppContext, stack: Stack| cx.create_detached_component(document, stack).unwrap();
    let leading = detached(&mut cx, fixed(Some(20.0), 20.0));
    let content = detached(&mut cx, Stack::column(0.0).width(LengthSpec::Fill));
    for _ in 0..2 {
        let line = detached(&mut cx, fixed(None, line));
        cx.append_child(content, line).unwrap();
    }
    // Short enough for a compact one-line row's band.
    let trailing = detached(&mut cx, fixed(Some(24.0), 24.0));
    cx.set_list_item_slots(
        item,
        ListItemSlots {
            leading: Some(leading.stable_id()),
            content: Some(content.stable_id()),
            trailing: Some(trailing.stable_id()),
        },
    )
    .unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    let row = Row {
        row: item.stable_id(),
        leading: leading.stable_id(),
        content: content.stable_id(),
        trailing: trailing.stable_id(),
    };
    (cx, item, row)
}

/// Slots given through `set_list_item_slots` go into their bands, whether
/// the row was put on its first line before they were given or after, and
/// out of them again; giving the same slots again changes nothing.
#[test]
fn issue396_slots_set_by_hand_get_their_bands_like_a_cold_layout() {
    let mut shaper = bundled_face_shaper();
    let (cold, _, cold_row) = slots_page(Some(TrailingAlign::FirstLine), None);
    assert_slots_placed(&cold, &cold_row, true);

    let (mut cx, item, row) = slots_page(None, None);
    assert_slots_placed(&cx, &row, false);
    for align in [Some(TrailingAlign::FirstLine), Some(TrailingAlign::Center)] {
        cx.update_component(item, |item, _| item.trailing_align = align)
            .unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, ..) = slots_page(align, None);
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_slots_placed(&cx, &row, align == Some(TrailingAlign::FirstLine));
    }
    cx.update_component(item, |item, _| {
        item.trailing_align = Some(TrailingAlign::FirstLine)
    })
    .unwrap();
    let slots = ListItemSlots {
        leading: Some(row.leading),
        content: Some(row.content),
        trailing: Some(row.trailing),
    };
    assert!(!cx.set_list_item_slots(item, slots).unwrap());
    // The sides swap: each slot keeps its band, which moves with it.
    let bands = [row.leading, row.trailing].map(|slot| cx.world().parent_id(slot).unwrap());
    let swapped = ListItemSlots {
        leading: Some(row.trailing),
        content: Some(row.content),
        trailing: Some(row.leading),
    };
    assert!(cx.set_list_item_slots(item, swapped).unwrap());
    assert_eq!(
        children(&cx, row.row),
        vec![bands[1], row.content, bands[0]]
    );
}

/// A theme switch moves a first-line row's band with the row's control
/// height: the band stays as tall as a one-line row, and the slots stay on
/// the first line, as a row laid out under that theme from the start does.
#[test]
fn issue396_a_theme_switch_moves_the_band_like_a_cold_layout() {
    let compact = ThemeDefinition::for_appearance(ThemeAppearance::Light)
        .with_metrics(nana_ui_core::COMPACT_METRICS);
    let mut shaper = bundled_face_shaper();
    let (mut cx, _, row) = slots_page(Some(TrailingAlign::FirstLine), None);
    let band = cx.world().parent_id(row.trailing).unwrap();
    let before = rect(&cx, band);
    cx.set_theme_definition(&compact).unwrap();
    product_frame(&mut cx, document(), viewport(), &mut shaper);
    let (mut cold, ..) = slots_page(Some(TrailingAlign::FirstLine), Some(&compact));
    assert_matches_cold(&mut cx, &mut cold, document());
    assert_slots_placed(&cx, &row, true);
    let after = rect(&cx, band);
    let one_line = nana_ui_core::ControlSize::Large.height_in(nana_ui_core::COMPACT_METRICS);
    assert_eq!(after.height, one_line, "as tall as a compact one-line row");
    assert!(after.height < before.height, "{before:?} -> {after:?}");
}

/// A first-line row whose size changes after its slots are in their bands
/// projects its bands again: they take the new one-line height, as a row of
/// that size laid out from the start does.
#[test]
fn issue396_a_band_follows_the_rows_size_like_a_cold_layout() {
    let mut shaper = bundled_face_shaper();
    let (mut cx, item, row) = slots_page(Some(TrailingAlign::FirstLine), None);
    let band = cx.world().parent_id(row.trailing).unwrap();
    for size in [ControlSize::Medium, ControlSize::Large, ControlSize::Small] {
        cx.update_component(item, |item, _| *item = item.clone().size(size))
            .unwrap();
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let (mut cold, ..) = sized_slots_page(Some(TrailingAlign::FirstLine), Some(size), None);
        assert_matches_cold(&mut cx, &mut cold, document());
        assert_eq!(cx.world().parent_id(row.trailing), Some(band));
        assert_eq!(
            rect(&cx, band).height,
            size.height_in(nana_ui_core::UI_METRICS),
            "{size:?}: as tall as a one-line row"
        );
    }
}
