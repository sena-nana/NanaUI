use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InlineContent, InteractionState,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};
use std::sync::Arc;

/// A paragraph of native text/spans and existing retained atomic children.
/// `InlineContent` is also accepted by controls; this component is the generic
/// paragraph facade and adds no wrappers around atomic nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineFlow {
    pub content: InlineContent,
    pub accessible_name: Option<Arc<str>>,
    pub style: NodeStyle,
}
impl InlineFlow {
    pub fn new(content: impl Into<InlineContent>) -> Self {
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).white_space = nana_ui_core::WhiteSpaceSpec::PreWrap;
        Self {
            content: content.into().normalized(),
            accessible_name: None,
            style,
        }
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).width = Some(width);
        self
    }
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }
}
impl ComponentView for InlineFlow {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::INLINE_FLOW.type_id);

    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }
    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }
    fn project(&self, id: StableNodeId, world: &UiWorld, queue: &mut MutationQueue) {
        super::world::inline_content::project_inline_content(id, world, &self.content, queue);
        super::view_components::project_common(
            id,
            world,
            queue,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                label: self.accessible_name.clone(),
                ..Default::default()
            },
        );
    }
}
