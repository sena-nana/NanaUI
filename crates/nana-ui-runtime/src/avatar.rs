//! Circular cover-fit host-texture slot. Distinct from [`crate::Thumbnail`]:
//! Cover mapping, host-owned pixel size, circular clip. NanaUI never stores
//! pixels or codecs. Empty or cleared slots use the Subtle placeholder; do not
//! paint initials.

use std::sync::Arc;

use nana_ui_core::{ContentFit, ImageSampling, LengthSpec, OverflowSpec, SemanticColorRole};

use crate::gpu_slots::{HOST_TEXTURE_RENDERER, TextureSlotName};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};

/// Default edge length when the host does not declare a size.
pub const DEFAULT_SIZE: f32 = nana_ui_core::ControlSize::Medium.height_in(nana_ui_core::UI_METRICS);

/// Circular image control (`nana.avatar`).
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    /// The registry slot shown. Re-registering it is all a host does to show
    /// new pixels; the control carries no revision of its own.
    pub resource: TextureSlotName,
    pub size: f32,
    /// Optional size intent resolved against the installed theme.
    pub themed_size: Option<nana_ui_core::theme::ThemeExpr>,
    pub label: Arc<str>,
    pub pointer_events: bool,
    /// Level 0 by default (the host sizes the slot from `painted_extent`);
    /// [`ImageSampling::Mipmap`] samples the host's mip chain trilinearly.
    pub sampling: ImageSampling,
    pub style: NodeStyle,
}

impl Avatar {
    pub fn new(resource: impl Into<TextureSlotName>) -> Self {
        Self {
            resource: resource.into(),
            size: DEFAULT_SIZE,
            themed_size: None,
            label: Arc::from(""),
            pointer_events: false,
            sampling: ImageSampling::Resample,
            style: NodeStyle::default(),
        }
    }

    pub fn empty() -> Self {
        Self::new("")
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = sanitize_size(size);
        self.themed_size = None;
        self
    }

    /// Keep avatar geometry and circular clipping on the same theme expression.
    pub fn size_theme(mut self, size: nana_ui_core::theme::ThemeExpr) -> Self {
        self.themed_size = Some(size);
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

    /// How the slot's texture is sampled; see [`ImageSampling`].
    pub const fn sampling(mut self, sampling: ImageSampling) -> Self {
        self.sampling = sampling;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.resource.is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(HOST_TEXTURE_RENDERER, self.resource.to_resource(), 0)
                .with_fit(ContentFit::Cover)
                .with_sampling(self.sampling),
        )
    }

    fn effective_style(&self) -> NodeStyle {
        let size = sanitize_size(self.size);
        let mut style = self.style.clone();
        if self.resource.is_empty() {
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
        if let Some(size) = &self.themed_size {
            apply_theme_size(layout, size, true);
        }
        style
    }
}

impl Default for Avatar {
    fn default() -> Self {
        Self::empty()
    }
}

impl ComponentView for Avatar {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::AVATAR.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

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
                    value: String::new().into(),
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

/// The image, its placeholder, hit target and circular clip resolve one size.
pub(crate) fn apply_theme_size(
    layout: &mut nana_ui_core::LayoutStyle,
    size: &nana_ui_core::theme::ThemeExpr,
    circular: bool,
) {
    layout.theme.width = Some(size.clone());
    layout.theme.height = Some(size.clone());
    layout.theme.min_width = Some(size.clone());
    layout.theme.min_height = Some(size.clone());
    layout.theme.max_width = Some(size.clone());
    layout.theme.max_height = Some(size.clone());
    if circular {
        layout.theme.border_radius = Some(size.clone().scaled(0.5));
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
    fn themed_avatar_and_hover_trigger_keep_geometry_and_circle_in_sync() {
        use nana_ui_core::theme::{ThemeExpr, ThemeToken};
        use nana_ui_core::{ControlSize, ThemeDefinition};
        let mut context = AppContext::new();
        let size = ThemeExpr::token(ThemeToken::Control(ControlSize::Small));
        let root = context
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let avatar = context
            .create_component(document(), Avatar::empty().size_theme(size.clone()))
            .unwrap();
        let hover = context
            .create_component(
                document(),
                crate::HoverCard::new()
                    .trigger_image("", "User")
                    .trigger_size_theme(size),
            )
            .unwrap();
        context.append_child(root, avatar).unwrap();
        context.append_child(root, hover).unwrap();
        for height in [28.0, 40.0] {
            let mut definition = ThemeDefinition::NANA_DARK;
            definition.tokens.metrics.compact_control_height = height;
            context.set_theme_definition(&definition).unwrap();
            context
                .layout_document(document(), LayoutViewport::new(240.0, 240.0))
                .unwrap();
            for node in [avatar.stable_id(), hover.stable_id()] {
                let bounds = context.world().layout_box(node).unwrap();
                assert_eq!(bounds.width, height);
                assert_eq!(bounds.height, height);
                assert_eq!(
                    context.world().layout_style(node).unwrap().border_radius,
                    Some(height * 0.5)
                );
            }
        }
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
        let box_ = context
            .world()
            .component_layout_box(avatar.stable_id())
            .unwrap();
        assert!((box_.width - 40.0).abs() < 0.5);
        assert!((box_.height - 40.0).abs() < 0.5);
        assert_eq!(context.world().text(avatar.stable_id()), Some(""));
        let a11y = context.world().accessibility(avatar.stable_id()).unwrap();
        assert_eq!(a11y.role, AccessibilityRole::Image);
        assert_eq!(a11y.label.as_deref(), Some("用户"));
        assert!(
            !context
                .world()
                .interaction(avatar.stable_id())
                .unwrap()
                .focusable
        );
        assert!(
            !context
                .world()
                .interaction(avatar.stable_id())
                .unwrap()
                .pointer_events
        );
    }

    #[test]
    fn unlabeled_empty_is_an_unnamed_image_placeholder() {
        let mut context = AppContext::new();
        let avatar = context
            .create_component(document(), Avatar::empty())
            .unwrap();
        let a11y = context.world().accessibility(avatar.stable_id()).unwrap();
        assert_eq!(a11y.role, AccessibilityRole::Image);
        assert!(a11y.label.is_none());
        assert_eq!(context.world().text(avatar.stable_id()), Some(""));
        assert_eq!(
            context
                .world()
                .node_style(avatar.stable_id())
                .unwrap()
                .background,
            Some(SemanticColorRole::Subtle)
        );
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

    /// Every view of one slot shares revision 0, so this control can share a
    /// slot with any other view; which texture it is lives in the registry.
    /// Pixel evidence is in `nana-ui-devtools/tests/host_texture_paint.rs`.
    #[test]
    fn host_texture_node_carries_no_revision() {
        let control = Avatar::new("cover:a");
        assert_eq!(control.custom_render().expect("ready slot").revision, 0);
    }
}
