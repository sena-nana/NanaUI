//! A word a host shaper measures wider than its box, in a column mounted
//! under a narrow fixed-width row (`perf/micro/stack-depth insert`).
//!
//! The column is a flex item of the row, so it is as wide as the word. The
//! host shaper says nothing about overflow: before this held, the text asked
//! layout for its box's width until it was shaped in that box and for its own
//! width after, with the same metrics, so no seed reached layout. The frame
//! kept the narrower box; a later pass widened it where it measured the text
//! again and kept it where a retained measurement answered, and which one
//! answered could follow the iteration order of the pass's hashed sets.
#![cfg(test)]

use nana_ui_core::{LayoutStyle, LengthSpec};

use super::{DocumentId, StableNodeId};
use crate::view::{AnyView, IntoView, text, widget};
use crate::{
    AppContext, ComputedStyle, LayoutBox, LayoutViewport, Stack, TextContent, TextMetrics,
    TextShapeConstraints, TextShaper,
};

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(1280.0, 800.0)
}

/// Eight px a byte, whatever the constraints: a word that cannot break.
struct FixedShaper;

impl TextShaper for FixedShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        _style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics {
            width: text.value.len() as f32 * 8.0,
            height: 18.0,
            ascent: None,
        }
    }
}

/// Eight px a character, wrapped greedily at spaces to the box it is given:
/// a text that can break, down to its widest word.
struct WrapShaper;

impl TextShaper for WrapShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        _style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let limit = constraints
            .max_width
            .filter(|_| constraints.wrap)
            .unwrap_or(f32::INFINITY);
        let (mut lines, mut line, mut widest) = (1.0, 0.0f32, 0.0f32);
        for word in text.value.split(' ') {
            let word = word.len() as f32 * 8.0;
            if line > 0.0 && line + 8.0 + word > limit {
                lines += 1.0;
                line = word;
            } else if line > 0.0 {
                line += 8.0 + word;
            } else {
                line = word;
            }
            widest = widest.max(line);
        }
        TextMetrics {
            width: widest,
            height: 18.0 * lines,
            ascent: None,
        }
    }
}

/// A column padded `pad` on every side.
fn column(pad: f32) -> Stack {
    Stack::column(0.0).with_layout(move |layout| {
        *layout = LayoutStyle {
            padding: Some(LengthSpec::Px(pad)),
            ..Default::default()
        }
    })
}

/// One frame, as `RuntimeDocument::flush` runs it; the first lays the
/// document out for the viewport.
fn flush(cx: &mut AppContext, first: bool) {
    flush_with(cx, first, &mut FixedShaper);
}

fn flush_with(cx: &mut AppContext, first: bool, shaper: &mut impl TextShaper) {
    cx.world_mut().observe_text_shaper(shaper);
    let mut force = first;
    loop {
        let work = cx.take_system_work_for(document());
        if work.is_empty() && !force {
            break;
        }
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        let mut seeds = work.layout_frontier_seeds.clone();
        seeds.extend(cx.take_layout_frontier_seeds(document()));
        if force {
            cx.layout_document_for_viewport(document(), viewport(), &seeds)
                .unwrap();
            force = false;
        } else if !seeds.is_empty() {
            cx.layout_document_with_frontier(document(), viewport(), &seeds)
                .unwrap();
        } else {
            continue;
        }
        let mut scope = cx.take_last_layout_scope();
        for _ in 0..4 {
            if !cx.shape_text_for_layout_scoped(&scope, shaper).unwrap() {
                break;
            }
            let seeds = cx.take_layout_frontier_seeds(document());
            if seeds.is_empty() {
                break;
            }
            cx.layout_document_with_frontier(document(), viewport(), &seeds)
                .unwrap();
            scope = cx.take_last_layout_scope();
        }
    }
}

/// A laid-out page with a row of one padded cell holding a row 24 px wide
/// (`min-height` 16 px), the bench's target leaf. Returns the leaf.
fn page(cx: &mut AppContext) -> StableNodeId {
    let leaf = leaf_row(cx).stable_id();
    flush(cx, true);
    leaf
}

/// The row of [`page`], 24 px wide.
fn narrow() -> Stack {
    Stack::row(0.0)
        .width(LengthSpec::Px(24.0))
        .min_height(LengthSpec::Px(16.0))
}

/// [`page`], not laid out yet.
fn leaf_row(cx: &mut AppContext) -> crate::Entity<Stack> {
    let root = cx
        .create_component(document(), Stack::column(0.0).width(LengthSpec::Px(1280.0)))
        .unwrap();
    let row = cx
        .create_detached_component(document(), Stack::row(4.0))
        .unwrap();
    cx.append_child(root, row).unwrap();
    let cell = cx
        .create_detached_component(document(), column(0.25))
        .unwrap();
    cx.append_child(row, cell).unwrap();
    let leaf = cx.create_detached_component(document(), narrow()).unwrap();
    cx.append_child(cell, leaf).unwrap();
    leaf
}

/// A chain `depth` columns deep, `pad` padded, over the word "leaf" (32 px).
fn chain(depth: usize, pad: f32) -> AnyView {
    let mut view = text("leaf").into_any();
    for _ in 0..depth {
        view = widget(column(pad)).children((view,)).into_any();
    }
    view
}

#[test]
fn a_word_wider_than_its_row_widens_the_column_in_the_frame_it_mounts() {
    let mut cx = AppContext::new();
    let leaf = page(&mut cx);
    let made = cx.mount_view(leaf, || chain(1, 0.25)).unwrap();
    flush(&mut cx, false);
    // The word and the column's padding: its automatic minimum width.
    let held = made.roots()[0];
    assert_eq!(cx.world().local_layout_box(held).unwrap().width, 32.5);
}

#[test]
fn a_sibling_mounted_beside_the_word_lays_out_as_a_full_layout() {
    let mut cx = AppContext::new();
    let leaf = page(&mut cx);
    let made = cx.mount_view(leaf, || chain(1, 0.25)).unwrap();
    flush(&mut cx, false);
    let held = made.roots()[0];
    // The guard (`layout_engine::verify`) compares this pass with a full
    // layout: the retained one kept the column 24 px wide, the full one
    // 32.5 px.
    cx.mount_view(held, || widget(column(0.25)).into_any())
        .unwrap();
    flush(&mut cx, false);
    assert_eq!(cx.world().local_layout_box(held).unwrap().width, 32.5);
}

/// `perf/micro/stack-depth insert under-deep` with an uneven padding: a
/// chain `depth` deep under the leaf, then `siblings` columns beside its
/// word, mounted `batch` at a time. The chain's top box, and every box
/// after the frame in document order.
fn under_deep(depth: usize, siblings: usize, batch: usize) -> (LayoutBox, Vec<LayoutBox>) {
    let mut cx = AppContext::new();
    let leaf = page(&mut cx);
    let made = cx.mount_view(leaf, move || chain(depth, 0.37)).unwrap();
    flush(&mut cx, false);
    let top = made.roots()[0];
    let mut bottom = top;
    while let Some(&child) = cx.world().record(bottom).hierarchy.children.first()
        && !cx.world().record(child).hierarchy.children.is_empty()
    {
        bottom = child;
    }
    let mut mounted = 0;
    while mounted < siblings {
        let count = batch.min(siblings - mounted);
        cx.mount_view(bottom, move || {
            (0..count)
                .map(|_| widget(column(0.37)).into_any())
                .collect::<Vec<_>>()
        })
        .unwrap();
        mounted += count;
    }
    flush(&mut cx, false);
    let world = cx.world();
    (
        world.local_layout_box(top).unwrap(),
        world
            .document_order(document())
            .into_iter()
            .map(|id| world.local_layout_box(id).unwrap_or_default())
            .collect(),
    )
}

#[test]
fn siblings_under_a_deep_chain_lay_out_the_same_whatever_the_hash_order() {
    // The chain's padding alone is wider than the row: the word is shaped
    // in no width at all, and the chain is that padding plus the word. Each
    // run hashes its sets with new keys; the batches change the order the
    // siblings arrive in as well.
    let _unguarded = crate::layout_engine::verify::skip_layout_verify();
    let (top, first) = under_deep(40, 8, 8);
    assert!(
        (top.width - (40.0 * 0.74 + 32.0)).abs() < 0.01,
        "the chain holds its word: {top:?}"
    );
    for (run, batch) in [8, 1, 3, 8, 2, 8, 5, 8].into_iter().enumerate() {
        assert_eq!(
            under_deep(40, 8, batch).1,
            first,
            "run {run}, {batch} a batch"
        );
    }
}

#[test]
fn words_that_stuck_out_of_a_narrow_row_wrap_again_once_they_fit() {
    // "aaaa bbbb": 72 px on a line, 32 px a word. In the 24 px row no line
    // fits and the widest word stuck out; widened to 200 px the line fits,
    // and narrowed to 50 px the words wrap to it again -- one that kept
    // counting as unbreakable would hold the column 72.5 px wide.
    let mut cx = AppContext::new();
    let leaf = leaf_row(&mut cx);
    flush_with(&mut cx, true, &mut WrapShaper);
    let made = cx
        .mount_view(leaf.stable_id(), || {
            widget(column(0.25))
                .children((text("aaaa bbbb"),))
                .into_any()
        })
        .unwrap();
    flush_with(&mut cx, false, &mut WrapShaper);
    let held = made.roots()[0];
    let width = |cx: &AppContext| cx.world().local_layout_box(held).unwrap().width;
    assert_eq!(width(&cx), 32.5, "as wide as its widest word");
    for (row, column) in [(200.0, 72.5), (50.0, 50.0), (24.0, 32.5)] {
        cx.set_component(leaf, narrow().width(LengthSpec::Px(row)))
            .unwrap();
        flush_with(&mut cx, false, &mut WrapShaper);
        assert_eq!(width(&cx), column, "in a row {row} px wide");
    }
}
