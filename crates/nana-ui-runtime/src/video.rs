//! Retained video playback surface.
//!
//! `<video>` samples a host-owned texture slot (`"nana.host-texture"`,
//! resource `"video:{id}"`) exactly like [`crate::GpuTextureView`]. The
//! framework owns the presentation contract and the declarative playback
//! attributes; the frame source (decoder, clock, transport) stays with the
//! host, which pushes frames and dispatches `play` / `pause` / `ended`.

use std::sync::Arc;

use nana_ui_core::ContentFit;

use crate::gpu_slots::{fill_layout, finite_opacity, finite_radius};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};

/// Displays a host-fed video surface inside a layout region.
///
/// `resource` is the host texture registry slot the frame bridge registers.
/// `autoplay` / `loop_` / `muted` mirror the declarative HTML attributes; the
/// live playback truth stays with the host push side. Pointer events default
/// to off.
#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub resource: crate::TextureSlotName,
    pub opacity: f32,
    pub corner_radius: f32,
    pub fit: ContentFit,
    pub style: NodeStyle,
    pub pointer_events: bool,
    pub autoplay: bool,
    pub loop_: bool,
    pub muted: bool,
}

impl Video {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn new(resource: impl Into<crate::TextureSlotName>) -> Self {
        Self {
            resource: resource.into(),
            opacity: 1.0,
            corner_radius: 0.0,
            fit: ContentFit::Fill,
            style: NodeStyle::default(),
            pointer_events: false,
            autoplay: false,
            loop_: false,
            muted: false,
        }
    }

    /// Host-texture Scene node. Empty identities are omitted so the world
    /// does not reject a blank `resource`. The painter resolves the slot in
    /// the host texture registry every frame, so the node carries revision 0.
    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.resource.is_empty() {
            return None;
        }
        Some(
            CustomRenderNode::new(crate::HOST_TEXTURE_RENDERER, self.resource.to_resource(), 0)
                .with_fit(self.fit),
        )
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = fill_layout(&self.style);
        let layout = Arc::make_mut(&mut style.layout);
        layout.opacity = Some(finite_opacity(self.opacity));
        layout.border_radius = Some(finite_radius(self.corner_radius));
        if self.custom_render().is_some() {
            layout.paint.content_image = None;
            layout.paint.skipped_replaced = None;
        }
        style
    }
}

impl ComponentView for Video {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::VIDEO.type_id);

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "video".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
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
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::LayoutStyle;

    #[test]
    fn custom_render_uses_host_texture_renderer_and_fit() {
        let mut video = Video::new("video:7");
        video.fit = ContentFit::Contain;
        let custom = video.custom_render().expect("slot present");
        assert_eq!(custom.renderer.as_ref(), crate::HOST_TEXTURE_RENDERER);
        assert_eq!(custom.resource.as_ref(), "video:7");
        assert_eq!(custom.revision, 0);
        assert_eq!(custom.fit, ContentFit::Contain);
    }

    #[test]
    fn empty_resource_omits_custom_render() {
        assert!(Video::new("").custom_render().is_none());
    }

    fn mount(video: Video) -> (crate::UiWorld, StableNodeId) {
        let mut world = crate::UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let document = crate::DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document, video.node_kind());
        video.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn slotted_video_drops_poster_content_image() {
        let mut video = Video::new("video:7");
        let mut layout = LayoutStyle::default();
        layout.paint.content_image = Some(nana_ui_core::BackgroundImage::url("frame.png"));
        video.style.layout = Arc::new(layout);
        let (world, id) = mount(video);
        assert_eq!(
            world.custom_render(id).map(|node| node.resource.as_ref()),
            Some("video:7")
        );
        assert!(
            world
                .layout_style(id)
                .expect("projected")
                .paint
                .content_image
                .is_none(),
            "HostTexture video must not also paint poster"
        );
    }

    #[test]
    fn poster_only_video_keeps_content_image() {
        let mut video = Video::new("");
        let mut layout = LayoutStyle::default();
        layout.paint.content_image = Some(nana_ui_core::BackgroundImage::url("frame.png"));
        video.style.layout = Arc::new(layout);
        let (world, id) = mount(video);
        assert!(world.custom_render(id).is_none());
        match world
            .layout_style(id)
            .expect("projected")
            .paint
            .content_image
            .as_ref()
        {
            Some(nana_ui_core::BackgroundImage::Url { url, .. }) => {
                assert_eq!(url.as_str(), "frame.png")
            }
            other => panic!("expected poster content_image, got {other:?}"),
        }
    }
}
