//! Live heap per retained node: allocations minus frees, counted per thread
//! so parallel tests do not disturb each other.
//!
//! A `LayoutStyle` is about 2 KB. Nodes whose components build equal layouts
//! share one (`ComponentView::share_layouts`, the world's recent-layout
//! cache), so a thousand equal buttons hold one layout, not two thousand.

use crate::LIVE;
use std::cell::Cell;

use nana_ui_runtime::view::{column, widget};
use nana_ui_runtime::{
    AppContext, Button, ComponentView, DocumentId, LayoutViewport, ListItem, MeasureTextShaper,
    Stack, Text, TextInput,
};

/// Live bytes per node after building `n` nodes from `make`.
fn live_per_node<C: ComponentView>(n: usize, make: impl Fn(usize) -> C + Copy + 'static) -> f64 {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    // Grow the context's tables and the process-wide defaults first.
    cx.mount_view_root(document, || {
        column().children((0..8).map(|i| widget(make(i))).collect::<Vec<_>>())
    })
    .unwrap();
    let before = LIVE.with(Cell::get);
    cx.mount_view_root(document, || {
        column().children((0..n).map(|i| widget(make(i))).collect::<Vec<_>>())
    })
    .unwrap();
    let per_node = (LIVE.with(Cell::get) - before) as f64 / n as f64;
    drop(cx);
    per_node
}

#[test]
fn equal_nodes_share_their_layouts() {
    const N: usize = 2_000;
    let text = live_per_node(N, |i| Text::new(format!("行 {i}")));
    let stack = live_per_node(N, |_| Stack::column(0.0));
    let button = live_per_node(N, |i| Button::new(format!("按钮 {i}")));
    let input = live_per_node(N, |i| TextInput::new(format!("值 {i}")));
    eprintln!(
        "live bytes per node: text {text:.0}, stack {stack:.0}, button {button:.0}, \
         text input {input:.0}"
    );
    // Equal nodes share resolved layouts, styles, and default component visuals
    // rather than retaining per-node clones. Baselines at N=2,000 are ~301 B
    // (stack), ~704 B (text), ~935 B (button), and ~2,937 B (text input).
    assert!(stack <= 320.0, "{stack:.0} B per stack");
    assert!(text <= 860.0, "{text:.0} B per text");
    assert!(button <= 1_150.0, "{button:.0} B per button");
    assert!(input <= 3_600.0, "{input:.0} B per text input");
}

/// Run frames until style, text and layout work settle, so every node holds
/// its resolved style, shaped text and published layout result and parts.
fn settle(cx: &mut AppContext, document: DocumentId) {
    for _ in 0..4 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, &mut MeasureTextShaper).unwrap();
        cx.layout_document(document, LayoutViewport::new(1280.0, 800.0))
            .unwrap();
    }
}

/// Live bytes per node after building `n` nodes from `make` and laying
/// them out: mounting plus everything layout keeps for them.
fn live_per_laid_out_node<C: ComponentView>(
    n: usize,
    make: impl Fn(usize) -> C + Copy + 'static,
) -> f64 {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    cx.mount_view_root(document, || {
        column().children((0..8).map(|i| widget(make(i))).collect::<Vec<_>>())
    })
    .unwrap();
    settle(&mut cx, document);
    let before = LIVE.with(Cell::get);
    cx.mount_view_root(document, || {
        column().children((0..n).map(|i| widget(make(i))).collect::<Vec<_>>())
    })
    .unwrap();
    settle(&mut cx, document);
    let per_node = (LIVE.with(Cell::get) - before) as f64 / n as f64;
    drop(cx);
    per_node
}

/// Issue #206: what migrated components keep once laid out (resolved
/// style, shaped text, `LayoutResult` and its parts) stays bounded per node
/// and does not grow per node with the document.
#[test]
fn laid_out_nodes_keep_bounded_live_memory() {
    let mut per_size = Vec::new();
    for n in [1_000, 10_000] {
        let stack = live_per_laid_out_node(n, |_| Stack::column(0.0));
        let text = live_per_laid_out_node(n, |i| Text::new(format!("行 {i}")));
        let button = live_per_laid_out_node(n, |i| Button::new(format!("按钮 {i}")));
        let item = live_per_laid_out_node(n, |i| ListItem::new(format!("条目 {i}")));
        eprintln!(
            "{n} laid-out nodes, live bytes per node: stack {stack:.0}, text {text:.0}, \
             button {button:.0}, list item {item:.0}"
        );
        // Baselines after hot/cold split and slot packing:
        // - n=1,000: stack ~639 B, text ~1,338 B, button ~1,750 B, list item ~1,649 B
        // - n=10,000: stack ~634 B, text ~1,121 B, button ~1,335 B, list item ~1,247 B
        if n == 1_000 {
            assert!(stack <= 780.0, "{stack:.0} B per 1k laid-out stack");
            assert!(text <= 1_650.0, "{text:.0} B per 1k laid-out text");
            assert!(button <= 2_150.0, "{button:.0} B per 1k laid-out button");
            assert!(item <= 2_050.0, "{item:.0} B per 1k laid-out list item");
        } else {
            assert!(stack <= 650.0, "{stack:.0} B per 10k laid-out stack");
            assert!(text <= 1_380.0, "{text:.0} B per 10k laid-out text");
            assert!(button <= 1_400.0, "{button:.0} B per 10k laid-out button");
            assert!(item <= 1_400.0, "{item:.0} B per 10k laid-out list item");
        }
        per_size.push([stack, text, button, item]);
    }
    // Per-node cost does not rise with the document: no per-node table that
    // grows with every other node.
    for (small, large) in per_size[0].iter().zip(per_size[1]) {
        assert!(
            large < small * 1.15,
            "{large:.0} B at 10k vs {small:.0} B at 1k"
        );
    }
}
