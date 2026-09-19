mod attributes;
mod composition;
mod compositor;
use attributes::DrawAttributes;
pub use attributes::SceneDraw;
use compositor::CompositorRegistry;
pub use compositor::{
    CompositorLayer, CompositorLayerId, CompositorLayerKind, CompositorMotionBinding,
    CompositorPaintEncode, LAYER_DEMOTE_HOLD, LAYER_PROMOTE_HOLD,
};
mod visibility;
pub use composition::FramePlan;
use visibility::VisibilityIndex;
mod order;
mod primitives;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use nana_ui_core::{
    BackgroundImage, BorderImageSpec, ClipPath, ColorFilter, ControlSize, DirSpec, DrawerSide,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, Icon, LineBreakSpec, LineHeightSpec,
    MixBlendMode, SwitchControlPosition, UI_METRICS, WordBreakSpec, WritingModeSpec,
    icon_y_on_text_glyph_center,
};
use nana_ui_runtime::{
    ComponentElevation, ComponentGeometry, ComponentTextRegion, CustomRenderNode, ExtractedNode,
    LayoutBox, NodeKind, NodeMap, NodeSet, StableNodeId, StandardVisual, TextFoldGutter,
    TextHorizontalAlignment, TextShaping, TextVerticalAlignment, TextWhitespaceKind,
};

use crate::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};

const fn corner_radii(r: f32) -> [f32; 4] {
    [r; 4]
}

fn focus_ring_corner_radius(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    outset: f32,
) -> [f32; 4] {
    let radii = style.resolved_border_radii(bounds.width, bounds.height);
    let max_r = radii.into_iter().fold(0.0f32, f32::max);
    corner_radii(max_r + outset)
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineTransform(pub [f32; 6], pub [f32; 2]);

impl AffineTransform {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0]);

    pub const fn from_matrix(matrix: [f32; 6]) -> Self {
        Self(matrix, [0.0, 0.0])
    }

    pub fn is_projective(self) -> bool {
        self.1[0].abs() > 1e-8 || self.1[1].abs() > 1e-8
    }

    pub fn then(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h] = self.1;
        let [ra, rb, rc, rd, re, rf] = rhs.0;
        let [rg, rh] = rhs.1;
        let na = a * ra + c * rb + e * rg;
        let nb = b * ra + d * rb + f * rg;
        let nc = a * rc + c * rd + e * rh;
        let nd = b * rc + d * rd + f * rh;
        let ne = a * re + c * rf + e;
        let nf = b * re + d * rf + f;
        let ng = g * ra + h * rb + rg;
        let nh = g * rc + h * rd + rh;
        let ni = g * re + h * rf + 1.0;
        if !ni.is_finite() || ni.abs() < 1e-8 {
            return Self::IDENTITY;
        }
        let inv = 1.0 / ni;
        Self(
            [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
            [ng * inv, nh * inv],
        )
    }
}

impl From<[f32; 6]> for AffineTransform {
    fn from(matrix: [f32; 6]) -> Self {
        Self::from_matrix(matrix)
    }
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipRegion {
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    /// Rounded inset clip radius (px in border-box space). Zero = axis-aligned rect.
    pub corner_radius: f32,
    /// `clip-path: polygon(...)` vertices in [`Self::bounds`] local space (px).
    pub polygon_clip: Option<Vec<[f32; 2]>>,
}

impl ClipRegion {
    pub fn axis_aligned(bounds: SceneRect, transform: AffineTransform) -> Self {
        Self {
            bounds,
            transform,
            corner_radius: 0.0,
            polygon_clip: None,
        }
    }

    /// Ellipse filling `bounds` (`clip-path: circle()` / `ellipse()`).
    pub fn ellipse(bounds: SceneRect, transform: AffineTransform) -> Self {
        Self {
            bounds,
            transform,
            corner_radius: 0.0,
            polygon_clip: Some(vec![[f32::NEG_INFINITY, f32::NEG_INFINITY]]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveId {
    pub node: StableNodeId,
    /// Fixed component slots occupy 0..=255. Unbounded component collections
    /// use a separate namespace and a checked collection index.
    pub slot: u64,
}

fn collection_slot(namespace: u32, index: usize) -> u64 {
    debug_assert!(namespace != 0);
    (u64::from(namespace) << 32)
        | u64::from(u32::try_from(index).expect("primitive collection exceeds u32::MAX items"))
}

/// `::selection` fills share the custom-render slot's paint layer so they stay
/// behind the glyphs, but live in their own namespace so the two cannot
/// overwrite each other's key.
const DOCUMENT_TEXT_SELECTION: u32 = 6;
const TEXT_LINE_LABELS: u32 = 1;
const TEXT_DIAGNOSTIC_MARKERS: u32 = 2;
const TEXT_DIAGNOSTIC_LABELS: u32 = 3;
const TEXT_ATOM_ICONS: u32 = 4;
const TEXT_ATOM_LABELS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuadSurfacePaint {
    pub background_image: Option<BackgroundImage>,
    /// Extra CSS `background-image` layers after the first (below it).
    pub background_layers: Vec<BackgroundImage>,
    /// `<img src>` replaced content, painted above background layers.
    pub content_image: Option<BackgroundImage>,
    pub mask: Option<nana_ui_core::MaskImage>,
    /// Resolved polygon vertices in border-box coordinates (px).
    pub polygon_clip: Option<Vec<[f32; 2]>>,
    pub filter: Option<ColorFilter>,
    pub backdrop_filter: Option<nana_ui_core::BackdropFilter>,
    /// Extra `box-shadow` layers after the primary (GPU cap 4 including primary).
    pub extra_shadows: Vec<ComponentElevation>,
    pub outline_width: f32,
    pub outline_color: Option<[f32; 4]>,
    pub mix_blend: MixBlendMode,
    /// Per-side stroke (T,R,B,L). All-zero keeps [`ScenePrimitiveKind::Quad::border_width`].
    pub border_widths: [f32; 4],
    /// Per-side colors (T,R,B,L). Zero alpha falls back to the quad `border_color`.
    pub border_colors: [[f32; 4]; 4],
    /// Per-side shader style (T,R,B,L): 0 solid, 1 dashed, 2 dotted.
    pub border_styles: [u8; 4],
    /// Minimal `border-image` 9-slice (`url()` / linear-gradient + slice).
    pub border_image: Option<BorderImageSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScenePrimitiveKind {
    Quad {
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        surface: QuadSurfacePaint,
    },
    QuadBatch {
        bounds: Vec<SceneRect>,
        background: Option<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        surface: QuadSurfacePaint,
    },
    /// One batch of solid-color quads with per-item colors (editor color
    /// swatch decorators). Mirrors [`ScenePrimitiveKind::QuadBatch`]: one
    /// scene slot regardless of count and color variety, so a large set
    /// never saturates `u8` slot indices. No shadow and no surface paint —
    /// decorative overlays only.
    QuadColorBatch {
        bounds: Vec<SceneRect>,
        colors: Vec<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
    },
    Text {
        content: String,
        color: Option<[f32; 4]>,
        size: f32,
        weight: Option<u16>,
        family: Option<String>,
        line_height: Option<LineHeightSpec>,
        letter_spacing: f32,
        wrap: bool,
        ellipsis: bool,
        max_lines: Option<u16>,
        shaping: TextShaping,
        horizontal_alignment: TextHorizontalAlignment,
        vertical_alignment: TextVerticalAlignment,
        /// Theme-resolved committed-text spans. Empty means solid `color`.
        spans: Vec<SceneTextSpan>,
        text_shadow: Option<nana_ui_core::TextShadowSpec>,
        underline: bool,
        line_through: bool,
        font_features: Vec<nana_ui_core::FontFeatureSetting>,
        italic: bool,
        wrap_break: nana_ui_core::TextWrapBreak,
        /// OpenType / wrap subset from computed style. Defaults are CSS initial.
        opentype: SceneTextOpenType,
        /// The `nana-text` layout Runtime measured this text with, when the
        /// host resolved plain text through an engine (Issue #95). Present, it
        /// is the placement authority for these glyphs; the fields above stay
        /// for renderers that still lay text out themselves. Never a shaping
        /// backend's buffer.
        layout: Option<nana_ui_runtime::RetainedTextLayout>,
    },
    Icon {
        icon: Icon,
        color: Option<[f32; 4]>,
    },
    /// Many instances of one icon in a single primitive. Mirrors
    /// [`ScenePrimitiveKind::QuadBatch`]: one scene slot regardless of count,
    /// so a large set (editor whitespace tab arrows) never saturates `u8`
    /// slot indices.
    IconBatch {
        bounds: Vec<SceneRect>,
        icon: Icon,
        color: Option<[f32; 4]>,
    },
    Spinner {
        phase: u8,
        color: Option<[f32; 4]>,
    },
    Stroke {
        points: Vec<[f32; 2]>,
        width: f32,
        color: [f32; 4],
        /// Per-point stroke widths. Empty means every vertex uses [`Self::Stroke::width`].
        widths: Vec<f32>,
        cap: StrokeCap,
        /// Dash and per-point colors. `None` is the Graph / TimeSeries path:
        /// no extra heap and the painter keeps the solid uniform emit.
        pattern: Option<Box<StrokePattern>>,
    },
    Custom {
        node: CustomRenderNode,
        /// `mask-image` / `-webkit-mask-image` alpha for HostTexture sampling.
        /// Same value as [`QuadSurfacePaint::mask`] (gradient or `url()`).
        mask: Option<nana_ui_core::MaskImage>,
    },
}

impl ScenePrimitiveKind {
    /// Only Quad pipelines bind `motion.wgsl` `evaluate()`. Text / Icon /
    /// Mesh / HostTexture still consume CPU compositor presentation.
    pub fn evaluates_compositor_motion_on_gpu(&self) -> bool {
        matches!(
            self,
            Self::Quad { .. } | Self::QuadBatch { .. } | Self::QuadColorBatch { .. }
        )
    }
}

/// Optional dash and per-point colors for [`ScenePrimitiveKind::Stroke`].
///
/// Empty `dash` is solid. Empty `colors` uses the stroke's uniform `color`.
/// The painter only walks these slices when they are non-empty, so unused
/// decorations do not add GPU instance fields or shader work. Graph /
/// TimeSeries keep [`ScenePrimitiveKind::Stroke::pattern`] as `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrokePattern {
    /// SVG-style on/off lengths. Negative or non-finite values disable dash
    /// (treated as solid). A single-cycle odd list is repeated to even length.
    pub dash: Vec<f32>,
    pub dash_offset: f32,
    /// SVG `pathLength` for dasharray/dashoffset. Zero, negative, or
    /// non-finite is unset (use geometric length). Dashes are in these units:
    /// geometric `s` maps to `s * (path_length / geometric_length)` before
    /// phase. Ignored when `dash` is empty (solid). Scene Stroke callers set
    /// this field; Vue/CSS does not extract it (generic SVG is resvg).
    pub path_length: f32,
    /// Per-point colors. Used only when `len` matches the stroke point count;
    /// each segment takes the color of its start vertex.
    pub colors: Vec<[f32; 4]>,
}

/// End-cap of an articulated stroke segment.
///
/// Round is the Ciallo vanilla disc. Butt is a flat cut at the endpoint.
/// Square extends half-width past the endpoint, then cuts flat. The painter
/// expands Square on the CPU and reuses the Butt GPU path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeCap {
    #[default]
    Round,
    Butt,
    Square,
}

/// One paint-ready span inside a [`ScenePrimitiveKind::Text`] primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextSpan {
    pub start: usize,
    pub end: usize,
    pub color: [f32; 4],
}

/// OpenType and wrap extras on a [`ScenePrimitiveKind::Text`] run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneTextOpenType {
    pub features: Vec<FontFeatureSetting>,
    pub variations: Vec<FontVariationSetting>,
    pub kerning: FontKerningSpec,
    pub word_break: WordBreakSpec,
    pub line_break: LineBreakSpec,
    /// CSS `direction` after inherit. Drives the same RLI/PDI wrap as shaping.
    pub direction: DirSpec,
    /// CSS `writing-mode` after inherit. cosmic-text 0.19 has no vertical
    /// glyph orientation; paint still shapes horizontally.
    pub writing_mode: WritingModeSpec,
}

impl SceneTextOpenType {
    pub fn from_computed(style: &nana_ui_runtime::ComputedStyle) -> Self {
        Self {
            features: style.font_features.clone(),
            variations: style.font_variations.clone(),
            kerning: style.font_kerning,
            word_break: style.word_break,
            line_break: style.line_break,
            direction: style.direction,
            writing_mode: style.writing_mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenePrimitive {
    pub id: PrimitiveId,
    pub node: StableNodeId,
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    /// Ancestor clip chain, shared: every primitive of a node, and every node
    /// under the same clipping ancestors, points at one allocation.
    pub clips: Arc<[ClipRegion]>,
    /// Paint opacity excluding ancestor opacity groups.
    pub opacity: f32,
    pub z_index: i32,
    pub document_order: usize,
    pub kind: ScenePrimitiveKind,
}

/// Isolating ancestor whose subtree is composited as one layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpacityGroup {
    pub node: StableNodeId,
    pub opacity: f32,
    pub filter: ColorFilter,
    pub mix_blend: MixBlendMode,
    /// Inset `box-shadow` overlay recorded on this dest group. `None` when none.
    pub inset_shadow: Option<InsetShadowOverlay>,
}

/// Inset shadow painted onto a dest group after its descendants composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsetShadowOverlay {
    pub elevation: ComponentElevation,
    pub bounds: SceneRect,
    pub corner_radius: [f32; 4],
}

/// Isolating ancestor with a non-identity CSS `filter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterGroup {
    pub node: StableNodeId,
    pub filter: ColorFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SceneOrderKey {
    /// `(z_index, document_order)` for each isolating stacking group from
    /// outermost to innermost, then this primitive's node. Opacity groups,
    /// `isolation: isolate`, and positioned + `z-index` keep a subtree
    /// contiguous against siblings, except `position: fixed` surfaces which
    /// paint in the root stacking context so Popover/menu chrome is not trapped
    /// in a parent card. Not full CSS Appendix E.
    stack: GroupPrefix,
    /// Collection identity must not lift scrolling text above sticky bands,
    /// minimaps or popup surfaces owned by the same component.
    paint_layer: u64,
    slot: u64,
    node: StableNodeId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneDelta {
    pub added: Vec<StableNodeId>,
    pub removed: Vec<StableNodeId>,
    pub paint: Vec<StableNodeId>,
    pub transforms: Vec<StableNodeId>,
    pub clips: Vec<StableNodeId>,
    pub order_changed: bool,
    pub stats: SceneDeltaStats,
}

impl std::ops::Deref for SceneDelta {
    type Target = SceneDeltaStats;

    fn deref(&self) -> &Self::Target {
        &self.stats
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneDeltaStats {
    pub updated_nodes: usize,
    pub removed_nodes: usize,
    pub rebuilt_primitives: usize,
    pub order_rebuilt: bool,
    pub primitive_count: usize,
}

/// A primitive plus what the scene needs to keep it in paint order.
#[derive(Debug, Clone)]
struct RetainedPrimitive {
    primitive: ScenePrimitive,
    /// Where this primitive sits in `ordered`. Kept rather than recomputed:
    /// the walk that produces it is the expensive half of touching a
    /// primitive, and removal has to use the key it was filed under.
    key: SceneOrderKey,
    /// The rebuild that last wrote this slot, against the scene's `build`
    /// counter. A slot an ongoing rebuild has not written is one the node no
    /// longer has.
    build: u64,
}

#[derive(Debug)]
pub struct UiScene {
    frame_plan: OnceLock<Arc<FramePlan>>,
    visibility: OnceLock<VisibilityIndex>,
    attribute_epoch: u64,
    projections: NodeMap<(u64, AffineTransform, usize)>,
    /// Projections that cannot be adjusted by an inverse delta. Usually empty;
    /// ordinary scrolling must not scan every retained descendant.
    unadjustable_projections: NodeSet,
    draw_attributes: std::sync::Mutex<NodeMap<DrawAttributes>>,
    /// Dest groups per node, stamped with the scene instance they were read
    /// at. See [`UiScene::opacity_groups`].
    opacity_group_cache: std::sync::Mutex<OpacityGroupCache>,
    /// Ancestor compositor-layer opacity factors, stamped with the scene
    /// instance and the attribute epoch. See
    /// [`UiScene::compositor_paint_opacity`].
    pub(super) layer_factor_cache: std::sync::Mutex<LayerFactorCache>,
    /// What the primitive-rebuild pass would otherwise recompute for every
    /// node it touches. See [`RebuildScratch`].
    rebuild_scratch: std::sync::Mutex<RebuildScratch>,
    nodes: SceneNodes,
    node_order: NodeMap<usize>,
    primitives: BTreeMap<PrimitiveId, RetainedPrimitive>,
    ordered: BTreeSet<SceneOrderKey>,
    /// Bumped once per node rebuild and stamped onto every primitive that
    /// rebuild writes. See `rebuild_node_primitives`.
    build: u64,
    /// Whether this delta added, dropped or re-bound a primitive — the changes
    /// a compiled frame plan and the culling index are built on. The scene
    /// knows it as it happens: this is the pass that adds and drops them.
    /// Asking afterwards meant snapshotting every touched node's primitive
    /// list and diffing it, which is a range scan and an allocation per node.
    structure_changed: bool,
    compositor: CompositorRegistry,
    /// Identity that changes on node-changing mutation and on Clone.
    /// In-place [`UiScene::apply_delta`] that updates or removes nodes also
    /// gets a fresh value, because product flush mutates a unique `Arc` in
    /// place after the first paint. Painters key a validated op stream on
    /// this id. Never zero: two freshly created scenes must not share an
    /// identity.
    instance: u64,
}

impl Default for UiScene {
    fn default() -> Self {
        Self {
            frame_plan: OnceLock::new(),
            visibility: OnceLock::new(),
            attribute_epoch: 0,
            projections: NodeMap::default(),
            unadjustable_projections: NodeSet::default(),
            draw_attributes: std::sync::Mutex::new(NodeMap::default()),
            opacity_group_cache: std::sync::Mutex::new(OpacityGroupCache::default()),
            layer_factor_cache: std::sync::Mutex::new(LayerFactorCache::default()),
            rebuild_scratch: std::sync::Mutex::new(RebuildScratch::default()),
            nodes: SceneNodes::default(),
            node_order: NodeMap::default(),
            primitives: BTreeMap::new(),
            ordered: BTreeSet::new(),
            build: next_primitive_revision(),
            structure_changed: false,
            compositor: CompositorRegistry::default(),
            instance: next_scene_instance(),
        }
    }
}

impl Clone for UiScene {
    fn clone(&self) -> Self {
        Self {
            frame_plan: self.frame_plan.clone(),
            visibility: self.visibility.clone(),
            attribute_epoch: self.attribute_epoch,
            projections: self.projections.clone(),
            unadjustable_projections: self.unadjustable_projections.clone(),
            draw_attributes: std::sync::Mutex::new(
                self.draw_attributes
                    .lock()
                    .expect("scene attributes")
                    .clone(),
            ),
            // Stamped with the instance they were read at, and a clone is a
            // new instance, so carrying them over would only be work.
            opacity_group_cache: std::sync::Mutex::new(OpacityGroupCache::default()),
            layer_factor_cache: std::sync::Mutex::new(LayerFactorCache::default()),
            rebuild_scratch: std::sync::Mutex::new(RebuildScratch::default()),
            nodes: self.nodes.clone(),
            node_order: self.node_order.clone(),
            primitives: self.primitives.clone(),
            ordered: self.ordered.clone(),
            build: self.build,
            structure_changed: self.structure_changed,
            compositor: self.compositor.clone(),
            instance: next_scene_instance(),
        }
    }
}

/// Identity for one write of a primitive, unique across every scene in the
/// process. See [`UiScene::build`].
pub(super) fn next_primitive_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_scene_instance() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl UiScene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutation-unique identity. Clone and in-place node-changing
    /// [`Self::apply_delta`] both get a fresh value; idle `apply_delta([], [])`
    /// keeps it so unchanged-scene paint caches still hit.
    pub fn instance_id(&self) -> u64 {
        self.instance
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }

    pub fn primitives(&self) -> impl Iterator<Item = &ScenePrimitive> {
        self.ordered.iter().filter_map(|key| {
            self.primitives
                .get(&PrimitiveId {
                    node: key.node,
                    slot: key.slot,
                })
                .map(|held| &held.primitive)
        })
    }

    pub fn node_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        self.nodes.get(&id).map(|node| SceneRect {
            x: node.layout.x,
            y: node.layout.y,
            width: node.layout.width,
            height: node.layout.height,
        })
    }

    /// Isolating opacity groups from outermost to innermost that contain `node`.
    /// Dest groups containing `node`, outermost first.
    ///
    /// Asked once per primitive per frame, and again by culling, so the
    /// *ancestor* half of the answer is memoized: a container's list is its
    /// parent's list plus itself when it is a group. The queried node's own
    /// list is not stored — every primitive names a different node, so an
    /// entry for it would be written once and read never.
    ///
    /// A node that is not a group **shares** its parent's list rather than
    /// copying it, so a shell whose containers are all opaque allocates
    /// nothing here at all.
    ///
    /// The stamp is the scene instance, which `apply_delta` moves whenever a
    /// node, a style or a parent changed — the only three things this reads.
    pub fn opacity_groups(&self, node: StableNodeId) -> Arc<[OpacityGroup]> {
        let Some(candidate) = self.nodes.get(&node) else {
            return empty_opacity_groups();
        };
        let base = match candidate.parent {
            Some(parent) => {
                let mut cache = self
                    .opacity_group_cache
                    .lock()
                    .expect("scene opacity groups");
                self.ancestor_opacity_groups(&mut cache, parent, 0)
            }
            None => empty_opacity_groups(),
        };
        if is_dest_group(&self.nodes, candidate) {
            let mut groups = base.to_vec();
            groups.push(dest_group(&self.nodes, node, candidate));
            return Arc::from(groups);
        }
        base
    }

    fn ancestor_opacity_groups(
        &self,
        cache: &mut OpacityGroupCache,
        id: StableNodeId,
        depth: usize,
    ) -> Arc<[OpacityGroup]> {
        if depth >= MAX_ANCESTOR_DEPTH {
            return empty_opacity_groups();
        }
        if let Some((stamp, groups)) = cache.get(&id)
            && *stamp == self.instance
        {
            return Arc::clone(groups);
        }
        let Some(candidate) = self.nodes.get(&id) else {
            return empty_opacity_groups();
        };
        let base = match candidate.parent {
            Some(parent) => self.ancestor_opacity_groups(cache, parent, depth + 1),
            None => empty_opacity_groups(),
        };
        let groups = if is_dest_group(&self.nodes, candidate) {
            let mut groups = base.to_vec();
            groups.push(dest_group(&self.nodes, id, candidate));
            Arc::from(groups)
        } else {
            base
        };
        cache.insert(id, (self.instance, Arc::clone(&groups)));
        groups
    }

    /// Isolating filter groups from outermost to innermost that contain `node`.
    pub fn filter_groups(&self, node: StableNodeId) -> Vec<FilterGroup> {
        filter_groups_from(&self.nodes, node)
    }

    pub fn is_node_in_subtree(&self, root: StableNodeId, candidate: StableNodeId) -> bool {
        ancestor_ids(&self.nodes, candidate).any(|id| id == root)
    }

    /// Apply Runtime's dirty extraction and tombstone stream atomically.
    /// Updating or removing nodes refreshes [`Self::instance_id`]; an empty
    /// no-op keeps the current instance.
    pub fn apply_delta(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        let mut delta = SceneDelta::default();
        self.structure_changed = false;
        let mut removed_nodes = 0;
        let mut changed = Vec::new();
        let mut hierarchy_changed = false;
        let mut inherited_roots = HashSet::new();
        for id in removals {
            if let Some(old) = self.nodes.remove(&id) {
                delta.removed.push(id);
                self.projections.remove(&id);
                self.unadjustable_projections.remove(&id);
                if !old.children.is_empty() {
                    self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
                    inherited_roots.insert(id);
                }
                self.draw_attributes
                    .get_mut()
                    .expect("scene attributes")
                    .remove(&id);
                removed_nodes += 1;
                hierarchy_changed |= old.parent.is_some() || !old.children.is_empty();
                self.remove_node_primitives(id);
                self.forget_compositor_node(id);
            }
        }
        let mut updated_nodes = 0;
        // Roots whose retained descendants have to be rebuilt because what
        // they inherit changed. A scrolled projective subtree cannot be
        // re-projected, and a group that stops isolating hands its opacity
        // down to primitives that baked it.
        let mut subtree_rebuild = Vec::new();
        let mut scroll_translations = Vec::new();
        let mut stacking_changed = false;
        let mut inherited_geometry_changed = !inherited_roots.is_empty();
        for node in extracted {
            let previous = self.nodes.get(&node.id);
            let inherited_changed = previous.map_or(!node.children.is_empty(), |old| {
                old.parent != node.parent
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            });
            if inherited_changed {
                // Retained descendants may not be extracted when an ancestor's
                // clip or transform changes. Refresh their inherited projection
                // without rebuilding invertible retained geometry.
                self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
                inherited_geometry_changed = true;
                inherited_roots.insert(node.id);
            }
            if previous.is_none() {
                delta.added.push(node.id);
            }
            if previous.is_none_or(|old| {
                old.layout != node.layout || old.scroll_offset != node.scroll_offset
            }) {
                delta.transforms.push(node.id);
            }
            // Layout/style can affect inherited clips; consumers resolve the
            // changed attribute roots rather than guessing from paint counts.
            if previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            }) {
                delta.clips.push(node.id);
            }
            delta.paint.push(node.id);
            hierarchy_changed |= previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.children != node.children
                    || old.source_style.layout.position != node.source_style.layout.position
            });
            let scroll_changed =
                previous.is_some_and(|old| old.scroll_offset != node.scroll_offset);
            // A node's z_index and whether it opens a group are part of every
            // descendant's paint-order key, and descendants do not have to be
            // re-extracted with it, so such a change costs a full reorder.
            stacking_changed |=
                previous.is_some_and(|old| paint_order_facts(old) != paint_order_facts(&node));
            // Of what a node hands down, its opacity is the one a descendant
            // bakes into its primitive rather than re-deriving at draw time. A
            // group isolates it, so the number that reaches descendants is 1.0
            // until the node stops being a group.
            if previous.is_some_and(|old| {
                inherited_opacity(&self.nodes, old).to_bits()
                    != inherited_opacity(&self.nodes, &node).to_bits()
            }) {
                subtree_rebuild.push(node.id);
            }
            changed.push(node.id);
            if scroll_changed {
                inherited_roots.insert(node.id);
                let old = self.nodes.get(&node.id).expect("scrolling retained node");
                let (parent, _, _, blocks_3d) = self.ancestor_state(old);
                let transform = parent.then(node_scene_transform(
                    &old.source_style.layout,
                    old.layout,
                    blocks_3d,
                ));
                if transform.is_projective() {
                    subtree_rebuild.push(node.id);
                    self.visibility.take();
                } else {
                    let dx = old.scroll_offset.x - node.scroll_offset.x;
                    let dy = old.scroll_offset.y - node.scroll_offset.y;
                    let [a, b, c, d, _, _] = transform.0;
                    scroll_translations.push((node.id, [a * dx + c * dy, b * dx + d * dy]));
                }
                self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
            }
            // The primitives stay where they are: every extracted node is
            // rebuilt below, and that rebuild overwrites the slots it still
            // has and retires the ones it does not. Dropping them here would
            // only mean taking each one out of `ordered` and putting it back.
            self.retain_compositor_requests(&node);
            self.nodes.insert(node.id, Arc::new(node));
            updated_nodes += 1;
        }
        let order_rebuilt = (updated_nodes != 0 || removed_nodes != 0)
            && (hierarchy_changed || self.node_order.len() != self.nodes.len());
        let mut rebuilt_primitives = 0;
        if updated_nodes != 0 || removed_nodes != 0 {
            if order_rebuilt {
                self.rebuild_document_order();
                for held in self.primitives.values_mut() {
                    if let Some(order) = self.node_order.get(&held.primitive.node) {
                        held.primitive.document_order = *order;
                    }
                }
            }
            let mut rebuild = changed;
            if !subtree_rebuild.is_empty() {
                let extracted: NodeSet = rebuild.iter().copied().collect();
                for root in subtree_rebuild {
                    collect_unextracted_descendants(&self.nodes, root, &extracted, &mut rebuild);
                }
            }
            // An old projective/singular base cannot be mapped to the new
            // inherited transform. Rebuild only those affected projections;
            // invertible siblings and the ordinary scroll fast path stay retained.
            let mut rebuilding: HashSet<_> = rebuild.iter().copied().collect();
            for &id in &self.unadjustable_projections {
                if !rebuilding.contains(&id)
                    && inherited_roots
                        .iter()
                        .any(|root| self.node_descends_from(id, *root))
                {
                    rebuilding.insert(id);
                    rebuild.push(id);
                }
            }
            rebuild.retain(|id| rebuilding.remove(id));
            // `self.nodes` is final by now and nothing below moves it, which
            // is the whole precondition for reusing what a parent projects.
            if let Ok(mut scratch) = self.rebuild_scratch.lock() {
                scratch.begin();
            }
            for &id in &rebuild {
                rebuilt_primitives += self.rebuild_node_primitives(id);
                self.invalidate_compositor_cache(id);
            }
            if let Ok(mut scratch) = self.rebuild_scratch.lock() {
                scratch.end();
            }
            // Rebuilt nodes re-enter `ordered` at their own key, so a reorder is
            // only needed when keys the delta did not touch also moved.
            if order_rebuilt || stacking_changed {
                self.sort_primitives();
            }
            if order_rebuilt || stacking_changed || removed_nodes != 0 || self.structure_changed {
                self.frame_plan.take();
                self.visibility.take();
            }
            if let Some(mut visibility) = self.visibility.take() {
                for (root, offset) in scroll_translations {
                    visibility.translate_subtree(root, offset);
                }
                if inherited_geometry_changed {
                    // Every projected bound under these roots moved. Which
                    // primitives are under them did not, and that is the half
                    // rebuilding the index would pay for again.
                    for &root in &inherited_roots {
                        visibility.refresh_subtree(self, root);
                    }
                }
                visibility.update(self, &rebuild);
                let _ = self.visibility.set(visibility);
            }
            self.instance = next_scene_instance();
        }
        delta.order_changed = order_rebuilt || stacking_changed;
        delta.stats = SceneDeltaStats {
            updated_nodes,
            removed_nodes,
            rebuilt_primitives,
            order_rebuilt,
            primitive_count: self.primitives.len(),
        };
        delta
    }

    /// What a frame plan reads out of a primitive beyond its identity.
    fn custom_binding(kind: &ScenePrimitiveKind) -> Option<(&Arc<str>, &Arc<str>)> {
        match kind {
            ScenePrimitiveKind::Custom { node, .. } => Some((&node.renderer, &node.resource)),
            _ => None,
        }
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&ScenePrimitive> {
        self.primitives.get(&id).map(|held| &held.primitive)
    }

    /// The primitive and the rebuild that last wrote it. See
    /// [`SceneDraw::revision`].
    fn primitive_at(&self, id: PrimitiveId) -> Option<(&ScenePrimitive, u64)> {
        self.primitives
            .get(&id)
            .map(|held| (&held.primitive, held.build))
    }

    /// Rewrite one primitive kind and bump instance identity.
    ///
    /// Painter tests use this to probe stroke variants without a second
    /// Runtime extraction ABI.
    pub fn replace_primitive_kind(&mut self, id: PrimitiveId, kind: ScenePrimitiveKind) -> bool {
        self.build = next_primitive_revision();
        let build = self.build;
        let Some(held) = self.primitives.get_mut(&id) else {
            return false;
        };
        held.primitive.kind = kind;
        // Anyone keeping a resolved copy of this primitive keys it on the
        // rebuild that wrote it, and this is a write.
        held.build = build;
        self.frame_plan.take();
        self.visibility.take();
        self.instance = next_scene_instance();
        true
    }

    /// Skip duplicate Vue `#text` children when the host already paints that
    /// string. Independent element children, such as list rows inside a Card,
    /// keep their own labels.
    fn parent_already_paints_text(&self, node: &ExtractedNode) -> bool {
        if !matches!(&*node.kind, NodeKind::Text) {
            return false;
        }
        let Some(parent) = node.parent.and_then(|id| self.nodes.get(&id)) else {
            return false;
        };
        if let Some(ComponentGeometry::Card { title, .. }) = parent.component_geometry.as_deref() {
            return title.as_ref().is_some_and(|title| {
                node.text
                    .as_ref()
                    .is_some_and(|text| text.value == title.content.as_ref())
            });
        }
        component_geometry_owns_text(parent.component_geometry.as_deref())
            || parent
                .text
                .as_ref()
                .is_some_and(|text| !text.value.is_empty())
    }

    /// Compact leading glyphs share the parent (or own) text line-box center.
    fn icon_y_aligned_to_adjacent_text(
        &self,
        node: &ExtractedNode,
        icon_bounds: SceneRect,
        extent: f32,
    ) -> f32 {
        let geometric = icon_bounds.y + (icon_bounds.height - extent) / 2.0;
        if node.layout.height > extent + 0.5 || node.layout.width > extent + 0.5 {
            return geometric;
        }
        let host = if node
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
        {
            node
        } else {
            match node.parent.and_then(|id| self.nodes.get(&id)) {
                Some(parent)
                    if parent
                        .text
                        .as_ref()
                        .is_some_and(|text| !text.value.is_empty())
                        || matches!(
                            parent.standard_visual.as_ref(),
                            Some(StandardVisual::ListItem { .. })
                        ) =>
                {
                    parent
                }
                _ => return geometric,
            }
        };
        let text_box = match host.component_geometry.as_deref() {
            Some(ComponentGeometry::ListItem {
                content: Some(content),
                ..
            }) => *content,
            _ => host.layout,
        };
        let centered = matches!(
            host.source_style.text_vertical_alignment,
            TextVerticalAlignment::Center
        );
        icon_y_on_text_glyph_center(
            text_box.y,
            text_box.height,
            host.style.font_size,
            host.style.line_height,
            centered,
            extent,
        )
    }
}

fn node_scene_transform(
    style: &nana_ui_core::LayoutStyle,
    layout: LayoutBox,
    block_3d: bool,
) -> AffineTransform {
    if block_3d && style.transform_3d.is_some() {
        return AffineTransform::IDENTITY;
    }
    style
        .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
        .map(|(matrix, persp)| AffineTransform(matrix, persp))
        .unwrap_or_default()
}

impl UiScene {
    fn ancestor_state(
        &self,
        node: &ExtractedNode,
    ) -> (AffineTransform, f32, Arc<[ClipRegion]>, bool) {
        self.project_ancestor_state(node, false)
    }

    fn draw_ancestor_state(
        &self,
        node: &ExtractedNode,
    ) -> (AffineTransform, f32, Arc<[ClipRegion]>, bool) {
        self.project_ancestor_state(node, true)
    }

    fn project_ancestor_state(&self, node: &ExtractedNode, visual: bool) -> AncestorState {
        // What the answer depends on, beyond the chain itself: whether this
        // node breaks out of it, and whether the caller wants the presented
        // values or the logical ones.
        let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
        let key = node.parent.map(|parent| (parent, fixed, visual));
        if let Some(key) = key
            && let Ok(cache) = self.rebuild_scratch.lock()
        {
            if cache.active {
                if let Some((held, state)) = cache.ancestor_state.as_ref()
                    && *held == key
                {
                    return state.clone();
                }
            } else if let Some((held, stamp, state)) = cache.drawn_ancestor_state.as_ref()
                && *held == key
                && *stamp == self.draw_stamp()
            {
                return state.clone();
            }
        }
        let (state, node_dependent) = self.compute_ancestor_state(node, visual, fixed);
        if let Some(key) = key
            && !node_dependent
            && let Ok(mut cache) = self.rebuild_scratch.lock()
        {
            if cache.active {
                cache.ancestor_state = Some((key, state.clone()));
            } else {
                cache.drawn_ancestor_state = Some((key, self.draw_stamp(), state.clone()));
            }
        }
        state
    }

    /// What a remembered draw-time answer stays valid for.
    ///
    /// `attribute_epoch` moves whenever what a node inherits changes, and
    /// `instance` whenever the scene's nodes do — a leaf leaving does not touch
    /// the epoch but can stop its parent being an opacity group.
    fn draw_stamp(&self) -> DrawStamp {
        (self.instance, self.attribute_epoch)
    }

    /// The second half of the answer is whether it read anything about `node`
    /// that its siblings do not share — a modal frame above it reads this
    /// node's focus and whether it is in the body, and a workspace resize
    /// handle drops its parent's overflow clip. Neither is cacheable by parent.
    fn compute_ancestor_state(
        &self,
        node: &ExtractedNode,
        visual: bool,
        fixed: bool,
    ) -> (AncestorState, bool) {
        let mut node_dependent = is_workspace_resize_handle(node);
        let mut ancestors = node
            .parent
            .map(|parent| {
                ancestor_nodes(&self.nodes, parent)
                    .map(|(_, node)| node)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ancestors.reverse();
        // Layout resolves fixed boxes against the viewport even below a
        // transformed ancestor. Keep structural opacity, but begin geometry at
        // the nearest fixed boundary instead of inheriting its outer scroll,
        // transform and clip chain.
        let geometry_start = if fixed {
            ancestors.len()
        } else {
            ancestors
                .iter()
                .rposition(|ancestor| {
                    ancestor.source_style.layout.position == nana_ui_core::PositionSpec::Fixed
                })
                .unwrap_or(0)
        };
        let mut transform = AffineTransform::IDENTITY;
        let mut opacity = 1.0;
        let mut clips = Vec::new();
        let mut blocks_3d = false;
        for (index, ancestor) in ancestors.into_iter().enumerate() {
            if !is_opacity_group(&self.nodes, ancestor) {
                opacity *= if visual {
                    self.resolved_local_opacity(ancestor)
                } else {
                    local_opacity(ancestor)
                };
            }
            if index < geometry_start {
                continue;
            }
            let layout = ancestor.layout;
            let local = if visual {
                self.resolved_local_transform(ancestor, false)
            } else {
                node_scene_transform(ancestor.source_style.layout.as_ref(), layout, false)
            };
            transform = transform.then(local);
            if ancestor.source_style.layout.fails_closed_3d_context() {
                blocks_3d = true;
            }
            if let Some((x, y, w, h)) = ancestor.source_style.layout.overflow_clip_box(
                layout.x,
                layout.y,
                layout.width,
                layout.height,
            ) && !(is_workspace_resize_handle(node) && Some(ancestor.id) == node.parent)
            {
                clips.push(ClipRegion {
                    bounds: SceneRect {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
            }
            if let Some(ComponentGeometry::EmptyState { root_clip, .. }) =
                ancestor.component_geometry.as_deref()
            {
                clips.push(ClipRegion {
                    bounds: scene_rect(*root_clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
            }
            if let Some(ComponentGeometry::ModalFrame { surface, body, .. }) =
                ancestor.component_geometry.as_deref()
            {
                node_dependent = true;
                let focus_inset = if node.focused { 3.0 } else { 0.0 };
                clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: surface.x - focus_inset,
                        y: surface.y - focus_inset,
                        width: surface.width + focus_inset * 2.0,
                        height: surface.height + focus_inset * 2.0,
                    },
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                if let Some(StandardVisual::ModalFrame { slots, .. }) =
                    ancestor.standard_visual.as_ref()
                    && slots
                        .body
                        .is_some_and(|body_root| self.node_descends_from(node.id, body_root))
                {
                    clips.push(ClipRegion {
                        bounds: scene_rect(*body),
                        transform,
                        corner_radius: 0.0,
                        polygon_clip: None,
                    });
                }
            }
            let ancestor_bounds = SceneRect {
                x: layout.x,
                y: layout.y,
                width: layout.width,
                height: layout.height,
            };
            if let Some(region) = clip_path_region(
                ancestor.source_style.layout.as_ref(),
                ancestor_bounds,
                transform,
            ) {
                clips.push(region);
            }
            transform = transform.then(AffineTransform::from_matrix([
                1.0,
                0.0,
                0.0,
                1.0,
                -ancestor.scroll_offset.x,
                -ancestor.scroll_offset.y,
            ]));
        }
        (
            (transform, opacity, clips.into(), blocks_3d),
            node_dependent,
        )
    }

    fn remove_node_primitives(&mut self, id: StableNodeId) {
        self.retire_node_primitives(id, |_| true);
    }

    /// Drop the node's primitives that `doomed` names, and their places in
    /// `ordered`.
    ///
    /// The key comes off the primitive rather than out of a fresh ancestor
    /// walk: a key computed against a node the delta has already replaced
    /// would not match the one this primitive entered `ordered` at, and would
    /// leave it there forever.
    fn retire_node_primitives(
        &mut self,
        id: StableNodeId,
        doomed: impl Fn(&RetainedPrimitive) -> bool,
    ) {
        let slots = self
            .node_primitives(id)
            .filter(|(_, held)| doomed(held))
            .map(|(slot, _)| *slot)
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(held) = self.primitives.remove(&slot) {
                self.ordered.remove(&held.key);
                self.structure_changed = true;
            }
        }
    }

    #[cfg(test)]
    fn primitives_for_node(&self, node: StableNodeId) -> impl Iterator<Item = &ScenePrimitive> {
        self.node_primitives(node).map(|(_, held)| &held.primitive)
    }

    fn node_primitive_count(&self, node: StableNodeId) -> usize {
        self.node_primitives(node).count()
    }

    fn node_primitives(
        &self,
        node: StableNodeId,
    ) -> impl Iterator<Item = (&PrimitiveId, &RetainedPrimitive)> {
        self.primitives.range(
            PrimitiveId { node, slot: 0 }..=PrimitiveId {
                node,
                slot: u64::MAX,
            },
        )
    }

    /// The paint-order key.
    ///
    /// Every primitive of a node sits under the same stack — the z-index and
    /// document order in it are the node's — so while the rebuild pass runs it
    /// is computed once per node and every primitive shares that `Arc`.
    fn scene_order_key(&self, primitive: &ScenePrimitive) -> SceneOrderKey {
        if let Ok(mut scratch) = self.rebuild_scratch.lock()
            && scratch.active
        {
            if let Some((node, stack)) = scratch.order_stack.as_ref()
                && *node == primitive.node
            {
                return SceneOrderKey::at(Arc::clone(stack), primitive);
            }
            let prefix = self.group_prefix_of(&mut scratch, primitive.node);
            let stack = order_stack(&self.nodes, &prefix, primitive);
            scratch.order_stack = Some((primitive.node, Arc::clone(&stack)));
            return SceneOrderKey::at(stack, primitive);
        }
        order_key(&self.nodes, &self.node_order, primitive)
    }

    /// A node's paint-order prefix: what it inherits from the chain above it,
    /// plus its own entry when it opens a stacking group.
    ///
    /// The inherited half is what siblings share, so it is the half worth
    /// remembering. A node that opens no group of its own **shares** that
    /// `Arc` rather than copying it.
    fn group_prefix_of(&self, scratch: &mut RebuildScratch, node: StableNodeId) -> GroupPrefix {
        let Some(candidate) = self.nodes.get(&node) else {
            return Arc::from(Vec::new());
        };
        // A viewport-fixed node starts its own chain: a triggered menu must not
        // stay inside the isolation group of whatever opened it.
        let inherited =
            if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
                Arc::from(Vec::new())
            } else {
                match candidate.parent {
                    Some(parent) => {
                        if let Some((held, prefix)) = scratch.inherited_prefix.as_ref()
                            && *held == parent
                        {
                            Arc::clone(prefix)
                        } else {
                            let prefix: Arc<[(i32, usize)]> =
                                group_prefix(&self.nodes, &self.node_order, parent).into();
                            scratch.inherited_prefix = Some((parent, Arc::clone(&prefix)));
                            prefix
                        }
                    }
                    None => Arc::from(Vec::new()),
                }
            };
        if !is_stacking_group(&self.nodes, candidate) {
            return inherited;
        }
        let mut prefix = inherited.to_vec();
        prefix.push((
            candidate.z_index,
            self.node_order.get(&node).copied().unwrap_or(0),
        ));
        Arc::from(prefix)
    }

    fn node_descends_from(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.nodes.get(&candidate).and_then(|node| node.parent);
        }
        false
    }

    fn insert_primitive(&mut self, primitive: ScenePrimitive) {
        let key = self.scene_order_key(&primitive);
        let build = self.build;
        if let Some(held) = self.primitives.get_mut(&primitive.id) {
            // A rebuild usually puts the same primitive back in the same
            // place. Re-entering `ordered` at a key it already holds is the
            // work this avoids.
            let moved = held.key != key;
            // A frame plan names custom nodes by renderer and resource, so one
            // slot swapping which it draws is a structural change even though
            // the slot itself stays.
            self.structure_changed |=
                Self::custom_binding(&held.primitive.kind) != Self::custom_binding(&primitive.kind);
            let previous = std::mem::replace(&mut held.key, key.clone());
            held.primitive = primitive;
            held.build = build;
            if moved {
                self.ordered.remove(&previous);
                self.ordered.insert(key);
            }
            return;
        }
        self.structure_changed = true;
        self.primitives.insert(
            primitive.id,
            RetainedPrimitive {
                primitive,
                key: key.clone(),
                build,
            },
        );
        self.ordered.insert(key);
    }
}

fn collect_unextracted_descendants(
    nodes: &SceneNodes,
    root: StableNodeId,
    extracted: &NodeSet,
    out: &mut Vec<StableNodeId>,
) {
    let Some(node) = nodes.get(&root) else {
        return;
    };
    for &child in node.children.iter() {
        if !extracted.contains(&child) {
            out.push(child);
        }
        collect_unextracted_descendants(nodes, child, extracted, out);
    }
}

#[derive(PartialEq)]
enum InheritedComponentClip {
    EmptyState(LayoutBox),
    ModalFrame {
        surface: LayoutBox,
        body: LayoutBox,
        body_root: Option<StableNodeId>,
    },
}

fn inherited_component_clip(node: &ExtractedNode) -> Option<InheritedComponentClip> {
    match node.component_geometry.as_deref() {
        Some(ComponentGeometry::EmptyState { root_clip, .. }) => {
            Some(InheritedComponentClip::EmptyState(*root_clip))
        }
        Some(ComponentGeometry::ModalFrame { surface, body, .. }) => {
            let body_root = match node.standard_visual.as_ref() {
                Some(StandardVisual::ModalFrame { slots, .. }) => slots.body,
                _ => None,
            };
            Some(InheritedComponentClip::ModalFrame {
                surface: *surface,
                body: *body,
                body_root,
            })
        }
        _ => None,
    }
}

fn local_opacity(node: &ExtractedNode) -> f32 {
    node.source_style
        .layout
        .opacity
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

fn is_workspace_resize_handle(node: &ExtractedNode) -> bool {
    matches!(
        node.kind.as_ref(),
        NodeKind::Element { tag } if tag == "workspace-resize-handle"
    )
}

fn is_descendant_of_rasterized_svg(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(parent) = node.parent else {
        return false;
    };
    for (_, parent) in ancestor_nodes(nodes, parent) {
        if parent
            .custom_render
            .as_ref()
            .is_some_and(|custom| custom.renderer.as_ref() == "nana.host-texture")
            && matches!(
                parent.kind.as_ref(),
                NodeKind::Element { tag } if tag.eq_ignore_ascii_case("svg")
            )
        {
            return true;
        }
    }
    false
}

fn is_descendant_of_icon_visual(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(parent) = node.parent else {
        return false;
    };
    ancestor_nodes(nodes, parent)
        .any(|(_, parent)| matches!(parent.standard_visual, Some(StandardVisual::Icon { .. })))
}

fn has_extracted_child(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    node.children.iter().any(|child| nodes.contains_key(child))
}

fn dest_filter_applies(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(filter) = node
        .source_style
        .layout
        .paint
        .filter
        .filter(|filter| !filter.is_identity())
    else {
        return false;
    };
    filter.blur_radius > 0.0
        || filter.drop_shadow.is_some()
        || has_extracted_child(nodes, node)
        || node
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
        || node.custom_render.is_some()
}

/// What a node's own paint style contributes to the paint-order keys of its
/// subtree — the reason a change to it costs a reorder.
///
/// The *value* of `opacity` is not in it: [`is_opacity_group`] only asks
/// whether the node is translucent at all, so fading a container from 0.35 to
/// 0.37 must not re-sort the scene. Neither is an identity filter, which
/// [`dest_filter_applies`] already reads as no filter.
#[derive(PartialEq)]
struct PaintOrderFacts {
    z_index: i32,
    translucent: bool,
    filter: Option<ColorFilter>,
    mix_blend: MixBlendMode,
    stacking_context: bool,
}

fn paint_order_facts(node: &ExtractedNode) -> PaintOrderFacts {
    let opacity = local_opacity(node);
    let paint = &node.source_style.layout.paint;
    PaintOrderFacts {
        z_index: node.z_index,
        translucent: opacity > 0.0 && opacity < 1.0,
        filter: paint.filter.filter(|filter| !filter.is_identity()),
        mix_blend: paint.mix_blend,
        stacking_context: node.source_style.layout.creates_paint_stacking_context(),
    }
}

fn is_opacity_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let opacity = local_opacity(node);
    let translucent = opacity > 0.0 && opacity < 1.0 && has_extracted_child(nodes, node);
    translucent
        || dest_filter_applies(nodes, node)
        || !node.source_style.layout.paint.mix_blend.is_normal()
}

/// Whether the two styles differ in anything a descendant inherits the
/// *projection* of — the transform it sits under, the clips it is cut by,
/// where the fixed boundary is.
///
/// `opacity` is left out. It does reach descendants, but not through the
/// projection: [`inherited_opacity`] decides whether it moved and asks for the
/// subtree to be rebuilt, and a fade that keeps a container a group does not
/// move anything at all. Bumping the attribute epoch for it would make every
/// descendant re-derive its transform and clips on the next frame's draw.
///
/// Every other field is compared as a whole, so a field added later cannot be
/// forgotten here.
fn inherited_layout_changed(
    old: &Arc<nana_ui_core::LayoutStyle>,
    new: &Arc<nana_ui_core::LayoutStyle>,
) -> bool {
    if Arc::ptr_eq(old, new) || old == new {
        return false;
    }
    if old.opacity == new.opacity {
        return true;
    }
    let mut without_the_fade = old.as_ref().clone();
    without_the_fade.opacity = new.opacity;
    &without_the_fade != new.as_ref()
}

/// The opacity this node multiplies into every descendant's primitive.
///
/// A group composites its subtree as one layer, so its own opacity is applied
/// there once and never reaches a descendant's primitive.
fn inherited_opacity(nodes: &SceneNodes, node: &ExtractedNode) -> f32 {
    if is_opacity_group(nodes, node) {
        1.0
    } else {
        local_opacity(node)
    }
}

fn is_stacking_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    is_opacity_group(nodes, node)
        || (has_extracted_child(nodes, node)
            && node.source_style.layout.creates_paint_stacking_context())
}

fn is_filter_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    dest_filter_applies(nodes, node)
}

fn is_dest_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    is_opacity_group(nodes, node)
}

fn inset_shadow_overlay(node: &ExtractedNode) -> Option<InsetShadowOverlay> {
    let shadow = node
        .source_style
        .layout
        .paint
        .box_shadows
        .iter()
        .copied()
        .find(|shadow| shadow.inset)?;
    Some(InsetShadowOverlay {
        elevation: ComponentElevation::from_box_shadow(shadow),
        bounds: SceneRect {
            x: node.layout.x,
            y: node.layout.y,
            width: node.layout.width,
            height: node.layout.height,
        },
        corner_radius: surface_corner_radii(
            node.source_style.layout.as_ref(),
            node.layout.width,
            node.layout.height,
        ),
    })
}

/// A parent chain longer than this is a bug in extraction, not a deep tree.
///
/// It replaces the per-call `HashSet` these walks used to carry as a cycle
/// guard. They run once per primitive per frame — a set allocation each cost
/// more than the walk it was protecting, and a depth cap protects the same
/// thing.
pub(super) const MAX_ANCESTOR_DEPTH: usize = 4096;

/// `node` and its ancestors by id, innermost first.
///
/// Yields an id whether or not the scene still holds that node, and stops
/// after one it does not: the chain cannot continue past a node whose parent
/// nobody knows.
pub(super) fn ancestor_ids(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = StableNodeId> + '_ {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        current = nodes.get(&id).and_then(|node| node.parent);
        Some(id)
    })
}

/// `node` and its ancestors, innermost first, stopping at the first one the
/// scene no longer holds.
fn ancestor_nodes(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = (StableNodeId, &ExtractedNode)> {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        let entry = nodes.get(&id)?;
        current = entry.parent;
        Some((id, entry.as_ref()))
    })
}

/// Dest groups per node, stamped with the scene instance they were read at.
type OpacityGroupCache = NodeMap<(u64, Arc<[OpacityGroup]>)>;

/// Ancestor layer factors, stamped with the scene instance and the attribute
/// epoch.
pub(super) type LayerFactorCache = NodeMap<((u64, u64), f32)>;

/// The scene's nodes.
///
/// `Arc` rather than the node itself: rebuilding a node's primitives needs an
/// owned handle while `&mut self` inserts them, and an `ExtractedNode` is 784
/// bytes. A container style change rebuilds every descendant, so that clone
/// used to be most of a megabyte of memmove per frame.
pub(super) type SceneNodes = NodeMap<Arc<ExtractedNode>>;

/// Which chain an [`AncestorState`] was read for: the parent it starts at,
/// whether the node breaks out of it, and whether the caller wanted the
/// presented values or the logical ones.
type AncestorStateKey = (StableNodeId, bool, bool);

/// What a remembered draw-time answer is stamped with. See
/// [`UiScene::draw_stamp`].
type DrawStamp = (u64, u64);

/// What a node inherits from the chain above it.
type AncestorState = (AffineTransform, f32, Arc<[ClipRegion]>, bool);

/// One `(z-index, document order)` per stacking group above a node, outermost
/// first.
type GroupPrefix = Arc<[(i32, usize)]>;

/// Two answers the primitive-rebuild pass keeps asking for: what a node
/// inherits from the chain above it, and where that chain puts it in paint
/// order. A container style change rebuilds every descendant, and a node owns
/// several primitives, so both are asked many times for the same parent.
///
/// One entry each, not maps: the pass walks the subtree in order, so the next
/// question almost always has the same answer as the last, and comparing two
/// words beats hashing. A miss costs exactly what this used to cost every time.
///
/// Deliberately not stamped and not always on. It is filled only while
/// `apply_delta` rebuilds primitives — where `self.nodes` and `self.node_order`
/// are already final and cannot move under it — and emptied on the way out.
#[derive(Default, Debug)]
struct RebuildScratch {
    active: bool,
    ancestor_state: Option<(AncestorStateKey, AncestorState)>,
    /// The same answer for the draw pass, which has no bracket to reset it:
    /// siblings ask for it one after another while the painter walks paint
    /// order, so one entry carries a whole container. Stamped, because nothing
    /// clears it. See [`UiScene::draw_stamp`].
    drawn_ancestor_state: Option<(AncestorStateKey, DrawStamp, AncestorState)>,
    /// The paint-order stack of the node being rebuilt, shared by all of its
    /// primitives.
    order_stack: Option<(StableNodeId, GroupPrefix)>,
    /// The same, for the chain *above* a node: siblings share it, so the pass
    /// pays the walk once per container instead of once per node.
    inherited_prefix: Option<(StableNodeId, GroupPrefix)>,
}

impl RebuildScratch {
    fn begin(&mut self) {
        self.active = true;
        self.ancestor_state = None;
        self.order_stack = None;
        self.inherited_prefix = None;
    }

    fn end(&mut self) {
        self.active = false;
        self.ancestor_state = None;
        self.order_stack = None;
        self.inherited_prefix = None;
    }
}

/// The list every node that is not itself a group shares with its parent.
fn empty_opacity_groups() -> Arc<[OpacityGroup]> {
    static EMPTY: std::sync::OnceLock<Arc<[OpacityGroup]>> = std::sync::OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

fn dest_group(nodes: &SceneNodes, id: StableNodeId, candidate: &ExtractedNode) -> OpacityGroup {
    OpacityGroup {
        node: id,
        opacity: local_opacity(candidate),
        filter: if dest_filter_applies(nodes, candidate) {
            candidate
                .source_style
                .layout
                .paint
                .filter
                .unwrap_or_default()
        } else {
            ColorFilter::default()
        },
        mix_blend: candidate.source_style.layout.paint.mix_blend,
        inset_shadow: inset_shadow_overlay(candidate),
    }
}

fn clip_path_region(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    transform: AffineTransform,
) -> Option<ClipRegion> {
    let clip_path = style.paint.clip_path.as_ref()?;
    match clip_path {
        ClipPath::Inset(inset) => {
            let [top, right, bottom, left] = inset.resolve_offsets(bounds.width, bounds.height);
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + left,
                    y: bounds.y + top,
                    width: (bounds.width - left - right).max(0.0),
                    height: (bounds.height - top - bottom).max(0.0),
                },
                transform,
                corner_radius: inset.resolve_round(bounds.width, bounds.height),
                polygon_clip: None,
            })
        }
        ClipPath::Polygon(_) => {
            let points = clip_path.resolve_polygon_points(bounds.width, bounds.height)?;
            let min_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::INFINITY, f32::min);
            let min_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::INFINITY, f32::min);
            let max_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let max_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let local_points = points
                .iter()
                .map(|point| [point[0] - min_x, point[1] - min_y])
                .collect();
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + min_x,
                    y: bounds.y + min_y,
                    width: (max_x - min_x).max(0.0),
                    height: (max_y - min_y).max(0.0),
                },
                transform,
                corner_radius: 0.0,
                polygon_clip: Some(local_points),
            })
        }
        ClipPath::Circle(_) | ClipPath::Ellipse(_) => {
            let [x, y, w, h] = clip_path.resolve_ellipse_rect(bounds.width, bounds.height)?;
            Some(ClipRegion::ellipse(
                SceneRect {
                    x: bounds.x + x,
                    y: bounds.y + y,
                    width: w.max(0.0),
                    height: h.max(0.0),
                },
                transform,
            ))
        }
    }
}

fn filter_groups_from(nodes: &SceneNodes, node: StableNodeId) -> Vec<FilterGroup> {
    let mut groups = Vec::new();
    for (id, candidate) in ancestor_nodes(nodes, node) {
        if is_filter_group(nodes, candidate) {
            groups.push(FilterGroup {
                node: id,
                filter: candidate
                    .source_style
                    .layout
                    .paint
                    .filter
                    .unwrap_or_default(),
            });
        }
    }
    groups.reverse();
    groups
}

/// `(z_index, document_order)` of every isolating stacking group above (and
/// including) `node`, outermost first. Opacity / filter / mix-blend dest groups
/// plus `isolation` and positioned + `z-index`.
fn group_prefix(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    node: StableNodeId,
) -> Vec<(i32, usize)> {
    let mut stack = Vec::new();
    for (id, candidate) in ancestor_nodes(nodes, node) {
        if is_stacking_group(nodes, candidate) {
            let z_index = candidate.z_index;
            let order = node_order.get(&id).copied().unwrap_or(0);
            stack.push((z_index, order));
        }
        if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
            // Triggered menus and overlay surfaces are viewport-fixed. Their
            // paint order must not stay inside a parent's isolation group, or
            // a later sibling card would cover an open Popover.
            break;
        }
    }
    stack.reverse();
    stack
}

fn primitive_paint_layer(slot: u64) -> u64 {
    match slot >> 32 {
        // Behind the glyphs (layer 2), above a custom-rendered backdrop
        // (layer 1, broken by the much larger slot).
        namespace if namespace == u64::from(DOCUMENT_TEXT_SELECTION) => 1,
        namespace if namespace == u64::from(TEXT_LINE_LABELS) => 40,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_MARKERS) => 20,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_LABELS) => 58,
        namespace if namespace == u64::from(TEXT_ATOM_ICONS) => 27,
        namespace if namespace == u64::from(TEXT_ATOM_LABELS) => 32,
        _ => slot,
    }
}

fn order_key(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    primitive: &ScenePrimitive,
) -> SceneOrderKey {
    let prefix: GroupPrefix = group_prefix(nodes, node_order, primitive.node).into();
    SceneOrderKey::at(order_stack(nodes, &prefix, primitive), primitive)
}

/// The stacking entries a node's primitives paint under, outermost first.
pub(super) fn order_stack(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    primitive: &ScenePrimitive,
) -> GroupPrefix {
    // A group's own paint is the prefix, before its descendants. Repeating its
    // outer z-index here incorrectly puts lower-z children behind its backplate.
    if nodes
        .get(&primitive.node)
        .is_some_and(|node| is_stacking_group(nodes, node))
    {
        return Arc::clone(prefix);
    }
    let mut stack = Vec::with_capacity(prefix.len() + 1);
    stack.extend_from_slice(prefix);
    stack.push((primitive.z_index, primitive.document_order));
    Arc::from(stack)
}

impl SceneOrderKey {
    fn at(stack: GroupPrefix, primitive: &ScenePrimitive) -> Self {
        Self {
            stack,
            paint_layer: primitive_paint_layer(primitive.id.slot),
            slot: primitive.id.slot,
            node: primitive.node,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scene_rect(bounds: LayoutBox) -> SceneRect {
    SceneRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(0.0),
        height: bounds.height.max(0.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_select_handle(
    emit: &mut impl FnMut(ScenePrimitive),
    id: StableNodeId,
    handle: &LayoutBox,
    color: [f32; 4],
    transform: AffineTransform,
    clips: &Arc<[ClipRegion]>,
    opacity: f32,
    z_index: i32,
    document_order: usize,
) {
    let center_x = handle.x + handle.width / 2.0;
    let center_y = handle.y + handle.height / 2.0;
    let widths = [8.0, 6.0, 4.0, 2.0];
    for (index, width) in widths.iter().copied().enumerate() {
        emit(visual_quad(
            &VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index,
                document_order,
            },
            3 + index as u64,
            SceneRect {
                x: center_x - width / 2.0,
                y: center_y - 1.5 + index as f32,
                width,
                height: 1.0,
            },
            VisualQuadStyle::solid(color),
        ));
    }
}

fn component_geometry_owns_text(geometry: Option<&ComponentGeometry>) -> bool {
    matches!(
        geometry,
        Some(
            ComponentGeometry::Button { .. }
                | ComponentGeometry::TextInput { .. }
                | ComponentGeometry::Switch { .. }
                | ComponentGeometry::Range { .. }
                | ComponentGeometry::Card { .. }
                | ComponentGeometry::StatusBadge { .. }
                | ComponentGeometry::ValidationMessage { .. }
                | ComponentGeometry::EmptyState { .. }
                | ComponentGeometry::LabeledValue { .. }
                | ComponentGeometry::SelectionOption { .. }
                | ComponentGeometry::ModalFrame { .. }
                | ComponentGeometry::Progress { .. }
                | ComponentGeometry::FormField { .. }
                | ComponentGeometry::Select { .. }
                | ComponentGeometry::ActionMenuItem { .. }
                | ComponentGeometry::MenuSurface { .. }
                | ComponentGeometry::TreeView { .. }
                | ComponentGeometry::CommandPalette { .. }
                | ComponentGeometry::CalendarHeatmap { .. }
                | ComponentGeometry::ReorderList { .. }
                | ComponentGeometry::NativeMarkdown { .. }
                | ComponentGeometry::SelectableRichText { .. }
                | ComponentGeometry::GraphCanvas { .. }
                | ComponentGeometry::ImageViewer { .. }
                | ComponentGeometry::KeyCaptureLayer { .. }
                | ComponentGeometry::KeymapLayer { .. }
        )
    )
}

#[allow(clippy::too_many_arguments)]
fn component_text_primitive(
    id: StableNodeId,
    slot: u64,
    region: &ComponentTextRegion,
    horizontal_alignment: TextHorizontalAlignment,
    ellipsis: bool,
    node: &ExtractedNode,
    transform: AffineTransform,
    clips: Arc<[ClipRegion]>,
    opacity: f32,
    document_order: usize,
) -> ScenePrimitive {
    let multiline = matches!(
        node.component_geometry.as_deref(),
        Some(ComponentGeometry::TextInput {
            multiline: true,
            ..
        })
    );
    let intrinsic_multiline = matches!(
        node.standard_visual,
        Some(StandardVisual::EmptyState { .. } | StandardVisual::ModalFrame { .. })
    ) || matches!(
        node.component_geometry.as_deref(),
        Some(
            ComponentGeometry::NativeMarkdown { .. } | ComponentGeometry::SelectableRichText { .. }
        )
    );
    ScenePrimitive {
        id: PrimitiveId { node: id, slot },
        node: id,
        bounds: scene_rect(region.bounds),
        transform,
        clips,
        opacity,
        z_index: node.z_index,
        document_order,
        kind: ScenePrimitiveKind::Text {
            content: region.content.to_string(),
            color: region.color.or(node.style.color),
            size: region.font_size,
            weight: region.font_weight,
            family: node.style.font_family.as_deref().map(str::to_owned),
            line_height: node.style.line_height,
            letter_spacing: node.style.letter_spacing,
            wrap: multiline || intrinsic_multiline,
            ellipsis,
            max_lines: None,
            shaping: if node.text_input.is_some() {
                TextShaping::Advanced
            } else {
                TextShaping::Auto
            },
            horizontal_alignment,
            vertical_alignment: if multiline || intrinsic_multiline {
                TextVerticalAlignment::Top
            } else {
                TextVerticalAlignment::Center
            },
            spans: scene_text_spans(node, Some(region), region.content.as_ref()),
            text_shadow: node.source_style.layout.paint.text_shadow,
            underline: node
                .source_style
                .layout
                .text_decoration
                .is_some_and(|d| d.underline),
            line_through: node
                .source_style
                .layout
                .text_decoration
                .is_some_and(|d| d.line_through),
            font_features: node
                .source_style
                .layout
                .font_features
                .clone()
                .unwrap_or_default(),
            italic: node.style.italic,
            wrap_break: node.source_style.layout.text_wrap_break(),
            opentype: SceneTextOpenType::from_computed(&node.style),
            layout: None,
        },
    }
}

fn surface_corner_radii(style: &nana_ui_core::LayoutStyle, width: f32, height: f32) -> [f32; 4] {
    let mut radii = style.resolved_border_radii(width, height);
    if let Some(ClipPath::Inset(inset)) = style.paint.clip_path.as_ref() {
        let round = inset.resolve_round(width, height);
        if round > 0.0 {
            radii = radii.map(|radius| radius.max(round));
        }
    }
    radii
}

fn scene_text_spans(
    node: &ExtractedNode,
    region: Option<&ComponentTextRegion>,
    content: &str,
) -> Vec<SceneTextSpan> {
    if node.text_spans.is_empty() {
        return Vec::new();
    }
    // TextInput 主文本区域：span 由 extraction 按 TextInput 呈现内容
    // （折叠态经显示视图重投后的显示串）空间产出，直接按 content 校验
    // 边界。其余区域（行号标签、钉住行、Select 标题、普通文本节点等）
    // 不承载该空间，维持值空间等值守卫——内容与 span 空间不一致时整批
    // 丢弃。
    let editor_main_text = region.is_some_and(|region| {
        matches!(
            node.component_geometry.as_deref(),
            Some(ComponentGeometry::TextInput { text, .. }) if std::ptr::eq(text, region)
        )
    });
    if !editor_main_text {
        let Some(source) = node.text.as_ref() else {
            return Vec::new();
        };
        if source.value != content {
            return Vec::new();
        }
    }
    node.text_spans
        .iter()
        .filter(|span| {
            span.start < span.end
                && span.end <= content.len()
                && content.is_char_boundary(span.start)
                && content.is_char_boundary(span.end)
        })
        .map(|span| SceneTextSpan {
            start: span.start,
            end: span.end,
            color: span.color,
        })
        .collect()
}

struct VisualPrimitiveContext<'a> {
    node: StableNodeId,
    transform: AffineTransform,
    clips: &'a Arc<[ClipRegion]>,
    opacity: f32,
    z_index: i32,
    document_order: usize,
}

struct VisualQuadStyle {
    background: Option<[f32; 4]>,
    border_color: Option<[f32; 4]>,
    border_width: f32,
    corner_radius: [f32; 4],
}

impl VisualQuadStyle {
    /// 纯色填充：无边框、直角。装饰条、参考线、高亮底色共用。
    fn solid(background: [f32; 4]) -> Self {
        Self {
            background: Some(background),
            border_color: None,
            border_width: 0.0,
            corner_radius: corner_radii(0.0),
        }
    }
}

fn visual_quad(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::Quad {
            background: style.background,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
            surface: QuadSurfacePaint::default(),
        },
    }
}

fn quad_surface_from_style(
    style: &nana_ui_core::LayoutStyle,
    width: f32,
    height: f32,
) -> QuadSurfacePaint {
    QuadSurfacePaint {
        background_image: style.paint.background_image.clone(),
        background_layers: style.paint.background_layers.clone(),
        content_image: style.paint.content_image.clone(),
        mask: style.paint.mask.clone(),
        polygon_clip: style
            .paint
            .clip_path
            .as_ref()
            .and_then(|path| path.resolve_polygon_points(width, height)),
        filter: style.paint.filter.filter(|filter| !filter.is_identity()),
        backdrop_filter: style
            .paint
            .backdrop_filter
            .filter(|filter| filter.is_active()),
        extra_shadows: style
            .paint
            .box_shadows
            .iter()
            .skip(1)
            .copied()
            .map(ComponentElevation::from_box_shadow)
            .collect(),
        outline_width: if style.paint.outline.is_active() {
            style.paint.outline.width
        } else {
            0.0
        },
        outline_color: style
            .paint
            .outline
            .is_active()
            .then_some(style.paint.outline.color)
            .flatten(),
        mix_blend: style.paint.mix_blend,
        border_widths: [0.0; 4],
        border_colors: [[0.0; 4]; 4],
        border_styles: [0; 4],
        border_image: if style.paint.unsupported_border_image {
            None
        } else {
            style.paint.border_image.clone()
        },
    }
}

#[cfg(any(feature = "charts", feature = "graph-canvas", feature = "rich-text"))]
fn visual_stroke(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    points: Vec<[f32; 2]>,
    width: f32,
    color: [f32; 4],
) -> ScenePrimitive {
    // Graph/TimeSeries keep `pattern: None`. Vue pathLength stays in SVG markup (resvg).
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::Stroke {
            points,
            width,
            color,
            widths: Vec::new(),
            cap: StrokeCap::Round,
            pattern: None,
        },
    }
}

fn insert_text_decoration_strokes(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    color: [f32; 4],
    deco: nana_ui_core::TextDecorationLine,
    mut sink: impl FnMut(ScenePrimitive),
) {
    let width = 1.0_f32.max(bounds.height * 0.06);
    let mut emit = |slot: u64, y: f32| {
        sink(ScenePrimitive {
            id: PrimitiveId {
                node: context.node,
                slot,
            },
            node: context.node,
            bounds: SceneRect {
                x: bounds.x,
                y: y - width * 0.5,
                width: bounds.width,
                height: width,
            },
            transform: context.transform,
            clips: Arc::clone(context.clips),
            opacity: context.opacity,
            z_index: context.z_index,
            document_order: context.document_order,
            kind: ScenePrimitiveKind::Stroke {
                points: vec![[bounds.x, y], [bounds.x + bounds.width, y]],
                width,
                color,
                widths: Vec::new(),
                cap: StrokeCap::Butt,
                pattern: None,
            },
        });
    };
    if deco.underline {
        emit(12, bounds.y + bounds.height - width);
    }
    if deco.line_through {
        emit(13, bounds.y + bounds.height * 0.5);
    }
}

/// 批次图元共用的批次壳：对逐项 bounds 求并集作为图元 bounds，kind 由
/// 调用方给出（QuadBatch / QuadColorBatch / IconBatch）。
fn batch_primitive(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    quad_bounds: Vec<SceneRect>,
    kind: impl FnOnce(Vec<SceneRect>) -> ScenePrimitiveKind,
) -> ScenePrimitive {
    debug_assert!(!quad_bounds.is_empty());
    let bounds = quad_bounds
        .iter()
        .copied()
        .reduce(|left, right| {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let right_edge = (left.x + left.width).max(right.x + right.width);
            let bottom_edge = (left.y + left.height).max(right.y + right.height);
            SceneRect {
                x,
                y,
                width: right_edge - x,
                height: bottom_edge - y,
            }
        })
        .unwrap_or_default();
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: kind(quad_bounds),
    }
}

fn visual_quad_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: impl IntoIterator<Item = SceneRect>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let quad_bounds = bounds.into_iter().collect::<Vec<_>>();
    batch_primitive(context, slot, quad_bounds, |bounds| {
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background: style.background,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
            surface: QuadSurfacePaint::default(),
        }
    })
}

fn visual_quad_color_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    items: impl IntoIterator<Item = (SceneRect, [f32; 4])>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let (quad_bounds, colors): (Vec<SceneRect>, Vec<[f32; 4]>) = items.into_iter().unzip();
    debug_assert_eq!(quad_bounds.len(), colors.len());
    batch_primitive(context, slot, quad_bounds, |bounds| {
        ScenePrimitiveKind::QuadColorBatch {
            bounds,
            colors,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
        }
    })
}

/// 锚定浮层面板的共享绘制原语（补全弹层 slot 90 与 hover 浮窗 slot 120
/// 共用）：圆角面板底 + 1px 边框，浮在编辑器内容之上。
fn overlay_panel_primitive(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    background: [f32; 4],
    border: [f32; 4],
) -> ScenePrimitive {
    visual_quad(
        context,
        slot,
        bounds,
        VisualQuadStyle {
            background: Some(background),
            border_color: Some(border),
            border_width: 1.0,
            corner_radius: corner_radii(6.0),
        },
    )
}

/// 锚定浮层文本的共享绘制原语：与编辑器文本同族，但不换行（浮层行高
/// 固定），超宽用省略号截断。
#[allow(clippy::too_many_arguments)]
fn overlay_text_primitive(
    id: StableNodeId,
    slot: u64,
    region: &ComponentTextRegion,
    horizontal_alignment: TextHorizontalAlignment,
    node: &ExtractedNode,
    transform: AffineTransform,
    clips: Arc<[ClipRegion]>,
    opacity: f32,
    document_order: usize,
) -> ScenePrimitive {
    ScenePrimitive {
        id: PrimitiveId { node: id, slot },
        node: id,
        bounds: scene_rect(region.bounds),
        transform,
        clips,
        opacity,
        z_index: node.z_index,
        document_order,
        kind: ScenePrimitiveKind::Text {
            content: region.content.to_string(),
            color: region.color.or(node.style.color),
            size: region.font_size,
            weight: region.font_weight,
            family: node.style.font_family.as_deref().map(str::to_owned),
            line_height: node.style.line_height,
            letter_spacing: node.style.letter_spacing,
            wrap: false,
            ellipsis: true,
            max_lines: None,
            shaping: if node.text_input.is_some() {
                TextShaping::Advanced
            } else {
                TextShaping::Auto
            },
            horizontal_alignment,
            vertical_alignment: TextVerticalAlignment::Center,
            spans: Vec::new(),
            text_shadow: None,
            underline: false,
            line_through: false,
            font_features: Vec::new(),
            italic: false,
            wrap_break: nana_ui_core::TextWrapBreak::default(),
            opentype: SceneTextOpenType::default(),
            layout: None,
        },
    }
}

#[cfg(test)]
mod tests;
