use std::sync::Arc;

use nana_ui_core::{ControlSize, OverflowSpec, PaintTransform, SemanticColorRole};

use crate::overlay_surfaces::modal_root_style;
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, CustomRenderNode, InteractionState,
    LayoutBox, MutationQueue, NodeKind, NodeStyle, StableNodeId, StandardVisual, TextContent,
    UiWorld,
};

/// Scene HostTexture registry renderer (`resource` is the host slot identity).
pub const HOST_TEXTURE_RENDERER: &str = "nana.host-texture";

/// Iced `ZoomPan` scale step and clamp.
pub const ZOOM_STEP: f32 = 1.12;
pub const ZOOM_MIN: f32 = 1.0;
pub const ZOOM_MAX: f32 = 6.0;

const SURFACE_PAD_TOP: f32 = 54.0;
const SURFACE_PAD_RIGHT: f32 = 54.0;
const SURFACE_PAD_BOTTOM: f32 = 24.0;
const SURFACE_PAD_LEFT: f32 = 54.0;
const CLOSE_INSET: f32 = 14.0;
const METADATA_GAP: f32 = 10.0;
const METADATA_HEIGHT: f32 = 16.0;
const COVERAGE: f32 = 0.75;

/// Close, outside (scrim), and surface interaction are distinct.
/// Escape stays a host subscription, as with [`crate::Dialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageViewerEvent {
    Close,
    Outside,
    Interaction,
}

/// Application-owned visual content. NanaUI never stores pixels or codecs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ImageViewerContent {
    #[default]
    None,
    Child(StableNodeId),
    HostTexture(Arc<str>),
    CustomRender(CustomRenderNode),
}

impl ImageViewerContent {
    pub fn child(id: StableNodeId) -> Self {
        Self::Child(id)
    }

    pub fn host_texture(slot: impl Into<Arc<str>>) -> Self {
        Self::HostTexture(slot.into())
    }

    pub fn custom_render(node: CustomRenderNode) -> Self {
        Self::CustomRender(node)
    }

    pub fn as_child(&self) -> Option<StableNodeId> {
        match self {
            Self::Child(id) => Some(*id),
            _ => None,
        }
    }

    /// HostTexture slots use [`HOST_TEXTURE_RENDERER`]. Empty identities are omitted.
    pub fn as_custom_render(&self) -> Option<CustomRenderNode> {
        match self {
            Self::HostTexture(slot) if !slot.trim().is_empty() => Some(CustomRenderNode {
                renderer: Arc::from(HOST_TEXTURE_RENDERER),
                resource: Arc::clone(slot),
                revision: 0,
            }),
            Self::CustomRender(node)
                if !node.renderer.trim().is_empty() && !node.resource.trim().is_empty() =>
            {
                Some(node.clone())
            }
            _ => None,
        }
    }
}

impl From<StableNodeId> for ImageViewerContent {
    fn from(id: StableNodeId) -> Self {
        Self::Child(id)
    }
}

impl From<CustomRenderNode> for ImageViewerContent {
    fn from(node: CustomRenderNode) -> Self {
        Self::CustomRender(node)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageViewerOffset {
    pub x: f32,
    pub y: f32,
}

impl ImageViewerOffset {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageViewerDrag {
    pub pointer_id: u64,
    pub origin_x: f32,
    pub origin_y: f32,
    pub starting_offset: ImageViewerOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageViewerHit {
    Close,
    Stage,
    Surface,
    Scrim,
    Miss,
}

/// Overlay chrome plus the zoom/pan-transformed content box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageViewerGeometry {
    pub scrim: LayoutBox,
    pub surface: LayoutBox,
    pub stage: LayoutBox,
    pub close: LayoutBox,
    pub name: Option<LayoutBox>,
    pub metadata: Option<LayoutBox>,
    pub content: LayoutBox,
}

impl ImageViewerGeometry {
    pub fn hit(self, x: f32, y: f32) -> ImageViewerHit {
        if self.close.contains(x, y) {
            ImageViewerHit::Close
        } else if self.stage.contains(x, y) {
            ImageViewerHit::Stage
        } else if self.surface.contains(x, y) {
            ImageViewerHit::Surface
        } else if self.scrim.contains(x, y) {
            ImageViewerHit::Scrim
        } else {
            ImageViewerHit::Miss
        }
    }
}

/// Full-window overlay viewer. Application owns decode and HostTexture/content.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageViewer {
    pub name: Option<Arc<str>>,
    pub metadata: Option<Arc<str>>,
    pub content: ImageViewerContent,
    pub zoom: f32,
    pub offset: ImageViewerOffset,
    pub dragging: Option<ImageViewerDrag>,
    pub style: NodeStyle,
}

impl ImageViewer {
    pub fn new(content: impl Into<ImageViewerContent>) -> Self {
        Self {
            name: None,
            metadata: None,
            content: content.into(),
            zoom: ZOOM_MIN,
            offset: ImageViewerOffset::ZERO,
            dragging: None,
            style: overlay_style(),
        }
    }

    pub fn name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn metadata(mut self, metadata: impl Into<Arc<str>>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn close_size() -> f32 {
        ControlSize::Small.height()
    }

    pub fn geometry(&self, bounds: LayoutBox) -> ImageViewerGeometry {
        let surface = inset(
            bounds,
            SURFACE_PAD_LEFT,
            SURFACE_PAD_TOP,
            SURFACE_PAD_RIGHT,
            SURFACE_PAD_BOTTOM,
        );
        let has_name = self.name.is_some();
        let has_metadata = self.metadata.is_some();
        let band = if has_name || has_metadata {
            METADATA_GAP + METADATA_HEIGHT
        } else {
            0.0
        };
        let stage = LayoutBox {
            x: surface.x,
            y: surface.y,
            width: surface.width,
            height: (surface.height - band).max(0.0),
        };
        let (name, metadata) = caption_boxes(surface, has_name, has_metadata);
        let zoom = clamp_zoom(self.zoom);
        let offset = clamp_offset(self.offset, zoom, stage);
        ImageViewerGeometry {
            scrim: bounds,
            surface,
            stage,
            close: close_box(surface),
            name,
            metadata,
            content: transform_about(stage, stage, zoom, offset),
        }
    }

    /// CSS matrix around the stage center: scale(zoom) then translate(offset).
    pub fn content_transform(&self) -> PaintTransform {
        let zoom = clamp_zoom(self.zoom);
        let offset = if zoom <= 1.0 {
            ImageViewerOffset::ZERO
        } else {
            self.offset
        };
        PaintTransform {
            a: zoom,
            d: zoom,
            e: offset.x,
            f: offset.y,
            ..PaintTransform::default()
        }
    }

    /// Applies zoom/pan about `stage` center, matching Iced `ZoomPan` draw.
    pub fn transformed_bounds(&self, content: LayoutBox, stage: LayoutBox) -> LayoutBox {
        let zoom = clamp_zoom(self.zoom);
        transform_about(content, stage, zoom, clamp_offset(self.offset, zoom, stage))
    }

    pub fn pointer_down(
        &mut self,
        geometry: &ImageViewerGeometry,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Option<ImageViewerEvent> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        match geometry.hit(x, y) {
            ImageViewerHit::Close => {
                self.dragging = None;
                Some(ImageViewerEvent::Close)
            }
            ImageViewerHit::Stage => {
                self.begin_pan(pointer_id, x, y);
                Some(ImageViewerEvent::Interaction)
            }
            ImageViewerHit::Surface => {
                self.dragging = None;
                Some(ImageViewerEvent::Interaction)
            }
            ImageViewerHit::Scrim => {
                self.dragging = None;
                Some(ImageViewerEvent::Outside)
            }
            ImageViewerHit::Miss => None,
        }
    }

    pub fn pointer_move(
        &mut self,
        geometry: &ImageViewerGeometry,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> bool {
        let Some(drag) = self.dragging.filter(|drag| drag.pointer_id == pointer_id) else {
            return false;
        };
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        self.offset = clamp_offset(
            ImageViewerOffset::new(
                drag.starting_offset.x + (x - drag.origin_x),
                drag.starting_offset.y + (y - drag.origin_y),
            ),
            clamp_zoom(self.zoom),
            geometry.stage,
        );
        true
    }

    pub fn pointer_up(&mut self, pointer_id: u64) -> bool {
        if self
            .dragging
            .is_some_and(|drag| drag.pointer_id == pointer_id)
        {
            self.dragging = None;
            true
        } else {
            false
        }
    }

    pub fn wheel(&mut self, geometry: &ImageViewerGeometry, x: f32, y: f32, delta_y: f32) -> bool {
        if !geometry.stage.contains(x, y) || !delta_y.is_finite() || delta_y == 0.0 {
            return false;
        }
        let previous = clamp_zoom(self.zoom);
        self.zoom = if delta_y > 0.0 {
            previous * ZOOM_STEP
        } else {
            previous / ZOOM_STEP
        }
        .clamp(ZOOM_MIN, ZOOM_MAX);
        let factor = self.zoom / previous - 1.0;
        let cx = geometry.stage.x + geometry.stage.width * 0.5;
        let cy = geometry.stage.y + geometry.stage.height * 0.5;
        self.offset = clamp_offset(
            ImageViewerOffset::new(
                self.offset.x + (x - cx) * factor + self.offset.x * factor,
                self.offset.y + (y - cy) * factor + self.offset.y * factor,
            ),
            self.zoom,
            geometry.stage,
        );
        true
    }

    fn begin_pan(&mut self, pointer_id: u64, x: f32, y: f32) {
        self.zoom = clamp_zoom(self.zoom);
        if self.zoom > ZOOM_MIN {
            self.dragging = Some(ImageViewerDrag {
                pointer_id,
                origin_x: x,
                origin_y: y,
                starting_offset: self.offset,
            });
        } else {
            self.dragging = None;
            self.offset = ImageViewerOffset::ZERO;
        }
    }
}

impl Default for ImageViewer {
    fn default() -> Self {
        Self::new(ImageViewerContent::None)
    }
}

impl ComponentView for ImageViewer {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "image-viewer".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let visual = StandardVisual::ImageViewer {
            name: self.name.clone(),
            metadata: self.metadata.clone(),
            zoom: self.zoom,
            offset_x: self.offset.x,
            offset_y: self.offset.y,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let label = self.name.clone();
        let text = label.as_deref().unwrap_or_default();
        if world.text(id) != Some(text) {
            mutations.set_text(
                id,
                TextContent {
                    value: text.to_owned(),
                },
            );
        }
        let custom = self.content.as_custom_render();
        if world.custom_render(id) != custom.as_ref() {
            mutations.set_custom_render(id, custom);
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Dialog,
                label,
                description: self.metadata.clone(),
                modal: true,
                ..AccessibilityState::default()
            },
        );
    }
}

impl crate::AppContext {
    pub fn image_viewer_pointer_down(
        &mut self,
        viewer: crate::Entity<ImageViewer>,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<Option<ImageViewerEvent>, crate::FrameworkError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(crate::FrameworkError::InvalidInput);
        }
        let Some(bounds) = self.world().layout_box(viewer.stable_id()) else {
            return Ok(None);
        };
        let id = viewer.stable_id();
        self.update_component(viewer, |viewer, cx| {
            let event = viewer.pointer_down(&viewer.geometry(bounds), pointer_id, x, y);
            if viewer.dragging.is_some() {
                cx.mutations().capture_pointer(pointer_id, id);
            }
            if let Some(event) = event {
                cx.emit(event);
            }
            event
        })
    }

    pub fn image_viewer_pointer_move(
        &mut self,
        viewer: crate::Entity<ImageViewer>,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(crate::FrameworkError::InvalidInput);
        }
        let Some(bounds) = self.world().layout_box(viewer.stable_id()) else {
            return Ok(false);
        };
        self.update_component(viewer, |viewer, _| {
            viewer.pointer_move(&viewer.geometry(bounds), pointer_id, x, y)
        })
    }

    pub fn image_viewer_pointer_up(
        &mut self,
        viewer: crate::Entity<ImageViewer>,
        pointer_id: u64,
    ) -> Result<bool, crate::FrameworkError> {
        let id = viewer.stable_id();
        self.update_component(viewer, |viewer, cx| {
            let ended = viewer.pointer_up(pointer_id);
            if ended {
                cx.mutations().release_pointer(pointer_id, id);
            }
            ended
        })
    }

    pub fn image_viewer_wheel(
        &mut self,
        viewer: crate::Entity<ImageViewer>,
        x: f32,
        y: f32,
        delta_y: f32,
    ) -> Result<bool, crate::FrameworkError> {
        if !x.is_finite() || !y.is_finite() || !delta_y.is_finite() {
            return Err(crate::FrameworkError::InvalidInput);
        }
        let Some(bounds) = self.world().layout_box(viewer.stable_id()) else {
            return Ok(false);
        };
        self.update_component(viewer, |viewer, _| {
            viewer.wheel(&viewer.geometry(bounds), x, y, delta_y)
        })
    }
}

fn overlay_style() -> NodeStyle {
    let mut style = modal_root_style();
    let layout = Arc::make_mut(&mut style.layout);
    layout.overflow_x = OverflowSpec::Hidden;
    layout.overflow_y = OverflowSpec::Hidden;
    style.background = Some(SemanticColorRole::Background);
    style
}

fn inset(bounds: LayoutBox, left: f32, top: f32, right: f32, bottom: f32) -> LayoutBox {
    LayoutBox {
        x: bounds.x + left,
        y: bounds.y + top,
        width: (bounds.width - left - right).max(0.0),
        height: (bounds.height - top - bottom).max(0.0),
    }
}

fn close_box(surface: LayoutBox) -> LayoutBox {
    let size = ImageViewer::close_size();
    LayoutBox {
        x: surface.x + surface.width - CLOSE_INSET - size,
        y: surface.y + CLOSE_INSET,
        width: size,
        height: size,
    }
}

fn caption_boxes(
    surface: LayoutBox,
    has_name: bool,
    has_metadata: bool,
) -> (Option<LayoutBox>, Option<LayoutBox>) {
    if !has_name && !has_metadata {
        return (None, None);
    }
    let y = surface.y + surface.height - METADATA_HEIGHT;
    let row = LayoutBox {
        x: surface.x,
        y,
        width: surface.width,
        height: METADATA_HEIGHT.max(0.0),
    };
    if has_name && has_metadata {
        let gap = METADATA_GAP;
        let half = ((surface.width - gap) / 2.0).max(0.0);
        (
            Some(LayoutBox {
                x: surface.x,
                y,
                width: half,
                height: row.height,
            }),
            Some(LayoutBox {
                x: surface.x + half + gap,
                y,
                width: half,
                height: row.height,
            }),
        )
    } else if has_name {
        (Some(row), None)
    } else {
        (None, Some(row))
    }
}

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    } else {
        ZOOM_MIN
    }
}

fn clamp_offset(offset: ImageViewerOffset, zoom: f32, stage: LayoutBox) -> ImageViewerOffset {
    if zoom <= 1.0 {
        return ImageViewerOffset::ZERO;
    }
    ImageViewerOffset::new(
        clamp_axis(offset.x, stage.width * zoom, stage.width),
        clamp_axis(offset.y, stage.height * zoom, stage.height),
    )
}

fn clamp_axis(value: f32, rendered: f32, viewport: f32) -> f32 {
    let required_coverage = viewport * COVERAGE;
    let max = ((viewport + rendered) / 2.0 - required_coverage).max(0.0);
    if value.is_finite() {
        value.clamp(-max, max)
    } else {
        0.0
    }
}

fn transform_about(
    bounds: LayoutBox,
    stage: LayoutBox,
    zoom: f32,
    offset: ImageViewerOffset,
) -> LayoutBox {
    let cx = stage.x + stage.width * 0.5;
    let cy = stage.y + stage.height * 0.5;
    LayoutBox {
        x: zoom * (bounds.x - cx) + cx + offset.x,
        y: zoom * (bounds.y - cy) + cy + offset.y,
        width: (bounds.width * zoom).max(0.0),
        height: (bounds.height * zoom).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, OverlayHost};
    use std::sync::{Arc, Mutex};

    fn bounds() -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        }
    }

    fn stage_point(geometry: &ImageViewerGeometry, nx: f32, ny: f32) -> (f32, f32) {
        (
            geometry.stage.x + geometry.stage.width * nx,
            geometry.stage.y + geometry.stage.height * ny,
        )
    }

    fn write_layout(context: &mut AppContext, id: StableNodeId, layout: LayoutBox) {
        let mut mutations = MutationQueue::new();
        mutations.write_layout(id, layout);
        context.commit_mutations(mutations).unwrap();
    }

    #[test]
    fn wheel_zoom_around_a_point_changes_zoom_and_offset() {
        let mut viewer = ImageViewer::new(ImageViewerContent::None);
        let geometry = viewer.geometry(bounds());
        let (x, y) = stage_point(&geometry, 0.75, 0.5);
        assert!(viewer.wheel(&geometry, x, y, 1.0));
        assert!((viewer.zoom - ZOOM_STEP).abs() < 1e-6);
        let factor = ZOOM_STEP / ZOOM_MIN - 1.0;
        let cx = geometry.stage.x + geometry.stage.width * 0.5;
        assert!((viewer.offset.x - (x - cx) * factor).abs() < 1e-5);
        assert!(viewer.offset.y.abs() < 1e-5);
        assert!(viewer.geometry(bounds()).content.width > geometry.stage.width);
    }

    #[test]
    fn pan_updates_offset() {
        let mut viewer = ImageViewer::new(ImageViewerContent::None);
        viewer.zoom = 2.0;
        let geometry = viewer.geometry(bounds());
        let (x, y) = stage_point(&geometry, 0.5, 0.5);
        assert_eq!(
            viewer.pointer_down(&geometry, 1, x, y),
            Some(ImageViewerEvent::Interaction)
        );
        assert!(viewer.pointer_move(&geometry, 1, x + 20.0, y - 16.0));
        assert!((viewer.offset.x - 20.0).abs() < 1e-5);
        assert!((viewer.offset.y + 16.0).abs() < 1e-5);
        assert!(viewer.pointer_up(1));
        assert!(viewer.dragging.is_none());
    }

    #[test]
    fn close_and_outside_are_distinct_events() {
        let mut viewer = ImageViewer::new(ImageViewerContent::None)
            .name("preview")
            .metadata("1600 × 900");
        let geometry = viewer.geometry(bounds());
        let close = (
            geometry.close.x + geometry.close.width * 0.5,
            geometry.close.y + geometry.close.height * 0.5,
        );
        let (stage_x, stage_y) = stage_point(&geometry, 0.4, 0.4);
        assert_eq!(
            viewer.pointer_down(&geometry, 1, close.0, close.1),
            Some(ImageViewerEvent::Close)
        );
        assert_eq!(
            viewer.pointer_down(&geometry, 2, 8.0, 8.0),
            Some(ImageViewerEvent::Outside)
        );
        assert_eq!(
            viewer.pointer_down(&geometry, 3, stage_x, stage_y),
            Some(ImageViewerEvent::Interaction)
        );
        assert_ne!(ImageViewerEvent::Close, ImageViewerEvent::Outside);
    }

    #[test]
    fn zoom_clamps_to_iced_min_max() {
        let mut viewer = ImageViewer::new(ImageViewerContent::None);
        let geometry = viewer.geometry(bounds());
        let (x, y) = stage_point(&geometry, 0.2, 0.3);
        for _ in 0..64 {
            viewer.wheel(&geometry, x, y, 1.0);
        }
        assert_eq!(viewer.zoom, ZOOM_MAX);
        for _ in 0..64 {
            viewer.wheel(&geometry, x, y, -1.0);
        }
        assert_eq!(viewer.zoom, ZOOM_MIN);
        assert_eq!(viewer.offset, ImageViewerOffset::ZERO);
        viewer.zoom = 99.0;
        viewer.wheel(&geometry, x, y, 1.0);
        assert_eq!(viewer.zoom, ZOOM_MAX);
    }

    #[test]
    fn pan_clamp_keeps_required_content_coverage_visible() {
        let stage = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        assert_eq!(
            clamp_offset(ImageViewerOffset::new(500.0, -500.0), 2.0, stage),
            ImageViewerOffset::new(75.0, -60.0)
        );
        assert_eq!(
            clamp_offset(ImageViewerOffset::new(20.0, 20.0), 1.0, stage),
            ImageViewerOffset::ZERO
        );
    }

    #[test]
    fn zoom_pan_transform_is_applied_to_content_bounds() {
        let mut viewer = ImageViewer::new(ImageViewerContent::None);
        viewer.zoom = 2.0;
        viewer.offset = ImageViewerOffset::new(10.0, -4.0);
        let stage = viewer.geometry(bounds()).stage;
        let content = viewer.transformed_bounds(stage, stage);
        assert!((content.width - stage.width * 2.0).abs() < 1e-5);
        assert!((content.height - stage.height * 2.0).abs() < 1e-5);
        assert!((content.x - (stage.x - stage.width * 0.5 + 10.0)).abs() < 1e-5);
        assert!((content.y - (stage.y - stage.height * 0.5 - 4.0)).abs() < 1e-5);
        let matrix = viewer.content_transform();
        assert_eq!(matrix.a, 2.0);
        assert_eq!(matrix.d, 2.0);
        assert_eq!(matrix.e, 10.0);
        assert_eq!(matrix.f, -4.0);
    }

    #[test]
    fn host_texture_projects_custom_render_identity() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let viewer = context
            .create_component(
                document,
                ImageViewer::new(ImageViewerContent::host_texture("preview-slot"))
                    .name("NanaUI 渲染预览"),
            )
            .unwrap();
        let custom = context
            .world()
            .custom_render(viewer.stable_id())
            .cloned()
            .unwrap();
        assert_eq!(custom.renderer.as_ref(), HOST_TEXTURE_RENDERER);
        assert_eq!(custom.resource.as_ref(), "preview-slot");
        let accessibility = context.world().accessibility(viewer.stable_id()).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Dialog);
        assert!(accessibility.modal);
        assert_eq!(accessibility.label.as_deref(), Some("NanaUI 渲染预览"));
        assert!(matches!(
            context.world().node(viewer.stable_id()).unwrap().kind,
            NodeKind::Element { tag } if tag == "image-viewer"
        ));
        assert!(matches!(
            context.world().standard_visual(viewer.stable_id()),
            Some(StandardVisual::ImageViewer {
                ref name,
                ref metadata,
                zoom,
                offset_x,
                offset_y,
            }) if name.as_deref() == Some("NanaUI 渲染预览")
                && metadata.is_none()
                && zoom == ZOOM_MIN
                && offset_x == 0.0
                && offset_y == 0.0
        ));

        let child = context
            .create_detached_component(document, crate::Button::new("decoded"))
            .unwrap();
        let slotted = context
            .create_component(
                document,
                ImageViewer::new(ImageViewerContent::child(child.stable_id())),
            )
            .unwrap();
        assert!(context.world().custom_render(slotted.stable_id()).is_none());
        assert!(matches!(
            context.world().standard_visual(slotted.stable_id()),
            Some(StandardVisual::ImageViewer { .. })
        ));
    }

    #[test]
    fn overlay_host_can_activate_the_viewer() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let viewer = context
            .create_component(
                document,
                ImageViewer::new(ImageViewerContent::None).name("preview"),
            )
            .unwrap();
        context.append_child(host, viewer).unwrap();
        assert!(context.activate_overlay(host, viewer).unwrap());
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            Some(viewer.stable_id())
        );
    }

    #[test]
    fn app_context_emits_close_and_outside() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let viewer = context
            .create_component(document, ImageViewer::new(ImageViewerContent::None))
            .unwrap();
        write_layout(&mut context, viewer.stable_id(), bounds());
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(viewer, move |_viewer, event: &ImageViewerEvent, _cx| {
                observed.lock().unwrap().push(*event);
            })
            .unwrap();
        let geometry = ImageViewer::new(ImageViewerContent::None).geometry(bounds());
        let close = (geometry.close.x + 2.0, geometry.close.y + 2.0);
        assert_eq!(
            context
                .image_viewer_pointer_down(viewer, 1, close.0, close.1)
                .unwrap(),
            Some(ImageViewerEvent::Close)
        );
        assert_eq!(
            context
                .image_viewer_pointer_down(viewer, 2, 4.0, 4.0)
                .unwrap(),
            Some(ImageViewerEvent::Outside)
        );
        assert_eq!(
            *events.lock().unwrap(),
            [ImageViewerEvent::Close, ImageViewerEvent::Outside]
        );
    }
}
