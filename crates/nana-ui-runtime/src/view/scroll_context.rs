//! Ambient reactive context identifying the nearest enclosing
//! [`crate::ScrollView`] container.
//!
//! [`crate::view::EachVirtual::within_nearest_scroll`] reads this context (and
//! falls back to walking the retained ancestor chain for any enclosing
//! [`crate::ScrollView`]), so virtualized lists do not need callers to thread a
//! [`NodeRef`] manually through intermediate components.

use super::node::{IntoView, NodeRef, ViewBuilder, widget};
use super::reactive::{provide, use_context};
use crate::ScrollView;

/// Reactive context naming the nearest enclosing [`ScrollView`].
#[derive(Clone, Copy, Debug)]
pub struct NearestScrollContext {
    scroll: NodeRef,
}

impl NearestScrollContext {
    pub fn new(scroll: NodeRef) -> Self {
        Self { scroll }
    }

    pub fn node_ref(self) -> NodeRef {
        self.scroll
    }
}

/// Provide `scroll` as the nearest enclosing scroll container for descendants
/// built within the current reactive scope.
pub fn provide_scroll_context(scroll: NodeRef) {
    provide(NearestScrollContext::new(scroll));
}

/// Read the nearest enclosing [`NearestScrollContext`], if an ancestor
/// provided one.
pub fn nearest_scroll_context() -> Option<NearestScrollContext> {
    use_context::<NearestScrollContext>()
}

/// Declarative [`ScrollView`] wrapper that automatically publishes its
/// [`NodeRef`] via [`provide_scroll_context`] so descendant
/// [`crate::view::EachVirtual::within_nearest_scroll`] lists attach without
/// manual ref threading.
pub struct ScrollContextScope<V> {
    scroll: ScrollView,
    node_ref: NodeRef,
    children: V,
}

/// Wrap `children` in `scroll` and publish its [`NodeRef`] as the ambient
/// [`NearestScrollContext`].
pub fn scroll_context<V: IntoView>(scroll: ScrollView, children: V) -> ScrollContextScope<V> {
    ScrollContextScope {
        scroll,
        node_ref: super::node::node_ref(),
        children,
    }
}

impl<V> ScrollContextScope<V> {
    /// Also bind the scroll container's [`NodeRef`] to a caller-owned handle.
    pub fn node_ref(mut self, node_ref: NodeRef) -> Self {
        self.node_ref = node_ref;
        self
    }
}

impl<V: IntoView> IntoView for ScrollContextScope<V> {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        provide_scroll_context(self.node_ref);
        widget(self.scroll)
            .node_ref(self.node_ref)
            .children(self.children)
            .build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{IntoView, each_virtual, signal, text};
    use crate::{AppContext, DocumentId, LayoutViewport, ScrollAxes};

    #[test]
    fn within_nearest_scroll_attaches_to_enclosing_scroll_context_and_plain_scroll_view() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let items = signal((0..20usize).collect::<Vec<_>>());
        let list_ref = crate::view::virtual_list_ref::<usize>();
        let list_ref_for_view = list_ref.clone();
        cx.mount_view_root(document, move || {
            scroll_context(
                ScrollView::new(ScrollAxes::Vertical).with_layout(|l| {
                    l.width = Some(nana_ui_core::LengthSpec::Px(320.0));
                    l.height = Some(nana_ui_core::LengthSpec::Px(120.0));
                }),
                each_virtual(items, |i| *i, 24.0, |i| text(format!("row {i}")))
                    .gap(4.0)
                    .within_nearest_scroll()
                    .list_ref(list_ref_for_view),
            )
            .into_any()
        })
        .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(320.0, 120.0))
            .unwrap();
        cx.flush_reactive().unwrap();

        assert_eq!(list_ref.len(), 20);
        let first = list_ref.item(&0).expect("first item geometry");
        let second = list_ref.item(&1).expect("second item geometry");
        assert!(
            (first.extent - 28.0).abs() < 0.5,
            "row height + gap: {}",
            first.extent
        );
        assert!(
            (second.offset - 28.0).abs() < 0.5,
            "second offset: {}",
            second.offset
        );
    }

    #[test]
    fn within_nearest_scroll_falls_back_to_ancestor_scroll_view_without_context() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let items = signal((0..10usize).collect::<Vec<_>>());
        let list_ref = crate::view::virtual_list_ref::<usize>();
        let list_ref_for_view = list_ref.clone();
        cx.mount_view_root(document, move || {
            widget(ScrollView::new(ScrollAxes::Vertical).with_layout(|l| {
                l.width = Some(nana_ui_core::LengthSpec::Px(300.0));
                l.height = Some(nana_ui_core::LengthSpec::Px(100.0));
            }))
            .children(
                each_virtual(items, |i| *i, 20.0, |i| text(format!("item {i}")))
                    .gap_step(nana_ui_core::SpacingStep::Xs)
                    .within_nearest_scroll()
                    .list_ref(list_ref_for_view),
            )
            .into_any()
        })
        .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(300.0, 100.0))
            .unwrap();
        cx.flush_reactive().unwrap();

        assert_eq!(list_ref.len(), 10);
        let expected_gap =
            nana_ui_core::SpacingTokens::DEFAULT.resolve(nana_ui_core::SpacingStep::Xs);
        let first = list_ref.item(&0).expect("first item geometry");
        assert!((first.extent - (20.0 + expected_gap)).abs() < 0.5);
    }
}
