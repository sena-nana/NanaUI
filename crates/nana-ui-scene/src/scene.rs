mod composition;
mod order;
mod primitives;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use nana_ui_core::{
    BackgroundImage, BorderImageSpec, ClipPath, ColorFilter, ControlSize, DirSpec, DrawerSide,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, Icon, LineBreakSpec, LineHeightSpec,
    MixBlendMode, SwitchControlPosition, UI_METRICS, WordBreakSpec, WritingModeSpec,
    icon_y_on_text_glyph_center,
};
use nana_ui_runtime::{
    ComponentElevation, ComponentGeometry, ComponentTextRegion, CustomRenderNode, ExtractedNode,
    LayoutBox, NodeKind, StableNodeId, StandardVisual, TextFoldGutter, TextHorizontalAlignment,
    TextShaping, TextVerticalAlignment, TextWhitespaceKind,
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
    pub slot: u8,
}

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
    /// contiguous against siblings. Not full CSS Appendix E.
    stack: Vec<(i32, usize)>,
    slot: u8,
    node: StableNodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneDelta {
    pub updated_nodes: usize,
    pub removed_nodes: usize,
    pub rebuilt_primitives: usize,
    pub order_rebuilt: bool,
    pub primitive_count: usize,
}

#[derive(Debug)]
pub struct UiScene {
    nodes: HashMap<StableNodeId, ExtractedNode>,
    node_order: HashMap<StableNodeId, usize>,
    primitives: BTreeMap<PrimitiveId, ScenePrimitive>,
    ordered: BTreeSet<SceneOrderKey>,
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
            nodes: HashMap::new(),
            node_order: HashMap::new(),
            primitives: BTreeMap::new(),
            ordered: BTreeSet::new(),
            instance: next_scene_instance(),
        }
    }
}

impl Clone for UiScene {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            node_order: self.node_order.clone(),
            primitives: self.primitives.clone(),
            ordered: self.ordered.clone(),
            instance: next_scene_instance(),
        }
    }
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

    pub fn primitives(&self) -> impl Iterator<Item = &ScenePrimitive> {
        self.ordered.iter().filter_map(|key| {
            self.primitives.get(&PrimitiveId {
                node: key.node,
                slot: key.slot,
            })
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
    pub fn opacity_groups(&self, node: StableNodeId) -> Vec<OpacityGroup> {
        opacity_groups_from(&self.nodes, node)
    }

    /// Isolating filter groups from outermost to innermost that contain `node`.
    pub fn filter_groups(&self, node: StableNodeId) -> Vec<FilterGroup> {
        filter_groups_from(&self.nodes, node)
    }

    pub fn is_node_in_subtree(&self, root: StableNodeId, candidate: StableNodeId) -> bool {
        let mut current = Some(candidate);
        let mut visited = HashSet::new();
        while let Some(id) = current.filter(|id| visited.insert(*id)) {
            if id == root {
                return true;
            }
            current = self.nodes.get(&id).and_then(|node| node.parent);
        }
        false
    }

    /// Apply Runtime's dirty extraction and tombstone stream atomically.
    /// Updating or removing nodes refreshes [`Self::instance_id`]; an empty
    /// no-op keeps the current instance.
    pub fn apply_delta(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        let mut removed_nodes = 0;
        let mut changed = Vec::new();
        let mut hierarchy_changed = false;
        for id in removals {
            if let Some(old) = self.nodes.remove(&id) {
                removed_nodes += 1;
                hierarchy_changed |= old.parent.is_some() || !old.children.is_empty();
                self.remove_node_primitives(id);
            }
        }
        let mut updated_nodes = 0;
        let mut scroll_rebuild = Vec::new();
        let mut stacking_changed = false;
        for node in extracted {
            let previous = self.nodes.get(&node.id);
            hierarchy_changed |= previous
                .is_none_or(|old| old.parent != node.parent || old.children != node.children);
            let scroll_changed =
                previous.is_some_and(|old| old.scroll_offset != node.scroll_offset);
            // A node's z_index and group opacity are part of every descendant's
            // paint-order key, and descendants do not have to be re-extracted
            // with it, so such a change costs a full reorder.
            stacking_changed |= previous.is_some_and(|old| {
                old.z_index != node.z_index
                    || local_opacity(old).to_bits() != local_opacity(&node).to_bits()
                    || old.source_style.layout.paint.filter != node.source_style.layout.paint.filter
                    || old.source_style.layout.paint.mix_blend
                        != node.source_style.layout.paint.mix_blend
                    || old.source_style.layout.creates_paint_stacking_context()
                        != node.source_style.layout.creates_paint_stacking_context()
            });
            changed.push(node.id);
            if scroll_changed {
                scroll_rebuild.push(node.id);
            }
            // Drop the retained primitives while the old node is still in
            // place: their order keys are derived from it, and a key computed
            // against the new node would leave stale entries in `ordered`.
            self.remove_node_primitives(node.id);
            self.nodes.insert(node.id, node);
            updated_nodes += 1;
        }
        let order_rebuilt = (updated_nodes != 0 || removed_nodes != 0)
            && (hierarchy_changed || self.node_order.len() != self.nodes.len());
        let mut rebuilt_primitives = 0;
        if updated_nodes != 0 || removed_nodes != 0 {
            if order_rebuilt {
                self.rebuild_document_order();
                for primitive in self.primitives.values_mut() {
                    if let Some(order) = self.node_order.get(&primitive.node) {
                        primitive.document_order = *order;
                    }
                }
            }
            let mut rebuild = changed;
            if !scroll_rebuild.is_empty() {
                let extracted: HashSet<_> = rebuild.iter().copied().collect();
                for root in scroll_rebuild {
                    collect_unextracted_descendants(&self.nodes, root, &extracted, &mut rebuild);
                }
            }
            for &id in &rebuild {
                rebuilt_primitives += self.rebuild_node_primitives(id);
            }
            // Rebuilt nodes re-enter `ordered` at their own key, so a reorder is
            // only needed when keys the delta did not touch also moved.
            if order_rebuilt || stacking_changed {
                self.sort_primitives();
            }
            self.instance = next_scene_instance();
        }
        SceneDelta {
            updated_nodes,
            removed_nodes,
            rebuilt_primitives,
            order_rebuilt,
            primitive_count: self.primitives.len(),
        }
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&ScenePrimitive> {
        self.primitives.get(&id)
    }

    /// Rewrite one primitive kind and bump instance identity.
    ///
    /// Painter tests use this to probe stroke variants without a second
    /// Runtime extraction ABI.
    pub fn replace_primitive_kind(&mut self, id: PrimitiveId, kind: ScenePrimitiveKind) -> bool {
        let Some(primitive) = self.primitives.get_mut(&id) else {
            return false;
        };
        primitive.kind = kind;
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
        component_geometry_owns_text(parent.component_geometry.as_ref())
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
        let text_box = match &host.component_geometry {
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
        let mut ancestors = Vec::new();
        let mut parent = node.parent;
        let mut visited = HashSet::new();
        while let Some(id) = parent.filter(|id| visited.insert(*id)) {
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            ancestors.push(node);
            parent = node.parent;
        }
        ancestors.reverse();
        let mut transform = AffineTransform::IDENTITY;
        let mut opacity = 1.0;
        let mut clips = Vec::new();
        let mut blocks_3d = false;
        for ancestor in ancestors {
            let layout = ancestor.layout;
            let local = node_scene_transform(ancestor.source_style.layout.as_ref(), layout, false);
            transform = transform.then(local);
            if ancestor.source_style.layout.fails_closed_3d_context() {
                blocks_3d = true;
            }
            if !is_opacity_group(&self.nodes, ancestor) {
                opacity *= local_opacity(ancestor);
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
                ancestor.component_geometry.as_ref()
            {
                clips.push(ClipRegion {
                    bounds: scene_rect(*root_clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
            }
            if let Some(ComponentGeometry::ModalFrame { surface, body, .. }) =
                ancestor.component_geometry.as_ref()
            {
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
        (transform, opacity, clips.into(), blocks_3d)
    }

    fn remove_node_primitives(&mut self, id: StableNodeId) {
        let slots = self
            .primitives_for_node(id)
            .map(|primitive| primitive.id)
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(primitive) = self.primitives.remove(&slot) {
                self.ordered
                    .remove(&order_key(&self.nodes, &self.node_order, &primitive));
            }
        }
    }

    fn primitives_for_node(&self, node: StableNodeId) -> impl Iterator<Item = &ScenePrimitive> {
        self.primitives
            .range(
                PrimitiveId { node, slot: 0 }..=PrimitiveId {
                    node,
                    slot: u8::MAX,
                },
            )
            .map(|(_, primitive)| primitive)
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
        let key = order_key(&self.nodes, &self.node_order, &primitive);
        if let Some(previous) = self.primitives.insert(primitive.id, primitive) {
            self.ordered
                .remove(&order_key(&self.nodes, &self.node_order, &previous));
        }
        self.ordered.insert(key);
    }
}

fn collect_unextracted_descendants(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    root: StableNodeId,
    extracted: &HashSet<StableNodeId>,
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

fn is_descendant_of_rasterized_svg(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node: &ExtractedNode,
) -> bool {
    let mut current = node.parent;
    let mut visited = HashSet::new();
    while let Some(id) = current.filter(|id| visited.insert(*id)) {
        let Some(parent) = nodes.get(&id) else {
            break;
        };
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
        current = parent.parent;
    }
    false
}

fn is_descendant_of_icon_visual(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node: &ExtractedNode,
) -> bool {
    let mut current = node.parent;
    let mut visited = HashSet::new();
    while let Some(id) = current.filter(|id| visited.insert(*id)) {
        let Some(parent) = nodes.get(&id) else {
            break;
        };
        if matches!(parent.standard_visual, Some(StandardVisual::Icon { .. })) {
            return true;
        }
        current = parent.parent;
    }
    false
}

fn has_extracted_child(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
    node.children.iter().any(|child| nodes.contains_key(child))
}

fn dest_filter_applies(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
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

fn is_opacity_group(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
    let opacity = local_opacity(node);
    let translucent = opacity > 0.0 && opacity < 1.0 && has_extracted_child(nodes, node);
    translucent
        || dest_filter_applies(nodes, node)
        || !node.source_style.layout.paint.mix_blend.is_normal()
}

fn is_stacking_group(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
    is_opacity_group(nodes, node)
        || (has_extracted_child(nodes, node)
            && node.source_style.layout.creates_paint_stacking_context())
}

fn is_filter_group(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
    dest_filter_applies(nodes, node)
}

fn is_dest_group(nodes: &HashMap<StableNodeId, ExtractedNode>, node: &ExtractedNode) -> bool {
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

fn opacity_groups_from(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node: StableNodeId,
) -> Vec<OpacityGroup> {
    let mut groups = Vec::new();
    let mut current = Some(node);
    let mut visited = HashSet::new();
    while let Some(id) = current.filter(|id| visited.insert(*id)) {
        let Some(candidate) = nodes.get(&id) else {
            break;
        };
        if is_dest_group(nodes, candidate) {
            groups.push(OpacityGroup {
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
            });
        }
        current = candidate.parent;
    }
    groups.reverse();
    groups
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

fn filter_groups_from(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node: StableNodeId,
) -> Vec<FilterGroup> {
    let mut groups = Vec::new();
    let mut current = Some(node);
    let mut visited = HashSet::new();
    while let Some(id) = current.filter(|id| visited.insert(*id)) {
        let Some(candidate) = nodes.get(&id) else {
            break;
        };
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
        current = candidate.parent;
    }
    groups.reverse();
    groups
}

/// `(z_index, document_order)` of every isolating stacking group above (and
/// including) `node`, outermost first. Opacity / filter / mix-blend dest groups
/// plus `isolation` and positioned + `z-index`.
fn group_prefix(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node_order: &HashMap<StableNodeId, usize>,
    node: StableNodeId,
) -> Vec<(i32, usize)> {
    let mut stack = Vec::new();
    let mut current = Some(node);
    let mut visited = HashSet::new();
    while let Some(id) = current.filter(|id| visited.insert(*id)) {
        let Some(candidate) = nodes.get(&id) else {
            break;
        };
        if is_stacking_group(nodes, candidate) {
            let z_index = candidate.z_index;
            let order = node_order.get(&id).copied().unwrap_or(0);
            stack.push((z_index, order));
        }
        current = candidate.parent;
    }
    stack.reverse();
    stack
}

fn order_key(
    nodes: &HashMap<StableNodeId, ExtractedNode>,
    node_order: &HashMap<StableNodeId, usize>,
    primitive: &ScenePrimitive,
) -> SceneOrderKey {
    let mut stack = group_prefix(nodes, node_order, primitive.node);
    stack.push((primitive.z_index, primitive.document_order));
    SceneOrderKey {
        stack,
        slot: primitive.id.slot,
        node: primitive.node,
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
            3 + index as u8,
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
    slot: u8,
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
        node.component_geometry,
        Some(ComponentGeometry::TextInput {
            multiline: true,
            ..
        })
    );
    let intrinsic_multiline = matches!(
        node.standard_visual,
        Some(StandardVisual::EmptyState { .. } | StandardVisual::ModalFrame { .. })
    ) || matches!(
        node.component_geometry,
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
            spans: scene_text_spans(node, region.content.as_ref()),
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

fn scene_text_spans(node: &ExtractedNode, content: &str) -> Vec<SceneTextSpan> {
    if node.text_spans.is_empty() {
        return Vec::new();
    }
    let Some(source) = node.text.as_ref() else {
        return Vec::new();
    };
    if source.value != content {
        return Vec::new();
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
    slot: u8,
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

#[cfg(any(feature = "charts", feature = "graph-canvas"))]
fn visual_stroke(
    context: &VisualPrimitiveContext<'_>,
    slot: u8,
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
    let mut emit = |slot: u8, y: f32| {
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
    slot: u8,
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
    slot: u8,
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
    slot: u8,
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
    slot: u8,
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
    slot: u8,
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
        },
    }
}

#[cfg(test)]
mod tests;
