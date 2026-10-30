//! Issue #392: a container hidden after it was laid out keeps no layout
//! context and leaves its descendants' boxes where a cold layout puts them,
//! through hide, show and hide again.
#![cfg(test)]

use nana_ui_core::{DisplaySpec, FlexDirection, LayoutStyle};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use crate::{
    AppContext, DocumentId, FormattingContextKind, LayoutBox, LayoutViewport, MutationQueue,
    StableNodeId,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(600.0, 400.0)
}

fn container(hidden: bool) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(FlexDirection::Column),
        hidden,
        ..LayoutStyle::default()
    }
}

/// A row holding a text and a flex column of two texts, one of them in a
/// nested flex box, with the column hidden when asked, laid out once.
/// Returns the column, the nested box and the text in it.
fn page(hidden: bool) -> (AppContext, [StableNodeId; 3]) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let (mut b, page) = Builder::page(document, 1, 400.0);
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        },
    );
    b.label(row, "Before");
    let column = b.element(row, container(hidden));
    b.label(column, "First");
    let nested = b.element(column, container(false));
    let inner = b.label(nested, "Second");
    cx.commit_mutations(b.queue).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, [column, nested, inner])
}

/// Hidden, the column and the box in it establish no context and every box
/// under it is the empty box at the origin; shown, they lay out again. Each
/// step moves the column's context generation and counts no transition, and
/// matches a cold layout of the same tree, boxes and contexts included.
#[test]
fn issue392_hiding_a_laid_out_container_matches_cold_layout() {
    let document = DocumentId::new(1).unwrap();
    let (mut cx, ids @ [column, nested, inner]) = page(false);
    for hidden in [true, false, true] {
        let mut q = MutationQueue::new();
        q.set_style(column, styled(container(hidden)));
        cx.commit_mutations(q).unwrap();
        let generation = cx.world().layout_node(column).unwrap().context_generation;
        let counters = product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        // Dropped or recorded again, a context is not switched for another.
        assert_eq!(counters.layout_context_transitions, 0, "hidden: {hidden}");
        let node = cx.world().layout_node(column).unwrap();
        assert_eq!(node.context_generation, generation + 1, "hidden: {hidden}");
        let established = |id| cx.world().layout_node(id).and_then(|node| node.established);
        let context = (!hidden).then_some(FormattingContextKind::Flex);
        assert_eq!(established(column), context, "hidden: {hidden}");
        assert_eq!(established(nested), context, "hidden: {hidden}");
        let collapsed = cx.world().layout_box(inner) == Some(LayoutBox::default());
        assert_eq!(collapsed, hidden);
        let (mut cold, cold_ids) = page(hidden);
        assert_eq!(cold_ids, ids);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}
