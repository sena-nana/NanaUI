//! Circular cover-fit host-texture slot. Distinct from [`crate::Thumbnail`]:
//! Cover mapping, host-owned pixel size, circular clip. NanaUI never stores
//! pixels or codecs.

use std::sync::Arc;

use nana_ui_core::{ContentFit, LengthSpec, OverflowSpec, SemanticColorRole};

use crate::gpu_slots::HOST_TEXTURE_RENDERER;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};

/// Default edge length when the host does not declare a size.
pub const DEFAULT_SIZE: f32 = 32.0;

/// Circular image control (`nana.avatar`).
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    pub resource: Arc<str>,
    pub size: f32,
    pub label: Arc<str>,
    pub pointer_events: bool,
    pub style: NodeStyle,
}

impl Avatar {
    pub fn new(resource: impl Into<Arc<str>>) -> Self {
        Self {
            resource: resource.into(),
            size: DEFAULT_SIZE,
            label: Arc::from(""),
            pointer_events: false,
            style: NodeStyle::default(),
        }
    }

    pub fn empty() -> Self {
        Self::new("")
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = sanitize_size(size);
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn pointer_events(mut self, pointer_events: bool) -> Self {
        self.pointer_events = pointer_events;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.resource.trim().is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(HOST_TEXTURE_RENDERER, Arc::clone(&self.resource), 0)
                .with_fit(ContentFit::Cover),
        )
    }

    fn effective_style(&self) -> NodeStyle {
        let size = sanitize_size(self.size);
        let mut style = self.style.clone();
        if self.resource.trim().is_empty() {
            style.background = Some(SemanticColorRole::Subtle);
        } else {
            style.background = None;
        }
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(size));
        layout.height = Some(LengthSpec::Px(size));
        layout.min_width = Some(LengthSpec::Px(size));
        layout.min_height = Some(LengthSpec::Px(size));
        layout.max_width = Some(LengthSpec::Px(size));
        layout.max_height = Some(LengthSpec::Px(size));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.border_width = Some(0.0);
        layout.border_radius = Some(size * 0.5);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        style
    }
}

impl Default for Avatar {
    fn default() -> Self {
        Self::empty()
    }
}

impl ComponentView for Avatar {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "avatar".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                crate::TextContent {
                    value: String::new(),
                },
            );
        }
        let custom = self.custom_render();
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: self.pointer_events,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                label: (!self.label.is_empty()).then(|| Arc::clone(&self.label)),
                ..AccessibilityState::default()
            },
        );
    }
}

fn sanitize_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        DEFAULT_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn empty_is_circular_placeholder() {
        let mut context = AppContext::new();
        let avatar = context
            .create_component(document(), Avatar::empty().size(40.0).label("用户"))
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(120.0, 80.0))
            .unwrap();
        let style = context.world().node_style(avatar.stable_id()).unwrap();
        assert_eq!(style.layout.border_radius, Some(20.0));
        assert_eq!(style.background, Some(SemanticColorRole::Subtle));
        assert!(context.world().custom_render(avatar.stable_id()).is_none());
        let box_ = context.world().layout_box(avatar.stable_id()).unwrap();
        assert!((box_.width - 40.0).abs() < 0.5);
        assert!((box_.height - 40.0).abs() < 0.5);
    }

    #[test]
    fn ready_samples_host_texture_cover() {
        let avatar = Avatar::new("user.avatar");
        let render = avatar.custom_render().unwrap();
        assert_eq!(render.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert_eq!(render.resource.as_ref(), "user.avatar");
        assert_eq!(render.fit, ContentFit::Cover);
    }

    #[test]
    fn invalid_size_falls_back_to_default() {
        for invalid in [0.0, -8.0, f32::NAN, f32::INFINITY] {
            assert_eq!(sanitize_size(invalid), DEFAULT_SIZE);
        }
    }
}
