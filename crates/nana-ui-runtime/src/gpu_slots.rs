//! Backend-neutral GPU content slots.
//!
//! [`GpuTextureView`] projects a [`CustomRenderNode`] that `IcedSceneView` can
//! bind as `"nana.host-texture"`. [`GpuView`] projects [`GPU_VIEW_RENDERER`];
//! default `IcedSceneView::new` / `from_shared` install a host painter.
//! Device, Queue, Surface, and any WGPU objects stay host-owned; Runtime never
//! constructs a second renderer.

use std::sync::Arc;

use nana_ui_core::{LengthSpec, PhysicalRect};

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
};

/// Scene renderer key for [`GpuView`].
///
/// [`GpuView::project`] attaches [`GpuView::custom_render`]. Default
/// `IcedSceneView::new` / `from_shared` install a `"gpu-view"` painter;
/// an explicit empty registry still leaves the node unpaintable.
pub const GPU_VIEW_RENDERER: &str = "gpu-view";
/// Scene renderer key for [`GpuTextureView`].
///
/// Must stay [`crate::HOST_TEXTURE_RENDERER`] (`"nana.host-texture"`) so
/// `IcedSceneView` can bind a host-owned texture without a second Device/Queue.
/// `"gpu-texture-view"` is not a registered Scene painter. The node's
/// `resource` is the same slot string the host registers.
pub const GPU_TEXTURE_VIEW_RENDERER: &str = crate::HOST_TEXTURE_RENDERER;

/// Packs identity and content counters for [`CustomRenderNode::revision`].
///
/// `generation` occupies the high 32 bits and changes when the host replaces
/// the view. `version` occupies the low 32 bits and changes on content
/// invalidation. The halves are independent.
pub const fn pack_gpu_revision(generation: u64, version: u64) -> u64 {
    ((generation & 0xFFFF_FFFF) << 32) | (version & 0xFFFF_FFFF)
}

/// Splits a packed [`CustomRenderNode::revision`] into `(generation, version)`.
pub const fn unpack_gpu_revision(revision: u64) -> (u64, u64) {
    (revision >> 32, revision & 0xFFFF_FFFF)
}

/// Selects how a [`GpuView`] should contribute commands to the current frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum GpuViewMode {
    /// Reuse the host painter's current render pass.
    #[default]
    Inline,
    /// Open a dedicated pass on the same encoder and target.
    Standalone,
}

/// Retained palette for the host painter. Runtime does not shade these colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuViewPalette {
    pub background: [f32; 4],
    pub accent: [f32; 4],
}

impl Default for GpuViewPalette {
    fn default() -> Self {
        Self {
            background: [0.0, 0.0, 0.0, 1.0],
            accent: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// A reusable GPU content slot identified by a stable host `slot_id`.
///
/// Layout, interaction, and [`Self::custom_render`] are projected. Paint
/// requires a host-registered Scene GPU renderer for [`GPU_VIEW_RENDERER`].
#[derive(Debug, Clone, PartialEq)]
pub struct GpuView {
    pub slot_id: u64,
    pub mode: GpuViewMode,
    pub generation: u64,
    pub version: u64,
    pub palette: GpuViewPalette,
    /// Host-painter seed. Not mixed into revision; bump [`Self::version`] to
    /// request a redraw.
    pub seed: f32,
    pub style: NodeStyle,
}

impl GpuView {
    pub fn new(slot_id: u64) -> Self {
        Self {
            slot_id,
            mode: GpuViewMode::Inline,
            generation: 0,
            version: 0,
            palette: GpuViewPalette::default(),
            seed: 0.0,
            style: NodeStyle::default(),
        }
    }

    pub const fn mode(mut self, mode: GpuViewMode) -> Self {
        self.mode = mode;
        self
    }

    pub const fn palette(mut self, palette: GpuViewPalette) -> Self {
        self.palette = palette;
        self
    }

    pub fn seed(mut self, seed: f32) -> Self {
        self.seed = finite_seed(seed);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Marks the current view as containing new host-rendered content.
    pub fn invalidate_content(&mut self) -> u64 {
        self.version = self.version.saturating_add(1);
        self.version
    }

    /// Records that the host replaced the underlying view. Does not touch
    /// [`Self::version`].
    pub fn replace_view(&mut self, generation: u64) -> u64 {
        self.generation = generation;
        self.generation
    }

    pub const fn revision(&self) -> u64 {
        pack_gpu_revision(self.generation, self.version)
    }

    /// Backend-neutral custom node for hosts that register a Scene GPU
    /// renderer for [`GPU_VIEW_RENDERER`]. [`ComponentView::project`] attaches
    /// this node, so default IcedSceneView construction fails until that
    /// renderer is registered.
    ///
    /// `resource` is the decimal [`Self::slot_id`].
    pub fn custom_render(&self) -> CustomRenderNode {
        CustomRenderNode {
            renderer: Arc::from(GPU_VIEW_RENDERER),
            resource: resource_key(self.slot_id),
            revision: self.revision(),
        }
    }

    fn effective_style(&self) -> NodeStyle {
        fill_layout(&self.style)
    }
}

/// Displays a host-owned texture inside a layout region.
///
/// `resource` is the host texture registry slot identity (the same string the
/// host registers for `"nana.host-texture"`). Decimal IDs such as `"42"` are
/// valid if the host registers that key. `generation` changes only when the
/// sampled view is replaced. `version` changes for every content invalidation.
/// Presentation fields stay on this view so CustomRenderNode can keep a
/// backend-neutral id/revision contract.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuTextureView {
    pub resource: Arc<str>,
    pub generation: u64,
    pub version: u64,
    pub opacity: f32,
    pub corner_radius: f32,
    pub style: NodeStyle,
}

impl GpuTextureView {
    pub fn new(resource: impl Into<Arc<str>>) -> Self {
        Self {
            resource: resource.into(),
            generation: 0,
            version: 0,
            opacity: 1.0,
            corner_radius: 0.0,
            style: NodeStyle::default(),
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = finite_opacity(opacity);
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = finite_radius(radius);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Marks the current view as containing new host-rendered content.
    pub fn invalidate_content(&mut self) -> u64 {
        self.version = self.version.saturating_add(1);
        self.version
    }

    /// Records that the host replaced the sampled view. Does not touch
    /// [`Self::version`].
    pub fn replace_view(&mut self, generation: u64) -> u64 {
        self.generation = generation;
        self.generation
    }

    pub const fn revision(&self) -> u64 {
        pack_gpu_revision(self.generation, self.version)
    }

    /// Host-texture Scene node. Empty identities are omitted so the world
    /// does not reject a blank `resource`.
    pub fn custom_render(&self) -> Option<CustomRenderNode> {
        if self.resource.trim().is_empty() {
            return None;
        }
        Some(CustomRenderNode {
            renderer: Arc::from(GPU_TEXTURE_VIEW_RENDERER),
            resource: Arc::clone(&self.resource),
            revision: self.revision(),
        })
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = fill_layout(&self.style);
        let layout = Arc::make_mut(&mut style.layout);
        layout.opacity = Some(finite_opacity(self.opacity));
        layout.border_radius = Some(finite_radius(self.corner_radius));
        style
    }
}

/// A stable logical/physical region suitable for a viewport and scissor.
///
/// Physical pixels are derived with the same floor/ceil scale math as the Iced
/// compatibility `RenderSlot`: edges cover fractional coverage, and a
/// non-finite or non-positive scale becomes `1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSlot {
    pub id: u64,
    pub logical: LayoutBox,
    pub scale: f32,
}

impl RenderSlot {
    pub fn new(id: u64, logical: LayoutBox, scale: f32) -> Self {
        Self {
            id,
            logical: sanitize_logical(logical),
            scale: sanitize_scale(scale),
        }
    }

    /// Physical scissor/viewport rectangle derived from [`Self::logical`] and
    /// [`Self::scale`].
    pub fn physical(&self) -> PhysicalRect {
        let scale = self.scale;
        let left = (self.logical.x * scale).floor().max(0.0);
        let top = (self.logical.y * scale).floor().max(0.0);
        let right = ((self.logical.x + self.logical.width) * scale)
            .ceil()
            .max(left);
        let bottom = ((self.logical.y + self.logical.height) * scale)
            .ceil()
            .max(top);
        PhysicalRect {
            x: saturating_u32(left),
            y: saturating_u32(top),
            width: saturating_u32(right - left),
            height: saturating_u32(bottom - top),
        }
    }

    pub fn clipped_physical(self, target_width: u32, target_height: u32) -> PhysicalRect {
        let physical = self.physical();
        let right = physical.x.saturating_add(physical.width).min(target_width);
        let bottom = physical
            .y
            .saturating_add(physical.height)
            .min(target_height);
        let x = physical.x.min(target_width);
        let y = physical.y.min(target_height);
        PhysicalRect {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    /// Unrounded physical viewport (`logical * scale`), matching Iced
    /// `slot_for_bounds`. Distinct from the floored/ceiled scissor in
    /// [`Self::physical`].
    pub fn viewport(&self) -> [f32; 4] {
        let scale = self.scale;
        [
            self.logical.x * scale,
            self.logical.y * scale,
            self.logical.width * scale,
            self.logical.height * scale,
        ]
    }
}

impl ComponentView for GpuView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: GPU_VIEW_RENDERER.into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let custom = Some(self.custom_render());
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                ..AccessibilityState::default()
            },
        );
    }
}

impl ComponentView for GpuTextureView {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: GPU_TEXTURE_VIEW_RENDERER.into(),
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
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Image,
                ..AccessibilityState::default()
            },
        );
    }
}

fn resource_key(id: u64) -> Arc<str> {
    Arc::from(id.to_string())
}

fn fill_layout(style: &NodeStyle) -> NodeStyle {
    let mut style = style.clone();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    style
}

fn sanitize_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn sanitize_logical(logical: LayoutBox) -> LayoutBox {
    LayoutBox {
        x: logical.x,
        y: logical.y,
        width: sanitize_extent(logical.width),
        height: sanitize_extent(logical.height),
    }
}

fn sanitize_extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn saturating_u32(value: f32) -> u32 {
    value.clamp(0.0, u32::MAX as f32) as u32
}

fn finite_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn finite_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    }
}

fn finite_seed(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, UiWorld};

    fn layout(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x,
            y,
            width,
            height,
        }
    }

    fn mount<C: ComponentView>(component: C) -> (UiWorld, StableNodeId) {
        let mut world = UiWorld::new();
        let id = StableNodeId::new(1).unwrap();
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id, document, component.node_kind());
        component.project(id, &world, &mut queue);
        world.commit(queue).unwrap();
        (world, id)
    }

    #[test]
    fn render_slot_scale_factor_maps_to_physical_rect() {
        let slot = RenderSlot::new(7, layout(10.25, 20.5, 100.5, 50.25), 1.5);
        assert_eq!(slot.scale, 1.5);
        assert_eq!(
            slot.physical(),
            PhysicalRect {
                x: 15,
                y: 30,
                width: 152,
                height: 77,
            }
        );
        assert_eq!(slot.viewport(), [15.375, 30.75, 150.75, 75.375]);
        assert_eq!(
            slot.clipped_physical(120, 90),
            PhysicalRect {
                x: 15,
                y: 30,
                width: 105,
                height: 60,
            }
        );
    }

    #[test]
    fn render_slot_treats_non_positive_or_non_finite_scale_as_one() {
        let logical = layout(50.0, 60.0, 20.0, 30.0);
        let expected = PhysicalRect {
            x: 50,
            y: 60,
            width: 20,
            height: 30,
        };
        for scale in [0.0, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let slot = RenderSlot::new(1, logical, scale);
            assert_eq!(slot.scale, 1.0, "scale {scale}");
            assert_eq!(slot.physical(), expected, "scale {scale}");
        }
        assert_eq!(
            RenderSlot::new(1, logical, f32::NAN).clipped_physical(10, 10),
            PhysicalRect {
                x: 10,
                y: 10,
                width: 0,
                height: 0,
            }
        );
    }

    #[test]
    fn generation_and_version_bumps_are_independent() {
        let mut texture = GpuTextureView::new("preview");
        let mut view = GpuView::new(11).mode(GpuViewMode::Standalone);
        assert_eq!(texture.generation, 0);
        assert_eq!(texture.version, 0);
        assert_eq!(view.generation, 0);
        assert_eq!(view.version, 0);

        assert_eq!(texture.invalidate_content(), 1);
        assert_eq!(view.invalidate_content(), 1);
        assert_eq!(texture.generation, 0);
        assert_eq!(view.generation, 0);
        assert_eq!(texture.version, 1);
        assert_eq!(view.version, 1);

        assert_eq!(texture.replace_view(4), 4);
        assert_eq!(view.replace_view(4), 4);
        assert_eq!(texture.generation, 4);
        assert_eq!(view.generation, 4);
        assert_eq!(texture.version, 1);
        assert_eq!(view.version, 1);

        assert_eq!(texture.invalidate_content(), 2);
        assert_eq!(texture.generation, 4);
        assert_eq!(texture.version, 2);
    }

    #[test]
    fn pack_gpu_revision_keeps_generation_and_version_independent() {
        assert_eq!(pack_gpu_revision(0, 0), 0);
        assert_eq!(pack_gpu_revision(1, 0), 1 << 32);
        assert_eq!(pack_gpu_revision(0, 1), 1);
        assert_eq!(unpack_gpu_revision(pack_gpu_revision(7, 9)), (7, 9));
        assert_eq!(
            pack_gpu_revision(3, 2).wrapping_add(1),
            pack_gpu_revision(3, 3)
        );
        assert_eq!(
            pack_gpu_revision(3, 2).wrapping_add(1 << 32),
            pack_gpu_revision(4, 2)
        );
        assert_eq!(pack_gpu_revision(1 << 32, 1 << 32), 0);
        assert_eq!(
            unpack_gpu_revision(pack_gpu_revision(u64::MAX, u64::MAX)),
            (0xFFFF_FFFF, 0xFFFF_FFFF)
        );
    }

    #[test]
    fn texture_view_uses_host_texture_renderer_and_registry_resource() {
        let mut texture = GpuTextureView::new("preview-slot")
            .with_opacity(0.5)
            .with_corner_radius(8.0);
        texture.replace_view(3);
        texture.invalidate_content();
        texture.invalidate_content();

        let node = texture.custom_render().expect("non-empty host slot");
        assert_eq!(node.renderer.as_ref(), "nana.host-texture");
        assert_eq!(node.renderer.as_ref(), GPU_TEXTURE_VIEW_RENDERER);
        assert_eq!(node.renderer.as_ref(), crate::HOST_TEXTURE_RENDERER);
        assert_ne!(node.renderer.as_ref(), "gpu-texture-view");
        assert_eq!(node.resource.as_ref(), "preview-slot");
        assert_eq!(node.revision, pack_gpu_revision(3, 2));
        assert_eq!(unpack_gpu_revision(node.revision), (3, 2));
        assert_eq!(node.revision, (3 << 32) | 2);

        let decimal = GpuTextureView::new("42").custom_render().unwrap();
        assert_eq!(decimal.renderer.as_ref(), "nana.host-texture");
        assert_eq!(decimal.resource.as_ref(), "42");
        assert_eq!(decimal.revision, pack_gpu_revision(0, 0));

        assert!(GpuTextureView::new("").custom_render().is_none());
        assert!(GpuTextureView::new("   ").custom_render().is_none());
        let (world, id) = mount(GpuTextureView::new(""));
        assert!(world.custom_render(id).is_none());
    }

    #[test]
    fn custom_render_node_encodes_packed_revision() {
        let mut view = GpuView::new(9)
            .mode(GpuViewMode::Standalone)
            .palette(GpuViewPalette {
                background: [0.1, 0.2, 0.3, 1.0],
                accent: [0.9, 0.8, 0.7, 1.0],
            })
            .seed(0.34);
        view.replace_view(5);
        view.invalidate_content();
        let node = view.custom_render();
        assert_eq!(node.renderer.as_ref(), GPU_VIEW_RENDERER);
        assert_eq!(node.renderer.as_ref(), "gpu-view");
        assert_eq!(node.resource.as_ref(), "9");
        assert_eq!(node.revision, pack_gpu_revision(5, 1));
        assert_eq!(unpack_gpu_revision(node.revision), (5, 1));
    }

    #[test]
    fn gpu_view_mode_is_retained_across_identity_updates() {
        let mut view = GpuView::new(7).mode(GpuViewMode::Standalone);
        assert_eq!(view.mode, GpuViewMode::Standalone);
        view.invalidate_content();
        view.replace_view(2);
        view.seed = 1.25;
        view.palette.accent = [0.2, 0.4, 0.6, 1.0];
        assert_eq!(view.mode, GpuViewMode::Standalone);
        assert_eq!(view.slot_id, 7);
        assert_eq!(view.generation, 2);
        assert_eq!(view.version, 1);

        let (world, id) = mount(view.clone());
        assert_eq!(world.custom_render(id), Some(&view.custom_render()));
        assert_eq!(view.custom_render().revision, pack_gpu_revision(2, 1));
        assert_eq!(view.custom_render().renderer.as_ref(), GPU_VIEW_RENDERER);
    }

    #[test]
    fn gpu_view_project_attaches_custom_render() {
        let view = GpuView::new(1);
        let (world, id) = mount(view.clone());
        let node = world
            .custom_render(id)
            .expect("GpuView project attaches custom_render");
        assert_eq!(node.renderer.as_ref(), "gpu-view");
        assert_eq!(node.resource.as_ref(), "1");
        assert_eq!(node.revision, pack_gpu_revision(0, 0));
        assert_eq!(*node, view.custom_render());
    }

    #[test]
    fn component_view_projects_host_texture_node_and_gpu_view_layout() {
        let texture = GpuTextureView::new("preview")
            .with_opacity(0.25)
            .with_corner_radius(6.0);
        let (world, id) = mount(texture.clone());
        assert!(matches!(
            world.node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == GPU_TEXTURE_VIEW_RENDERER
        ));
        assert_eq!(world.custom_render(id), texture.custom_render().as_ref());
        assert_eq!(
            world.custom_render(id).map(|node| node.renderer.as_ref()),
            Some("nana.host-texture")
        );
        let style = world.node_style(id).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(style.layout.height, Some(LengthSpec::Fill));
        assert_eq!(style.layout.opacity, Some(0.25));
        assert_eq!(style.layout.border_radius, Some(6.0));
        assert_eq!(
            world.interaction(id),
            Some(InteractionState {
                pointer_events: false,
                focusable: false,
            })
        );

        let view = GpuView::new(8).mode(GpuViewMode::Inline);
        let (world, id) = mount(view.clone());
        assert!(matches!(
            world.node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == GPU_VIEW_RENDERER
        ));
        assert_eq!(world.custom_render(id), Some(&view.custom_render()));
        assert_eq!(
            world.custom_render(id).map(|node| node.renderer.as_ref()),
            Some(GPU_VIEW_RENDERER)
        );
        assert_eq!(
            world.custom_render(id).map(|node| node.resource.as_ref()),
            Some("8")
        );
        assert_eq!(
            world.node_style(id).unwrap().layout.width,
            Some(LengthSpec::Fill)
        );
        assert_eq!(
            world.interaction(id),
            Some(InteractionState {
                pointer_events: true,
                focusable: false,
            })
        );
        let mut idle = MutationQueue::new();
        view.project(id, &world, &mut idle);
        assert!(idle.is_empty());
    }
}
