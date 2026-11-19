//! Open component visual & inline GPU payload contracts.
//!
//! Instead of adding a closed enum arm to `StandardVisual` / `ComponentGeometry` /
//! `ScenePrimitiveKind` for every control, widgets implement [`WidgetVisual`]
//! directly in their own modules and emit a [`WidgetPaintOutput`] composed of
//! `UiScene`'s 5 core primitive families (`Quad`/`QuadBatch`, `Text`, `Icon`,
//! `Image`, and `Stroke`/`Vector`/`CustomGpu`) in **node-local** coordinates.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use nana_ui_core::{CompiledTheme, StyleModelRef};

use crate::{
    ComponentElevation, ComponentTextRegion, ComputedStyle, LayoutBox, LineCap, NodeStyle,
    PaintRecording, SemanticColorRole, StableNodeId, TextHorizontalAlignment, TextValue,
    TextVerticalAlignment, UiWorld,
};

/// Renderer key used by [`crate::CustomRenderNode`] when a `ChartVisual`
/// forwards [`nana_ui_charts::ChartGpuPayload`] to `SceneWgpuPainter`.
pub const CHART_RENDERER: &str = "nana.chart";

trait CustomGpuPayloadData: Any + fmt::Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn dyn_eq(&self, other: &dyn Any) -> bool;
}

impl<T> CustomGpuPayloadData for T
where
    T: Any + fmt::Debug + PartialEq + Send + Sync + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn dyn_eq(&self, other: &dyn Any) -> bool {
        other.downcast_ref::<T>().is_some_and(|rhs| self == rhs)
    }
}

/// Type-erased, backend-neutral payload carried on [`crate::CustomRenderNode`]
/// so component crates (such as `nana-ui-charts`) never leak types into `nana-ui-scene`.
#[derive(Clone)]
pub struct CustomGpuPayload(Arc<dyn CustomGpuPayloadData>);

impl CustomGpuPayload {
    /// Wraps a typed payload in a shared, equality-comparable handle.
    pub fn new<T>(payload: T) -> Self
    where
        T: Any + fmt::Debug + PartialEq + Send + Sync + 'static,
    {
        Self(Arc::new(payload))
    }

    /// Downcasts the underlying payload to `T`.
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref::<T>()
    }
}

impl fmt::Debug for CustomGpuPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for CustomGpuPayload {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.dyn_eq(other.0.as_any())
    }
}

/// Read-only context passed into [`WidgetVisual::paint`].
///
/// All coordinates on `bounds` and `content` are **node-local** (`bounds.x == 0.0`,
/// `bounds.y == 0.0`) so moving an ancestor never invalidates extracted records.
pub struct WidgetPaintCtx<'a> {
    pub id: StableNodeId,
    pub world: &'a UiWorld,
    pub bounds: LayoutBox,
    pub content: LayoutBox,
    pub style: &'a ComputedStyle,
    pub source_style: &'a NodeStyle,
    pub style_model: StyleModelRef,
    pub theme: &'a CompiledTheme,
    pub chrome_radii: nana_ui_core::ChromeRadii,
}

impl WidgetPaintCtx<'_> {
    /// Resolved primary text color for this node in its current color scope.
    pub fn text_color(&self) -> [f32; 4] {
        self.style
            .paint_colors
            .color
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(self.style.color)
            .unwrap_or_else(|| {
                self.style_model
                    .color(SemanticColorRole::Text)
                    .as_rgba_array()
            })
    }

    /// Matching authored wide-gamut text paint color when `target` still matches it.
    pub fn matching_text_paint(&self, target: [f32; 4]) -> Option<nana_ui_core::PaintColor> {
        matching_paint_color(self.style.paint_colors.color, target)
    }

    /// Matching authored wide-gamut background paint color when `target` still matches it.
    pub fn matching_background_paint(&self, target: [f32; 4]) -> Option<nana_ui_core::PaintColor> {
        matching_paint_color(self.style.paint_colors.background, target)
    }

    /// Matching authored wide-gamut border paint color when `target` still matches it.
    pub fn matching_border_paint(&self, target: [f32; 4]) -> Option<nana_ui_core::PaintColor> {
        matching_paint_color(self.style.paint_colors.border, target)
    }

    pub fn secondary_text_color(&self) -> [f32; 4] {
        self.world.secondary_text_color(self.id)
    }

    pub fn is_focus_visible(&self) -> bool {
        self.world.is_focus_visible_node(self.id)
    }

    pub fn is_focused(&self) -> bool {
        self.world.is_focused_node(self.id)
    }

    pub fn content_part(&self, kind: crate::LayoutPartKind) -> Option<LayoutBox> {
        self.world.content_part_box(self.id, kind)
    }

    pub fn content_parts(&self) -> crate::layout_engine::parts::ContentParts {
        self.world.component_content_parts(self.id)
    }

    pub fn slot_box_from(&self, slot: StableNodeId) -> Option<LayoutBox> {
        self.world.layout_box_from(self.id, slot)
    }

    /// Layout box of `slot` in this widget's own coordinate space (`bounds.x`/`y`
    /// plus `slot`'s local offset), returning `None` if `slot` omits a box.
    pub fn slot_box(&self, slot: StableNodeId) -> Option<LayoutBox> {
        if self
            .world
            .node_style(slot)
            .is_some_and(|style| style.layout.omits_box())
        {
            return None;
        }
        let local = self.slot_box_from(slot)?;
        Some(LayoutBox {
            x: self.bounds.x + local.x,
            y: self.bounds.y + local.y,
            ..local
        })
    }
}

pub(crate) fn matching_paint_color(
    authored: Option<nana_ui_core::PaintColor>,
    target: [f32; 4],
) -> Option<nana_ui_core::PaintColor> {
    authored.filter(|paint| paint.to_srgb() == target)
}

/// Intrinsics contributed by a [`WidgetVisual`] beside node text during measure.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WidgetVisualContent {
    pub width: f32,
    pub height: f32,
    pub text: f32,
}

/// Overrides applied to the node's own `slot: 0` background/border/elevation surface.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetRootSurface {
    pub background: Option<Option<[f32; 4]>>,
    pub border_color: Option<Option<[f32; 4]>>,
    pub border_width: Option<f32>,
    pub fallback_elevation: Option<ComponentElevation>,
}

/// Policy controlling whether child elements' plain text is suppressed when the parent paints text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WidgetChildTextSuppression {
    #[default]
    RespectOwnsNodeText,
    Suppress,
    Allow,
    /// Suppress only when the child plain text matches this value (for example a Card's title).
    Matching(Arc<str>),
}

/// Slot namespace reserved for [`ScrollView`] chrome overlays so bars sort above
/// container descendants.
pub const SCROLLBAR_CHROME_SLOT_BASE: u64 = 0xFFFF_FFF8_u64 << 32;

/// Tags one scrollbar chrome primitive slot (`0..=3`: H track/thumb, V track/thumb).
pub const fn scrollbar_chrome_slot(index: u64) -> u64 {
    SCROLLBAR_CHROME_SLOT_BASE | index
}

/// Slot reserved for modal scrim primitives whose opacity fades independently
/// of the modal card's compositor opacity layer.
pub const MODAL_SCRIM_SLOT: u64 = 10;

/// Slot reserved for a triggered overlay's floating surface wrapped around
/// viewport-fixed content above the page.
pub const TRIGGERED_OVERLAY_SURFACE_SLOT: u64 = 0xFFFF_FFE0_u64 << 32;

/// Popup menu surface slot (`Select`, `Dropdown`, `SearchDropdown`).
pub const SELECT_MENU_SURFACE_SLOT: u64 = 8;
pub const SELECT_MENU_BACKGROUNDS: u32 = 9;
pub const SELECT_MENU_MARKS: u32 = 10;
pub const SELECT_MENU_LABELS: u32 = 11;
pub const SELECT_MENU_HINTS: u32 = 12;
pub const SELECT_MENU_SCROLLBAR: u32 = 13;
pub const CONTEXT_MENU_HINTS: u32 = 14;
pub const CONTEXT_MENU_IMAGE_BACKDROPS: u32 = 15;
pub const CONTEXT_MENU_IMAGES: u32 = 16;
pub const SELECT_MENU_IMAGE_BACKDROPS: u32 = 17;
pub const SELECT_MENU_IMAGES: u32 = 18;

pub const TEXT_LINE_LABELS_NS: u32 = 1;
pub const TEXT_DIAGNOSTIC_MARKERS_NS: u32 = 2;
pub const TEXT_DIAGNOSTIC_LABELS_NS: u32 = 3;
pub const TEXT_ATOM_ICONS_NS: u32 = 4;
pub const TEXT_ATOM_LABELS_NS: u32 = 5;

/// Namespace base (`0xFFFF_FE00 | slot`) allowing a [`WidgetPaintOp::Vector`] in
/// slot `1..=255` to sort at its own fixed component slot layer.
pub const WIDGET_VECTOR_LAYER_BASE: u32 = 0xFFFF_FE00;

/// Packs an unbounded collection index into a scene slot under `namespace`.
pub fn widget_collection_slot(namespace: u32, index: usize) -> u64 {
    debug_assert!(namespace != 0);
    (u64::from(namespace) << 32)
        | u64::from(u32::try_from(index).expect("primitive collection exceeds u32::MAX items"))
}

/// Whether `slot` belongs to an attached popup menu (`SELECT_MENU_*` namespaces or surface slot).
pub const fn is_select_menu_slot(slot: u64) -> bool {
    slot == SELECT_MENU_SURFACE_SLOT
        || (slot >> 32 >= SELECT_MENU_BACKGROUNDS as u64
            && slot >> 32 <= SELECT_MENU_SCROLLBAR as u64)
        || (slot >> 32 >= SELECT_MENU_IMAGE_BACKDROPS as u64
            && slot >> 32 <= SELECT_MENU_IMAGES as u64)
}

/// Classification for floating overlay surfaces emitted by a [`WidgetVisual`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetFloatingOverlayKind {
    /// Node is the trigger while the overlay surface and non-trigger children float.
    TriggeredSurface,
    /// Inline trigger node with an attached floating popup list.
    AttachedPopup,
}

/// Floating overlay routing metadata carried on [`WidgetPaintOutput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetFloatingOverlay {
    pub kind: WidgetFloatingOverlayKind,
    pub overlay_mount: nana_ui_core::OverlayMount,
    pub trigger_content: Option<StableNodeId>,
}

/// Modal frame surface/body clipping and scrim isolation metadata carried on [`WidgetPaintOutput`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidgetModalFrame {
    pub surface: LayoutBox,
    pub body: LayoutBox,
    pub body_root: Option<StableNodeId>,
    pub scrim_slot: Option<u64>,
}

/// Whether `slot` belongs to the floating overlay layer of `paint`.
pub fn is_floating_overlay_slot(paint: &WidgetPaintOutput, slot: u64) -> bool {
    match paint.floating_overlay.as_ref().map(|overlay| overlay.kind) {
        Some(WidgetFloatingOverlayKind::TriggeredSurface) => slot == TRIGGERED_OVERLAY_SURFACE_SLOT,
        Some(WidgetFloatingOverlayKind::AttachedPopup) => is_select_menu_slot(slot),
        None => false,
    }
}

/// Output produced by [`WidgetVisual::paint`] in node-local logical coordinates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetPaintOutput {
    pub ops: Vec<WidgetTaggedOp>,
    /// When `true`, `UiScene` suppresses default node text emission because this
    /// widget projects its own text primitives.
    pub owns_node_text: bool,
    /// When `true`, `UiScene` suppresses the automatic node-style root background
    /// quad (`slot: 0`) while still honoring CSS surface paint / borders.
    pub suppress_root_surface: bool,
    /// When `true`, `UiScene` suppresses even CSS surface/border slot `0` paint
    /// because the widget renders its own surface primitive.
    pub suppress_css_root_surface: bool,
    /// When `true`, `UiScene` suppresses automatic `node.custom_render` emission at
    /// slot `1` because the widget emits its own [`WidgetPaintOp::CustomGpu`].
    pub owns_custom_render: bool,
    /// When `true`, this scroll container paints scrollbar chrome above descendants.
    pub has_scrollbar_chrome: bool,
    /// When `true`, in-flow descendants under this node are skipped because the
    /// painted glyph replaces vector child markup.
    pub suppresses_in_flow_descendants: bool,
    /// When `true`, `UiScene` clips descendant primitives to this node's layout box.
    pub clips_descendants: bool,
    /// When `true`, compact child [`WidgetIcon`] nodes with `align_to_adjacent_text`
    /// treat this parent as a text host (using `node_text_bounds` when set, else its box).
    pub aligns_child_icon_to_text: bool,
    /// When `true`, emitted geometry is already resolved in page coordinates.
    pub geometry_on_page: bool,
    pub root_surface: Option<WidgetRootSurface>,
    pub node_text_bounds: Option<LayoutBox>,
    pub child_text_suppression: WidgetChildTextSuppression,
    pub presentation_pivot: Option<[f32; 2]>,
    pub floating_overlay: Option<WidgetFloatingOverlay>,
    pub modal_frame: Option<WidgetModalFrame>,
    pub content_clip: Option<LayoutBox>,
}

impl WidgetPaintOutput {
    /// Creates an empty output.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a tagged paint operation in emission order.
    pub fn push(&mut self, slot: u64, op: WidgetPaintOp) {
        self.ops.push(WidgetTaggedOp::new(slot, op));
    }

    /// Pushes an already-configured [`WidgetTaggedOp`] in emission order.
    pub fn push_tagged(&mut self, tagged: WidgetTaggedOp) {
        self.ops.push(tagged);
    }
}

/// One node-local paint operation tagged with its scene primitive slot and placement modifiers.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetTaggedOp {
    pub slot: u64,
    pub op: WidgetPaintOp,
    pub z_offset: i32,
    pub min_z_index: Option<i32>,
    pub use_parent_clips: bool,
    pub extra_clip: Option<LayoutBox>,
    pub in_floating_overlay: bool,
    pub use_content_clip: bool,
    pub rounded_extra_clip: Option<(LayoutBox, f32)>,
    pub opacity_multiplier: f32,
    pub local_translation: Option<[f32; 2]>,
}

impl WidgetTaggedOp {
    /// Wraps `op` at `slot` with default placement and clipping.
    pub fn new(slot: u64, op: WidgetPaintOp) -> Self {
        Self {
            slot,
            op,
            z_offset: 0,
            min_z_index: None,
            use_parent_clips: false,
            extra_clip: None,
            in_floating_overlay: false,
            use_content_clip: false,
            rounded_extra_clip: None,
            opacity_multiplier: 1.0,
            local_translation: None,
        }
    }

    /// Offsets this primitive's scene `z_index` relative to `node.z_index`.
    pub fn with_z_offset(mut self, offset: i32) -> Self {
        self.z_offset = offset;
        self
    }

    /// Clamps this primitive's scene `z_index` to at least `min_z`.
    pub fn with_min_z_index(mut self, min_z: i32) -> Self {
        self.min_z_index = Some(min_z);
        self
    }

    /// Emits this primitive under the node's parent clip chain instead of its own clips.
    pub fn with_parent_clips(mut self) -> Self {
        self.use_parent_clips = true;
        self
    }

    /// Appends a node-local axis-aligned clip rect to this primitive's clip chain.
    pub fn with_extra_clip(mut self, clip: LayoutBox) -> Self {
        self.extra_clip = Some(clip);
        self
    }

    /// Marks this primitive as part of the widget's floating overlay surface/layer.
    pub fn with_floating_overlay(mut self) -> Self {
        self.in_floating_overlay = true;
        self
    }

    /// Emits this primitive under [`WidgetPaintOutput::content_clip`] instead of outer clips.
    pub fn with_content_clip(mut self) -> Self {
        self.use_content_clip = true;
        self
    }

    /// Appends a rounded clip region (e.g. popup viewport inset from surface border).
    pub fn with_rounded_extra_clip(mut self, clip: LayoutBox, radius: f32) -> Self {
        self.rounded_extra_clip = Some((clip, radius));
        self
    }

    /// Multiplies the inherited primitive opacity by `opacity`.
    pub fn with_opacity_multiplier(mut self, opacity: f32) -> Self {
        self.opacity_multiplier = opacity;
        self
    }

    /// Applies a post-clip local translation `[tx, ty]` to the primitive transform.
    pub fn with_local_translation(mut self, tx: f32, ty: f32) -> Self {
        self.local_translation = Some([tx, ty]);
        self
    }
}

/// Core drawable primitive families accepted by `UiScene` from any [`WidgetVisual`].
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetPaintOp {
    Quad(WidgetQuad),
    QuadBatch(WidgetQuadBatch),
    Text(WidgetText),
    Icon(WidgetIcon),
    IconBatch {
        bounds: LayoutBox,
        rects: Arc<[LayoutBox]>,
        icon: nana_ui_core::Icon,
        color: Option<[f32; 4]>,
    },
    Image(WidgetImage),
    Stroke(WidgetStroke),
    Vector {
        bounds: LayoutBox,
        recording: Arc<PaintRecording>,
    },
    CustomGpu {
        bounds: LayoutBox,
        node: crate::CustomRenderNode,
    },
    FocusRing {
        bounds: LayoutBox,
        color: [f32; 4],
        extra_outset: f32,
    },
}

/// Rectangle primitive with optional border, corners, and elevation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetQuad {
    pub rect: LayoutBox,
    pub fill: Option<[f32; 4]>,
    pub fill_paint: Option<nana_ui_core::PaintColor>,
    pub border: Option<([f32; 4], f32)>,
    pub border_paint: Option<nana_ui_core::PaintColor>,
    pub corner_radius: [f32; 4],
    pub elevation: Option<ComponentElevation>,
    pub use_parent_clips: bool,
    pub content_image: Option<nana_ui_core::BackgroundImage>,
    pub background_image: Option<nana_ui_core::BackgroundImage>,
    pub backdrop_blur: f32,
}

/// Color source for [`WidgetQuadBatch`].
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetBatchColors {
    Uniform([f32; 4]),
    Selection([f32; 4]),
    PerQuad(Arc<[[f32; 4]]>),
    None,
}

/// Batched rectangle primitive sharing corner radius and optional border.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetQuadBatch {
    pub bounds: LayoutBox,
    pub rects: Arc<[LayoutBox]>,
    pub colors: WidgetBatchColors,
    pub fill_paint: Option<nana_ui_core::PaintColor>,
    pub border: Option<([f32; 4], f32)>,
    pub corner_radius: [f32; 4],
}

/// Policy for adopting a node's retained shaped [`RetainedTextLayout`] onto [`WidgetText`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetRetainedText {
    /// Shape from this op's plain string & styling; ignore `node.text_layout`.
    #[default]
    None,
    /// Adopt `node.text_layout` when rich text or child nodes are present and `node.text` matches.
    NodeTextIfRich,
    /// Always adopt `node.text_layout` when `node.text` matches and clip overflow to bounds.
    AlwaysClipOverflow,
}

/// Positioned text run primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetText {
    pub bounds: LayoutBox,
    pub content: TextValue,
    pub color: Option<[f32; 4]>,
    pub paint_color: Option<nana_ui_core::PaintColor>,
    pub font_size: f32,
    pub font_weight: Option<u16>,
    pub horizontal_alignment: TextHorizontalAlignment,
    pub vertical_alignment: TextVerticalAlignment,
    pub ellipsis: bool,
    pub wrap: bool,
    pub max_lines: Option<usize>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub line_through: Option<bool>,
    pub suppress_spans: bool,
    pub preserve_lines: Option<bool>,
    pub unstyled_glyph: bool,
    pub line_height: Option<Option<nana_ui_core::LineHeightSpec>>,
    pub retained_text: WidgetRetainedText,
    pub tabular_nums: bool,
    pub adopt_node_text_spans: bool,
    pub advanced_shaping: bool,
    pub rich_text: Option<nana_ui_core::RichText>,
    pub force_horizontal_writing: bool,
    pub overlay_chrome: bool,
}

impl Default for WidgetText {
    fn default() -> Self {
        Self {
            bounds: LayoutBox::default(),
            content: TextValue::from(""),
            color: None,
            paint_color: None,
            font_size: 14.0,
            font_weight: None,
            horizontal_alignment: TextHorizontalAlignment::default(),
            vertical_alignment: TextVerticalAlignment::default(),
            ellipsis: false,
            wrap: false,
            max_lines: None,
            italic: None,
            underline: None,
            line_through: None,
            suppress_spans: false,
            preserve_lines: None,
            unstyled_glyph: false,
            line_height: None,
            retained_text: WidgetRetainedText::default(),
            tabular_nums: false,
            adopt_node_text_spans: false,
            advanced_shaping: false,
            rich_text: None,
            force_horizontal_writing: false,
            overlay_chrome: false,
        }
    }
}

impl WidgetText {
    /// Uses overlay-chrome text defaults (no spans, shadows, decorations, font
    /// features, italic, OpenType settings, or retained rich layout).
    #[must_use]
    pub fn with_overlay_chrome(mut self) -> Self {
        self.overlay_chrome = true;
        self
    }
}

/// Built-in vector glyph kind emitted by [`WidgetIcon`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetGlyph {
    Icon(nana_ui_core::Icon),
    Spinner { phase: u8 },
}

/// Positioned vector icon or spinner glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetIcon {
    pub bounds: LayoutBox,
    pub glyph: WidgetGlyph,
    pub color: Option<[f32; 4]>,
    pub paint_color: Option<nana_ui_core::PaintColor>,
    /// When `true`, align Y vertically to the adjacent text glyph center on this node or its parent.
    pub align_to_adjacent_text: bool,
}

/// Positioned raster/component image primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetImage {
    pub bounds: LayoutBox,
    pub source: Arc<str>,
    pub fit: nana_ui_core::ContentFit,
    pub sampling: nana_ui_core::ImageSampling,
    pub corner_radius: [f32; 4],
    pub opacity: f32,
}

/// Polyline stroke primitive in node-local coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetStroke {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
    pub color: [f32; 4],
    pub cap: LineCap,
    pub dash: Option<Vec<f32>>,
    /// When `true`, horizontal and vertical strokes snap to half-pixel device
    /// boundaries against the node's page origin.
    pub snap_hairline: bool,
}

/// Open widget visual contract stored sparsely per node in [`UiWorld`].
pub trait WidgetVisual: Any + fmt::Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn dyn_eq(&self, other: &dyn Any) -> bool;

    /// Projects this widget's node-local paint operations for extraction.
    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput;

    /// Returns `false` if this visual carries out-of-contract numeric parameters.
    fn is_valid(&self) -> bool {
        true
    }

    /// Returns `true` if `paint` measures text per frame (so font updates mark this node).
    fn measures_text(&self) -> bool {
        false
    }

    /// Returns `true` if deriving geometry reads page coordinates outside this node's frame.
    fn geometry_reads_page(&self) -> bool {
        false
    }

    /// Returns `true` if this widget projects virtual accessibility children under its node.
    fn has_virtual_nodes(&self) -> bool {
        false
    }

    /// Returns `true` if inline alignment treats this widget as replaced content (bottom margin edge).
    fn aligns_by_bottom_edge(&self) -> bool {
        false
    }

    fn visual_parts_read_theme(&self) -> bool {
        false
    }

    fn reads_toast_recipe(&self) -> bool {
        false
    }

    fn intrinsic_chrome_content(&self, _id: StableNodeId, _world: &UiWorld) -> WidgetVisualContent {
        WidgetVisualContent::default()
    }

    fn solve_content_parts(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
        _bounds: LayoutBox,
        _content: LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        None
    }

    fn leading_parts(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
    ) -> Option<crate::layout_engine::parts::LeadingParts> {
        None
    }

    fn clamped_node_text_lines(&self) -> Option<usize> {
        None
    }

    fn node_text_max_width(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
        _layout: LayoutBox,
        _available_width: f32,
        _available_height: f32,
    ) -> Option<f32> {
        None
    }

    fn text_visual_key(&self) -> Option<(std::any::TypeId, u64)> {
        None
    }

    fn is_document_select_target(&self) -> bool {
        false
    }

    /// Returns `true` when overflowing single-line inline text aligns relative to the content box.
    fn offsets_overflowing_inline_text(&self) -> bool {
        false
    }

    /// Returns `true` when this visual paints a leaf glyph replacing in-flow vector children.
    fn is_icon_visual(&self) -> bool {
        false
    }

    /// Returns `true` when `TextContent` part width changes should invalidate node text constraints.
    fn tracks_text_content_part_budget(&self) -> bool {
        false
    }

    /// Returns `true` when non-wrapping node text capped at part width recovers natural width.
    fn caps_control_text(&self) -> bool {
        false
    }

    /// Returns `true` when this widget clips its descendants to its own layout bounds.
    fn clips_descendants(&self) -> bool {
        false
    }

    /// Returns `true` when this widget applies Card header title typography and clamps to 1 line.
    fn uses_card_title_typography(&self) -> bool {
        false
    }

    /// Returns `Some(true)` for a non-empty button label, `Some(false)` for an empty button label.
    fn button_label_state(&self) -> Option<bool> {
        None
    }

    /// Returns `(leading, content)` slots when this visual spreads trailing items like a `ListItem`.
    fn lone_list_trailing_slots(&self) -> Option<(StableNodeId, Option<StableNodeId>)> {
        None
    }

    fn owns_intrinsic_text(&self) -> bool {
        false
    }

    fn content_layout_changed(&self, _prev: Option<&dyn Any>) -> bool {
        false
    }

    fn clearing_changes_content_layout(&self) -> bool {
        false
    }

    fn invalidates_node_text_constraint(&self) -> bool {
        false
    }

    fn parts_placement_only(&self, _prev: &dyn Any) -> bool {
        false
    }

    /// Returns an updated visual when the shared `LOADING` track advances, or `None`.
    fn advance_loading_phase(&self, _phase: f32) -> Option<WidgetVisualHandle> {
        None
    }

    /// Returns `true` when this visual makes the node a scroll container.
    fn is_scroll_container(&self) -> bool {
        false
    }

    /// Returns `true` when this scroll container can display scrollbar chrome.
    fn has_scrollbar_chrome(&self) -> bool {
        false
    }

    /// Returns `true` when this visual represents a confirmation modal frame.
    fn is_confirm_modal(&self) -> bool {
        false
    }

    /// Returns triggered overlay configuration when this node hosts an open triggered menu.
    fn triggered_menu_overlay(&self) -> Option<crate::TriggeredMenuOverlay> {
        None
    }

    /// Returns `true` when this node is a closed menu hiding its non-trigger `child`.
    fn closed_menu_hides(&self, _child: StableNodeId) -> bool {
        false
    }

    /// Returns `true` when this node is an open `ActionMenu` (grouping child menu items).
    fn is_action_menu_open(&self) -> bool {
        false
    }

    /// Returns `true` when this node is a `MenuSurface` (closed or open).
    fn is_menu_surface(&self) -> bool {
        false
    }

    /// Returns `true` when this node is a `Select`/`Dropdown` with an open popup menu.
    fn is_open_select_menu(&self) -> bool {
        false
    }

    /// Returns `true` when this node is a `ModalFrame`.
    fn is_modal_frame(&self) -> bool {
        false
    }

    /// Returns `true` when this node is a `TextInput`.
    fn is_text_input(&self) -> bool {
        false
    }

    /// Returns explicit overlay mount policy if this visual acts as an overlay surface.
    fn overlay_mount(&self) -> Option<nana_ui_core::OverlayMount> {
        None
    }

    /// Returns modal slot layout inputs when this node lays out modal chrome slots.
    fn modal_layout_input(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
    ) -> Option<crate::ModalLayoutInput> {
        None
    }

    /// Returns resolved modal chrome child slot ids.
    fn modal_slots(&self) -> Option<crate::ModalSlots> {
        None
    }

    /// Returns `true` when this node's own paint/clip/hit area depends on `child`'s layout box.
    fn paint_depends_on_child(&self, _child: StableNodeId) -> bool {
        false
    }

    /// Pivot offset (relative to `bounds` origin) for presentation transform scaling.
    fn presentation_pivot(
        &self,
        _id: StableNodeId,
        _world: &UiWorld,
        _bounds: LayoutBox,
    ) -> Option<[f32; 2]> {
        None
    }

    /// Which theme token and recipe classes this visual reads when painting (#106).
    ///
    /// Narrow overrides let unrelated theme changes (e.g. accent-only on a
    /// palette-only control, or button-recipe-only on a non-button control)
    /// skip re-extracting and repainting unchanged controls.
    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::PALETTE
            .union(nana_ui_core::ThemeDependencyMask::ACCENT)
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
            .union(nana_ui_core::ThemeDependencyMask::RECIPE_FAMILY)
    }
}

/// Shared, equality-comparable handle to a [`WidgetVisual`] stored per node.
#[derive(Clone, Debug)]
pub struct WidgetVisualHandle(Arc<dyn WidgetVisual>);

impl WidgetVisualHandle {
    /// Wraps a concrete [`WidgetVisual`] implementation.
    pub fn new<V: WidgetVisual>(visual: V) -> Self {
        Self(Arc::new(visual))
    }

    #[inline]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Returns a dynamic reference to the inner visual.
    pub fn as_any(&self) -> &dyn Any {
        self.0.as_any()
    }

    /// Downcasts the inner visual to `V`.
    pub fn downcast_ref<V: WidgetVisual>(&self) -> Option<&V> {
        self.0.as_any().downcast_ref::<V>()
    }

    /// Whether this visual's parameters satisfy widget invariants.
    pub fn is_valid(&self) -> bool {
        self.0.is_valid()
    }

    /// Whether font registrations invalidate this node's extraction.
    pub fn measures_text(&self) -> bool {
        self.0.measures_text()
    }

    /// Whether this visual reads page-space geometry outside its own frame.
    pub fn geometry_reads_page(&self) -> bool {
        self.0.geometry_reads_page()
    }

    /// Whether this widget projects virtual accessibility children under its node.
    pub fn has_virtual_nodes(&self) -> bool {
        self.0.has_virtual_nodes()
    }

    /// Whether inline alignment treats this widget as replaced content.
    pub fn aligns_by_bottom_edge(&self) -> bool {
        self.0.aligns_by_bottom_edge()
    }

    pub fn visual_parts_read_theme(&self) -> bool {
        self.0.visual_parts_read_theme()
    }

    pub fn reads_toast_recipe(&self) -> bool {
        self.0.reads_toast_recipe()
    }

    pub fn intrinsic_chrome_content(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> WidgetVisualContent {
        self.0.intrinsic_chrome_content(id, world)
    }

    pub fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: LayoutBox,
        content: LayoutBox,
        writing: nana_ui_core::WritingContext,
        own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        self.0
            .solve_content_parts(id, world, bounds, content, writing, own_frame)
    }

    pub fn leading_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> Option<crate::layout_engine::parts::LeadingParts> {
        self.0.leading_parts(id, world)
    }

    pub fn clamped_node_text_lines(&self) -> Option<usize> {
        self.0.clamped_node_text_lines()
    }

    pub fn node_text_max_width(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        layout: LayoutBox,
        available_width: f32,
        available_height: f32,
    ) -> Option<f32> {
        self.0
            .node_text_max_width(id, world, layout, available_width, available_height)
    }

    pub fn text_visual_key(&self) -> Option<(std::any::TypeId, u64)> {
        self.0.text_visual_key()
    }

    pub fn is_document_select_target(&self) -> bool {
        self.0.is_document_select_target()
    }

    pub fn offsets_overflowing_inline_text(&self) -> bool {
        self.0.offsets_overflowing_inline_text()
    }

    pub fn is_icon_visual(&self) -> bool {
        self.0.is_icon_visual()
    }

    pub fn tracks_text_content_part_budget(&self) -> bool {
        self.0.tracks_text_content_part_budget()
    }

    pub fn caps_control_text(&self) -> bool {
        self.0.caps_control_text()
    }

    pub fn clips_descendants(&self) -> bool {
        self.0.clips_descendants()
    }

    pub fn uses_card_title_typography(&self) -> bool {
        self.0.uses_card_title_typography()
    }

    pub fn button_label_state(&self) -> Option<bool> {
        self.0.button_label_state()
    }

    pub fn lone_list_trailing_slots(&self) -> Option<(StableNodeId, Option<StableNodeId>)> {
        self.0.lone_list_trailing_slots()
    }

    pub fn owns_intrinsic_text(&self) -> bool {
        self.0.owns_intrinsic_text()
    }

    pub fn content_layout_changed(&self, prev: Option<&WidgetVisualHandle>) -> bool {
        self.0
            .content_layout_changed(prev.map(|handle| handle.0.as_any()))
    }

    pub fn clearing_changes_content_layout(&self) -> bool {
        self.0.clearing_changes_content_layout()
    }

    pub fn invalidates_node_text_constraint(&self) -> bool {
        self.0.invalidates_node_text_constraint()
    }

    pub fn parts_placement_only(&self, prev: &WidgetVisualHandle) -> bool {
        self.0.parts_placement_only(prev.0.as_any())
    }

    pub fn advance_loading_phase(&self, phase: f32) -> Option<WidgetVisualHandle> {
        self.0.advance_loading_phase(phase)
    }

    pub fn is_scroll_container(&self) -> bool {
        self.0.is_scroll_container()
    }

    pub fn has_scrollbar_chrome(&self) -> bool {
        self.0.has_scrollbar_chrome()
    }

    pub fn is_confirm_modal(&self) -> bool {
        self.0.is_confirm_modal()
    }

    pub fn triggered_menu_overlay(&self) -> Option<crate::TriggeredMenuOverlay> {
        self.0.triggered_menu_overlay()
    }

    pub fn closed_menu_hides(&self, child: StableNodeId) -> bool {
        self.0.closed_menu_hides(child)
    }

    pub fn is_action_menu_open(&self) -> bool {
        self.0.is_action_menu_open()
    }

    pub fn is_menu_surface(&self) -> bool {
        self.0.is_menu_surface()
    }

    pub fn is_open_select_menu(&self) -> bool {
        self.0.is_open_select_menu()
    }

    pub fn is_modal_frame(&self) -> bool {
        self.0.is_modal_frame()
    }

    pub fn is_text_input(&self) -> bool {
        self.0.is_text_input()
    }

    pub fn overlay_mount(&self) -> Option<nana_ui_core::OverlayMount> {
        self.0.overlay_mount()
    }

    pub fn modal_layout_input(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> Option<crate::ModalLayoutInput> {
        self.0.modal_layout_input(id, world)
    }

    pub fn modal_slots(&self) -> Option<crate::ModalSlots> {
        self.0.modal_slots()
    }

    pub fn paint_depends_on_child(&self, child: StableNodeId) -> bool {
        self.0.paint_depends_on_child(child)
    }

    pub fn presentation_pivot(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        bounds: LayoutBox,
    ) -> Option<[f32; 2]> {
        self.0.presentation_pivot(id, world, bounds)
    }

    pub fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        self.0.theme_dependency_mask()
    }

    /// Projects paint operations in node-local coordinates.
    pub fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        self.0.paint(ctx)
    }
}

impl PartialEq for WidgetVisualHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.dyn_eq(other.0.as_any())
    }
}

/// Projects a [`ComponentTextRegion`] into a standard single-line widget text op.
pub(crate) fn widget_text_region(
    region: &ComponentTextRegion,
    ctx: &WidgetPaintCtx<'_>,
    horizontal_alignment: TextHorizontalAlignment,
    ellipsis: bool,
) -> WidgetText {
    styled_text_region(region, ctx.style, horizontal_alignment, ellipsis)
}

/// Projects a [`ComponentTextRegion`] into a standard single-line widget text op using `style`.
pub(crate) fn styled_text_region(
    region: &ComponentTextRegion,
    style: &ComputedStyle,
    horizontal_alignment: TextHorizontalAlignment,
    ellipsis: bool,
) -> WidgetText {
    WidgetText {
        bounds: region.bounds,
        content: region.content.clone(),
        color: region.color,
        paint_color: region
            .color
            .and_then(|c| matching_paint_color(style.paint_colors.color, c)),
        font_size: region.font_size,
        font_weight: region.font_weight,
        horizontal_alignment,
        vertical_alignment: TextVerticalAlignment::Center,
        ellipsis,
        wrap: false,
        max_lines: None,
        italic: None,
        underline: None,
        line_through: None,
        suppress_spans: false,
        preserve_lines: None,
        unstyled_glyph: false,
        line_height: None,
        retained_text: WidgetRetainedText::None,
        tabular_nums: false,
        ..Default::default()
    }
}

/// Tight bounding box enclosing all rectangles in `rects`, or an empty zero box.
#[allow(dead_code)]
pub(crate) fn union_layout_rects(rects: &[LayoutBox]) -> LayoutBox {
    let mut iter = rects.iter().copied();
    let Some(first) = iter.next() else {
        return LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    };
    iter.fold(first, |acc, rect| {
        let x = acc.x.min(rect.x);
        let y = acc.y.min(rect.y);
        let right = (acc.x + acc.width).max(rect.x + rect.width);
        let bottom = (acc.y + acc.height).max(rect.y + rect.height);
        LayoutBox {
            x,
            y,
            width: (right - x).max(0.0),
            height: (bottom - y).max(0.0),
        }
    })
}

/// Validates theme focus-ring stroke width and offset against finite non-negative bounds.
pub(crate) fn focus_ring_stroke(ring: nana_ui_core::FocusRingMetrics) -> (f32, f32) {
    let valid = |value: f32, fallback: f32| {
        if value.is_finite() && value >= 0.0 {
            value
        } else {
            fallback
        }
    };
    (
        valid(ring.width, nana_ui_core::FOCUS_RING_METRICS.width),
        valid(ring.offset, nana_ui_core::FOCUS_RING_METRICS.offset),
    )
}

/// Emits the three concentric contrast-ring strokes starting at `first_slot`.
pub(crate) fn push_contrast_ring_quads(
    out: &mut WidgetPaintOutput,
    first_slot: u64,
    rect: LayoutBox,
    ring: crate::ContrastRing,
) {
    let edge = nana_ui_core::HAIRLINE;
    let band = nana_ui_core::space::XXS;
    let inset = |rect: LayoutBox, by: f32| LayoutBox {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - by * 2.0).max(0.0),
        height: (rect.height - by * 2.0).max(0.0),
    };
    for (offset, rect, color, width) in [
        (0_u64, rect, ring.edge, edge),
        (1, inset(rect, edge), ring.ring, band),
        (2, inset(rect, edge + band), ring.edge, edge),
    ] {
        out.push(
            first_slot + offset,
            WidgetPaintOp::Quad(WidgetQuad {
                rect,
                border: Some((color, width)),
                corner_radius: [rect.width.max(rect.height) / 2.0; 4],
                ..WidgetQuad::default()
            }),
        );
    }
}

/// Emits a component image slot's round backdrop circle (`backdrop_slot`) and,
/// when non-empty, its cover-fit picture (`picture_slot`), transforming each
/// emitted [`WidgetTaggedOp`] (for example with `.with_floating_overlay()`).
pub(crate) fn push_component_image(
    out: &mut WidgetPaintOutput,
    backdrop_slot: u64,
    picture_slot: u64,
    image: &crate::ComponentImage,
    mut transform: impl FnMut(WidgetTaggedOp) -> WidgetTaggedOp,
) {
    let radius = [image.bounds.width.min(image.bounds.height) * 0.5; 4];
    out.push_tagged(transform(WidgetTaggedOp::new(
        backdrop_slot,
        WidgetPaintOp::Quad(WidgetQuad {
            rect: image.bounds,
            fill: Some(image.placeholder),
            corner_radius: radius,
            ..WidgetQuad::default()
        }),
    )));
    if image.resource.is_empty() {
        return;
    }
    out.push_tagged(transform(WidgetTaggedOp::new(
        picture_slot,
        WidgetPaintOp::Image(WidgetImage {
            bounds: image.bounds,
            source: image.resource.to_resource(),
            fit: nana_ui_core::ContentFit::Cover,
            sampling: nana_ui_core::ImageSampling::default(),
            corner_radius: radius,
            opacity: 1.0,
        }),
    )));
}
