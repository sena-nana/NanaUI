//! Compact list-row image slot. NanaUI never stores pixels or codecs.
//!
//! Hosts declare a HostTexture slot and an optional aspect. The box is
//! [`ControlSize`] height × aspect (default 1:1). Empty, loading, ready, and
//! unavailable share that geometry. Ready samples `"nana.host-texture"` with
//! Contain; empty identities omit the Scene node.

use std::sync::Arc;

use nana_ui_core::{
    ContentFit, ControlSize, LengthSpec, OverflowSpec, SemanticColorRole, ThemeMetrics,
};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, HOST_TEXTURE_RENDERER,
    InteractionState, MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual,
    TextContent, UiWorld,
};

/// Width ÷ height when the host does not declare an aspect.
pub const DEFAULT_ASPECT: f32 = 1.0;

const SPINNER_SIZE: f32 = 14.0;

/// Presentation of a [`Thumbnail`] box. All four states keep the same size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThumbnailState {
    #[default]
    Empty,
    Loading,
    Ready,
    Unavailable,
}

/// Compact image control for [`crate::ListItem`] leading slots.
///
/// Pointer events stay off so the parent row owns hit-testing.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    pub resource: Arc<str>,
    pub state: ThumbnailState,
    pub size: ControlSize,
    pub aspect: f32,
    pub label: Arc<str>,
    pub style: NodeStyle,
}

impl Thumbnail {
    /// Ready when `resource` is non-empty after trim; otherwise empty.
    pub fn new(resource: impl Into<Arc<str>>) -> Self {
        let resource = resource.into();
        let state = if resource.trim().is_empty() {
            ThumbnailState::Empty
        } else {
            ThumbnailState::Ready
        };
        Self {
            resource,
            state,
            size: ControlSize::Small,
            aspect: DEFAULT_ASPECT,
            label: Arc::from(""),
            style: NodeStyle::default(),
        }
    }

    pub fn empty() -> Self {
        Self::new("").state(ThumbnailState::Empty)
    }

    pub fn loading() -> Self {
        Self::new("").state(ThumbnailState::Loading)
    }

    pub fn unavailable() -> Self {
        Self::new("").state(ThumbnailState::Unavailable)
    }

    pub const fn state(mut self, state: ThumbnailState) -> Self {
        self.state = state;
        self
    }

    pub fn aspect(mut self, aspect: f32) -> Self {
        self.aspect = sanitize_aspect(aspect);
        self
    }

    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = label.into();
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Logical box: control height × host aspect (default 1:1).
    pub fn box_extent(&self, metrics: ThemeMetrics) -> (f32, f32) {
        let height = self.size.height_in(metrics);
        (height * sanitize_aspect(self.aspect), height)
    }

    /// Host-texture Scene node. Only Ready with a non-empty slot attaches one.
    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.state != ThumbnailState::Ready || self.resource.trim().is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(HOST_TEXTURE_RENDERER, Arc::clone(&self.resource), 0)
                .with_fit(ContentFit::Contain),
        )
    }

    fn effective_style(&self, world: &UiWorld) -> NodeStyle {
        let metrics = world.theme_metrics();
        let (width, height) = self.box_extent(metrics);
        let mut style = self.style.clone();
        style.background = match self.state {
            ThumbnailState::Ready => None,
            _ => Some(SemanticColorRole::Subtle),
        };
        style.border = None;
        style.foreground = Some(SemanticColorRole::Muted);
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(width));
        layout.height = Some(LengthSpec::Px(height));
        layout.min_width = Some(LengthSpec::Px(width));
        layout.min_height = Some(LengthSpec::Px(height));
        layout.max_width = Some(LengthSpec::Px(width));
        layout.max_height = Some(LengthSpec::Px(height));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.border_width = Some(0.0);
        layout.border_radius = Some(metrics.radius_xs);
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        style
    }
}

impl Default for Thumbnail {
    fn default() -> Self {
        Self::empty()
    }
}

impl ComponentView for Thumbnail {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "thumbnail".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new(),
                },
            );
        }
        let custom = self.custom_render();
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        let visual = (self.state == ThumbnailState::Loading).then(|| StandardVisual::Spinner {
            label: Arc::from(""),
            size: SPINNER_SIZE.min(self.box_extent(world.theme_metrics()).1),
            phase: 0.0,
        });
        if world.standard_visual(id) != visual {
            mutations.set_standard_visual(id, visual);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                label: (!self.label.is_empty()).then(|| Arc::clone(&self.label)),
                busy: self.state == ThumbnailState::Loading,
                invalid: self.state == ThumbnailState::Unavailable,
                ..AccessibilityState::default()
            },
        );
    }
}

fn sanitize_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        DEFAULT_ASPECT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, LayoutViewport, ListItem, ListItemSlots};
    use nana_ui_core::UI_METRICS;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn mount(component: Thumbnail) -> (crate::UiWorld, StableNodeId) {
        let mut world = crate::UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document(), component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn box_is_control_height_by_host_aspect() {
        let square = ControlSize::Small.height();
        assert_eq!(Thumbnail::empty().box_extent(UI_METRICS), (square, square));
        let wide = Thumbnail::empty().aspect(16.0 / 9.0).box_extent(UI_METRICS);
        assert_eq!(wide.1, square);
        assert!((wide.0 - square * 16.0 / 9.0).abs() < f32::EPSILON);
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                Thumbnail::empty().aspect(invalid).box_extent(UI_METRICS),
                (square, square)
            );
        }
        let mut context = AppContext::new();
        let expected = (square, square);
        for thumb in [
            Thumbnail::empty(),
            Thumbnail::loading(),
            Thumbnail::new("stage.thumb"),
            Thumbnail::unavailable(),
        ] {
            let entity = context.create_component(document(), thumb).unwrap();
            context
                .layout_document(document(), LayoutViewport::new(240.0, 80.0))
                .unwrap();
            let bounds = context.world().layout_box(entity.stable_id()).unwrap();
            assert_eq!((bounds.width, bounds.height), expected);
        }
    }

    #[test]
    fn ready_samples_host_texture_contain_otherwise_omits_node() {
        let node = Thumbnail::new("stage.thumb")
            .custom_render()
            .expect("ready slot");
        assert_eq!(node.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert_eq!(node.resource.as_ref(), "stage.thumb");
        assert_eq!(node.fit, ContentFit::Contain);
        assert!(Thumbnail::new("").custom_render().is_none());
        assert!(
            Thumbnail::new("stage.thumb")
                .state(ThumbnailState::Loading)
                .custom_render()
                .is_none()
        );
        let (world, id) = mount(Thumbnail::new("stage.thumb"));
        assert_eq!(
            world.custom_render(id).map(|node| node.fit),
            Some(ContentFit::Contain)
        );
        let (world, id) = mount(Thumbnail::loading());
        assert!(world.custom_render(id).is_none());
        assert!(matches!(
            world.standard_visual(id),
            Some(StandardVisual::Spinner { .. })
        ));
        assert!(world.accessibility(id).unwrap().busy);
        let (world, id) = mount(Thumbnail::unavailable());
        assert!(world.accessibility(id).unwrap().invalid);
        assert_eq!(
            world.interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );
    }

    #[test]
    fn list_item_leading_keeps_the_box() {
        let mut context = AppContext::new();
        let item = context
            .create_component(document(), ListItem::new("角色"))
            .unwrap();
        let leading = context
            .create_component(document(), Thumbnail::empty())
            .unwrap();
        context.append_child(item, leading).unwrap();
        context
            .set_list_item_slots(
                item,
                ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: None,
                    trailing: None,
                },
            )
            .unwrap();
        context
            .layout_document(document(), LayoutViewport::new(240.0, 80.0))
            .unwrap();
        let thumb = context.world().layout_box(leading.stable_id()).unwrap();
        assert_eq!(thumb.width, ControlSize::Small.height());
        assert_eq!(thumb.height, ControlSize::Small.height());
        assert_eq!(
            context.world().node(item.stable_id()).unwrap().children,
            vec![leading.stable_id()]
        );
    }
}
