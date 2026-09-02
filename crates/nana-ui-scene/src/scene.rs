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
    TextShaping, TextVerticalAlignment, TextWhitespaceKind, TimeSeriesChart,
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

    /// Build the default frame pass. Custom operations remain in exact scene
    /// order and split standard draw segments, allowing a backend extension to
    /// encode a real pass between ordinary UI items. Opaque custom resources
    /// are explicit external graph inputs rather than hidden backend state.
    pub fn frame_graph(&self, target: ResourceId) -> Result<CompiledRenderGraph, GraphError> {
        let mut graph = RenderGraph::new();
        graph.add_resource(RenderResource {
            id: target,
            label: "ui-target".into(),
            external: true,
        })?;
        let mut next_resource = 1_u64;
        let mut custom_nodes: BTreeMap<Arc<str>, (PrimitiveId, CustomRenderNode)> = BTreeMap::new();
        for primitive in self.primitives() {
            let ScenePrimitiveKind::Custom { node: custom, .. } = &primitive.kind else {
                continue;
            };
            if let Some((_, previous)) = custom_nodes.get(&custom.resource)
                && (previous.revision != custom.revision || previous.renderer != custom.renderer)
            {
                return Err(GraphError::ConflictingExternalResource(
                    custom.resource.to_string(),
                ));
            }
            custom_nodes
                .entry(custom.resource.clone())
                .or_insert((primitive.id, custom.clone()));
        }
        let custom_resources = custom_nodes
            .into_iter()
            .map(|(resource, (representative, _))| {
                while ResourceId(next_resource) == target {
                    next_resource += 1;
                }
                let id = ResourceId(next_resource);
                next_resource += 1;
                (resource, (id, representative))
            })
            .collect::<HashMap<_, _>>();
        for (label, (id, _)) in &custom_resources {
            graph.add_resource(RenderResource {
                id: *id,
                label: label.to_string(),
                external: true,
            })?;
        }
        let mut pass_id = 1_u64;
        let mut ordered_resources = custom_resources.iter().collect::<Vec<_>>();
        ordered_resources.sort_by_key(|(label, _)| *label);
        for (label, (resource, representative)) in ordered_resources {
            graph.add_pass(RenderPass {
                id: PassId(pass_id),
                label: format!("prepare:{label}"),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: *resource,
                    mode: AccessMode::Write,
                }],
                operations: vec![RenderOperation::PrepareExternal(*representative)],
            })?;
            pass_id += 1;
        }
        let mut standard = Vec::new();
        let flush_standard = |graph: &mut RenderGraph,
                              pass_id: &mut u64,
                              standard: &mut Vec<RenderOperation>|
         -> Result<(), GraphError> {
            if standard.is_empty() {
                return Ok(());
            }
            graph.add_pass(RenderPass {
                id: PassId(*pass_id),
                label: "ui-standard".into(),
                dependencies: Vec::new(),
                resources: vec![ResourceAccess {
                    resource: target,
                    mode: AccessMode::ReadWrite,
                }],
                operations: std::mem::take(standard),
            })?;
            *pass_id += 1;
            Ok(())
        };
        for primitive in self.primitives() {
            match &primitive.kind {
                ScenePrimitiveKind::Custom { node: custom, .. } => {
                    flush_standard(&mut graph, &mut pass_id, &mut standard)?;
                    let resource = custom_resources[&custom.resource].0;
                    graph.add_pass(RenderPass {
                        id: PassId(pass_id),
                        label: format!("custom:{}", custom.renderer),
                        dependencies: Vec::new(),
                        resources: vec![
                            ResourceAccess {
                                resource: target,
                                mode: AccessMode::ReadWrite,
                            },
                            ResourceAccess {
                                resource,
                                mode: AccessMode::Read,
                            },
                        ],
                        operations: vec![RenderOperation::InvokeCustom(primitive.id)],
                    })?;
                    pass_id += 1;
                }
                _ => standard.push(RenderOperation::Draw(primitive.id)),
            }
        }
        flush_standard(&mut graph, &mut pass_id, &mut standard)?;
        graph.compile()
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

    fn rebuild_document_order(&mut self) {
        self.node_order.clear();
        let mut roots = self
            .nodes
            .values()
            .filter(|node| node.parent.is_none() || !self.nodes.contains_key(&node.parent.unwrap()))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut visited = HashSet::new();
        let mut order = 0;
        for root in roots {
            self.visit_order(root, &mut visited, &mut order);
        }
        // Malformed/incomplete deltas must not make retained nodes disappear.
        let mut detached = self.nodes.keys().copied().collect::<Vec<_>>();
        detached.sort_unstable();
        for id in detached {
            if !visited.contains(&id) {
                self.visit_order(id, &mut visited, &mut order);
            }
        }
    }

    fn visit_order(
        &mut self,
        id: StableNodeId,
        visited: &mut HashSet<StableNodeId>,
        order: &mut usize,
    ) {
        if !visited.insert(id) {
            return;
        }
        self.node_order.insert(id, *order);
        *order += 1;
        let children = self
            .nodes
            .get(&id)
            .map(|node| Arc::clone(&node.children))
            .unwrap_or_else(|| Arc::new(Vec::new()));
        for child in children.iter().copied() {
            self.visit_order(child, visited, order);
        }
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

    fn sort_primitives(&mut self) {
        // The group prefix is a per-node parent walk; a node usually owns
        // several primitives, so walk once and reuse it.
        let mut prefixes: HashMap<StableNodeId, Vec<(i32, usize)>> = HashMap::new();
        let keys: Vec<SceneOrderKey> = self
            .primitives
            .values()
            .map(|primitive| {
                let prefix = prefixes
                    .entry(primitive.node)
                    .or_insert_with(|| group_prefix(&self.nodes, &self.node_order, primitive.node));
                let mut stack = Vec::with_capacity(prefix.len() + 1);
                stack.extend_from_slice(prefix);
                stack.push((primitive.z_index, primitive.document_order));
                SceneOrderKey {
                    stack,
                    slot: primitive.id.slot,
                    node: primitive.node,
                }
            })
            .collect();
        self.ordered.clear();
        self.ordered.extend(keys);
    }

    fn rebuild_node_primitives(&mut self, id: StableNodeId) -> usize {
        self.remove_node_primitives(id);
        let Some(node) = self.nodes.get(&id).cloned() else {
            return 0;
        };
        if is_descendant_of_rasterized_svg(&self.nodes, &node)
            || is_descendant_of_icon_visual(&self.nodes, &node)
        {
            return 0;
        }
        let before = self.primitives.len();
        let (parent_transform, parent_opacity, parent_clips, parent_blocks_3d) =
            self.ancestor_state(&node);
        let layout = node.layout;
        let bounds = SceneRect {
            x: layout.x,
            y: layout.y,
            width: layout.width,
            height: layout.height,
        };
        let local_transform =
            node_scene_transform(node.source_style.layout.as_ref(), layout, parent_blocks_3d);
        let transform = parent_transform.then(local_transform);
        let local_opacity = local_opacity(&node);
        let opacity = if is_opacity_group(&self.nodes, &node) {
            parent_opacity
        } else {
            parent_opacity * local_opacity
        };
        let style = node.source_style.layout.as_ref();
        let clips: Arc<[ClipRegion]> = {
            let mut chain = if let Some((x, y, w, h)) = node.source_style.layout.overflow_clip_box(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            ) {
                let mut own = parent_clips.to_vec();
                own.push(ClipRegion::axis_aligned(
                    SceneRect {
                        x,
                        y,
                        width: w,
                        height: h,
                    },
                    transform,
                ));
                own
            } else {
                parent_clips.to_vec()
            };
            if let Some(region) = clip_path_region(style, bounds, transform) {
                chain.push(region);
            }
            chain.into()
        };
        let surface_clips: Arc<[ClipRegion]> = Arc::clone(&clips);
        let empty_state_content_clips: Arc<[ClipRegion]> =
            if let Some(ComponentGeometry::EmptyState { content_clip, .. }) =
                node.component_geometry.as_ref()
            {
                let mut content_clips = clips.to_vec();
                content_clips.push(ClipRegion {
                    bounds: scene_rect(*content_clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                content_clips.into()
            } else {
                Arc::clone(&clips)
            };
        let text_input_clips: Arc<[ClipRegion]> =
            if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                let padding = node
                    .source_style
                    .layout
                    .resolved_padding_against(Some(bounds.width));
                let border = node.source_style.layout.resolved_border_width();
                let mut text_input_clips = clips.to_vec();
                text_input_clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: bounds.x + border + padding.left,
                        y: bounds.y + border + padding.top,
                        width: (bounds.width - border * 2.0 - padding.left - padding.right)
                            .max(0.0),
                        height: (bounds.height - border * 2.0 - padding.top - padding.bottom)
                            .max(0.0),
                    },
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                text_input_clips.into()
            } else {
                Arc::clone(&clips)
            };
        let node_order = self.node_order.get(&id).copied().unwrap_or_default();
        if node.style.visible && opacity > 0.0 {
            let standard_visual_uses_root_surface = matches!(
                node.standard_visual,
                Some(
                    StandardVisual::Button { .. }
                        | StandardVisual::TextInput { .. }
                        | StandardVisual::Icon { .. }
                        | StandardVisual::Switch { .. }
                        | StandardVisual::Card { .. }
                        | StandardVisual::ListItem { .. }
                        | StandardVisual::StatusBadge { .. }
                        | StandardVisual::SelectionOption { .. }
                        | StandardVisual::ModalFrame { .. }
                        | StandardVisual::Toast { .. }
                        | StandardVisual::XYPad { .. }
                        | StandardVisual::Select { .. }
                        | StandardVisual::ActionMenuItem { .. }
                        | StandardVisual::TreeView { .. }
                        | StandardVisual::CommandPalette { .. }
                )
            );
            let surface_border_color =
                if matches!(node.standard_visual, Some(StandardVisual::Switch { .. })) {
                    None
                } else {
                    node.style.border_color
                };
            let (surface_background, surface_border_color, surface_border_width) =
                match node.component_geometry.as_ref() {
                    Some(ComponentGeometry::Button {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::TextInput {
                        background,
                        border,
                        border_width,
                        ..
                    }) => (*background, *border, *border_width),
                    Some(ComponentGeometry::StatusBadge { background, .. }) => {
                        (Some(*background), None, 0.0)
                    }
                    _ => {
                        if matches!(node.standard_visual, Some(StandardVisual::Switch { .. })) {
                            (node.style.background, None, 0.0)
                        } else {
                            let edges = style.paint_border_edges();
                            (
                                node.style.background,
                                style.resolved_border_color().or(surface_border_color),
                                edges.top.max(edges.right).max(edges.bottom).max(edges.left),
                            )
                        }
                    }
                };
            if !matches!(
                node.standard_visual,
                Some(StandardVisual::MenuSurface { .. })
            ) && (style.has_surface_paint()
                || style.paints_any_border()
                || ((node.standard_visual.is_none() || standard_visual_uses_root_surface)
                    && (node.style.background.is_some()
                        || node.style.border_color.is_some()
                        || style.paints_any_border())))
            {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 0 },
                    node: id,
                    bounds,
                    transform,
                    clips: Arc::clone(&surface_clips),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Quad {
                        background: surface_background,
                        border_color: surface_border_color,
                        border_width: surface_border_width,
                        corner_radius: surface_corner_radii(style, bounds.width, bounds.height),
                        shadow: style
                            .paint
                            .primary_box_shadow()
                            .map(ComponentElevation::from_box_shadow)
                            .or(match node.component_geometry.as_ref() {
                                Some(ComponentGeometry::Card { elevation, .. }) => *elevation,
                                _ => None,
                            }),
                        surface: {
                            let mut surface =
                                quad_surface_from_style(style, bounds.width, bounds.height);
                            if is_filter_group(&self.nodes, &node) {
                                surface.filter = None;
                            }
                            let component_owns_border = matches!(
                                node.component_geometry.as_ref(),
                                Some(
                                    ComponentGeometry::Button { .. }
                                        | ComponentGeometry::TextInput { .. }
                                )
                            ) || matches!(
                                node.standard_visual,
                                Some(StandardVisual::Switch { .. })
                            );
                            if !component_owns_border {
                                let edges = style.paint_border_edges();
                                surface.border_widths =
                                    [edges.top, edges.right, edges.bottom, edges.left];
                                surface.border_colors = style.paint_border_edge_colors();
                                surface.border_styles = style.paint_border_style_codes();
                            }
                            surface
                        },
                    },
                });
            }
            if let Some(custom) = node.custom_render.clone() {
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 1 },
                    node: id,
                    bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Custom {
                        node: custom,
                        mask: style.paint.mask.clone(),
                    },
                });
            }
            let component_owns_text =
                component_geometry_owns_text(node.component_geometry.as_ref());
            if let Some(text) = node.text.as_ref().filter(|text| {
                !text.value.is_empty()
                    && !component_owns_text
                    && !self.parent_already_paints_text(&node)
            }) {
                let padding = style.resolved_padding_against(Some(bounds.width));
                let border = style.resolved_border_width();
                let leading_visual = match node.standard_visual {
                    Some(StandardVisual::Checkbox { size, .. }) => {
                        size.indicator_size() + size.indicator_gap()
                    }
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::Start,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let trailing_visual = match node.standard_visual {
                    Some(StandardVisual::Switch {
                        control_position: SwitchControlPosition::End,
                        ..
                    }) => 38.0,
                    _ => 0.0,
                };
                let mut text_bounds = SceneRect {
                    x: bounds.x + border + padding.left + leading_visual,
                    y: bounds.y + border + padding.top,
                    width: (bounds.width
                        - border * 2.0
                        - padding.left
                        - padding.right
                        - leading_visual
                        - trailing_visual)
                        .max(0.0),
                    height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
                };
                if let Some(ComponentGeometry::ListItem {
                    content: Some(content),
                    ..
                }) = node.component_geometry
                {
                    text_bounds = scene_rect(content);
                }
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId { node: id, slot: 2 },
                    node: id,
                    bounds: text_bounds,
                    transform,
                    clips: clips.clone(),
                    opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind: ScenePrimitiveKind::Text {
                        content: text.value.clone(),
                        color: node.style.color,
                        size: node.style.font_size,
                        weight: node.style.font_weight,
                        family: node.style.font_family.as_deref().map(str::to_owned),
                        line_height: node.style.line_height,
                        letter_spacing: node.style.letter_spacing,
                        wrap: style.text_wraps(),
                        ellipsis: style.uses_text_ellipsis(),
                        max_lines: style.resolved_line_clamp(),
                        shaping: if node.text_input.is_some() {
                            TextShaping::Advanced
                        } else {
                            TextShaping::Auto
                        },
                        horizontal_alignment: node.source_style.text_horizontal_alignment,
                        vertical_alignment: node.source_style.text_vertical_alignment,
                        spans: scene_text_spans(&node, &text.value),
                        text_shadow: style.paint.text_shadow,
                        underline: style.text_decoration.is_some_and(|d| d.underline),
                        line_through: style.text_decoration.is_some_and(|d| d.line_through),
                        font_features: style.font_features.clone().unwrap_or_default(),
                        italic: node.style.italic,
                        wrap_break: style.text_wrap_break(),
                        opentype: SceneTextOpenType::from_computed(&node.style),
                    },
                });
                if let Some(deco) = style.text_decoration.filter(|d| d.is_active()) {
                    insert_text_decoration_strokes(
                        self,
                        id,
                        text_bounds,
                        transform,
                        clips.clone(),
                        opacity,
                        node.z_index,
                        node_order,
                        node.style.color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        deco,
                    );
                }
            }
            match node.component_geometry.as_ref() {
                Some(ComponentGeometry::ModalFrame {
                    scrim,
                    surface,
                    body: _,
                    title,
                    description,
                    body_text,
                    background,
                    elevation,
                    ..
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        10,
                        scene_rect(*scrim),
                        VisualQuadStyle {
                            background: Some([0.0, 0.0, 0.0, 0.45]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    let radius = UI_METRICS.radius_md;
                    let docked = match node.standard_visual.as_ref() {
                        Some(StandardVisual::ModalFrame {
                            kind: nana_ui_runtime::ModalSurfaceKind::Drawer(side),
                            ..
                        }) => Some(*side),
                        _ => None,
                    };
                    let mut surface_bounds = scene_rect(*surface);
                    let mut surface_clips = clips.to_vec();
                    if let Some(side) = docked {
                        surface_clips.push(ClipRegion {
                            bounds: scene_rect(*scrim),
                            transform,
                            corner_radius: 0.0,
                            polygon_clip: None,
                        });
                        match side {
                            DrawerSide::Right => surface_bounds.width += radius,
                            DrawerSide::Left => {
                                surface_bounds.x -= radius;
                                surface_bounds.width += radius;
                            }
                            DrawerSide::Bottom => surface_bounds.height += radius,
                        }
                    }
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 11 },
                        node: id,
                        bounds: surface_bounds,
                        transform,
                        clips: surface_clips.into(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Quad {
                            background: Some(*background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(radius),
                            shadow: Some(*elevation),
                            surface: QuadSurfacePaint::default(),
                        },
                    });
                    self.insert_primitive(component_text_primitive(
                        id,
                        12,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(description) = description {
                        self.insert_primitive(component_text_primitive(
                            id,
                            13,
                            description,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(body_text) = body_text {
                        self.insert_primitive(component_text_primitive(
                            id,
                            14,
                            body_text,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Button { label, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Center,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::TextInput {
                    text,
                    selection,
                    selection_color,
                    steppers,
                    ..
                }) => {
                    if let Some(steppers) = steppers {
                        for (slot, icon, bounds, color) in [
                            (
                                8,
                                nana_ui_core::Icon::ChevronUp,
                                steppers.increment,
                                steppers.increment_color,
                            ),
                            (
                                9,
                                nana_ui_core::Icon::ChevronDown,
                                steppers.decrement,
                                steppers.decrement_color,
                            ),
                        ] {
                            let extent = steppers
                                .glyph_size
                                .min(bounds.width)
                                .min(bounds.height)
                                .max(0.0);
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId { node: id, slot },
                                node: id,
                                bounds: SceneRect {
                                    x: bounds.x + (bounds.width - extent) / 2.0,
                                    y: bounds.y + (bounds.height - extent) / 2.0,
                                    width: extent,
                                    height: extent,
                                },
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Icon {
                                    icon,
                                    color: Some(color),
                                },
                            });
                        }
                    }
                    if !selection.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &text_input_clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            1,
                            selection.iter().map(|selection| scene_rect(*selection)),
                            VisualQuadStyle {
                                background: Some(*selection_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        text,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        text_input_clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::Switch { label, hint, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(hint) = hint {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            hint,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Range {
                    label, value, unit, ..
                }) => {
                    if let Some(label) = label {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        6,
                        value,
                        TextHorizontalAlignment::End,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(unit) = unit {
                        self.insert_primitive(component_text_primitive(
                            id,
                            7,
                            unit,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::Card {
                    title: Some(title), ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::StatusBadge {
                    indicator,
                    label,
                    foreground,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: Some(*foreground),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(999.0),
                        },
                    ));
                }
                Some(ComponentGeometry::ValidationMessage {
                    indicator,
                    label,
                    foreground,
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: None,
                            border_color: Some(*foreground),
                            border_width: 1.0,
                            corner_radius: corner_radii(999.0),
                        },
                    ));
                }
                Some(ComponentGeometry::EmptyState {
                    icon,
                    title,
                    message,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        empty_state_content_clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some((icon, bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*bounds),
                            transform,
                            clips: empty_state_content_clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    if let Some(message) = message {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            message,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            empty_state_content_clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::LabeledValue { label, value, .. }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        3,
                        value,
                        TextHorizontalAlignment::End,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::SelectionOption {
                    icon,
                    label,
                    focus_ring,
                    indicator,
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        if indicator.is_some() {
                            TextHorizontalAlignment::Start
                        } else {
                            TextHorizontalAlignment::Center
                        },
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(indicator) = indicator {
                        let indicator_context = VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        };
                        self.insert_primitive(visual_quad(
                            &indicator_context,
                            8,
                            scene_rect(indicator.ring),
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(indicator.ring_color),
                                border_width: if indicator.dot.is_some() { 2.0 } else { 1.0 },
                                corner_radius: corner_radii(indicator.ring.height / 2.0),
                            },
                        ));
                        if let Some((dot, color)) = indicator.dot {
                            self.insert_primitive(visual_quad(
                                &indicator_context,
                                9,
                                scene_rect(dot),
                                VisualQuadStyle {
                                    background: Some(color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(dot.height / 2.0),
                                },
                            ));
                        }
                    }
                    if let Some((icon, icon_bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*icon_bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    if let Some(color) = focus_ring {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &parent_clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            7,
                            SceneRect {
                                x: bounds.x - 4.0,
                                y: bounds.y - 4.0,
                                width: bounds.width + 8.0,
                                height: bounds.height + 8.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(*color),
                                border_width: 2.0,
                                corner_radius: focus_ring_corner_radius(
                                    style,
                                    SceneRect {
                                        x: bounds.x - 4.0,
                                        y: bounds.y - 4.0,
                                        width: bounds.width + 8.0,
                                        height: bounds.height + 8.0,
                                    },
                                    4.0,
                                ),
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Progress {
                    track,
                    fill,
                    label,
                    cancel,
                    corner_radius,
                }) => {
                    if let Some(label) = label {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*track),
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                                corner_radius: corner_radii(*corner_radius),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        4,
                        scene_rect(*fill),
                        VisualQuadStyle {
                            background: node.standard_visual_foreground,
                            border_color: None,
                            border_width: 0.0,
                                corner_radius: corner_radii(*corner_radius),
                        },
                    ));
                    if let Some(cancel) = cancel {
                        self.insert_primitive(component_text_primitive(
                            id,
                            5,
                            &ComponentTextRegion {
                                bounds: *cancel,
                                content: Arc::from("×"),
                                color: node.style.color,
                                font_size: 15.0,
                                font_weight: None,
                            },
                            TextHorizontalAlignment::Center,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::FormField {
                    label,
                    support,
                    indicator,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(support) = support {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            support,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some((bounds, color)) = indicator {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            4,
                            scene_rect(*bounds),
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(*color),
                                border_width: 1.0,
                                corner_radius: corner_radii(999.0),
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Toast {
                    indicator,
                    title,
                    description,
                    dismiss,
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        1,
                        scene_rect(*indicator),
                        VisualQuadStyle {
                            background: node.standard_visual_foreground,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(999.0),
                        },
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(description) = description {
                        self.insert_primitive(component_text_primitive(
                            id,
                            3,
                            description,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(dismiss) = dismiss {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            &ComponentTextRegion {
                                bounds: *dismiss,
                                content: Arc::from("×"),
                                color: node.style.color,
                                font_size: 15.0,
                                font_weight: None,
                            },
                            TextHorizontalAlignment::Center,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::XYPad {
                    pad: _,
                    thumb,
                    h_axis,
                    v_axis,
                    thumb_color,
                    axis_color,
                    ..
                }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        1,
                        scene_rect(*h_axis),
                        VisualQuadStyle {
                            background: Some(*axis_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        2,
                        scene_rect(*v_axis),
                        VisualQuadStyle {
                            background: Some(*axis_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        3,
                        scene_rect(*thumb),
                        VisualQuadStyle {
                            background: Some(*thumb_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(999.0),
                        },
                    ));
                }
                Some(ComponentGeometry::QrCode { field, dark, .. }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        0,
                        scene_rect(*field),
                        VisualQuadStyle {
                            background: Some([1.0, 1.0, 1.0, 1.0]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_md),
                        },
                    ));
                    if !dark.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            1,
                            dark.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some([0.0, 0.0, 0.0, 1.0]),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::Select {
                    label,
                    handle,
                    handle_color,
                    menu,
                    ..
                }) => {
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    paint_select_handle(
                        self,
                        id,
                        handle,
                        *handle_color,
                        transform,
                        &clips,
                        opacity,
                        node.z_index,
                        node_order,
                    );
                    if let Some(menu) = menu {
                        let menu_z = node.z_index.max(1_000);
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 4 },
                            node: id,
                            bounds: scene_rect(menu.surface),
                            transform,
                            clips: Arc::clone(&parent_clips),
                            opacity,
                            z_index: menu_z,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Quad {
                                background: Some(menu.background),
                                border_color: Some(menu.border),
                                border_width: 1.0,
                                corner_radius: corner_radii(UI_METRICS.radius_md),
                                shadow: Some(menu.elevation),
                                surface: QuadSurfacePaint::default(),
                            },
                        });
                        for (index, option) in menu.options.iter().enumerate() {
                            let index = u8::try_from(index).unwrap_or(u8::MAX);
                            if option.checked {
                                let mut mark = component_text_primitive(
                                    id,
                                    70u8.saturating_add(index),
                                    &nana_ui_runtime::ComponentTextRegion {
                                        bounds: nana_ui_runtime::LayoutBox {
                                            x: option.bounds.x,
                                            y: option.bounds.y,
                                            width: 16.0,
                                            height: option.bounds.height,
                                        },
                                        content: Arc::from("✓"),
                                        color: node
                                            .standard_visual_foreground
                                            .or(option.label.color),
                                        font_size: option.label.font_size,
                                        font_weight: Some(700),
                                    },
                                    TextHorizontalAlignment::Center,
                                    false,
                                    &node,
                                    transform,
                                    Arc::clone(&parent_clips),
                                    opacity,
                                    node_order,
                                );
                                mark.z_index = menu_z;
                                self.insert_primitive(mark);
                            }
                            if let Some(background) = option.background {
                                self.insert_primitive(visual_quad(
                                    &VisualPrimitiveContext {
                                        node: id,
                                        transform,
                                        clips: &parent_clips,
                                        opacity,
                                        z_index: menu_z,
                                        document_order: node_order,
                                    },
                                    10u8.saturating_add(index),
                                    scene_rect(option.bounds),
                                    VisualQuadStyle {
                                        background: Some(background),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(UI_METRICS.radius_sm),
                                    },
                                ));
                            }
                            let mut label = component_text_primitive(
                                id,
                                40u8.saturating_add(index),
                                &option.label,
                                TextHorizontalAlignment::Start,
                                true,
                                &node,
                                transform,
                                Arc::clone(&parent_clips),
                                opacity,
                                node_order,
                            );
                            label.z_index = menu_z;
                            self.insert_primitive(label);
                        }
                    }
                }
                Some(ComponentGeometry::ActionMenuItem {
                    icon, label, hint, ..
                }) => {
                    if let Some((icon, icon_bounds, color)) = icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: scene_rect(*icon_bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: Some(*color),
                            },
                        });
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        label,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(hint) = hint {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            hint,
                            TextHorizontalAlignment::End,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::TreeView { rows }) => {
                    for (index, row) in rows.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        if let Some(background) = row.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                10u8.saturating_add(index),
                                scene_rect(row.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                                },
                            ));
                        }
                        if let Some(disclosure) = row.disclosure {
                            self.insert_primitive(component_text_primitive(
                                id,
                                40u8.saturating_add(index),
                                &nana_ui_runtime::ComponentTextRegion {
                                    bounds: disclosure,
                                    content: Arc::from(if row.expanded { "▾" } else { "▸" }),
                                    color: row.label.color,
                                    font_size: row.label.font_size,
                                    font_weight: None,
                                },
                                TextHorizontalAlignment::Center,
                                false,
                                &node,
                                transform,
                                clips.clone(),
                                opacity,
                                node_order,
                            ));
                        }
                        if let Some((icon, icon_bounds, color)) = row.icon {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId {
                                    node: id,
                                    slot: 80u8.saturating_add(index),
                                },
                                node: id,
                                bounds: scene_rect(icon_bounds),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Icon {
                                    icon,
                                    color: Some(color),
                                },
                            });
                        }
                        self.insert_primitive(component_text_primitive(
                            id,
                            110u8.saturating_add(index),
                            &row.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::CommandPalette {
                    scrim,
                    surface,
                    title,
                    input,
                    empty,
                    rows,
                    background,
                    input_background,
                    input_border,
                    elevation,
                }) => {
                    let overlay_z = node.z_index.max(1_000);
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: overlay_z,
                            document_order: node_order,
                        },
                        10,
                        scene_rect(*scrim),
                        VisualQuadStyle {
                            background: Some([0.0, 0.0, 0.0, 0.45]),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 11 },
                        node: id,
                        bounds: scene_rect(*surface),
                        transform,
                        clips: Arc::clone(&parent_clips),
                        opacity,
                        z_index: overlay_z,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Quad {
                            background: Some(*background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_md),
                            shadow: Some(*elevation),
                            surface: QuadSurfacePaint::default(),
                        },
                    });
                    let mut title_text = component_text_primitive(
                        id,
                        20,
                        title,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        Arc::clone(&parent_clips),
                        opacity,
                        node_order,
                    );
                    title_text.z_index = overlay_z;
                    self.insert_primitive(title_text);
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: overlay_z,
                            document_order: node_order,
                        },
                        12,
                        scene_rect(input.bounds),
                        VisualQuadStyle {
                            background: Some(*input_background),
                            border_color: Some(*input_border),
                            border_width: 1.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                    let mut input_text = component_text_primitive(
                        id,
                        21,
                        input,
                        TextHorizontalAlignment::Start,
                        true,
                        &node,
                        transform,
                        Arc::clone(&parent_clips),
                        opacity,
                        node_order,
                    );
                    input_text.z_index = overlay_z;
                    self.insert_primitive(input_text);
                    if let Some(empty) = empty {
                        let mut empty_text = component_text_primitive(
                            id,
                            22,
                            empty,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            Arc::clone(&parent_clips),
                            opacity,
                            node_order,
                        );
                        empty_text.z_index = overlay_z;
                        self.insert_primitive(empty_text);
                    }
                    for (index, row) in rows.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        if let Some(background) = row.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: overlay_z,
                                    document_order: node_order,
                                },
                                23u8.saturating_add(index),
                                scene_rect(row.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                                },
                            ));
                        }
                        let mut label = component_text_primitive(
                            id,
                            40u8.saturating_add(index),
                            &row.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            Arc::clone(&parent_clips),
                            opacity,
                            node_order,
                        );
                        label.z_index = overlay_z;
                        self.insert_primitive(label);
                        if let Some(category) = &row.category {
                            let mut category = component_text_primitive(
                                id,
                                70u8.saturating_add(index),
                                category,
                                TextHorizontalAlignment::Start,
                                true,
                                &node,
                                transform,
                                Arc::clone(&parent_clips),
                                opacity,
                                node_order,
                            );
                            category.z_index = overlay_z;
                            self.insert_primitive(category);
                        }
                        if let Some(shortcut) = &row.shortcut {
                            let mut shortcut = component_text_primitive(
                                id,
                                100u8.saturating_add(index),
                                shortcut,
                                TextHorizontalAlignment::End,
                                true,
                                &node,
                                transform,
                                Arc::clone(&parent_clips),
                                opacity,
                                node_order,
                            );
                            shortcut.z_index = overlay_z;
                            self.insert_primitive(shortcut);
                        }
                    }
                }
                Some(ComponentGeometry::MenuSurface {
                    trigger,
                    trigger_icon,
                    trigger_surface,
                    surface,
                    search,
                    search_field,
                    options,
                    elevation,
                    background,
                    border,
                }) => {
                    if let Some(chrome) = trigger_surface
                        && (chrome.background.is_some() || chrome.border.is_some())
                    {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 1 },
                            node: id,
                            bounds: scene_rect(chrome.bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Quad {
                                background: chrome.background,
                                border_color: chrome.border,
                                border_width: 1.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                                shadow: None,
                                surface: QuadSurfacePaint::default(),
                            },
                        });
                    }
                    if let Some((icon, icon_bounds)) = trigger_icon {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 2 },
                            node: id,
                            bounds: scene_rect(*icon_bounds),
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Icon {
                                icon: *icon,
                                color: node
                                    .standard_visual_foreground
                                    .or(node.style.color),
                            },
                        });
                    } else if let Some(trigger) = trigger {
                        self.insert_primitive(component_text_primitive(
                            id,
                            2,
                            trigger,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if surface.height > 1.0 && surface.width > 1.0 {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 0 },
                            node: id,
                            bounds: scene_rect(*surface),
                            transform,
                            clips: Arc::clone(&parent_clips),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Quad {
                                background: Some(*background),
                                border_color: Some(*border),
                                border_width: 1.0,
                                corner_radius: corner_radii(UI_METRICS.radius_md),
                                shadow: Some(*elevation),
                                surface: QuadSurfacePaint::default(),
                            },
                        });
                    }
                    if let Some(field) = search_field {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            3,
                            scene_rect(*field),
                            VisualQuadStyle {
                                background: Some(*background),
                                border_color: Some(*border),
                                border_width: 1.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                            },
                        ));
                    }
                    if let Some(search) = search {
                        self.insert_primitive(component_text_primitive(
                            id,
                            4,
                            search,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    for (index, option) in options.iter().enumerate() {
                        if let Some(background) = option.background {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                10u8.saturating_add(index as u8),
                                scene_rect(option.bounds),
                                VisualQuadStyle {
                                    background: Some(background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(UI_METRICS.radius_sm),
                                },
                            ));
                        }
                        if let Some((icon, icon_bounds, color)) = option.icon {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId {
                                    node: id,
                                    slot: 80u8.saturating_add(index as u8),
                                },
                                node: id,
                                bounds: scene_rect(icon_bounds),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Icon {
                                    icon,
                                    color: Some(color),
                                },
                            });
                        }
                        self.insert_primitive(component_text_primitive(
                            id,
                            40u8.saturating_add(index as u8),
                            &option.label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::CalendarHeatmap {
                    cells,
                    labels,
                    hover,
                }) => {
                    let mut groups: Vec<([f32; 4], Vec<SceneRect>)> = Vec::new();
                    for (cell, color) in cells {
                        match groups.iter_mut().find(|(existing, _)| existing == color) {
                            Some((_, rects)) => rects.push(scene_rect(*cell)),
                            None => groups.push((*color, vec![scene_rect(*cell)])),
                        }
                    }
                    for (index, (color, rects)) in groups.into_iter().enumerate() {
                        if rects.is_empty() {
                            continue;
                        }
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            10u8.saturating_add(index as u8),
                            rects,
                            VisualQuadStyle {
                                background: Some(color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(UI_METRICS.radius_xs),
                            },
                        ));
                    }
                    for (index, label) in labels.iter().enumerate() {
                        self.insert_primitive(component_text_primitive(
                            id,
                            40u8.saturating_add(index as u8),
                            label,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(hover) = hover {
                        let hover_context = VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        };
                        self.insert_primitive(visual_quad(
                            &hover_context,
                            70,
                            scene_rect(hover.ring),
                            VisualQuadStyle {
                                background: None,
                                border_color: Some(hover.ring_color),
                                border_width: 1.5,
                                corner_radius: corner_radii(UI_METRICS.radius_xs + 1.0),
                            },
                        ));
                        self.insert_primitive(visual_quad(
                            &hover_context,
                            71,
                            scene_rect(hover.tooltip),
                            VisualQuadStyle {
                                background: Some(hover.tooltip_fill),
                                border_color: Some(hover.tooltip_border),
                                border_width: 1.0,
                                corner_radius: corner_radii(nana_ui_core::TooltipConfig::RADIUS),
                            },
                        ));
                        self.insert_primitive(component_text_primitive(
                            id,
                            72,
                            &hover.title,
                            TextHorizontalAlignment::Start,
                            false,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::TimeSeriesChart {
                    grid,
                    area,
                    line,
                    grid_color,
                    area_color,
                    line_color,
                }) => {
                    let context = VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: &clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    };
                    if !grid.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &context,
                            10,
                            grid.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*grid_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    if !area.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &context,
                            11,
                            area.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*area_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    if line.len() >= 2 {
                        self.insert_primitive(visual_stroke(
                            &context,
                            12,
                            bounds,
                            line.clone(),
                            TimeSeriesChart::LINE_WIDTH,
                            *line_color,
                        ));
                    }
                }
                Some(ComponentGeometry::ReorderList { rows, insert }) => {
                    let selected = rows
                        .iter()
                        .filter_map(|(row, _, fill)| fill.map(|color| (scene_rect(*row), color)))
                        .collect::<Vec<_>>();
                    if !selected.is_empty() {
                        let color = selected[0].1;
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            10,
                            selected.iter().map(|(rect, _)| *rect),
                            VisualQuadStyle {
                                background: Some(color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                            },
                        ));
                    }
                    if let Some((line, color)) = insert {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            11,
                            scene_rect(*line),
                            VisualQuadStyle {
                                background: Some(*color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    for (index, (_, label, _)) in rows.iter().enumerate() {
                        self.insert_primitive(component_text_primitive(
                            id,
                            40u8.saturating_add(index as u8),
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::NativeMarkdown {
                    text,
                    selection,
                    selection_color,
                })
                | Some(ComponentGeometry::SelectableRichText {
                    text,
                    selection,
                    selection_color,
                }) => {
                    if !selection.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            1,
                            selection.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*selection_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        text,
                        TextHorizontalAlignment::Start,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::GraphCanvas {
                    nodes: graph_nodes,
                    separators,
                    ports,
                    port_labels,
                    edges,
                    edge_labels,
                    grid,
                    background,
                    grid_color,
                    separator_color,
                }) => {
                    let context = VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: &clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    };
                    self.insert_primitive(visual_quad(
                        &context,
                        10,
                        bounds,
                        VisualQuadStyle {
                            background: Some(*background),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    if !grid.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &context,
                            11,
                            grid.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*grid_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    for (index, (points, color)) in edges.iter().enumerate() {
                        if points.len() < 2 {
                            continue;
                        }
                        self.insert_primitive(visual_stroke(
                            &context,
                            12u8.saturating_add(index as u8),
                            bounds,
                            points.clone(),
                            1.6,
                            *color,
                        ));
                    }
                    for (index, (node_bounds, label, fill, border)) in
                        graph_nodes.iter().enumerate()
                    {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        self.insert_primitive(visual_quad(
                            &context,
                            20u8.saturating_add(index),
                            scene_rect(*node_bounds),
                            VisualQuadStyle {
                                background: Some(*fill),
                                border_color: *border,
                                border_width: 1.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                            },
                        ));
                        self.insert_primitive(component_text_primitive(
                            id,
                            50u8.saturating_add(index),
                            label,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if !separators.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &context,
                            40,
                            separators.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*separator_color),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    for (index, (port, fill, border, border_width)) in ports.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        self.insert_primitive(visual_quad(
                            &context,
                            80u8.saturating_add(index),
                            scene_rect(*port),
                            VisualQuadStyle {
                                background: Some(*fill),
                                border_color: Some(*border),
                                border_width: *border_width,
                                corner_radius: corner_radii(999.0),
                            },
                        ));
                    }
                    for (index, (label, alignment)) in port_labels.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        self.insert_primitive(component_text_primitive(
                            id,
                            110u8.saturating_add(index),
                            label,
                            *alignment,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    for (index, label) in edge_labels.iter().enumerate() {
                        let index = u8::try_from(index).unwrap_or(u8::MAX);
                        self.insert_primitive(component_text_primitive(
                            id,
                            140u8.saturating_add(index),
                            label,
                            TextHorizontalAlignment::Center,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::GraphMinimap {
                    nodes,
                    node_fill,
                    indicator,
                    indicator_fill,
                    indicator_border,
                }) => {
                    let context = VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: &clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    };
                    if !nodes.is_empty() {
                        self.insert_primitive(visual_quad_batch(
                            &context,
                            10,
                            nodes.iter().copied().map(scene_rect),
                            VisualQuadStyle {
                                background: Some(*node_fill),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                    if let Some(indicator) = indicator {
                        self.insert_primitive(visual_quad(
                            &context,
                            11,
                            scene_rect(*indicator),
                            VisualQuadStyle {
                                background: Some(*indicator_fill),
                                border_color: Some(*indicator_border),
                                border_width: 1.5,
                                corner_radius: corner_radii(0.0),
                            },
                        ));
                    }
                }
                Some(ComponentGeometry::ImageViewer {
                    scrim,
                    surface,
                    stage,
                    close,
                    name,
                    metadata,
                    scrim_color,
                    surface_color,
                    stage_color,
                    ..
                }) => {
                    let context = VisualPrimitiveContext {
                        node: id,
                        transform,
                        clips: &clips,
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                    };
                    self.insert_primitive(visual_quad(
                        &context,
                        10,
                        scene_rect(*scrim),
                        VisualQuadStyle {
                            background: Some(*scrim_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &context,
                        11,
                        scene_rect(*surface),
                        VisualQuadStyle {
                            background: Some(*surface_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_md),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &context,
                        12,
                        scene_rect(*stage),
                        VisualQuadStyle {
                            background: Some(*stage_color),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(0.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &context,
                        13,
                        scene_rect(*close),
                        VisualQuadStyle {
                            background: None,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        16,
                        &ComponentTextRegion {
                            bounds: *close,
                            content: Arc::from("×"),
                            color: node.style.color,
                            font_size: 15.0,
                            font_weight: None,
                        },
                        TextHorizontalAlignment::Center,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                    if let Some(name) = name {
                        self.insert_primitive(component_text_primitive(
                            id,
                            14,
                            name,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                    if let Some(metadata) = metadata {
                        self.insert_primitive(component_text_primitive(
                            id,
                            15,
                            metadata,
                            TextHorizontalAlignment::Start,
                            true,
                            &node,
                            transform,
                            clips.clone(),
                            opacity,
                            node_order,
                        ));
                    }
                }
                Some(ComponentGeometry::KeyCaptureLayer { badge, background }) => {
                    if let Some(background) = background {
                        self.insert_primitive(visual_quad(
                            &VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            },
                            10,
                            scene_rect(badge.bounds),
                            VisualQuadStyle {
                                background: Some(*background),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(UI_METRICS.radius_sm),
                            },
                        ));
                    }
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        badge,
                        TextHorizontalAlignment::Center,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::KeymapLayer { badge }) => {
                    self.insert_primitive(visual_quad(
                        &VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        },
                        10,
                        scene_rect(badge.bounds),
                        VisualQuadStyle {
                            background: node.style.background.or(badge
                                .color
                                .map(|color| [color[0], color[1], color[2], 0.12])),
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(UI_METRICS.radius_sm),
                        },
                    ));
                    self.insert_primitive(component_text_primitive(
                        id,
                        2,
                        badge,
                        TextHorizontalAlignment::Center,
                        false,
                        &node,
                        transform,
                        clips.clone(),
                        opacity,
                        node_order,
                    ));
                }
                Some(ComponentGeometry::Card { title: None, .. })
                | Some(ComponentGeometry::ListItem { .. })
                // Scrollbar chrome is emitted with the StandardVisual slots
                // below so it draws over the node's own content.
                | Some(ComponentGeometry::Scrollbar { .. })
                | None => {}
            }
            let visual_context = VisualPrimitiveContext {
                node: id,
                transform,
                clips: if matches!(node.standard_visual, Some(StandardVisual::TextInput { .. })) {
                    &text_input_clips
                } else {
                    &clips
                },
                opacity,
                z_index: node.z_index,
                document_order: node_order,
            };
            match node.standard_visual {
                Some(StandardVisual::Button { loading_phase, .. }) => {
                    if let Some(ComponentGeometry::Button {
                        spinner,
                        focus_ring,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        if let Some(spinner) = spinner {
                            self.insert_primitive(ScenePrimitive {
                                id: PrimitiveId { node: id, slot: 3 },
                                node: id,
                                bounds: scene_rect(*spinner),
                                transform,
                                clips: clips.clone(),
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                                kind: ScenePrimitiveKind::Spinner {
                                    phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                    color: node.standard_visual_foreground.or(node.style.color),
                                },
                            });
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: focus_ring_corner_radius(
                                        style,
                                        SceneRect {
                                            x: bounds.x - 3.0,
                                            y: bounds.y - 3.0,
                                            width: bounds.width + 6.0,
                                            height: bounds.height + 6.0,
                                        },
                                        3.0,
                                    ),
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::TextInput { .. }) => {
                    if let Some(ComponentGeometry::TextInput {
                        caret,
                        additional_carets,
                        additional_caret_color,
                        preedit,
                        focus_ring,
                        caret_color,
                        preedit_color,
                        diagnostic_markers,
                        match_markers,
                        caret_line,
                        bracket_markers,
                        occurrence_markers,
                        whitespace_marks,
                        whitespace_color,
                        wrap_guides,
                        indent_guides,
                        line_labels,
                        line_labels_color,
                        line_labels_font_size,
                        folds,
                        completion_popup,
                        hover_popup,
                        minimap,
                        ..
                    }) = node.component_geometry.as_ref()
                    {
                        // 折叠 gutter 标记：折叠态（实心，slot 14）与展开态
                        // （描边，slot 15）各一个 quad 批次，与行号同级的外层
                        // 裁剪；合批后数量不受 slot 上限约束，点击切换由
                        // Runtime 指针路径处理。
                        if !folds.gutters.is_empty() {
                            let gutter_context = VisualPrimitiveContext {
                                node: id,
                                transform,
                                clips: &clips,
                                opacity,
                                z_index: node.z_index,
                                document_order: node_order,
                            };
                            let collapsed: Vec<&TextFoldGutter> = folds
                                .gutters
                                .iter()
                                .filter(|gutter| gutter.collapsed)
                                .collect();
                            let expanded: Vec<&TextFoldGutter> = folds
                                .gutters
                                .iter()
                                .filter(|gutter| !gutter.collapsed)
                                .collect();
                            if !collapsed.is_empty() {
                                self.insert_primitive(visual_quad_batch(
                                    &gutter_context,
                                    14,
                                    collapsed.iter().map(|gutter| scene_rect(gutter.bounds)),
                                    VisualQuadStyle {
                                        background: Some(collapsed[0].color),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                            if !expanded.is_empty() {
                                self.insert_primitive(visual_quad_batch(
                                    &gutter_context,
                                    15,
                                    expanded.iter().map(|gutter| scene_rect(gutter.bounds)),
                                    VisualQuadStyle {
                                        background: None,
                                        border_color: Some(expanded[0].color),
                                        border_width: 1.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                        }
                        // 行号标签绘制在左内边距区域，使用外层裁剪。
                        if !line_labels.is_empty() {
                            let padding = node
                                .source_style
                                .layout
                                .resolved_padding_against(Some(bounds.width));
                            let border = node.source_style.layout.resolved_border_width();
                            let gutter_width = padding.left;
                            if gutter_width > 4.0 {
                                for (label_index, label) in line_labels.iter().enumerate() {
                                    let region = ComponentTextRegion {
                                        bounds: LayoutBox {
                                            x: bounds.x + border + 2.0,
                                            y: label.y,
                                            width: (gutter_width - 4.0).max(0.0),
                                            height: label.height,
                                        },
                                        content: Arc::from(label.number.to_string().as_str()),
                                        color: Some(*line_labels_color),
                                        font_size: *line_labels_font_size,
                                        font_weight: None,
                                    };
                                    self.insert_primitive(component_text_primitive(
                                        id,
                                        40 + label_index as u8,
                                        &region,
                                        TextHorizontalAlignment::End,
                                        false,
                                        &node,
                                        transform,
                                        std::sync::Arc::clone(&clips),
                                        opacity,
                                        node_order,
                                    ));
                                }
                            }
                        }
                        for (marker_index, (rect, color)) in diagnostic_markers.iter().enumerate() {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                20 + marker_index as u8,
                                scene_rect(*rect),
                                VisualQuadStyle {
                                    background: Some(*color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 查找匹配高亮：普通匹配（slot 3，文本之上、光标之
                        // 下）与当前匹配（slot 6，更强）各一个 quad 批次，
                        // 同类共用世界解析出的统一颜色。
                        let (normal_matches, current_matches): (Vec<_>, Vec<_>) =
                            match_markers.iter().partition(|marker| !marker.current);
                        if !normal_matches.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                3,
                                normal_matches.iter().map(|marker| scene_rect(marker.rect)),
                                VisualQuadStyle {
                                    background: Some(normal_matches[0].color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        if !current_matches.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                6,
                                current_matches.iter().map(|marker| scene_rect(marker.rect)),
                                VisualQuadStyle {
                                    background: Some(current_matches[0].color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 当前行条：slot 1 与选区同一层级（互斥：选区收起时
                        // 才有当前行条），绘制在文本之下。
                        if let Some((rect, color)) = caret_line {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                1,
                                scene_rect(*rect),
                                VisualQuadStyle {
                                    background: Some(*color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 缩进参考线：1px 竖线批次，低对比结构标记。
                        if !indent_guides.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                10,
                                indent_guides.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: Some(indent_guides[0].1),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 出现高亮：淡底色填充批次（slot 11，缩进参考线之
                        // 上、括号描边之下），弱于查找匹配的两级强调。
                        if !occurrence_markers.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                11,
                                occurrence_markers.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: Some(occurrence_markers[0].1),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 空白字符显示：空格画小圆点（slot 16 单一批次），
                        // Tab 画箭头图标（slot 60 单一批次，镜像折叠箭头
                        // 14/15 的合批先例——数量不受 slot 上限约束）。
                        if !whitespace_marks.is_empty() {
                            let dots: Vec<&LayoutBox> = whitespace_marks
                                .iter()
                                .filter_map(|(rect, kind)| {
                                    (*kind == TextWhitespaceKind::Space).then_some(rect)
                                })
                                .collect();
                            if !dots.is_empty() {
                                // 圆点直径随行号字号缩放，钳在小尺寸带，
                                // 保持"标点"观感而不遮挡字形。
                                let extent = (*line_labels_font_size * 0.2).clamp(2.0, 3.0);
                                self.insert_primitive(visual_quad_batch(
                                    &visual_context,
                                    16,
                                    dots.iter().map(|rect| {
                                        let mut bounds = scene_rect(**rect);
                                        bounds.width = extent;
                                        bounds.height = extent;
                                        bounds.x += (scene_rect(**rect).width - extent) / 2.0;
                                        bounds.y += (scene_rect(**rect).height - extent) / 2.0;
                                        bounds
                                    }),
                                    VisualQuadStyle {
                                        background: Some(*whitespace_color),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(999.0),
                                    },
                                ));
                            }
                            let arrows: Vec<SceneRect> = whitespace_marks
                                .iter()
                                .filter(|(_, kind)| *kind == TextWhitespaceKind::Tab)
                                .map(|(rect, _)| {
                                    let cell = scene_rect(*rect);
                                    // 箭头尺寸按字符单元高度缩放，居中放置。
                                    let extent = (cell.height * 0.55).clamp(6.0, 14.0);
                                    SceneRect {
                                        x: cell.x + (cell.width - extent) / 2.0,
                                        y: cell.y + (cell.height - extent) / 2.0,
                                        width: extent,
                                        height: extent,
                                    }
                                })
                                .collect();
                            if !arrows.is_empty() {
                                let bounds = arrows
                                    .iter()
                                    .copied()
                                    .reduce(|left, right| {
                                        let x = left.x.min(right.x);
                                        let y = left.y.min(right.y);
                                        let right_edge =
                                            (left.x + left.width).max(right.x + right.width);
                                        let bottom_edge =
                                            (left.y + left.height).max(right.y + right.height);
                                        SceneRect {
                                            x,
                                            y,
                                            width: right_edge - x,
                                            height: bottom_edge - y,
                                        }
                                    })
                                    .unwrap_or_default();
                                self.insert_primitive(ScenePrimitive {
                                    id: PrimitiveId { node: id, slot: 60 },
                                    node: id,
                                    bounds,
                                    transform,
                                    clips: clips.clone(),
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                    kind: ScenePrimitiveKind::IconBatch {
                                        bounds: arrows,
                                        icon: Icon::ArrowRight,
                                        color: Some(*whitespace_color),
                                    },
                                });
                            }
                        }
                        // wrap guide：按列的全高 1px 竖线批次（slot 17）。
                        // 与缩进参考线（slot 10，行内缩进深度）同为低对比
                        // 竖线，但贯穿整个内容区高度。
                        if !wrap_guides.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                17,
                                wrap_guides.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: Some(wrap_guides[0].1),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // minimap：面板（slot 70）、行条 + 1px 分隔线（slot
                        // 71，同一 faint 色）、视口指示器（slot 72，半透明
                        // accent）各一个批次。占用 70-72：位于行号/Tab 批次
                        // （40+/60）之上、补全弹层（90+）与 hover 浮窗（120+，
                        // 正文行可用到 131）之下，minimap 不得盖住浮层。
                        if let Some(minimap) = minimap {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                70,
                                scene_rect(minimap.panel),
                                VisualQuadStyle {
                                    background: Some(minimap.panel_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                71,
                                std::iter::once(scene_rect(minimap.separator))
                                    .chain(minimap.bars.iter().map(|bar| scene_rect(*bar))),
                                VisualQuadStyle {
                                    background: Some(minimap.bar_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                            if let Some(indicator) = minimap.indicator {
                                self.insert_primitive(visual_quad(
                                    &visual_context,
                                    72,
                                    scene_rect(indicator),
                                    VisualQuadStyle {
                                        background: Some(minimap.indicator_color),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                        }
                        // 括号匹配：两端各一个 1px accent 描边框，绘制在文本
                        // 之上（描边不遮挡字形）。
                        if !bracket_markers.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                12,
                                bracket_markers.iter().map(|(rect, _)| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(bracket_markers[0].1),
                                    border_width: 1.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        if let Some(caret) = caret {
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                4,
                                scene_rect(*caret),
                                VisualQuadStyle {
                                    background: Some(*caret_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        // 附加多光标：与主光标同形、半透明色的 quad 批次
                        // （slot 13，与主光标同层）。
                        if !additional_carets.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                13,
                                additional_carets.iter().map(|rect| scene_rect(*rect)),
                                VisualQuadStyle {
                                    background: Some(*additional_caret_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        if !preedit.is_empty() {
                            self.insert_primitive(visual_quad_batch(
                                &visual_context,
                                5,
                                preedit.iter().map(|preedit| scene_rect(*preedit)),
                                VisualQuadStyle {
                                    background: Some(*preedit_color),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(0.0),
                                },
                            ));
                        }
                        if let Some(color) = focus_ring {
                            self.insert_primitive(visual_quad(
                                &VisualPrimitiveContext {
                                    node: id,
                                    transform,
                                    clips: &parent_clips,
                                    opacity,
                                    z_index: node.z_index,
                                    document_order: node_order,
                                },
                                7,
                                SceneRect {
                                    x: bounds.x - 3.0,
                                    y: bounds.y - 3.0,
                                    width: bounds.width + 6.0,
                                    height: bounds.height + 6.0,
                                },
                                VisualQuadStyle {
                                    background: None,
                                    border_color: Some(*color),
                                    border_width: 2.0,
                                    corner_radius: focus_ring_corner_radius(
                                        style,
                                        SceneRect {
                                            x: bounds.x - 3.0,
                                            y: bounds.y - 3.0,
                                            width: bounds.width + 6.0,
                                            height: bounds.height + 6.0,
                                        },
                                        3.0,
                                    ),
                                },
                            ));
                        }
                        // 补全弹层与 hover 浮窗：编辑器覆盖层的最上层
                        // （面板 slot 90 / 120，其余文本在各自段内递增），
                        // 高于行号（40+）与折叠 gutter（14/15）。面板绘制
                        // 共用 `overlay_panel_primitive`；行文本不换行，
                        // 超宽省略号截断。使用与焦点环同级的外层裁剪。
                        let overlay_context = VisualPrimitiveContext {
                            node: id,
                            transform,
                            clips: &parent_clips,
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                        };
                        let overlay_text =
                            |slot: u8,
                             region: &ComponentTextRegion,
                             alignment: TextHorizontalAlignment| {
                                overlay_text_primitive(
                                    id,
                                    slot,
                                    region,
                                    alignment,
                                    &node,
                                    transform,
                                    std::sync::Arc::clone(&parent_clips),
                                    opacity,
                                    node_order,
                                )
                            };
                        if let Some(popup) = completion_popup {
                            self.insert_primitive(overlay_panel_primitive(
                                &overlay_context,
                                90,
                                scene_rect(popup.panel),
                                popup.background,
                                popup.border,
                            ));
                            let selected_in_view = popup
                                .rows
                                .iter()
                                .enumerate()
                                .find(|(index, _)| popup.first_row + index == popup.selected)
                                .map(|(index, _)| index);
                            if let Some(index) = selected_in_view {
                                self.insert_primitive(visual_quad(
                                    &overlay_context,
                                    91,
                                    scene_rect(popup.rows[index].bounds),
                                    VisualQuadStyle {
                                        background: Some(popup.selected_background),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                            for (index, row) in popup.rows.iter().enumerate() {
                                self.insert_primitive(overlay_text(
                                    92 + index as u8,
                                    &row.label,
                                    TextHorizontalAlignment::Start,
                                ));
                                if let Some(detail) = row.detail.as_ref() {
                                    self.insert_primitive(overlay_text(
                                        100 + index as u8,
                                        detail,
                                        TextHorizontalAlignment::Start,
                                    ));
                                }
                                if let Some(kind) = row.kind.as_ref() {
                                    self.insert_primitive(overlay_text(
                                        108 + index as u8,
                                        kind,
                                        TextHorizontalAlignment::End,
                                    ));
                                }
                            }
                        }
                        if let Some(popup) = hover_popup {
                            self.insert_primitive(overlay_panel_primitive(
                                &overlay_context,
                                120,
                                scene_rect(popup.panel),
                                popup.background,
                                popup.border,
                            ));
                            self.insert_primitive(overlay_text(
                                121,
                                &popup.title,
                                TextHorizontalAlignment::Start,
                            ));
                            for (index, row) in popup.body_rows.iter().enumerate() {
                                self.insert_primitive(overlay_text(
                                    122 + index as u8,
                                    row,
                                    TextHorizontalAlignment::Start,
                                ));
                            }
                        }
                    }
                }
                Some(StandardVisual::Checkbox {
                    checked,
                    indeterminate,
                    size,
                }) => {
                    let extent = size.indicator_size().min(bounds.height);
                    let indicator = SceneRect {
                        x: bounds.x,
                        y: bounds.y + (bounds.height - extent) / 2.0,
                        width: extent,
                        height: extent,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        indicator,
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: corner_radii(4.0),
                        },
                    ));
                    if indeterminate {
                        let dash_height = (extent / 8.0).max(1.5);
                        let dash_inset = extent / 4.0;
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            4,
                            SceneRect {
                                x: indicator.x + dash_inset,
                                y: indicator.y + (extent - dash_height) / 2.0,
                                width: (extent - dash_inset * 2.0).max(0.0),
                                height: dash_height,
                            },
                            VisualQuadStyle {
                                background: node.standard_visual_foreground,
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: corner_radii(dash_height / 2.0),
                            },
                        ));
                    } else if checked {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 4 },
                            node: id,
                            bounds: indicator,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Text {
                                content: "✓".into(),
                                color: node.standard_visual_foreground,
                                size: extent * 0.75,
                                weight: Some(700),
                                family: None,
                                line_height: None,
                                letter_spacing: 0.0,
                                wrap: false,
                                ellipsis: false,
                                max_lines: None,
                                shaping: TextShaping::Auto,
                                horizontal_alignment: TextHorizontalAlignment::Center,
                                vertical_alignment: TextVerticalAlignment::Center,
                                spans: Vec::new(),
                                text_shadow: None,
                                underline: false,
                                line_through: false,
                                font_features: Vec::new(),
                                italic: false,
                                wrap_break: nana_ui_core::TextWrapBreak::Word,
                                opentype: SceneTextOpenType::default(),
                            },
                        });
                    }
                }
                Some(StandardVisual::Icon { icon, size, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    let x = bounds.x + (bounds.width - extent) / 2.0;
                    let y = self.icon_y_aligned_to_adjacent_text(&node, bounds, extent);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x,
                            y,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Icon {
                            icon,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                }
                Some(StandardVisual::Switch {
                    checked,
                    loading,
                    loading_phase,
                    ..
                }) => {
                    let (track, track_background, track_border, thumb_background) =
                        match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Switch {
                                control,
                                track_background,
                                track_border,
                                thumb_background,
                                ..
                            }) => (
                                scene_rect(*control),
                                Some(*track_background),
                                Some(*track_border),
                                Some(*thumb_background),
                            ),
                            _ => (
                                SceneRect {
                                    x: bounds.x,
                                    y: bounds.y + (bounds.height - 16.0) / 2.0,
                                    width: 30.0,
                                    height: 16.0,
                                },
                                node.style.background,
                                node.style.border_color,
                                node.standard_visual_foreground,
                            ),
                        };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        track,
                        VisualQuadStyle {
                            background: track_background,
                            border_color: track_border,
                            border_width: 1.0,
                            corner_radius: corner_radii(8.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: track.x + if checked { 17.0 } else { 3.0 },
                            y: track.y + 3.0,
                            width: 10.0,
                            height: 10.0,
                        },
                        VisualQuadStyle {
                            background: thumb_background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(5.0),
                        },
                    ));
                    if loading {
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 6 },
                            node: id,
                            bounds: SceneRect {
                                x: track.x + 1.0,
                                y: track.y + 1.0,
                                width: 14.0,
                                height: 14.0,
                            },
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                    if node.focused {
                        self.insert_primitive(visual_quad(
                            &visual_context,
                            7,
                            SceneRect {
                                x: track.x - 4.0,
                                y: track.y - 4.0,
                                width: track.width + 8.0,
                                height: track.height + 8.0,
                            },
                            VisualQuadStyle {
                                background: None,
                                border_color: node.style.border_color,
                                border_width: 2.0,
                                corner_radius: corner_radii(12.0),
                            },
                        ));
                    }
                }
                Some(StandardVisual::Range { ratio, size, .. }) => {
                    let ratio = ratio.clamp(0.0, 1.0);
                    let track_band = match node.component_geometry.as_ref() {
                        Some(ComponentGeometry::Range { track, .. }) => scene_rect(*track),
                        _ => SceneRect {
                            x: bounds.x + 7.0,
                            y: bounds.y + (bounds.height - 14.0) / 2.0,
                            width: (bounds.width - 14.0).max(0.0),
                            height: 14.0,
                        },
                    };
                    let thumb_extent = match size {
                        ControlSize::Small => 12.0_f32,
                        ControlSize::Medium => 14.0_f32,
                        ControlSize::Large => 16.0_f32,
                    }
                    .min(bounds.width)
                    .min(track_band.height.max(0.0));
                    let rail = SceneRect {
                        x: track_band.x,
                        y: track_band.y + (track_band.height - 4.0) / 2.0,
                        width: track_band.width,
                        height: 4.0,
                    };
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        3,
                        rail,
                        VisualQuadStyle {
                            background: node.style.border_color,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(2.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        4,
                        SceneRect {
                            width: rail.width * ratio,
                            ..rail
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: None,
                            border_width: 0.0,
                            corner_radius: corner_radii(2.0),
                        },
                    ));
                    self.insert_primitive(visual_quad(
                        &visual_context,
                        5,
                        SceneRect {
                            x: track_band.x + track_band.width * ratio - thumb_extent / 2.0,
                            y: track_band.y + (track_band.height - thumb_extent) / 2.0,
                            width: thumb_extent,
                            height: thumb_extent,
                        },
                        VisualQuadStyle {
                            background: node.style.background,
                            border_color: node.style.border_color,
                            border_width: 1.0,
                            corner_radius: corner_radii(thumb_extent / 2.0),
                        },
                    ));
                }
                Some(StandardVisual::Scrollbar { .. }) => {
                    if let Some(ComponentGeometry::Scrollbar {
                        horizontal,
                        vertical,
                    }) = node.component_geometry.as_ref()
                    {
                        for (slot, bar) in [(3, vertical), (5, horizontal)] {
                            let Some(bar) = bar else {
                                continue;
                            };
                            if let Some(background) = bar.track_background {
                                self.insert_primitive(visual_quad(
                                    &visual_context,
                                    slot,
                                    scene_rect(bar.track),
                                    VisualQuadStyle {
                                        background: Some(background),
                                        border_color: None,
                                        border_width: 0.0,
                                        corner_radius: corner_radii(0.0),
                                    },
                                ));
                            }
                            self.insert_primitive(visual_quad(
                                &visual_context,
                                slot + 1,
                                scene_rect(bar.thumb),
                                VisualQuadStyle {
                                    background: Some(bar.thumb_background),
                                    border_color: None,
                                    border_width: 0.0,
                                    corner_radius: corner_radii(bar.thumb_radius),
                                },
                            ));
                        }
                    }
                }
                Some(StandardVisual::Card {
                    loading,
                    loading_phase,
                    ..
                }) => {
                    if loading {
                        let spinner_bounds = match node.component_geometry.as_ref() {
                            Some(ComponentGeometry::Card {
                                spinner: Some(spinner),
                                ..
                            }) => scene_rect(*spinner),
                            _ => {
                                let extent = 20.0_f32.min(bounds.width).min(bounds.height);
                                SceneRect {
                                    x: bounds.x + (bounds.width - extent) / 2.0,
                                    y: bounds.y + (bounds.height - extent) / 2.0,
                                    width: extent,
                                    height: extent,
                                }
                            }
                        };
                        self.insert_primitive(ScenePrimitive {
                            id: PrimitiveId { node: id, slot: 3 },
                            node: id,
                            bounds: spinner_bounds,
                            transform,
                            clips: clips.clone(),
                            opacity,
                            z_index: node.z_index,
                            document_order: node_order,
                            kind: ScenePrimitiveKind::Spinner {
                                phase: (loading_phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                                color: node.standard_visual_foreground.or(node.style.color),
                            },
                        });
                    }
                }
                Some(StandardVisual::Spinner { size, phase, .. }) => {
                    let extent = size.max(0.0).min(bounds.width).min(bounds.height);
                    self.insert_primitive(ScenePrimitive {
                        id: PrimitiveId { node: id, slot: 3 },
                        node: id,
                        bounds: SceneRect {
                            x: bounds.x,
                            y: bounds.y + (bounds.height - extent) / 2.0,
                            width: extent,
                            height: extent,
                        },
                        transform,
                        clips: clips.clone(),
                        opacity,
                        z_index: node.z_index,
                        document_order: node_order,
                        kind: ScenePrimitiveKind::Spinner {
                            phase: (phase.clamp(0.0, 1.0) * 8.0).floor() as u8 % 8,
                            color: node.standard_visual_foreground.or(node.style.color),
                        },
                    });
                }
                Some(
                    StandardVisual::ListItem { .. }
                    | StandardVisual::StatusBadge { .. }
                    | StandardVisual::ValidationMessage { .. }
                    | StandardVisual::EmptyState { .. }
                    | StandardVisual::LabeledValue { .. }
                    | StandardVisual::SelectionOption { .. }
                    | StandardVisual::ModalFrame { .. }
                    | StandardVisual::Progress { .. }
                    | StandardVisual::LevelMeter { .. }
                    | StandardVisual::FormField { .. }
                    | StandardVisual::Toast { .. }
                    | StandardVisual::XYPad { .. }
                    | StandardVisual::QrCode { .. }
                    | StandardVisual::Select { .. }
                    | StandardVisual::MenuSurface { .. }
                    | StandardVisual::ActionMenuItem { .. }
                    | StandardVisual::TreeView { .. }
                    | StandardVisual::CommandPalette { .. }
                    | StandardVisual::CalendarHeatmap { .. }
                    | StandardVisual::TimeSeriesChart { .. }
                    | StandardVisual::ReorderList { .. }
                    | StandardVisual::NativeMarkdown { .. }
                    | StandardVisual::SelectableRichText { .. }
                    | StandardVisual::GraphCanvas { .. }
                    | StandardVisual::GraphMinimap { .. }
                    | StandardVisual::ImageViewer { .. }
                    | StandardVisual::KeyCaptureLayer { .. }
                    | StandardVisual::KeymapLayer,
                ) => {
                    // The row surface and fallback label are emitted above;
                    // typed slots remain ordinary retained child nodes.
                }
                None => {}
            }
        }
        self.primitives.len() - before
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
    scene: &mut UiScene,
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
        scene.insert_primitive(visual_quad(
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
            VisualQuadStyle {
                background: Some(color),
                border_color: None,
                border_width: 0.0,
                corner_radius: corner_radii(0.0),
            },
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
        Some(
            StandardVisual::EmptyState { .. }
                | StandardVisual::ModalFrame { .. }
                | StandardVisual::NativeMarkdown { .. }
                | StandardVisual::SelectableRichText { .. }
        )
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
    scene: &mut UiScene,
    node: StableNodeId,
    bounds: SceneRect,
    transform: AffineTransform,
    clips: Arc<[ClipRegion]>,
    opacity: f32,
    z_index: i32,
    document_order: usize,
    color: [f32; 4],
    deco: nana_ui_core::TextDecorationLine,
) {
    let width = 1.0_f32.max(bounds.height * 0.06);
    let mut emit = |slot: u8, y: f32| {
        scene.insert_primitive(ScenePrimitive {
            id: PrimitiveId { node, slot },
            node,
            bounds: SceneRect {
                x: bounds.x,
                y: y - width * 0.5,
                width: bounds.width,
                height: width,
            },
            transform,
            clips: Arc::clone(&clips),
            opacity,
            z_index,
            document_order,
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

fn visual_quad_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u8,
    bounds: impl IntoIterator<Item = SceneRect>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let quad_bounds = bounds.into_iter().collect::<Vec<_>>();
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
        kind: ScenePrimitiveKind::QuadBatch {
            bounds: quad_bounds,
            background: style.background,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
            surface: QuadSurfacePaint::default(),
        },
    }
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
mod tests {
    use std::sync::Arc;

    use nana_ui_runtime::{
        ComputedStyle, CustomRenderNode, LayoutBox, NodeKind, NodeStyle, TextContent,
        TextMatchMarker,
    };

    use super::*;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn primitive_icon(kind: &ScenePrimitiveKind) -> Option<nana_ui_core::Icon> {
        match kind {
            ScenePrimitiveKind::Icon { icon, .. } => Some(*icon),
            _ => None,
        }
    }

    fn style_mut(node: &mut ExtractedNode) -> &mut ComputedStyle {
        Arc::make_mut(&mut node.style)
    }

    fn node(value: u64, parent: Option<u64>, children: &[u64]) -> ExtractedNode {
        ExtractedNode {
            id: id(value),
            kind: Arc::new(NodeKind::Element { tag: "div".into() }),
            parent: parent.map(id),
            children: Arc::new(children.iter().copied().map(id).collect()),
            layout: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            scroll_offset: nana_ui_runtime::ScrollOffset::default(),
            source_style: NodeStyle::default(),
            style: Arc::new(ComputedStyle::default()),
            text: None,
            text_metrics: None,
            z_index: 0,
            focused: false,
            ime: None,
            text_input: None,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
        }
    }

    #[test]
    fn hidden_nodes_skip_scene_primitives() {
        let mut hidden = node(1, None, &[]);
        style_mut(&mut hidden).visible = false;
        style_mut(&mut hidden).background = Some([1.0, 0.0, 0.0, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([hidden], []);
        assert!(scene.primitives().all(|primitive| primitive.node != id(1)));
    }

    #[test]
    fn workspace_resize_handle_is_not_clipped_by_its_region() {
        let mut region = node(1, None, &[2]);
        region.layout = LayoutBox {
            x: 200.0,
            y: 0.0,
            width: 180.0,
            height: 400.0,
        };
        let layout = Arc::make_mut(&mut region.source_style.layout);
        layout.overflow_x = nana_ui_core::OverflowSpec::Hidden;
        layout.overflow_y = nana_ui_core::OverflowSpec::Hidden;

        let mut handle = node(2, Some(1), &[]);
        handle.kind = Arc::new(NodeKind::Element {
            tag: "workspace-resize-handle".into(),
        });
        handle.layout = LayoutBox {
            x: 196.0,
            y: 0.0,
            width: 8.0,
            height: 400.0,
        };
        style_mut(&mut handle).background = Some([0.5, 0.5, 0.5, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([region, handle], []);
        let painted = scene
            .primitives()
            .find(|primitive| primitive.node == id(2))
            .expect("handle quad");
        assert_eq!(painted.bounds.width, 8.0);
        assert!(
            painted.clips.is_empty(),
            "region overflow must not clip the overlay bar, got {:?}",
            painted.clips
        );
    }

    #[test]
    fn apply_delta_refreshes_instance_on_node_changes_not_idle() {
        let mut scene = UiScene::new();
        let created = scene.instance_id();
        scene.apply_delta([], []);
        assert_eq!(
            scene.instance_id(),
            created,
            "empty apply_delta must keep the instance so idle paint caches hit"
        );

        scene.apply_delta([node(1, None, &[])], []);
        let after_insert = scene.instance_id();
        assert_ne!(
            after_insert, created,
            "inserting a node in place must refresh the instance"
        );

        let mut updated = node(1, None, &[]);
        updated.layout.width = 40.0;
        scene.apply_delta([updated], []);
        let after_update = scene.instance_id();
        assert_ne!(
            after_update, after_insert,
            "updating a node in place must refresh the instance"
        );

        scene.apply_delta([], []);
        assert_eq!(
            scene.instance_id(),
            after_update,
            "empty apply_delta after a real delta must keep the instance"
        );

        let cloned = scene.clone();
        assert_ne!(
            cloned.instance_id(),
            scene.instance_id(),
            "Clone still gets a distinct instance"
        );
    }

    #[test]
    fn extracted_text_spans_travel_on_the_text_primitive() {
        let mut labeled = node(1, None, &[]);
        labeled.text = Some(TextContent {
            value: "fn main".into(),
        });
        labeled.text_spans = vec![nana_ui_runtime::ExtractedTextSpan {
            start: 0,
            end: 2,
            color: [0.2, 0.6, 1.0, 1.0],
        }];
        let mut scene = UiScene::new();
        scene.apply_delta([labeled], []);
        let Some(ScenePrimitiveKind::Text {
            ref content,
            ref spans,
            ..
        }) = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .map(|primitive| primitive.kind.clone())
        else {
            panic!("expected a text primitive");
        };
        assert_eq!(content, "fn main");
        assert_eq!(
            spans,
            &vec![SceneTextSpan {
                start: 0,
                end: 2,
                color: [0.2, 0.6, 1.0, 1.0],
            }]
        );
    }

    #[test]
    fn generic_text_em_padding_uses_computed_font_size() {
        let mut labeled = node(1, None, &[]);
        labeled.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        labeled.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                font_size: Some(32.0),
                padding: Some(nana_ui_core::LengthSpec::Em(1.0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        labeled.text = Some(TextContent {
            value: "hello".into(),
        });
        let mut scene = UiScene::new();
        scene.apply_delta([labeled], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .expect("generic text");
        assert_eq!(
            primitive.bounds,
            SceneRect {
                x: 32.0,
                y: 32.0,
                width: 136.0,
                height: 16.0,
            },
            "1em padding at font-size 32px must inset text 32px, not 16px"
        );
    }

    #[test]
    fn text_input_clip_em_padding_uses_computed_font_size() {
        let mut input = node(1, None, &[]);
        input.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        input.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                font_size: Some(32.0),
                padding: Some(nana_ui_core::LengthSpec::Em(1.0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 32.0,
                    y: 32.0,
                    width: 136.0,
                    height: 16.0,
                },
                content: Arc::from("hi"),
                color: Some([1.0; 4]),
                font_size: 32.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        let text = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .expect("text input text");
        assert_eq!(text.clips.len(), 1);
        assert_eq!(
            text.clips[0].bounds,
            SceneRect {
                x: 32.0,
                y: 32.0,
                width: 136.0,
                height: 16.0,
            },
            "1em padding at font-size 32px must clip the field 32px inset, not 16px"
        );
    }

    #[test]
    fn extraction_preserves_custom_interleaving_and_removals() {
        let mut root = node(1, None, &[2]);
        root.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.1, 0.2, 0.3, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.custom_render = Some(CustomRenderNode::new("host-texture", "preview", 3));
        child.text = Some(TextContent {
            value: "caption".into(),
        });
        let mut scene = UiScene::new();
        let delta = scene.apply_delta([root, child], []);
        assert_eq!(delta.primitive_count, 3);
        let graph = scene.frame_graph(ResourceId(7)).unwrap();
        assert_eq!(
            graph
                .passes
                .iter()
                .flat_map(|pass| pass.operations.iter().cloned())
                .collect::<Vec<_>>(),
            vec![
                RenderOperation::PrepareExternal(PrimitiveId {
                    node: id(2),
                    slot: 1
                }),
                RenderOperation::Draw(PrimitiveId {
                    node: id(1),
                    slot: 0
                }),
                RenderOperation::InvokeCustom(PrimitiveId {
                    node: id(2),
                    slot: 1
                }),
                RenderOperation::Draw(PrimitiveId {
                    node: id(2),
                    slot: 2
                }),
            ]
        );
        assert_eq!(graph.passes.len(), 4);
        assert_eq!(graph.resources.len(), 2);
        assert_eq!(graph.resources[1].label, "preview");
        assert_eq!(graph.passes[0].label, "prepare:preview");
        assert_eq!(graph.passes[2].resources.len(), 2);
        assert!(graph.passes[2].dependencies.contains(&graph.passes[0].id));
        let root_before = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .unwrap()
            .clone();
        let mut changed_child = node(2, Some(1), &[]);
        changed_child.custom_render = Some(CustomRenderNode::new("host-texture", "preview", 4));
        let delta = scene.apply_delta([changed_child], []);
        assert!(!delta.order_rebuilt);
        assert_eq!(delta.rebuilt_primitives, 1);
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0,
                })
                .unwrap(),
            &root_before,
            "a local extraction must not rebuild unrelated primitives"
        );
        let delta = scene.apply_delta([], [id(2)]);
        assert_eq!(delta.removed_nodes, 1);
        assert_eq!(delta.primitive_count, 1);
    }

    #[test]
    fn selection_option_emits_surface_text_icon_and_focus_slots() {
        let mut option = node(7, None, &[]);
        option.layout = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 96.0,
            height: 26.0,
        };
        option.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.2, 0.2, 0.2, 1.0]),
                border_radius: Some(7.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        option.standard_visual = Some(StandardVisual::SelectionOption {
            label: Arc::from("Preview"),
            icon: Some(nana_ui_core::Icon::Search),
            selected: true,
            disabled: false,
            size: ControlSize::Medium,
            show_focus_ring: true,
            indicator: false,
        });
        option.component_geometry = Some(ComponentGeometry::SelectionOption {
            icon: Some((
                nana_ui_core::Icon::Search,
                LayoutBox {
                    x: 20.0,
                    y: 26.0,
                    width: 14.0,
                    height: 14.0,
                },
                [0.8, 0.8, 0.8, 1.0],
            )),
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 39.0,
                    y: 20.0,
                    width: 57.0,
                    height: 26.0,
                },
                content: Arc::from("Preview"),
                color: Some([1.0, 1.0, 1.0, 1.0]),
                font_size: 13.0,
                font_weight: Some(500),
            },
            focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
            indicator: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([option], []);
        for slot in [0, 2, 3, 7] {
            assert!(scene.primitive(PrimitiveId { node: id(7), slot }).is_some());
        }
        let focus = scene
            .primitive(PrimitiveId {
                node: id(7),
                slot: 7,
            })
            .unwrap();
        assert_eq!(
            focus.bounds,
            SceneRect {
                x: 6.0,
                y: 16.0,
                width: 104.0,
                height: 34.0,
            }
        );
        assert!(matches!(
            focus.kind,
            ScenePrimitiveKind::Quad {
                border_width: 2.0,
                corner_radius,
                ..
            } if corner_radius.iter().all(|r| (*r - 11.0).abs() < f32::EPSILON)
        ));
        assert!(matches!(
            scene.primitive(PrimitiveId { node: id(7), slot: 2 }).unwrap().kind,
            ScenePrimitiveKind::Text { ref content, wrap: false, .. } if content == "Preview"
        ));
        assert_eq!(
            primitive_icon(
                &scene
                    .primitive(PrimitiveId {
                        node: id(7),
                        slot: 3
                    })
                    .unwrap()
                    .kind
            ),
            Some(nana_ui_core::Icon::Search)
        );
    }

    #[test]
    fn menu_surface_paints_row_icon_and_iconless_labels() {
        let mut menu = node(3, None, &[]);
        menu.layout = LayoutBox {
            x: 8.0,
            y: 12.0,
            width: 200.0,
            height: 72.0,
        };
        menu.standard_visual = Some(StandardVisual::MenuSurface {
            open: true,
            kind: nana_ui_runtime::MenuSurfaceKind::ContextMenu,
            trigger: None,
            trigger_icon: None,
            gap: 0.0,
            query: None,
            rows: Arc::from([
                nana_ui_runtime::SelectOptionData {
                    label: Arc::from("Add"),
                    hint: None,
                    disabled: false,
                    checked: false,
                    icon: Some(nana_ui_core::Icon::Add),
                },
                nana_ui_runtime::SelectOptionData {
                    label: Arc::from("Rename"),
                    hint: None,
                    disabled: false,
                    checked: false,
                    icon: None,
                },
            ]),
            highlighted: None,
        });
        menu.component_geometry = Some(ComponentGeometry::MenuSurface {
            trigger_surface: None,
            trigger: None,
            trigger_icon: None,
            surface: LayoutBox {
                x: 8.0,
                y: 12.0,
                width: 200.0,
                height: 72.0,
            },
            search: None,
            search_field: None,
            options: vec![
                nana_ui_runtime::SelectOptionGeometry {
                    bounds: LayoutBox {
                        x: 12.0,
                        y: 16.0,
                        width: 192.0,
                        height: 26.0,
                    },
                    label: ComponentTextRegion {
                        bounds: LayoutBox {
                            x: 33.0,
                            y: 16.0,
                            width: 163.0,
                            height: 26.0,
                        },
                        content: Arc::from("Add"),
                        color: Some([1.0, 1.0, 1.0, 1.0]),
                        font_size: 12.0,
                        font_weight: Some(500),
                    },
                    selected: false,
                    checked: false,
                    disabled: false,
                    background: None,
                    icon: Some((
                        nana_ui_core::Icon::Add,
                        LayoutBox {
                            x: 20.0,
                            y: 22.0,
                            width: 13.0,
                            height: 13.0,
                        },
                        [0.7, 0.7, 0.7, 1.0],
                    )),
                },
                nana_ui_runtime::SelectOptionGeometry {
                    bounds: LayoutBox {
                        x: 12.0,
                        y: 43.0,
                        width: 192.0,
                        height: 26.0,
                    },
                    label: ComponentTextRegion {
                        bounds: LayoutBox {
                            x: 20.0,
                            y: 43.0,
                            width: 176.0,
                            height: 26.0,
                        },
                        content: Arc::from("Rename"),
                        color: Some([1.0, 1.0, 1.0, 1.0]),
                        font_size: 12.0,
                        font_weight: Some(500),
                    },
                    selected: false,
                    checked: false,
                    disabled: false,
                    background: None,
                    icon: None,
                },
            ],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.55],
                offset_x: 0.0,
                offset_y: 4.0,
                blur_radius: 18.0,
                spread_radius: 0.0,
                inset: false,
            },
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.2, 0.2, 0.2, 1.0],
        });
        let mut scene = UiScene::new();
        scene.apply_delta([menu], []);
        assert_eq!(
            primitive_icon(
                &scene
                    .primitive(PrimitiveId {
                        node: id(3),
                        slot: 80
                    })
                    .unwrap()
                    .kind
            ),
            Some(nana_ui_core::Icon::Add)
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 40
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { .. }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 41
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { .. }
        ));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 81
                })
                .is_none()
        );
    }

    #[test]
    fn frame_graph_rejects_conflicting_revisions_of_one_external_resource() {
        let mut first = node(1, None, &[]);
        first.custom_render = Some(CustomRenderNode::new("nana.host-texture", "program", 7));
        let mut second = node(2, None, &[]);
        second.custom_render = Some(CustomRenderNode::new("nana.host-texture", "program", 8));
        let mut scene = UiScene::new();
        scene.apply_delta([first, second], []);

        assert_eq!(
            scene.frame_graph(ResourceId(1)),
            Err(GraphError::ConflictingExternalResource("program".into()))
        );
    }

    #[test]
    fn rotate_pivot_follows_transform_origin() {
        let rotate_90 = nana_ui_core::PaintTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            ..Default::default()
        };
        let mut centered = node(1, None, &[]);
        centered.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 10.0,
        };
        centered.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
        centered.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transform: Some(rotate_90),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut corner = node(2, None, &[]);
        corner.layout = centered.layout;
        corner.custom_render = Some(CustomRenderNode::new("test", "resource", 1));
        corner.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transform: Some(rotate_90),
                transform_origin: Some(nana_ui_core::TransformOrigin {
                    x: nana_ui_core::LengthSpec::Px(0.0),
                    y: nana_ui_core::LengthSpec::Px(0.0),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([centered, corner], []);
        let center_tf = scene
            .primitives()
            .find(|primitive| primitive.node == id(1))
            .expect("center")
            .transform
            .0;
        let corner_tf = scene
            .primitives()
            .find(|primitive| primitive.node == id(2))
            .expect("corner")
            .transform
            .0;
        assert_eq!(center_tf, rotate_90.around_center(0.0, 0.0, 20.0, 10.0));
        assert_eq!(corner_tf, rotate_90.around_origin(0.0, 0.0, 0.0, 0.0));
        assert_ne!(center_tf, corner_tf);
    }

    #[test]
    fn perspective_rotate_y_stores_projective_on_the_primitive() {
        let mat = nana_ui_core::PaintMat4::perspective(800.0)
            .expect("d")
            .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()));
        let mut card = node(1, None, &[]);
        card.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        card.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
        card.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transform_3d: Some(mat),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([card], []);
        let primitive = scene
            .primitives()
            .find(|primitive| primitive.node == id(1))
            .expect("card");
        assert!(
            primitive.transform.is_projective(),
            "perspective+rotateY must not collapse to affine, persp={:?}",
            primitive.transform.1
        );
        let expected = mat
            .around_origin(0.0, 0.0, 100.0, 40.0)
            .planar_homography()
            .expect("homography");
        assert_eq!(primitive.transform.0, expected.0);
        assert_eq!(primitive.transform.1, expected.1);
    }

    #[test]
    fn parent_preserve_3d_fail_closes_child_3d() {
        let mat = nana_ui_core::PaintMat4::perspective(800.0)
            .expect("d")
            .then(nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians()));
        let mut parent = node(1, None, &[2]);
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                preserve_3d: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transform_3d: Some(mat),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let primitive = scene
            .primitives()
            .find(|primitive| primitive.node == id(2))
            .expect("child");
        assert_eq!(primitive.transform, AffineTransform::IDENTITY);
    }

    #[test]
    fn parent_perspective_fail_closes_child_3d() {
        let mat = nana_ui_core::PaintMat4::rotate_y(30_f32.to_radians());
        let mut parent = node(1, None, &[2]);
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                css_perspective: Some(800.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transform_3d: Some(mat),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let primitive = scene
            .primitives()
            .find(|primitive| primitive.node == id(2))
            .expect("child");
        assert_eq!(primitive.transform, AffineTransform::IDENTITY);
    }

    #[test]
    fn ancestor_clip_transform_and_opacity_are_composed() {
        let mut root = node(1, None, &[2]);
        root.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                overflow_x: nana_ui_core::OverflowSpec::Hidden,
                opacity: Some(0.5),
                transform: Some(nana_ui_core::PaintTransform {
                    e: 4.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.custom_render = Some(CustomRenderNode::new("test", "resource", 0));
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                opacity: Some(0.5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([root, child], []);
        let custom = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
            .unwrap();
        assert_eq!(custom.opacity, 0.5);
        assert_eq!(
            scene.opacity_groups(id(2)),
            vec![OpacityGroup {
                node: id(1),
                opacity: 0.5,
                filter: ColorFilter::default(),
                mix_blend: MixBlendMode::Normal,
                inset_shadow: None,
            }]
        );
        assert_eq!(custom.clips.len(), 1);
        assert_eq!(custom.transform.0[4], 4.0);
    }

    #[test]
    fn leaf_opacity_stays_on_the_primitive() {
        let mut leaf = node(1, None, &[]);
        leaf.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                opacity: Some(0.5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([leaf], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .unwrap();
        assert_eq!(primitive.opacity, 0.5);
        assert!(scene.opacity_groups(id(1)).is_empty());
    }

    #[test]
    fn opacity_group_keeps_high_z_child_contiguous() {
        let mut parent = node(1, None, &[2]);
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 0.0, 1.0, 1.0]),
                opacity: Some(0.5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.z_index = 10;
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut sibling = node(3, None, &[]);
        sibling.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 1.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child, sibling], []);
        let order = scene
            .primitives()
            .map(|primitive| primitive.node.get())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![1, 2, 3],
            "translucent parent must isolate its high-z child from a later sibling"
        );
    }

    #[test]
    fn losing_group_isolation_reorders_descendants_that_were_not_reextracted() {
        let solid = |color: [f32; 4], opacity: Option<f32>| NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some(color),
                opacity,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut parent = node(1, None, &[2]);
        parent.source_style = solid([0.0, 0.0, 1.0, 1.0], Some(0.5));
        let mut child = node(2, Some(1), &[]);
        child.z_index = 10;
        child.source_style = solid([1.0, 0.0, 0.0, 1.0], None);
        let mut sibling = node(3, None, &[]);
        sibling.source_style = solid([0.0, 1.0, 0.0, 1.0], None);
        let mut scene = UiScene::new();
        scene.apply_delta([parent.clone(), child, sibling.clone()], []);
        let order = |scene: &UiScene| {
            scene
                .primitives()
                .map(|primitive| primitive.node.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&scene), vec![1, 2, 3]);

        // Paint-only update: the child keeps its place without being extracted.
        sibling.source_style = solid([0.0, 1.0, 1.0, 1.0], None);
        scene.apply_delta([sibling], []);
        assert_eq!(
            order(&scene),
            vec![1, 2, 3],
            "a color-only update must not disturb paint order"
        );

        // The parent stops isolating, so the high-z child escapes the group and
        // sorts after the later sibling even though it was not re-extracted.
        parent.source_style = solid([0.0, 0.0, 1.0, 1.0], None);
        scene.apply_delta([parent], []);
        assert_eq!(
            order(&scene),
            vec![1, 3, 2],
            "losing group isolation must reorder the retained descendant"
        );
    }

    #[test]
    fn positioned_z_index_keeps_high_z_child_contiguous() {
        let mut parent = node(1, None, &[2]);
        parent.z_index = 0;
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 0.0, 1.0, 1.0]),
                position: nana_ui_core::PositionSpec::Relative,
                z_index: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.z_index = 10;
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                z_index: Some(10),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut sibling = node(3, None, &[]);
        sibling.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 1.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child, sibling], []);
        let order = scene
            .primitives()
            .map(|primitive| primitive.node.get())
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![1, 2, 3],
            "positioned z-index parent must isolate its high-z child from a later sibling"
        );
    }

    #[test]
    fn isolation_keeps_high_z_child_contiguous() {
        let mut parent = node(1, None, &[2]);
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 0.0, 1.0, 1.0]),
                isolation: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.z_index = 10;
        child.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut sibling = node(3, None, &[]);
        sibling.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 1.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([parent.clone(), child, sibling.clone()], []);
        let order = |scene: &UiScene| {
            scene
                .primitives()
                .map(|primitive| primitive.node.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&scene), vec![1, 2, 3]);

        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 0.0, 1.0, 1.0]),
                isolation: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        scene.apply_delta([parent], []);
        assert_eq!(
            order(&scene),
            vec![1, 3, 2],
            "losing isolation must reorder the retained high-z descendant"
        );
    }

    #[test]
    fn text_primitive_preserves_content_box_and_paint_semantics() {
        let mut text = node(1, None, &[]);
        text.text = Some(TextContent {
            value: "Build".into(),
        });
        text.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                padding_top: Some(nana_ui_core::LengthSpec::Px(2.0)),
                padding_right: Some(nana_ui_core::LengthSpec::Px(8.0)),
                padding_bottom: Some(nana_ui_core::LengthSpec::Px(4.0)),
                padding_left: Some(nana_ui_core::LengthSpec::Px(10.0)),
                border_width: Some(1.0),
                white_space_nowrap: true,
                text_overflow_ellipsis: true,
                ..Default::default()
            }),
            text_horizontal_alignment: TextHorizontalAlignment::Center,
            text_vertical_alignment: TextVerticalAlignment::Center,
            ..Default::default()
        };
        style_mut(&mut text).line_height = Some(LineHeightSpec::Absolute(18.0));

        let mut scene = UiScene::new();
        scene.apply_delta([text], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .unwrap();
        assert_eq!(
            primitive.bounds,
            SceneRect {
                x: 11.0,
                y: 3.0,
                width: 80.0,
                height: 72.0,
            }
        );
        assert!(matches!(
            primitive.kind,
            ScenePrimitiveKind::Text {
                line_height: Some(LineHeightSpec::Absolute(18.0)),
                wrap: false,
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::Center,
                vertical_alignment: TextVerticalAlignment::Center,
                ..
            }
        ));
    }

    #[test]
    #[test]
    fn text_input_editor_markers_and_line_labels_paint() {
        let mut input = node(1, None, &[]);
        input.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                padding_left: Some(nana_ui_core::LengthSpec::Px(40.0)),
                ..nana_ui_core::LayoutStyle::default()
            }),
            ..NodeStyle::default()
        };
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: true,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 40.0,
                    y: 0.0,
                    width: 84.0,
                    height: 48.0,
                },
                content: Arc::from("a\nb\nc"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: vec![
                (
                    LayoutBox {
                        x: 40.0,
                        y: 12.0,
                        width: 10.0,
                        height: 2.0,
                    },
                    [0.9, 0.1, 0.1, 1.0],
                ),
                (
                    LayoutBox {
                        x: 40.0,
                        y: 30.0,
                        width: 10.0,
                        height: 2.0,
                    },
                    [0.9, 0.7, 0.1, 1.0],
                ),
            ],
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            match_markers: Vec::new(),
            line_labels: vec![
                nana_ui_runtime::LineLabel {
                    y: 0.0,
                    height: 16.0,
                    number: 1,
                },
                nana_ui_runtime::LineLabel {
                    y: 16.0,
                    height: 16.0,
                    number: 2,
                },
            ],
            line_labels_color: [0.6, 0.6, 0.6, 1.0],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        // 每个标记一条 quad（颜色不同）。
        let marker = |slot: u8, y: f64| {
            scene
                .primitives()
                .find(|primitive| {
                    primitive.id.slot == slot && (primitive.bounds.y - y as f32).abs() < 0.5
                })
                .expect("marker quad")
        };
        let error_quad = marker(20, 12.0);
        let ScenePrimitiveKind::Quad { background, .. } = &error_quad.kind else {
            panic!("expected quad");
        };
        assert_eq!(*background, Some([0.9, 0.1, 0.1, 1.0]));
        let warning_quad = marker(21, 30.0);
        let ScenePrimitiveKind::Quad { background, .. } = &warning_quad.kind else {
            panic!("expected quad");
        };
        assert_eq!(*background, Some([0.9, 0.7, 0.1, 1.0]));
        // 行号标签为右对齐文本图元。
        let label = scene
            .primitives()
            .find(|primitive| {
                primitive.id.slot == 41
                    && matches!(&primitive.kind, ScenePrimitiveKind::Text { content, .. } if content == "2")
            })
            .expect("line label");
        let ScenePrimitiveKind::Text { content, .. } = &label.kind else {
            unreachable!()
        };
        assert_eq!(&**content, "2");
    }

    #[test]
    fn text_input_match_markers_paint_as_batches_and_current_match_emphasizes() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 32.0,
                },
                content: Arc::from("ab ab"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: vec![(
                LayoutBox {
                    x: 8.0,
                    y: 12.0,
                    width: 10.0,
                    height: 2.0,
                },
                [0.9, 0.1, 0.1, 1.0],
            )],
            match_markers: vec![
                TextMatchMarker {
                    rect: LayoutBox {
                        x: 8.0,
                        y: 0.0,
                        width: 12.0,
                        height: 14.0,
                    },
                    color: [0.48, 0.73, 0.94, 0.20],
                    current: false,
                },
                TextMatchMarker {
                    rect: LayoutBox {
                        x: 8.0,
                        y: 16.0,
                        width: 12.0,
                        height: 14.0,
                    },
                    color: [0.48, 0.73, 0.94, 0.45],
                    current: true,
                },
            ],
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        // 普通匹配为 slot 3 的 quad 批次，当前匹配为更强的 slot 6 批次。
        let batch = |slot: u8| {
            scene
                .primitive(PrimitiveId { node: id(1), slot })
                .expect("match batch")
        };
        let normal = batch(3);
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &normal.kind
        else {
            panic!("expected quad batch");
        };
        assert_eq!(bounds.len(), 1);
        assert_eq!(*background, Some([0.48, 0.73, 0.94, 0.20]));
        let current = batch(6);
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &current.kind
        else {
            panic!("expected quad batch");
        };
        assert_eq!(bounds.len(), 1);
        assert_eq!(*background, Some([0.48, 0.73, 0.94, 0.45]));
        // 诊断下划线（slot 20）与匹配高亮共存。
        let diagnostic = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 20,
            })
            .expect("diagnostic quad");
        let ScenePrimitiveKind::Quad { background, .. } = &diagnostic.kind else {
            panic!("expected quad");
        };
        assert_eq!(*background, Some([0.9, 0.1, 0.1, 1.0]));
    }

    #[test]
    fn text_input_minimap_paints_panel_bars_and_indicator_batches() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 80.0,
                },
                content: Arc::from("a\nb"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            completion_popup: None,
            hover_popup: None,
            minimap: Some(nana_ui_runtime::TextMinimapGeometry {
                panel: LayoutBox {
                    x: 136.0,
                    y: 0.0,
                    width: 64.0,
                    height: 80.0,
                },
                separator: LayoutBox {
                    x: 135.0,
                    y: 0.0,
                    width: 1.0,
                    height: 80.0,
                },
                bars: vec![
                    LayoutBox {
                        x: 136.0,
                        y: 0.0,
                        width: 32.0,
                        height: 2.0,
                    },
                    LayoutBox {
                        x: 136.0,
                        y: 2.0,
                        width: 64.0,
                        height: 2.0,
                    },
                ],
                indicator: Some(LayoutBox {
                    x: 136.0,
                    y: 4.0,
                    width: 64.0,
                    height: 12.0,
                }),
                panel_color: [0.12, 0.12, 0.14, 1.0],
                bar_color: [0.5, 0.5, 0.5, 1.0],
                indicator_color: [1.0, 0.2, 0.2, 0.2],
                stride: 1,
                line_count: 5,
            }),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            steppers: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([input], []);

        // 面板（70）与指示器（72）各一个 quad，行条 + 分隔线共享 slot 71
        // 的一个批次（faint 同色）。
        let panel = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 70,
            })
            .expect("minimap panel");
        let ScenePrimitiveKind::Quad { background, .. } = &panel.kind else {
            panic!("expected panel quad");
        };
        assert_eq!(*background, Some([0.12, 0.12, 0.14, 1.0]));
        assert_eq!(
            panel.bounds,
            SceneRect {
                x: 136.0,
                y: 0.0,
                width: 64.0,
                height: 80.0
            }
        );

        let bars = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 71,
            })
            .expect("minimap bars batch");
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &bars.kind
        else {
            panic!("expected bars batch");
        };
        assert_eq!(bounds.len(), 3, "separator + two bars share one batch");
        assert_eq!(
            bounds[0],
            SceneRect {
                x: 135.0,
                y: 0.0,
                width: 1.0,
                height: 80.0
            }
        );
        assert_eq!(*background, Some([0.5, 0.5, 0.5, 1.0]));

        let indicator = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 72,
            })
            .expect("minimap indicator");
        let ScenePrimitiveKind::Quad { background, .. } = &indicator.kind else {
            panic!("expected indicator quad");
        };
        assert_eq!(*background, Some([1.0, 0.2, 0.2, 0.2]));
    }

    #[test]
    fn occurrence_whitespace_and_wrap_guides_paint_in_dedicated_slots() {
        let occurrence_color = [0.48, 0.73, 0.94, 0.14];
        let faint = [0.35, 0.35, 0.35, 1.0];
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 42.0,
                },
                content: Arc::from("a b\tc"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            // 出现高亮：两条淡底色填充（slot 11 批次）。
            occurrence_markers: vec![
                (
                    LayoutBox {
                        x: 20.0,
                        y: 0.0,
                        width: 20.0,
                        height: 14.0,
                    },
                    occurrence_color,
                ),
                (
                    LayoutBox {
                        x: 60.0,
                        y: 14.0,
                        width: 20.0,
                        height: 14.0,
                    },
                    occurrence_color,
                ),
            ],
            // 空白显示：两个空格（slot 16 圆点批次）+ 一个 Tab（slot 60+
            // 箭头图标）。
            whitespace_marks: vec![
                (
                    LayoutBox {
                        x: 10.0,
                        y: 0.0,
                        width: 10.0,
                        height: 14.0,
                    },
                    nana_ui_runtime::TextWhitespaceKind::Space,
                ),
                (
                    LayoutBox {
                        x: 40.0,
                        y: 0.0,
                        width: 10.0,
                        height: 14.0,
                    },
                    nana_ui_runtime::TextWhitespaceKind::Space,
                ),
                (
                    LayoutBox {
                        x: 30.0,
                        y: 0.0,
                        width: 10.0,
                        height: 14.0,
                    },
                    nana_ui_runtime::TextWhitespaceKind::Tab,
                ),
            ],
            whitespace_color: faint,
            // wrap guide：列 5、10 的全高竖线（slot 17 批次）。
            wrap_guides: vec![
                (
                    LayoutBox {
                        x: 50.0,
                        y: 0.0,
                        width: 1.0,
                        height: 42.0,
                    },
                    faint,
                ),
                (
                    LayoutBox {
                        x: 100.0,
                        y: 0.0,
                        width: 1.0,
                        height: 42.0,
                    },
                    faint,
                ),
            ],
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        let batch = |slot: u8| {
            scene
                .primitive(PrimitiveId { node: id(1), slot })
                .unwrap_or_else(|| panic!("slot {slot} primitive"))
        };
        // 出现高亮批次：两条，共用淡底色。
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &batch(11).kind
        else {
            panic!("expected occurrence quad batch");
        };
        assert_eq!(bounds.len(), 2);
        assert_eq!(*background, Some(occurrence_color));
        // 空格圆点批次：两条小圆点。
        let ScenePrimitiveKind::QuadBatch { bounds, .. } = &batch(16).kind else {
            panic!("expected whitespace dot batch");
        };
        assert_eq!(bounds.len(), 2);
        // Tab 箭头：单一批次（slot 60），箭头图标图元。
        let tab = batch(60);
        let ScenePrimitiveKind::IconBatch {
            bounds: arrow_bounds,
            icon,
            color,
        } = &tab.kind
        else {
            panic!("expected tab arrow icon batch");
        };
        assert_eq!(arrow_bounds.len(), 1);
        assert_eq!(*icon, Icon::ArrowRight);
        assert_eq!(*color, Some(faint));
        // wrap guide 批次：两条全高竖线。
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &batch(17).kind
        else {
            panic!("expected wrap guide batch");
        };
        assert_eq!(bounds.len(), 2);
        assert_eq!(bounds[0].height, 42.0);
        assert_eq!(*background, Some(faint));
    }

    #[test]
    fn text_input_without_editor_extras_paints_no_occurrence_whitespace_or_wrap_slots() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 14.0,
                },
                content: Arc::from("plain"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        for slot in [11, 16, 17, 60] {
            assert!(
                scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
                "slot {slot} must stay empty"
            );
        }
    }

    #[test]
    fn fold_gutter_marks_paint_as_two_batches_and_survive_beyond_the_slot_cap() {
        const FOLDS: usize = 25;
        let mut gutters = Vec::with_capacity(FOLDS);
        for index in 0..FOLDS {
            gutters.push(nana_ui_runtime::TextFoldGutter {
                bounds: LayoutBox {
                    x: 2.0,
                    y: index as f32 * 14.0,
                    width: 14.0,
                    height: 14.0,
                },
                fold: nana_ui_runtime::TextCodeFold::new(index * 10, index * 10 + 8),
                collapsed: index % 2 == 0,
                color: [0.5, 0.5, 0.5, 0.4],
            });
        }
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 18.0,
                    y: 0.0,
                    width: 180.0,
                    height: 350.0,
                },
                content: Arc::from("fn a() {}"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry {
                gutters,
                markers: Vec::new(),
            },
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        // 折叠态（slot 14，实心）与展开态（slot 15，描边）各一个批次，
        // 超过旧 slot 上限（21）后仍全部渲染。
        let batch = |slot: u8| {
            scene
                .primitive(PrimitiveId { node: id(1), slot })
                .expect("gutter batch")
        };
        let collapsed = batch(14);
        let collapsed_len = match &collapsed.kind {
            ScenePrimitiveKind::QuadBatch {
                bounds, background, ..
            } => {
                assert_eq!(bounds.len(), 13);
                assert_eq!(*background, Some([0.5, 0.5, 0.5, 0.4]));
                assert_eq!(
                    bounds[0],
                    SceneRect {
                        x: 2.0,
                        y: 0.0,
                        width: 14.0,
                        height: 14.0,
                    }
                );
                bounds.len()
            }
            _ => panic!("expected collapsed gutter quad batch"),
        };
        let expanded = batch(15);
        let expanded_len = match &expanded.kind {
            ScenePrimitiveKind::QuadBatch {
                bounds,
                background,
                border_color,
                border_width,
                ..
            } => {
                assert_eq!(bounds.len(), 12);
                assert_eq!(*background, None);
                assert_eq!(*border_color, Some([0.5, 0.5, 0.5, 0.4]));
                assert_eq!(*border_width, 1.0);
                assert_eq!(
                    bounds[11],
                    SceneRect {
                        x: 2.0,
                        y: 23.0 * 14.0,
                        width: 14.0,
                        height: 14.0,
                    }
                );
                bounds.len()
            }
            _ => panic!("expected expanded gutter quad batch"),
        };
        // 全部 25 个箭头（>21）都渲染为批次内的 quad，不再互相覆盖。
        assert_eq!(collapsed_len + expanded_len, FOLDS);
    }

    #[test]
    fn tab_arrows_paint_as_one_batch_and_survive_beyond_the_slot_cap() {
        const TABS: usize = 300;
        let mut marks = Vec::with_capacity(TABS);
        for index in 0..TABS {
            marks.push((
                LayoutBox {
                    x: 10.0 + (index % 40) as f32 * 10.0,
                    y: (index / 40) as f32 * 14.0,
                    width: 10.0,
                    height: 14.0,
                },
                nana_ui_runtime::TextWhitespaceKind::Tab,
            ));
        }
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 18.0,
                    y: 0.0,
                    width: 180.0,
                    height: 350.0,
                },
                content: Arc::from("\t".repeat(TABS)),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            occurrence_markers: Vec::new(),
            whitespace_marks: marks,
            whitespace_color: [0.5, 0.5, 0.5, 1.0],
            wrap_guides: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        // 单一批次（slot 60）装下全部 300 个 Tab 箭头；旧实现按
        // 60 + index 分配 slot，超过 195 个后在 u8 上回绕互相覆盖。
        let batch = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 60,
            })
            .expect("tab arrow batch");
        let ScenePrimitiveKind::IconBatch {
            bounds: arrows,
            icon,
            color,
        } = &batch.kind
        else {
            panic!("expected tab arrow icon batch");
        };
        assert_eq!(arrows.len(), TABS);
        assert_eq!(*icon, Icon::ArrowRight);
        assert_eq!(*color, Some([0.5, 0.5, 0.5, 1.0]));
        for (index, arrow) in arrows.iter().enumerate() {
            let expected_cell_x = 10.0 + (index % 40) as f32 * 10.0;
            let expected_cell_y = (index / 40) as f32 * 14.0;
            // 箭头按字符单元高度的 0.55 居中放置。
            let extent = (14.0f32 * 0.55).clamp(6.0, 14.0);
            assert!((arrow.width - extent).abs() < f32::EPSILON);
            assert!(
                (arrow.x - (expected_cell_x + (10.0 - extent) / 2.0)).abs() < f32::EPSILON,
                "arrow {index} x mismatch"
            );
            assert!(
                (arrow.y - (expected_cell_y + (14.0 - extent) / 2.0)).abs() < f32::EPSILON,
                "arrow {index} y mismatch"
            );
        }
        // 批次外的任何 slot 都不承载 Tab 箭头。
        for slot in [61u8, 100, 200, 255] {
            assert!(
                scene.primitive(PrimitiveId { node: id(1), slot }).is_none(),
                "slot {slot} must stay empty"
            );
        }
    }

    #[test]
    fn text_input_paints_additional_cursors_as_a_batch_beside_the_primary_caret() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 40.0,
                    y: 0.0,
                    width: 84.0,
                    height: 48.0,
                },
                content: Arc::from("a\nb\nc"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: Some(LayoutBox {
                x: 48.0,
                y: 8.0,
                width: 1.0,
                height: 16.0,
            }),
            additional_carets: vec![
                LayoutBox {
                    x: 8.0,
                    y: 24.0,
                    width: 1.0,
                    height: 16.0,
                },
                LayoutBox {
                    x: 20.0,
                    y: 40.0,
                    width: 1.0,
                    height: 16.0,
                },
            ],
            additional_caret_color: [0.2, 0.2, 0.2, 0.55],
            preedit: Vec::new(),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);

        // 主光标保持单矩形 slot 4。
        let caret = scene
            .primitives()
            .find(|primitive| primitive.id.slot == 4)
            .expect("primary caret");
        assert_eq!(caret.bounds.y, 8.0);

        // 附加光标合并为一个半透明 quad 批次（slot 13）。
        let batch = scene
            .primitives()
            .find(|primitive| primitive.id.slot == 13)
            .expect("additional caret batch");
        let ScenePrimitiveKind::QuadBatch {
            bounds: rects,
            background,
            ..
        } = &batch.kind
        else {
            panic!("expected quad batch");
        };
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].y, 24.0);
        assert_eq!(rects[1].y, 40.0);
        assert_eq!(*background, Some([0.2, 0.2, 0.2, 0.55]));
    }

    #[test]
    fn text_input_editor_chrome_paints_caret_line_brackets_and_indent_guides() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: Some(Arc::from("\t")),
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 48.0,
                },
                content: Arc::from("ab"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            // 当前行条与选区同层（slot 1）。
            caret_line: Some((
                LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 16.0,
                },
                [0.18, 0.18, 0.18, 1.0],
            )),
            // 括号匹配两端共用 accent 描边。
            bracket_markers: vec![
                (
                    LayoutBox {
                        x: 8.0,
                        y: 0.0,
                        width: 6.0,
                        height: 16.0,
                    },
                    [0.48, 0.73, 0.94, 1.0],
                ),
                (
                    LayoutBox {
                        x: 20.0,
                        y: 16.0,
                        width: 6.0,
                        height: 16.0,
                    },
                    [0.48, 0.73, 0.94, 1.0],
                ),
            ],
            // 缩进参考线：两条 1px 竖线一个批次。
            indent_guides: vec![
                (
                    LayoutBox {
                        x: 10.0,
                        y: 0.0,
                        width: 1.0,
                        height: 16.0,
                    },
                    [0.16, 0.16, 0.16, 1.0],
                ),
                (
                    LayoutBox {
                        x: 10.0,
                        y: 16.0,
                        width: 1.0,
                        height: 16.0,
                    },
                    [0.16, 0.16, 0.16, 1.0],
                ),
            ],
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        let primitive = |slot: u8| {
            scene
                .primitive(PrimitiveId { node: id(1), slot })
                .expect("chrome primitive")
        };
        // 当前行条是 slot 1 的单个填充 quad。
        let line = primitive(1);
        let ScenePrimitiveKind::Quad { background, .. } = &line.kind else {
            panic!("expected caret line quad");
        };
        assert_eq!(*background, Some([0.18, 0.18, 0.18, 1.0]));
        assert_eq!(line.bounds.width, 84.0);
        // 缩进参考线是 slot 10 的填充批次，同一颜色合并。
        let guides = primitive(10);
        let ScenePrimitiveKind::QuadBatch {
            bounds, background, ..
        } = &guides.kind
        else {
            panic!("expected indent guide batch");
        };
        assert_eq!(bounds.len(), 2);
        assert_eq!(*background, Some([0.16, 0.16, 0.16, 1.0]));
        // 括号匹配是 slot 12 的描边批次（无填充，不遮挡字形）。
        let brackets = primitive(12);
        let ScenePrimitiveKind::QuadBatch {
            bounds,
            background,
            border_color,
            border_width,
            ..
        } = &brackets.kind
        else {
            panic!("expected bracket batch");
        };
        assert_eq!(bounds.len(), 2);
        assert_eq!(*background, None);
        assert_eq!(*border_color, Some([0.48, 0.73, 0.94, 1.0]));
        assert_eq!(*border_width, 1.0);
    }

    fn text_input_geometry_paints_selection_text_caret_preedit_and_focus_in_order() {
        let mut input = node(1, None, &[]);
        input.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.1, 0.1, 0.1, 1.0]),
                border_width: Some(1.0),
                border_color: Some([0.3, 0.3, 0.3, 1.0]),
                border_radius: Some(6.0),
                overflow_x: nana_ui_core::OverflowSpec::Hidden,
                opacity: Some(0.5),
                transform: Some(nana_ui_core::PaintTransform {
                    e: 4.0,
                    f: 6.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 32.0,
                },
                content: Arc::from("release/next"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: (0..10)
                .map(|line| LayoutBox {
                    x: 8.0,
                    y: 8.0 + line as f32 * 16.0,
                    width: 40.0 + line as f32,
                    height: 16.0,
                })
                .collect(),
            caret: Some(LayoutBox {
                x: 48.0,
                y: 8.0,
                width: 1.0,
                height: 16.0,
            }),
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: vec![
                LayoutBox {
                    x: 48.0,
                    y: 23.0,
                    width: 18.0,
                    height: 1.0,
                },
                LayoutBox {
                    x: 8.0,
                    y: 39.0,
                    width: 24.0,
                    height: 1.0,
                },
            ],
            background: Some([0.1, 0.1, 0.1, 1.0]),
            border: Some([0.3, 0.3, 0.3, 1.0]),
            border_width: 1.0,
            focus_ring: Some([0.2, 0.6, 1.0, 1.0]),
            selection_color: [0.2, 0.4, 0.7, 0.4],
            caret_color: [0.2, 0.6, 1.0, 1.0],
            preedit_color: [0.2, 0.6, 1.0, 1.0],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([input], []);
        assert_eq!(scene.primitives().count(), 6);
        for slot in [0, 1, 2, 4, 5, 7] {
            assert!(scene.primitive(PrimitiveId { node: id(1), slot }).is_some());
        }
        assert!(matches!(
            scene.primitive(PrimitiveId { node: id(1), slot: 2 }).unwrap().kind,
            ScenePrimitiveKind::Text {
                ref content,
                wrap: true,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            } if content == "release/next"
        ));
        for slot in [1, 5] {
            let primitive = scene.primitive(PrimitiveId { node: id(1), slot }).unwrap();
            assert_eq!(primitive.transform.0[4..], [4.0, 6.0]);
            assert_eq!(primitive.opacity, 0.5);
            assert_eq!(primitive.clips.len(), 2);
            let expected_count = if slot == 1 { 10 } else { 2 };
            assert!(matches!(
                primitive.kind,
                ScenePrimitiveKind::QuadBatch { ref bounds, .. }
                    if bounds.len() == expected_count
            ));
        }

        let mut single_line = node(2, None, &[]);
        single_line.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        single_line.component_geometry = input_component_geometry(false);
        scene.apply_delta([single_line], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: false,
                vertical_alignment: TextVerticalAlignment::Center,
                ..
            }
        ));
    }

    fn input_component_geometry(multiline: bool) -> Option<ComponentGeometry> {
        Some(ComponentGeometry::TextInput {
            multiline,
            text: nana_ui_runtime::ComponentTextRegion {
                bounds: LayoutBox {
                    x: 8.0,
                    y: 0.0,
                    width: 84.0,
                    height: 32.0,
                },
                content: Arc::from("release/next"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.2, 0.4, 0.7, 0.4],
            caret_color: [0.2, 0.6, 1.0, 1.0],
            preedit_color: [0.2, 0.6, 1.0, 1.0],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: None,
            hover_popup: None,
        })
    }

    #[test]
    fn feedback_geometry_emits_semantic_quad_text_and_icon_primitives() {
        let region = |content: &'static str, y, size, weight| ComponentTextRegion {
            bounds: LayoutBox {
                x: 20.0,
                y,
                width: 120.0,
                height: 18.0,
            },
            content: Arc::from(content),
            color: Some([0.8, 0.9, 1.0, 1.0]),
            font_size: size,
            font_weight: weight,
        };
        let mut badge = node(1, None, &[]);
        badge.source_style.layout = Arc::new(nana_ui_core::LayoutStyle {
            background: Some([0.1, 0.2, 0.3, 1.0]),
            ..Default::default()
        });
        badge.standard_visual = Some(StandardVisual::StatusBadge {
            label: Arc::from("Online"),
            tone: nana_ui_core::StatusTone::Success,
            compact: true,
        });
        badge.component_geometry = Some(ComponentGeometry::StatusBadge {
            indicator: LayoutBox {
                x: 8.0,
                y: 8.0,
                width: 3.0,
                height: 3.0,
            },
            label: region("Online", 0.0, 11.0, Some(500)),
            background: [0.1, 0.8, 0.4, 0.12],
            foreground: [0.1, 0.8, 0.4, 1.0],
        });

        let mut validation = node(2, None, &[]);
        validation.standard_visual = Some(StandardVisual::ValidationMessage {
            message: Arc::from("Required"),
            intent: nana_ui_core::ValidationIntent::Danger,
            compact: true,
        });
        validation.component_geometry = Some(ComponentGeometry::ValidationMessage {
            indicator: LayoutBox {
                x: 4.0,
                y: 7.0,
                width: 5.0,
                height: 5.0,
            },
            label: region("Required", 0.0, 11.0, None),
            foreground: [0.9, 0.2, 0.2, 1.0],
        });

        let mut empty = node(3, None, &[]);
        empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("No files"),
            message: Some(Arc::from("Create one")),
            icon: Some(nana_ui_core::Icon::Folder),
            compact: false,
            action: None,
        });
        empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: empty.layout,
            content_clip: LayoutBox {
                x: 16.0,
                y: 24.0,
                width: 68.0,
                height: 52.0,
            },
            icon: Some((
                nana_ui_core::Icon::Folder,
                LayoutBox {
                    x: 40.0,
                    y: 2.0,
                    width: 22.0,
                    height: 22.0,
                },
                [0.5, 0.5, 0.5, 1.0],
            )),
            title: region("No files", 26.0, 13.0, Some(600)),
            message: Some(region("Create one", 46.0, 12.0, None)),
            action: None,
        });

        let mut labeled = node(4, None, &[]);
        labeled.standard_visual = Some(StandardVisual::LabeledValue {
            label: Arc::from("Revision"),
            value: Arc::from("42"),
            value_role: nana_ui_core::SemanticColorRole::Text,
            value_weight: 600,
            compact: true,
            action: None,
        });
        labeled.component_geometry = Some(ComponentGeometry::LabeledValue {
            label: region("Revision", 0.0, 11.0, None),
            value: region("42", 14.0, 12.0, Some(600)),
            action: None,
        });

        let mut compact_empty = node(5, None, &[]);
        compact_empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("空状态"),
            message: None,
            icon: None,
            compact: true,
            action: None,
        });
        compact_empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: compact_empty.layout,
            content_clip: LayoutBox {
                x: 6.0,
                y: 8.0,
                width: 88.0,
                height: 84.0,
            },
            icon: None,
            title: region("空状态", 2.0, 12.0, Some(600)),
            message: None,
            action: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([badge, validation, empty, labeled, compact_empty], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([_, _, _, 0.12]),
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some(_),
                border_width: 0.0,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: None,
                border_width: 1.0,
                ..
            }
        ));
        assert_eq!(
            primitive_icon(
                &scene
                    .primitive(PrimitiveId {
                        node: id(3),
                        slot: 3
                    })
                    .unwrap()
                    .kind
            ),
            Some(nana_ui_core::Icon::Folder)
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 4
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { size: 12.0, .. }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                vertical_alignment: TextVerticalAlignment::Top,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                size: 11.0,
                weight: None,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                size: 12.0,
                weight: Some(600),
                ..
            }
        ));
    }

    #[test]
    fn labeled_value_and_card_text_primitives_enable_ellipsis() {
        // 长文本区域(label/value/标题)必须开启省略截断,否则 Start/End 对齐
        // 的溢出会盖过属性行对侧内容(如完整文件路径)。
        let long_path: Arc<str> =
            Arc::from("/Users/dev/workspace/very-long-project/assets/textures");
        let region = |content: Arc<str>, y| ComponentTextRegion {
            bounds: LayoutBox {
                x: 12.0,
                y,
                width: 96.0,
                height: 16.0,
            },
            content,
            color: None,
            font_size: 12.0,
            font_weight: None,
        };

        let mut labeled = node(1, None, &[]);
        labeled.standard_visual = Some(StandardVisual::LabeledValue {
            label: Arc::from("Source"),
            value: long_path.clone(),
            value_role: nana_ui_core::SemanticColorRole::Text,
            value_weight: 600,
            compact: false,
            action: None,
        });
        labeled.component_geometry = Some(ComponentGeometry::LabeledValue {
            label: region(Arc::from("Source"), 0.0),
            value: region(long_path.clone(), 14.0),
            action: None,
        });

        let mut card = node(2, None, &[]);
        card.standard_visual = Some(StandardVisual::Card {
            title: Some(Arc::from("渲染管线")),
            kind: nana_ui_core::CardKind::Surface,
            loading: false,
            loading_phase: 0.0,
        });
        card.component_geometry = Some(ComponentGeometry::Card {
            title: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 10.0,
                    y: 8.0,
                    width: 60.0,
                    height: 18.0,
                },
                content: Arc::from("渲染管线 / Render Pipeline Settings"),
                color: None,
                font_size: 13.0,
                font_weight: Some(600),
            }),
            content: LayoutBox {
                x: 10.0,
                y: 36.0,
                width: 80.0,
                height: 34.0,
            },
            elevation: None,
            spinner: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([labeled, card], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::End,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { ellipsis: true, .. }
        ));
    }

    #[test]
    fn action_menu_item_label_and_hint_text_primitives_enable_ellipsis() {
        let region = |content: &'static str, x, width| ComponentTextRegion {
            bounds: LayoutBox {
                x,
                y: 4.0,
                width,
                height: 18.0,
            },
            content: Arc::from(content),
            color: None,
            font_size: 13.0,
            font_weight: None,
        };

        let mut item = node(1, None, &[]);
        item.standard_visual = Some(StandardVisual::ActionMenuItem {
            label: Arc::from("Reveal in Finder"),
            hint: Some(Arc::from("/Users/dev/very-long-project-folder-name")),
            icon: Some(nana_ui_core::Icon::Folder),
            danger: false,
            active: false,
            disabled: false,
            size: nana_ui_core::ControlSize::Medium,
        });
        item.component_geometry = Some(ComponentGeometry::ActionMenuItem {
            icon: Some((
                nana_ui_core::Icon::Folder,
                LayoutBox {
                    x: 8.0,
                    y: 6.0,
                    width: 16.0,
                    height: 16.0,
                },
                [0.8, 0.8, 0.8, 1.0],
            )),
            label: region("/Users/dev/very-long-project-folder-name", 32.0, 100.0),
            hint: Some(region("/Users/dev/very-long-project-folder-name", 132.0, 60.0)),
            background: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([item], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 4
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: true,
                horizontal_alignment: TextHorizontalAlignment::End,
                ..
            }
        ));
    }

    #[test]
    fn modal_frame_emits_distinct_scrim_surface_and_intrinsic_text_slots() {
        let mut modal = node(50, None, &[]);
        modal.standard_visual = Some(StandardVisual::ModalFrame {
            title: Arc::from("Delete project"),
            description: Some(Arc::from("This cannot be undone")),
            body_text: None,
            kind: nana_ui_runtime::ModalSurfaceKind::Confirm(nana_ui_core::DialogSize::Compact),
            busy: false,
            danger: false,
            slots: nana_ui_runtime::ModalSlots::default(),
        });
        let text = |content: &'static str, y, height, size, weight| ComponentTextRegion {
            bounds: LayoutBox {
                x: 206.0,
                y,
                width: 388.0,
                height,
            },
            content: Arc::from(content),
            color: Some([1.0; 4]),
            font_size: size,
            font_weight: weight,
        };
        modal.component_geometry = Some(ComponentGeometry::ModalFrame {
            scrim: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            surface: LayoutBox {
                x: 190.0,
                y: 72.0,
                width: 420.0,
                height: 180.0,
            },
            body: LayoutBox {
                x: 206.0,
                y: 130.0,
                width: 388.0,
                height: 80.0,
            },
            title: text("Delete project", 86.0, 20.0, 14.0, Some(600)),
            description: Some(text("This cannot be undone", 110.0, 18.0, 12.0, None)),
            body_text: None,
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.24],
                offset_x: 0.0,
                offset_y: 8.0,
                blur_radius: 24.0,
                spread_radius: 0.0,
                inset: false,
            },
        });
        let mut scene = UiScene::default();
        scene.apply_delta([modal], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 10
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.0, 0.0, 0.0, 0.45]),
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 11
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                border_color: None,
                border_width: 0.0,
                corner_radius,
                shadow: Some(_),
                ..
            } if corner_radius
                .iter()
                .all(|r| (*r - UI_METRICS.radius_md).abs() < f32::EPSILON)
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 12
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                wrap: true,
                horizontal_alignment: TextHorizontalAlignment::Start,
                ..
            }
        ));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(50),
                    slot: 13
                })
                .is_some()
        );
    }

    #[test]
    fn command_palette_title_and_query_sort_above_surface_quads() {
        let mut palette = node(60, None, &[]);
        palette.standard_visual = Some(StandardVisual::CommandPalette {
            title: Arc::from("命令"),
            query: Arc::from("工作区"),
            placeholder: Arc::from("搜索操作"),
            empty: None,
            rows: Arc::from([]),
        });
        let text = |content: &'static str, y, height, size, weight| ComponentTextRegion {
            bounds: LayoutBox {
                x: 24.0,
                y,
                width: 360.0,
                height,
            },
            content: Arc::from(content),
            color: Some([1.0; 4]),
            font_size: size,
            font_weight: weight,
        };
        palette.component_geometry = Some(ComponentGeometry::CommandPalette {
            scrim: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            surface: LayoutBox {
                x: 160.0,
                y: 80.0,
                width: 480.0,
                height: 240.0,
            },
            title: text("命令", 96.0, 22.0, 16.0, Some(600)),
            input: text("工作区", 132.0, 32.0, 13.0, None),
            empty: Some(text("没有可用操作", 176.0, 40.0, 12.0, None)),
            rows: Vec::new(),
            background: [0.1, 0.1, 0.1, 1.0],
            input_background: [0.08, 0.08, 0.08, 1.0],
            input_border: [0.3, 0.3, 0.3, 1.0],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.4],
                offset_x: 0.0,
                offset_y: 12.0,
                blur_radius: 24.0,
                spread_radius: 0.0,
                inset: false,
            },
        });

        let mut scene = UiScene::default();
        scene.apply_delta([palette], []);
        let node = id(60);
        let ordered = scene
            .primitives()
            .filter(|primitive| primitive.node == node)
            .collect::<Vec<_>>();
        let position = |slot: u8| {
            ordered
                .iter()
                .position(|primitive| primitive.id.slot == slot)
                .unwrap_or_else(|| panic!("missing command-palette slot {slot}"))
        };
        let surface = scene.primitive(PrimitiveId { node, slot: 11 }).unwrap();
        let input_quad = scene.primitive(PrimitiveId { node, slot: 12 }).unwrap();
        let title = scene.primitive(PrimitiveId { node, slot: 20 }).unwrap();
        let query = scene.primitive(PrimitiveId { node, slot: 21 }).unwrap();
        let empty = scene.primitive(PrimitiveId { node, slot: 22 }).unwrap();

        assert!(matches!(surface.kind, ScenePrimitiveKind::Quad { .. }));
        assert!(matches!(input_quad.kind, ScenePrimitiveKind::Quad { .. }));
        assert!(matches!(
            &title.kind,
            ScenePrimitiveKind::Text { content, .. } if content == "命令"
        ));
        assert!(matches!(
            &query.kind,
            ScenePrimitiveKind::Text { content, .. } if content == "工作区"
        ));
        assert!(matches!(&empty.kind, ScenePrimitiveKind::Text { .. }));
        assert_eq!(title.z_index, surface.z_index);
        assert_eq!(query.z_index, input_quad.z_index);
        assert_eq!(title.document_order, surface.document_order);
        assert_eq!(query.document_order, input_quad.document_order);
        assert!(
            position(20) > position(11) && position(20) > position(12),
            "title must sort after surface and input quads"
        );
        assert!(
            position(21) > position(11) && position(21) > position(12),
            "query must sort after surface and input quads"
        );
        assert!(
            position(22) > position(11) && position(22) > position(12),
            "empty text must sort after surface and input quads"
        );
        assert!(scene.primitive(PrimitiveId { node, slot: 2 }).is_none());
        assert!(scene.primitive(PrimitiveId { node, slot: 3 }).is_none());
        assert!(scene.primitive(PrimitiveId { node, slot: 4 }).is_none());
    }

    #[test]
    fn docked_drawer_extends_the_flush_edge_so_clipping_squares_that_side() {
        let mut drawer = node(52, None, &[]);
        drawer.standard_visual = Some(StandardVisual::ModalFrame {
            title: Arc::from("Inspector"),
            description: None,
            body_text: None,
            kind: nana_ui_runtime::ModalSurfaceKind::Drawer(DrawerSide::Right),
            busy: false,
            danger: false,
            slots: nana_ui_runtime::ModalSlots::default(),
        });
        drawer.component_geometry = Some(ComponentGeometry::ModalFrame {
            scrim: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 420.0,
                height: 240.0,
            },
            surface: LayoutBox {
                x: 60.0,
                y: 0.0,
                width: 360.0,
                height: 240.0,
            },
            body: LayoutBox {
                x: 76.0,
                y: 64.0,
                width: 328.0,
                height: 160.0,
            },
            title: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 76.0,
                    y: 14.0,
                    width: 280.0,
                    height: 17.0,
                },
                content: Arc::from("Inspector"),
                color: Some([1.0; 4]),
                font_size: 14.0,
                font_weight: Some(600),
            },
            description: None,
            body_text: None,
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.0; 4],
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.45],
                offset_x: 0.0,
                offset_y: 14.0,
                blur_radius: 30.0,
                spread_radius: 0.0,
                inset: false,
            },
        });
        let mut scene = UiScene::default();
        scene.apply_delta([drawer], []);
        let surface = scene
            .primitive(PrimitiveId {
                node: id(52),
                slot: 11,
            })
            .unwrap();
        assert!((surface.bounds.width - (360.0 + UI_METRICS.radius_md)).abs() < f32::EPSILON);
        assert!((surface.bounds.x - 60.0).abs() < f32::EPSILON);
        assert_eq!(surface.clips.len(), 1);
        assert!((surface.clips[0].bounds.width - 420.0).abs() < f32::EPSILON);
    }

    #[test]
    fn confirm_action_scene_restores_label_after_busy_spinner_clears() {
        let mut action = node(51, None, &[]);
        let label = ComponentTextRegion {
            bounds: LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 24.0,
            },
            content: Arc::from("Delete"),
            color: Some([1.0; 4]),
            font_size: 13.0,
            font_weight: Some(500),
        };
        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Delete"),
            kind: nana_ui_core::ButtonKind::Danger,
            size: nana_ui_core::ControlSize::Medium,
            loading: true,
            loading_phase: 0.5,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label: label.clone(),
            spinner: Some(LayoutBox {
                x: 42.0,
                y: 14.0,
                width: 16.0,
                height: 16.0,
            }),
            background: Some([0.8, 0.1, 0.1, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([action.clone()], []);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 3
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Spinner { .. }
        ));

        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Delete"),
            kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label,
            spinner: None,
            background: Some([0.2, 0.4, 0.8, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: None,
        });
        scene.apply_delta([action], []);
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 3
                })
                .is_none()
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(51),
                    slot: 2
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { ref content, .. } if content == "Delete"
        ));
    }

    #[test]
    fn empty_state_separates_intrinsic_clip_from_focused_action_root_clip() {
        let content_clip = LayoutBox {
            x: 8.0,
            y: 9.0,
            width: 1.0,
            height: 2.0,
        };
        let mut empty = node(10, None, &[11]);
        empty.standard_visual = Some(StandardVisual::EmptyState {
            title: Arc::from("Title"),
            message: Some(Arc::from("Message")),
            icon: Some(nana_ui_core::Icon::Folder),
            compact: false,
            action: Some(id(11)),
        });
        empty.component_geometry = Some(ComponentGeometry::EmptyState {
            root_clip: empty.layout,
            content_clip,
            icon: Some((
                nana_ui_core::Icon::Folder,
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 22.0,
                    height: 22.0,
                },
                [0.5, 0.5, 0.5, 1.0],
            )),
            title: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 22.0,
                    width: 40.0,
                    height: 30.0,
                },
                content: Arc::from("Title"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: Some(600),
            },
            message: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 58.0,
                    width: 60.0,
                    height: 40.0,
                },
                content: Arc::from("Message"),
                color: Some([0.7; 4]),
                font_size: 12.0,
                font_weight: None,
            }),
            action: Some(LayoutBox {
                x: 20.0,
                y: 53.0,
                width: 60.0,
                height: 24.0,
            }),
        });
        let mut action = node(11, Some(10), &[]);
        action.layout = LayoutBox {
            x: 20.0,
            y: 53.0,
            width: 60.0,
            height: 24.0,
        };
        action.focused = true;
        action.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Action"),
            kind: nana_ui_core::ButtonKind::Primary,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        action.component_geometry = Some(ComponentGeometry::Button {
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 20.0,
                    y: 53.0,
                    width: 60.0,
                    height: 24.0,
                },
                content: Arc::from("Action"),
                color: Some([1.0; 4]),
                font_size: 13.0,
                font_weight: None,
            },
            spinner: None,
            background: Some([0.2, 0.4, 0.8, 1.0]),
            border: None,
            border_width: 0.0,
            focus_ring: Some([0.3, 0.6, 1.0, 1.0]),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([empty, action], []);
        let content = ClipRegion {
            bounds: scene_rect(content_clip),
            transform: AffineTransform::IDENTITY,
            corner_radius: 0.0,
            polygon_clip: None,
        };
        for primitive in [
            PrimitiveId {
                node: id(10),
                slot: 2,
            },
            PrimitiveId {
                node: id(10),
                slot: 3,
            },
            PrimitiveId {
                node: id(10),
                slot: 4,
            },
        ] {
            assert!(scene.primitive(primitive).unwrap().clips.contains(&content));
        }
        let root = ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            transform: AffineTransform::IDENTITY,
            corner_radius: 0.0,
            polygon_clip: None,
        };
        for primitive in [
            PrimitiveId {
                node: id(11),
                slot: 2,
            },
            PrimitiveId {
                node: id(11),
                slot: 7,
            },
        ] {
            let primitive = scene.primitive(primitive).unwrap();
            assert!(primitive.clips.contains(&root));
            assert!(!primitive.clips.contains(&content));
        }
        let ring = scene
            .primitive(PrimitiveId {
                node: id(11),
                slot: 7,
            })
            .unwrap();
        assert_eq!(ring.bounds.y + ring.bounds.height, root.bounds.height);
    }

    #[test]
    fn standard_control_visuals_expand_without_backend_tag_matching() {
        let mut checkbox = node(1, None, &[]);
        checkbox.text = Some(TextContent {
            value: "Notifications".into(),
        });
        checkbox.standard_visual = Some(StandardVisual::Checkbox {
            checked: true,
            indeterminate: false,
            size: nana_ui_core::ControlSize::Medium,
        });
        style_mut(&mut checkbox).background = Some([0.2, 0.5, 0.9, 1.0]);
        style_mut(&mut checkbox).border_color = Some([0.1, 0.2, 0.3, 1.0]);

        let mut slider = node(2, None, &[]);
        slider.standard_visual = Some(StandardVisual::Range {
            label: None,
            value: Arc::from("25"),
            unit: None,
            size: nana_ui_core::ControlSize::Medium,
            ratio: 0.25,
            invalid: false,
        });
        style_mut(&mut slider).background = Some([0.2, 0.5, 0.9, 1.0]);
        style_mut(&mut slider).border_color = Some([0.4, 0.4, 0.4, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([checkbox, slider], []);
        assert_eq!(scene.primitives().count(), 6);
        let checkbox_text = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .unwrap();
        assert_eq!(checkbox_text.bounds.x, 24.0);
        assert_eq!(checkbox_text.bounds.width, 76.0);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 4,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text { ref content, .. } if content == "✓"
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 4,
                })
                .unwrap()
                .bounds
                .width,
            21.5
        );
    }

    #[test]
    fn scrollbar_chrome_paints_ordinary_quads_over_the_scrollport() {
        let mut scroller = node(1, None, &[]);
        scroller.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 120.0,
        };
        {
            let layout = Arc::make_mut(&mut scroller.source_style.layout);
            layout.overflow_x = nana_ui_core::OverflowSpec::Scroll;
            layout.overflow_y = nana_ui_core::OverflowSpec::Scroll;
        }
        scroller.standard_visual = Some(StandardVisual::Scrollbar {
            axes: nana_ui_runtime::ScrollAxes::Vertical,
            visibility: nana_ui_core::ScrollbarVisibility::Always,
            revealed: true,
            dragging: None,
        });
        scroller.component_geometry = Some(ComponentGeometry::Scrollbar {
            horizontal: None,
            vertical: Some(nana_ui_runtime::ScrollbarBar {
                track: LayoutBox {
                    x: 188.0,
                    y: 0.0,
                    width: 12.0,
                    height: 120.0,
                },
                thumb: LayoutBox {
                    x: 191.0,
                    y: 0.0,
                    width: 6.0,
                    height: 72.0,
                },
                track_background: Some([0.1, 0.1, 0.1, 1.0]),
                thumb_background: [0.6, 0.6, 0.6, 1.0],
                thumb_radius: 3.0,
                max_offset: 80.0,
            }),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([scroller], []);
        let track = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3,
            })
            .expect("resident track");
        let thumb = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 4,
            })
            .expect("thumb");
        assert!(matches!(
            track.kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.1, 0.1, 0.1, 1.0]),
                ..
            }
        ));
        assert!(matches!(
            thumb.kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.6, 0.6, 0.6, 1.0]),
                corner_radius,
                ..
            } if corner_radius.iter().all(|r| (*r - 3.0).abs() < f32::EPSILON)
        ));
        assert_eq!(thumb.bounds.x, 191.0);
        assert_eq!(thumb.bounds.height, 72.0);
        // Both axes clip, so chrome shares the scrollport overflow clip and is
        // not cut by a tighter content clip.
        assert_eq!(
            track.clips.as_ref(),
            [ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 120.0,
                },
                transform: AffineTransform::default(),
                corner_radius: 0.0,
                polygon_clip: None,
            }]
        );
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 5,
                })
                .is_none(),
            "a vertical-only container emits no horizontal bar"
        );
    }

    #[test]
    fn scrollbar_skin_thickness_still_paints_ordinary_quads() {
        let mut scroller = node(1, None, &[]);
        scroller.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 120.0,
        };
        scroller.standard_visual = Some(StandardVisual::Scrollbar {
            axes: nana_ui_runtime::ScrollAxes::Vertical,
            visibility: nana_ui_core::ScrollbarVisibility::Always,
            revealed: true,
            dragging: None,
        });
        scroller.component_geometry = Some(ComponentGeometry::Scrollbar {
            horizontal: None,
            vertical: Some(nana_ui_runtime::ScrollbarBar {
                track: LayoutBox {
                    x: 192.0,
                    y: 0.0,
                    width: 8.0,
                    height: 120.0,
                },
                thumb: LayoutBox {
                    x: 194.0,
                    y: 8.0,
                    width: 4.0,
                    height: 48.0,
                },
                track_background: Some([0.2, 0.2, 0.2, 1.0]),
                thumb_background: [1.0, 0.0, 0.0, 1.0],
                thumb_radius: 2.0,
                max_offset: 80.0,
            }),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([scroller], []);
        let track = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3,
            })
            .expect("skinned track");
        let thumb = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 4,
            })
            .expect("skinned thumb");
        assert!(matches!(
            track.kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.2, 0.2, 0.2, 1.0]),
                ..
            }
        ));
        assert_eq!(track.bounds.width, 8.0);
        assert!(matches!(
            thumb.kind,
            ScenePrimitiveKind::Quad {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                corner_radius,
                ..
            } if corner_radius.iter().all(|r| (*r - 2.0).abs() < f32::EPSILON)
        ));
        assert_eq!(thumb.bounds.width, 4.0);
        assert_eq!(thumb.bounds.x, 194.0);
    }

    #[test]
    fn focused_icon_does_not_paint_an_external_ring() {
        let mut icon = node(1, None, &[]);
        icon.layout = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 28.0,
            height: 28.0,
        };
        icon.focused = true;
        icon.standard_visual = Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Settings,
            size: 16.0,
            tooltip: None,
        });
        style_mut(&mut icon).border_color = Some([0.2, 0.6, 1.0, 1.0]);
        style_mut(&mut icon).background = Some([0.2, 0.2, 0.2, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([icon], []);
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 3
                })
                .is_some()
        );
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 7
                })
                .is_none()
        );
    }

    #[test]
    fn compact_leading_icon_centers_on_the_parent_text_line() {
        let mut row = node(1, None, &[2]);
        row.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 28.0,
        };
        row.text = Some(TextContent {
            value: "舞台".into(),
        });
        row.standard_visual = Some(StandardVisual::ListItem {
            leading: Some(id(2)),
            content: None,
            trailing: None,
        });
        row.source_style.text_vertical_alignment = TextVerticalAlignment::Center;
        style_mut(&mut row).font_size = 12.0;
        style_mut(&mut row).line_height = Some(LineHeightSpec::Absolute(12.0));

        let mut icon = node(2, Some(1), &[]);
        icon.layout = LayoutBox {
            x: 8.0,
            y: 0.0,
            width: 12.0,
            height: 12.0,
        };
        icon.standard_visual = Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Workspace,
            size: 12.0,
            tooltip: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([row, icon], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 3,
            })
            .expect("leading icon");
        assert_eq!(
            primitive.bounds,
            SceneRect {
                x: 8.0,
                y: 8.0,
                width: 12.0,
                height: 12.0,
            }
        );
    }

    #[test]
    fn an_icon_trigger_paints_a_centered_glyph_instead_of_label_text() {
        let mut menu = node(1, None, &[]);
        menu.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 28.0,
            height: 28.0,
        };
        menu.standard_visual = Some(StandardVisual::MenuSurface {
            open: false,
            kind: nana_ui_runtime::MenuSurfaceKind::ActionMenu,
            trigger: None,
            trigger_icon: Some(nana_ui_core::Icon::Add),
            gap: 0.0,
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        });
        menu.component_geometry = Some(ComponentGeometry::MenuSurface {
            trigger_surface: Some(nana_ui_runtime::ComponentTriggerSurface {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 28.0,
                    height: 28.0,
                },
                background: Some([0.18, 0.18, 0.2, 1.0]),
                border: Some([0.4, 0.4, 0.45, 1.0]),
            }),
            trigger: None,
            trigger_icon: Some((
                nana_ui_core::Icon::Add,
                LayoutBox {
                    x: 7.5,
                    y: 7.5,
                    width: 13.0,
                    height: 13.0,
                },
            )),
            surface: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            search: None,
            search_field: None,
            options: Vec::new(),
            elevation: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.0],
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 0.0,
                spread_radius: 0.0,
                inset: false,
            },
            background: [0.1, 0.1, 0.1, 1.0],
            border: [0.3, 0.3, 0.3, 1.0],
        });

        let mut scene = UiScene::new();
        scene.apply_delta([menu], []);
        let glyph = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .expect("icon trigger glyph");
        assert_eq!(
            glyph.bounds,
            SceneRect {
                x: 7.5,
                y: 7.5,
                width: 13.0,
                height: 13.0,
            }
        );
        assert_eq!(primitive_icon(&glyph.kind), Some(nana_ui_core::Icon::Add));
    }

    #[test]
    fn migrated_components_consume_runtime_subregion_geometry() {
        let mut icon = node(1, None, &[]);
        icon.layout = LayoutBox {
            x: 10.0,
            y: 20.0,
            width: 32.0,
            height: 32.0,
        };
        icon.standard_visual = Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Search,
            size: 16.0,
            tooltip: None,
        });
        style_mut(&mut icon).background = Some([0.2, 0.3, 0.4, 1.0]);
        style_mut(&mut icon).border_color = Some([0.4, 0.5, 0.6, 1.0]);
        style_mut(&mut icon).color = Some([0.9, 0.9, 0.9, 1.0]);
        icon.standard_visual_foreground = Some([0.1, 0.6, 0.9, 1.0]);

        let mut switch = node(2, None, &[]);
        switch.standard_visual = Some(StandardVisual::Switch {
            label: Arc::from("Enabled"),
            hint: Some(Arc::from("Starts with the workspace")),
            checked: true,
            control_position: nana_ui_core::SwitchControlPosition::End,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        switch.component_geometry = Some(ComponentGeometry::Switch {
            label: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 8.0,
                    width: 100.0,
                    height: 18.0,
                },
                content: Arc::from("Enabled"),
                color: Some([0.9, 0.9, 0.9, 1.0]),
                font_size: 13.0,
                font_weight: Some(500),
            },
            hint: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 28.0,
                    width: 150.0,
                    height: 16.0,
                },
                content: Arc::from("Starts with the workspace"),
                color: Some([0.6, 0.6, 0.6, 1.0]),
                font_size: 12.0,
                font_weight: Some(400),
            }),
            control: LayoutBox {
                x: 170.0,
                y: 18.0,
                width: 30.0,
                height: 16.0,
            },
            track_background: [0.2, 0.5, 0.9, 1.0],
            track_border: [0.1, 0.4, 0.8, 1.0],
            thumb_background: [1.0, 1.0, 1.0, 1.0],
        });
        switch.layout.width = 200.0;
        switch.layout.height = 52.0;
        switch.focused = true;

        let mut range = node(3, None, &[]);
        range.standard_visual = Some(StandardVisual::Range {
            label: Some(Arc::from("Opacity")),
            value: Arc::from("25"),
            unit: Some(Arc::from("%")),
            size: nana_ui_core::ControlSize::Medium,
            ratio: 0.25,
            invalid: false,
        });
        range.component_geometry = Some(ComponentGeometry::Range {
            label: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 11.0,
                    width: 52.0,
                    height: 18.0,
                },
                content: Arc::from("Opacity"),
                color: None,
                font_size: 13.0,
                font_weight: Some(500),
            }),
            value: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 210.0,
                    y: 11.0,
                    width: 20.0,
                    height: 18.0,
                },
                content: Arc::from("25"),
                color: None,
                font_size: 13.0,
                font_weight: None,
            },
            unit: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 232.0,
                    y: 11.0,
                    width: 8.0,
                    height: 18.0,
                },
                content: Arc::from("%"),
                color: None,
                font_size: 13.0,
                font_weight: None,
            }),
            track: LayoutBox {
                x: 80.0,
                y: 12.0,
                width: 120.0,
                height: 16.0,
            },
        });
        range.layout.width = 240.0;
        range.layout.height = 40.0;
        style_mut(&mut range).background = Some([0.2, 0.5, 0.9, 1.0]);
        style_mut(&mut range).border_color = Some([0.4, 0.4, 0.4, 1.0]);

        let mut card = node(4, None, &[]);
        card.standard_visual = Some(StandardVisual::Card {
            title: Some(Arc::from("Actions")),
            kind: nana_ui_core::CardKind::Surface,
            loading: true,
            loading_phase: 0.5,
        });
        style_mut(&mut card).background = Some([0.12, 0.12, 0.12, 1.0]);
        style_mut(&mut card).border_color = Some([0.3, 0.3, 0.3, 1.0]);
        card.component_geometry = Some(ComponentGeometry::Card {
            title: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 10.0,
                    y: 8.0,
                    width: 50.0,
                    height: 18.0,
                },
                content: Arc::from("Actions"),
                color: None,
                font_size: 13.0,
                font_weight: Some(600),
            }),
            content: LayoutBox {
                x: 10.0,
                y: 36.0,
                width: 80.0,
                height: 34.0,
            },
            elevation: Some(ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.25],
                offset_x: 0.0,
                offset_y: 3.0,
                blur_radius: 8.0,
                spread_radius: 0.0,
                inset: false,
            }),
            spinner: Some(LayoutBox {
                x: 68.0,
                y: 10.0,
                width: 14.0,
                height: 14.0,
            }),
        });

        let mut list_item = node(5, None, &[]);
        list_item.text = Some(TextContent {
            value: "Project".into(),
        });
        list_item.standard_visual = Some(StandardVisual::ListItem {
            leading: None,
            content: None,
            trailing: None,
        });
        list_item.component_geometry = Some(ComponentGeometry::ListItem {
            leading: None,
            content: Some(LayoutBox {
                x: 30.0,
                y: 6.0,
                width: 55.0,
                height: 22.0,
            }),
            trailing: None,
        });
        style_mut(&mut list_item).background = Some([0.15, 0.15, 0.15, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([icon, switch, range, card, list_item], []);

        let icon = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 3,
            })
            .unwrap();
        assert_eq!(
            icon.bounds,
            SceneRect {
                x: 18.0,
                y: 28.0,
                width: 16.0,
                height: 16.0,
            }
        );
        assert_eq!(primitive_icon(&icon.kind), Some(nana_ui_core::Icon::Search));
        match &icon.kind {
            ScenePrimitiveKind::Icon {
                color: Some(color), ..
            } => assert_eq!(*color, [0.1, 0.6, 0.9, 1.0]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 0,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                background: Some([0.2, 0.3, 0.4, 1.0]),
                border_color: Some([0.4, 0.5, 0.6, 1.0]),
                ..
            }
        ));

        let switch_track = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 4,
            })
            .unwrap();
        assert_eq!(switch_track.bounds.width, 30.0);
        assert_eq!(switch_track.bounds.height, 16.0);
        assert_eq!(switch_track.bounds.x, 170.0);
        let switch_thumb = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 5,
            })
            .unwrap();
        assert_eq!(switch_thumb.bounds.width, 10.0);
        assert_eq!(switch_thumb.bounds.x, 187.0);
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2,
                })
                .unwrap()
                .bounds
                .height,
            18.0
        );
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 3,
                })
                .unwrap()
                .bounds
                .y,
            28.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 7,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                border_width: 2.0,
                ..
            }
        ));

        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 4,
                })
                .unwrap()
                .bounds
                .width,
            30.0
        );
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 5,
                })
                .unwrap()
                .bounds
                .x,
            103.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(3),
                    slot: 6,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                horizontal_alignment: TextHorizontalAlignment::End,
                ..
            }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 3,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Spinner { phase: 4, .. }
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 0,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Quad {
                shadow: Some(ComponentElevation {
                    offset_x: 0.0,
                    offset_y: 3.0,
                    blur_radius: 8.0,
                    ..
                }),
                ..
            }
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2,
                })
                .unwrap()
                .bounds
                .y,
            8.0
        );
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(4),
                    slot: 2,
                })
                .unwrap()
                .kind,
            ScenePrimitiveKind::Text {
                ellipsis: true,
                ..
            }
        ));
        assert_eq!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 2,
                })
                .unwrap()
                .bounds,
            SceneRect {
                x: 30.0,
                y: 6.0,
                width: 55.0,
                height: 22.0,
            }
        );
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(5),
                    slot: 0,
                })
                .is_some()
        );
    }

    #[test]
    fn subtree_membership_follows_retained_parent_links() {
        let scene = {
            let mut scene = UiScene::new();
            scene.apply_delta(
                [
                    node(1, None, &[2]),
                    node(2, Some(1), &[3]),
                    node(3, Some(2), &[]),
                ],
                [],
            );
            scene
        };
        assert!(scene.is_node_in_subtree(id(1), id(3)));
        assert!(scene.is_node_in_subtree(id(2), id(2)));
        assert!(!scene.is_node_in_subtree(id(3), id(1)));
    }

    #[test]
    fn scroll_offset_transforms_descendants_but_not_viewport_clip() {
        let mut scroller = node(1, None, &[2]);
        scroller.layout.height = 50.0;
        scroller.scroll_offset = nana_ui_runtime::ScrollOffset { x: 0.0, y: 60.0 };
        scroller.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                overflow_y: nana_ui_core::OverflowSpec::Scroll,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout.y = 80.0;
        child.text = Some(TextContent {
            value: "Visible".into(),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([scroller, child], []);
        let text = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .unwrap();
        assert_eq!(text.transform.0[5], -60.0);
        assert_eq!(text.bounds.y, 80.0);
        assert_eq!(text.clips.len(), 1);
        assert_eq!(text.clips[0].bounds.height, 50.0);
        assert_eq!(text.clips[0].transform, AffineTransform::IDENTITY);
    }

    #[test]
    fn scroll_offset_delta_rebuilds_descendant_primitives_without_reextracting_them() {
        let mut scroller = node(1, None, &[2]);
        scroller.layout.height = 50.0;
        scroller.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                overflow_y: nana_ui_core::OverflowSpec::Scroll,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout.y = 80.0;
        child.text = Some(TextContent {
            value: "Visible".into(),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([scroller.clone(), child], []);
        scroller.scroll_offset = nana_ui_runtime::ScrollOffset { x: 0.0, y: 60.0 };
        let delta = scene.apply_delta([scroller], []);
        assert_eq!(delta.updated_nodes, 1);
        let text = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .unwrap();
        assert_eq!(
            text.transform.0[5], -60.0,
            "extracting only the scroller must recompose descendant paint transforms"
        );
        assert_eq!(text.bounds.y, 80.0);
    }

    #[test]
    fn graph_canvas_high_slots_stay_in_paint_order_across_incremental_updates() {
        let edge = vec![[8.0, 20.0], [48.0, 22.0]];
        let geometry = |edges: Vec<(Vec<[f32; 2]>, [f32; 4])>| ComponentGeometry::GraphCanvas {
            nodes: Vec::new(),
            separators: Vec::new(),
            ports: Vec::new(),
            port_labels: Vec::new(),
            edges,
            edge_labels: Vec::new(),
            grid: Vec::new(),
            background: [0.1, 0.1, 0.1, 1.0],
            grid_color: [0.2, 0.2, 0.2, 0.5],
            separator_color: [0.2, 0.2, 0.2, 1.0],
        };
        let mut canvas = node(1, None, &[]);
        canvas.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 120.0,
        };
        canvas.standard_visual = Some(StandardVisual::GraphCanvas {
            nodes: Arc::from([]),
            ports: Arc::from([]),
            edges: Arc::from([]),
            connecting: None,
            grid_spacing: 24.0,
            viewport_offset_x: 0.0,
            viewport_offset_y: 0.0,
            viewport_zoom: 1.0,
        });
        canvas.component_geometry = Some(geometry(vec![(edge.clone(), [0.5, 0.5, 0.5, 1.0])]));

        let mut scene = UiScene::new();
        scene.apply_delta([canvas.clone()], []);
        assert!(scene.primitives().any(|primitive| primitive.id.slot == 12));
        assert!(!scene.primitives().any(|primitive| primitive.id.slot == 13));

        canvas.component_geometry = Some(geometry(vec![
            (edge.clone(), [0.5, 0.5, 0.5, 1.0]),
            (edge.clone(), [0.2, 0.6, 1.0, 1.0]),
        ]));
        scene.apply_delta([canvas.clone()], []);
        assert!(
            scene.primitives().any(|primitive| primitive.id.slot == 12),
            "base edge batch must remain in paint order"
        );
        assert!(
            scene.primitives().any(|primitive| primitive.id.slot == 13),
            "selected/connecting overlay must enter paint order on the next extract"
        );

        canvas.component_geometry = Some(geometry(vec![(edge, [0.5, 0.5, 0.5, 1.0])]));
        scene.apply_delta([canvas], []);
        assert!(scene.primitives().any(|primitive| primitive.id.slot == 12));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 13
                })
                .is_none(),
            "unused high slots must be removed instead of leaving a stale overlay"
        );
    }

    #[test]
    fn new_component_geometry_paints_owned_quads_and_skips_generic_text() {
        let mut chart = node(1, None, &[]);
        chart.standard_visual = Some(StandardVisual::TimeSeriesChart {
            values: Arc::from([0.0, 1.0]),
        });
        chart.component_geometry = Some(ComponentGeometry::TimeSeriesChart {
            grid: vec![LayoutBox {
                x: 8.0,
                y: 10.0,
                width: 92.0,
                height: 1.0,
            }],
            area: vec![LayoutBox {
                x: 8.0,
                y: 40.0,
                width: 2.0,
                height: 70.0,
            }],
            line: vec![[8.0, 40.0], [54.0, 40.0]],
            grid_color: [0.2, 0.2, 0.2, 0.55],
            area_color: [0.3, 0.5, 0.8, 0.16],
            line_color: [0.3, 0.5, 0.9, 1.0],
        });

        let mut markdown = node(2, None, &[]);
        markdown.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 48.0,
        };
        markdown.text = Some(TextContent {
            value: "hello".into(),
        });
        markdown.standard_visual = Some(StandardVisual::NativeMarkdown {
            text: Arc::from("hello"),
            selection: Some((0, 5)),
        });
        markdown.component_geometry = Some(ComponentGeometry::NativeMarkdown {
            text: ComponentTextRegion {
                bounds: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 120.0,
                    height: 48.0,
                },
                content: Arc::from("hello"),
                color: Some([1.0, 1.0, 1.0, 1.0]),
                font_size: 13.0,
                font_weight: None,
            },
            selection: vec![LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 48.0,
            }],
            selection_color: [0.2, 0.4, 0.8, 0.14],
        });

        let mut scene = UiScene::new();
        scene.apply_delta([chart, markdown], []);

        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 10
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::QuadBatch { .. })
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 11
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::QuadBatch { .. })
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 12
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::Stroke {
                width,
                points,
                widths,
                cap: StrokeCap::Round,
                pattern: None,
                ..
            }) if (*width - TimeSeriesChart::LINE_WIDTH).abs() < f32::EPSILON
                && points.len() == 2
                && widths.is_empty()
        ));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 2
                })
                .is_none(),
            "time series does not emit generic text"
        );

        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 1
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::QuadBatch {
                background: Some([0.2, 0.4, 0.8, 0.14]),
                ..
            })
        ));
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::Text {
                content,
                wrap: true,
                ..
            }) if content == "hello"
        ));
        let text_primitives = scene
            .primitives()
            .filter(|primitive| {
                primitive.node == id(2) && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
            })
            .count();
        assert_eq!(
            text_primitives, 1,
            "markdown must not double-paint generic text"
        );
    }

    fn visible_text_count(scene: &UiScene, nodes: &[StableNodeId]) -> usize {
        scene
            .primitives()
            .filter(|primitive| {
                nodes.contains(&primitive.node)
                    && matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Text { content, .. } if !content.trim().is_empty()
                    )
            })
            .count()
    }

    fn text_node(id: u64, parent: u64, value: &str) -> ExtractedNode {
        let mut child = node(id, Some(parent), &[]);
        child.kind = Arc::new(NodeKind::Text);
        child.text = Some(TextContent {
            value: value.into(),
        });
        child
    }

    #[test]
    fn host_and_child_text_extract_one_visible_text_primitive() {
        let mut button = node(1, None, &[2]);
        button.kind = Arc::new(NodeKind::Element {
            tag: "button".into(),
        });
        button.text = Some(TextContent {
            value: "Open".into(),
        });
        let label = ComponentTextRegion {
            bounds: LayoutBox {
                x: 8.0,
                y: 8.0,
                width: 48.0,
                height: 20.0,
            },
            content: Arc::from("Open"),
            color: Some([0.1, 0.1, 0.1, 1.0]),
            font_size: 13.0,
            font_weight: Some(500),
        };
        button.standard_visual = Some(StandardVisual::Button {
            label: Arc::from("Open"),
            kind: nana_ui_core::ButtonKind::Ghost,
            size: nana_ui_core::ControlSize::Medium,
            loading: false,
            loading_phase: 0.0,
            invalid: false,
        });
        button.component_geometry = Some(ComponentGeometry::Button {
            label,
            spinner: None,
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([button, text_node(2, 1, "Open")], []);
        assert_eq!(visible_text_count(&scene, &[id(1), id(2)]), 1);

        let mut heading = node(3, None, &[4]);
        heading.kind = Arc::new(NodeKind::Element { tag: "h1".into() });
        heading.text = Some(TextContent {
            value: "Title".into(),
        });
        scene.apply_delta([heading, text_node(4, 3, "Title")], []);
        assert_eq!(visible_text_count(&scene, &[id(3), id(4)]), 1);
    }

    #[test]
    fn card_child_list_item_keeps_its_label() {
        let mut card = node(1, None, &[2]);
        card.text = Some(TextContent {
            value: "Outputs".into(),
        });
        card.standard_visual = Some(StandardVisual::Card {
            title: Some(Arc::from("Outputs")),
            kind: nana_ui_core::CardKind::Surface,
            loading: false,
            loading_phase: 0.0,
        });
        card.component_geometry = Some(ComponentGeometry::Card {
            title: Some(ComponentTextRegion {
                bounds: LayoutBox {
                    x: 10.0,
                    y: 8.0,
                    width: 80.0,
                    height: 18.0,
                },
                content: Arc::from("Outputs"),
                color: None,
                font_size: 13.0,
                font_weight: Some(600),
            }),
            content: LayoutBox {
                x: 10.0,
                y: 36.0,
                width: 160.0,
                height: 36.0,
            },
            elevation: None,
            spinner: None,
        });

        let mut item = node(2, Some(1), &[]);
        item.kind = Arc::new(NodeKind::Element {
            tag: "list-item".into(),
        });
        item.text = Some(TextContent {
            value: "Window".into(),
        });
        item.standard_visual = Some(StandardVisual::ListItem {
            leading: None,
            content: None,
            trailing: None,
        });
        item.component_geometry = Some(ComponentGeometry::ListItem {
            leading: None,
            content: Some(LayoutBox {
                x: 10.0,
                y: 36.0,
                width: 160.0,
                height: 36.0,
            }),
            trailing: None,
        });

        let mut scene = UiScene::new();
        scene.apply_delta([card, item], []);
        assert_eq!(visible_text_count(&scene, &[id(1), id(2)]), 2);
        assert!(matches!(
            scene
                .primitive(PrimitiveId {
                    node: id(2),
                    slot: 2,
                })
                .map(|primitive| &primitive.kind),
            Some(ScenePrimitiveKind::Text { content, .. }) if content == "Window"
        ));
    }

    #[test]
    fn css_gradient_and_clip_path_surface_paint_travels_on_quad() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 1.0, 1.0, 0.5]),
                paint: nana_ui_core::PaintStyle {
                    background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                        nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                            angle_deg: 180.0,
                            stops: vec![
                                nana_ui_core::GradientStop {
                                    position: 0.0,
                                    color: [1.0, 1.0, 1.0, 1.0],
                                },
                                nana_ui_core::GradientStop {
                                    position: 1.0,
                                    color: [1.0, 1.0, 1.0, 0.0],
                                },
                            ],
                        }),
                    )),
                    clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                        top: nana_ui_core::LengthSpec::Percent(50.0),
                        right: nana_ui_core::LengthSpec::Percent(50.0),
                        bottom: nana_ui_core::LengthSpec::Percent(50.0),
                        left: nana_ui_core::LengthSpec::Percent(50.0),
                        round: None,
                    })),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 0.5]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        assert_eq!(
            primitive.clips.len(),
            1,
            "inset clip-path adds a clip region"
        );
        let clip = &primitive.clips[0];
        assert!((clip.bounds.width - 0.0).abs() < 0.01);
        assert!((clip.bounds.height - 0.0).abs() < 0.01);
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(surface.background_image.is_some());
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn css_clip_path_inset_round_applies_surface_corner_radius() {
        let mut painted = node(1, None, &[]);
        painted.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                        top: nana_ui_core::LengthSpec::Px(10.0),
                        right: nana_ui_core::LengthSpec::Px(10.0),
                        bottom: nana_ui_core::LengthSpec::Px(10.0),
                        left: nana_ui_core::LengthSpec::Px(10.0),
                        round: Some(nana_ui_core::LengthSpec::Px(8.0)),
                    })),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        assert!(
            (primitive.clips[0].corner_radius - 8.0).abs() < f32::EPSILON,
            "inset round travels on clip region"
        );
        match &primitive.kind {
            ScenePrimitiveKind::Quad { corner_radius, .. } => {
                assert!(
                    corner_radius
                        .iter()
                        .all(|r| (*r - 8.0).abs() < f32::EPSILON),
                    "inset round applies to owning quad radii, got {corner_radius:?}"
                );
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn css_clip_path_inset_clips_text_child() {
        let mut parent = node(1, None, &[2]);
        parent.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    clip_path: Some(nana_ui_core::ClipPath::Inset(nana_ui_core::ClipInset {
                        top: nana_ui_core::LengthSpec::Px(10.0),
                        right: nana_ui_core::LengthSpec::Px(10.0),
                        bottom: nana_ui_core::LengthSpec::Px(10.0),
                        left: nana_ui_core::LengthSpec::Px(10.0),
                        round: None,
                    })),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut child = node(2, Some(1), &[]);
        child.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        };
        child.text = Some(TextContent {
            value: "child".into(),
        });

        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let text = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 2,
            })
            .expect("text child");
        assert_eq!(text.clips.len(), 1);
        let clip = &text.clips[0];
        assert!((clip.bounds.x - 10.0).abs() < 0.01);
        assert!((clip.bounds.y - 10.0).abs() < 0.01);
        assert!((clip.bounds.width - 80.0).abs() < 0.01);
        assert!((clip.bounds.height - 60.0).abs() < 0.01);
    }

    #[test]
    fn css_filter_group_omits_leaf_shader_on_parent_quad() {
        let mut parent = node(1, None, &[2]);
        parent.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        };
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    filter: Some(nana_ui_core::ColorFilter {
                        brightness: 0.5,
                        saturate: 1.0,
                        contrast: 1.0,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut parent).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut child = node(2, Some(1), &[]);
        child.layout = LayoutBox {
            x: 0.0,
            y: 40.0,
            width: 100.0,
            height: 20.0,
        };

        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("parent quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(surface.filter.is_none(), "filter group owns parent filter");
            }
            other => panic!("expected quad, got {other:?}"),
        }
        assert_eq!(scene.filter_groups(id(2)).len(), 1);
        assert_eq!(
            scene.opacity_groups(id(2)),
            vec![OpacityGroup {
                node: id(1),
                opacity: 1.0,
                filter: nana_ui_core::ColorFilter {
                    brightness: 0.5,
                    saturate: 1.0,
                    contrast: 1.0,
                    ..Default::default()
                },
                mix_blend: MixBlendMode::Normal,
                inset_shadow: None,
            }]
        );
    }

    #[test]
    fn css_mix_blend_and_element_blur_isolate_dest_groups() {
        let mut blended = node(1, None, &[]);
        blended.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    mix_blend: MixBlendMode::Multiply,
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut blended).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([blended], []);
        assert_eq!(
            scene.opacity_groups(id(1)),
            vec![OpacityGroup {
                node: id(1),
                opacity: 1.0,
                filter: ColorFilter::default(),
                mix_blend: MixBlendMode::Multiply,
                inset_shadow: None,
            }]
        );

        let mut blurred = node(2, None, &[]);
        blurred.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.0, 1.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    filter: Some(nana_ui_core::ColorFilter {
                        blur_radius: 8.0,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut blurred).background = Some([0.0, 1.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([blurred], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(2),
                slot: 0,
            })
            .expect("blur quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(surface.filter.is_none(), "element blur is dest-group owned");
            }
            other => panic!("expected quad, got {other:?}"),
        }
        assert_eq!(scene.filter_groups(id(2)).len(), 1);
        assert_eq!(scene.opacity_groups(id(2))[0].filter.blur_radius, 8.0);
    }

    #[test]
    fn css_drop_shadow_isolates_dest_group_not_box_shadow() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    filter: Some(nana_ui_core::ColorFilter {
                        drop_shadow: Some(nana_ui_core::FilterDropShadow {
                            offset_x: 4.0,
                            offset_y: 6.0,
                            blur_radius: 8.0,
                            color: [0.0, 0.0, 0.0, 0.5],
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("drop-shadow quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad {
                surface, shadow, ..
            } => {
                assert!(
                    surface.filter.is_none(),
                    "drop-shadow is dest-group owned, not leaf shader"
                );
                assert!(
                    shadow.is_none(),
                    "drop-shadow must not reuse box-shadow quads"
                );
            }
            other => panic!("expected quad, got {other:?}"),
        }
        assert_eq!(scene.filter_groups(id(1)).len(), 1);
        let group = &scene.opacity_groups(id(1))[0];
        let shadow = group.filter.drop_shadow.expect("dest-group drop-shadow");
        assert!((shadow.offset_x - 4.0).abs() < 0.01);
        assert!((shadow.offset_y - 6.0).abs() < 0.01);
        assert!((shadow.blur_radius - 8.0).abs() < 0.01);
    }

    #[test]
    fn css_box_shadow_layers_outline_and_line_clamp_travel() {
        let mut painted = node(1, None, &[]);
        painted.text = Some(TextContent {
            value: "clamped".into(),
        });
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 1.0, 1.0, 1.0]),
                line_clamp: Some(2),
                text_overflow_ellipsis: true,
                paint: nana_ui_core::PaintStyle {
                    box_shadows: vec![
                        nana_ui_core::BoxShadowSpec {
                            offset_x: 2.0,
                            offset_y: 2.0,
                            blur_radius: 4.0,
                            spread_radius: 0.0,
                            color: [0.0, 0.0, 0.0, 1.0],
                            inset: true,
                        },
                        nana_ui_core::BoxShadowSpec {
                            offset_x: 0.0,
                            offset_y: 4.0,
                            blur_radius: 8.0,
                            spread_radius: 0.0,
                            color: [0.0, 0.0, 0.0, 0.5],
                            inset: false,
                        },
                    ],
                    outline: nana_ui_core::OutlineSpec {
                        width: 2.0,
                        color: Some([1.0, 0.0, 0.0, 1.0]),
                        style: nana_ui_core::OutlineStyle::Solid,
                    },
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("shadow quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad {
                shadow, surface, ..
            } => {
                assert!(shadow.is_some_and(|s| s.inset));
                assert_eq!(surface.extra_shadows.len(), 1);
                assert!(!surface.extra_shadows[0].inset);
                assert!((surface.outline_width - 2.0).abs() < f32::EPSILON);
            }
            other => panic!("expected quad, got {other:?}"),
        }
        let text = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 2,
            })
            .expect("text");
        match &text.kind {
            ScenePrimitiveKind::Text {
                max_lines,
                ellipsis,
                wrap,
                ..
            } => {
                assert_eq!(*max_lines, Some(2));
                assert!(*ellipsis);
                assert!(*wrap);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn inset_box_shadow_on_a_leaf_is_not_an_outset_elevation() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 1.0, 1.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    box_shadows: vec![nana_ui_core::BoxShadowSpec {
                        offset_x: 2.0,
                        offset_y: 4.0,
                        blur_radius: 6.0,
                        spread_radius: 1.0,
                        color: [0.0, 0.0, 0.0, 0.5],
                        inset: true,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("leaf quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad {
                shadow: Some(elevation),
                ..
            } => {
                assert!(
                    elevation.inset,
                    "leaf inset must travel as inset, not an outset drop shadow"
                );
                assert!((elevation.offset_y - 4.0).abs() < f32::EPSILON);
            }
            other => panic!("expected inset shadow quad, got {other:?}"),
        }
        assert!(
            scene.opacity_groups(id(1)).is_empty(),
            "a leaf inset does not open a dest group"
        );
    }

    #[test]
    fn inset_box_shadow_with_children_is_a_dest_group_not_parent_quad() {
        let mut parent = node(1, None, &[2]);
        parent.layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        };
        parent.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    box_shadows: vec![nana_ui_core::BoxShadowSpec {
                        offset_x: 0.0,
                        offset_y: 2.0,
                        blur_radius: 4.0,
                        spread_radius: 0.0,
                        color: [0.0, 0.0, 0.0, 0.4],
                        inset: true,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut parent).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut child = node(2, Some(1), &[]);
        child.layout = LayoutBox {
            x: 4.0,
            y: 4.0,
            width: 20.0,
            height: 20.0,
        };
        style_mut(&mut child).background = Some([0.0, 1.0, 0.0, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("parent quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad {
                shadow: Some(elevation),
                ..
            } => {
                assert!(
                    elevation.inset,
                    "origin paints inset on the parent quad (PAINT_SHADOW_INSET)"
                );
                assert!((elevation.offset_y - 2.0).abs() < f32::EPSILON);
            }
            other => panic!("expected inset shadow quad, got {other:?}"),
        }
        assert!(
            scene.opacity_groups(id(2)).is_empty(),
            "inset-only parents are not dest groups; origin opacity stacking is filter/opacity/mix-blend"
        );
    }

    #[test]
    fn css_mask_and_gradient_both_travel_on_quad() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([0.2, 0.2, 0.2, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                        nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                            angle_deg: 90.0,
                            stops: vec![nana_ui_core::GradientStop {
                                position: 0.0,
                                color: [1.0, 0.0, 0.0, 1.0],
                            }],
                        }),
                    )),
                    mask: Some(nana_ui_core::MaskImage::Gradient(
                        nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                            angle_deg: 180.0,
                            stops: vec![nana_ui_core::GradientStop {
                                position: 0.0,
                                color: [0.0, 0.0, 0.0, 1.0],
                            }],
                        }),
                    )),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([0.2, 0.2, 0.2, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(surface.background_image.is_some());
                assert!(surface.mask.is_some());
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn css_backdrop_filter_travels_on_quad_surface() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 1.0, 1.0, 0.4]),
                paint: nana_ui_core::PaintStyle {
                    backdrop_filter: Some(nana_ui_core::BackdropFilter {
                        blur_radius: 16.0,
                        saturate: 1.2,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 1.0, 1.0, 0.4]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                let backdrop = surface
                    .backdrop_filter
                    .expect("backdrop-filter must travel to scene quad");
                assert!((backdrop.blur_radius - 16.0).abs() < 0.01);
                assert!((backdrop.saturate - 1.2).abs() < 0.01);
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn img_content_image_and_two_background_layers_travel_on_quad() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    background_image: Some(nana_ui_core::BackgroundImage::url("fg.png")),
                    background_layers: vec![nana_ui_core::BackgroundImage::url("bg.png")],
                    content_image: Some(nana_ui_core::BackgroundImage::url_with_fit(
                        "photo.png",
                        nana_ui_core::BackgroundImageFit::Contain,
                    )),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert_eq!(
                    surface
                        .background_image
                        .as_ref()
                        .and_then(|image| image.url_str()),
                    Some("fg.png")
                );
                assert_eq!(surface.background_layers.len(), 1);
                assert_eq!(
                    surface
                        .content_image
                        .as_ref()
                        .and_then(|image| image.url_str()),
                    Some("photo.png")
                );
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn border_image_travels_on_quad() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    border_image: Some(nana_ui_core::BorderImageSpec {
                        source: nana_ui_core::BackgroundImage::url("frame.png"),
                        slice: [nana_ui_core::BorderImageSlice::Number(30.0); 4],
                        fill: true,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                let spec = surface.border_image.as_ref().expect("border-image");
                assert_eq!(spec.source.url_str(), Some("frame.png"));
                assert!(spec.fill);
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_border_image_does_not_travel_on_quad() {
        let mut painted = node(1, None, &[]);
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    unsupported_border_image: true,
                    border_image: Some(nana_ui_core::BorderImageSpec {
                        source: nana_ui_core::BackgroundImage::url("frame.png"),
                        slice: [nana_ui_core::BorderImageSlice::Number(30.0); 4],
                        fill: true,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([0.1, 0.1, 0.1, 1.0]);

        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &primitive.kind {
            ScenePrimitiveKind::Quad { surface, .. } => {
                assert!(
                    surface.border_image.is_none(),
                    "sticky unsupported must not project a 9-slice"
                );
            }
            other => panic!("expected quad, got {other:?}"),
        }
    }

    #[test]
    fn host_texture_custom_carries_css_mask() {
        let mut painted = node(1, None, &[]);
        painted.custom_render = Some(CustomRenderNode::new("nana.host-texture", "preview", 1));
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                paint: nana_ui_core::PaintStyle {
                    mask: Some(nana_ui_core::MaskImage::Gradient(
                        nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                            angle_deg: 180.0,
                            stops: vec![
                                nana_ui_core::GradientStop {
                                    position: 0.0,
                                    color: [1.0, 1.0, 1.0, 1.0],
                                },
                                nana_ui_core::GradientStop {
                                    position: 1.0,
                                    color: [1.0, 1.0, 1.0, 0.0],
                                },
                            ],
                        }),
                    )),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let primitive = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
            .expect("host texture custom primitive");
        match &primitive.kind {
            ScenePrimitiveKind::Custom { mask, node } => {
                assert_eq!(node.renderer.as_ref(), "nana.host-texture");
                assert!(mask.is_some(), "mask must travel on the Custom primitive");
            }
            other => panic!("expected custom, got {other:?}"),
        }
    }

    #[test]
    fn css_mask_url_travels_on_quad_and_host_texture() {
        let mut painted = node(1, None, &[]);
        painted.custom_render = Some(CustomRenderNode::new("nana.host-texture", "preview", 1));
        painted.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                paint: nana_ui_core::PaintStyle {
                    mask: Some(nana_ui_core::MaskImage::Url("fade.png".into())),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut painted).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([painted], []);
        let quad = scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 0,
            })
            .expect("surface quad");
        match &quad.kind {
            ScenePrimitiveKind::Quad { surface, .. } => match &surface.mask {
                Some(nana_ui_core::MaskImage::Url(url)) => assert_eq!(url, "fade.png"),
                other => panic!("expected url mask on quad, got {other:?}"),
            },
            other => panic!("expected quad, got {other:?}"),
        }
        let custom = scene
            .primitives()
            .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
            .expect("host texture custom");
        match &custom.kind {
            ScenePrimitiveKind::Custom { mask, .. } => match mask {
                Some(nana_ui_core::MaskImage::Url(url)) => assert_eq!(url, "fade.png"),
                other => panic!("expected url mask on custom, got {other:?}"),
            },
            other => panic!("expected custom, got {other:?}"),
        }
    }

    #[test]
    fn rasterized_svg_host_texture_skips_vector_children() {
        let mut svg = node(1, None, &[2]);
        svg.kind = Arc::new(NodeKind::Element { tag: "svg".into() });
        svg.custom_render = Some(CustomRenderNode::new("nana.host-texture", "svg:1", 1));
        let mut path = node(2, Some(1), &[]);
        path.kind = Arc::new(NodeKind::Element { tag: "path".into() });
        path.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut path).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([svg, path], []);
        assert!(
            scene.primitives().any(|primitive| matches!(
                primitive.kind,
                ScenePrimitiveKind::Custom { .. }
            ) && primitive.node == id(1)),
            "svg root still samples HostTexture"
        );
        assert!(
            scene.primitives().all(|primitive| primitive.node != id(2)),
            "path children of a rasterized svg must not paint as boxes"
        );
    }

    #[test]
    fn icon_visual_skips_vector_children() {
        let mut icon = node(1, None, &[2]);
        icon.standard_visual = Some(StandardVisual::Icon {
            icon: nana_ui_core::Icon::Search,
            size: 16.0,
            tooltip: None,
        });
        let mut path = node(2, Some(1), &[]);
        path.kind = Arc::new(NodeKind::Element { tag: "path".into() });
        path.source_style = NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                background: Some([1.0, 0.0, 0.0, 1.0]),
                ..Default::default()
            }),
            ..Default::default()
        };
        style_mut(&mut path).background = Some([1.0, 0.0, 0.0, 1.0]);
        let mut scene = UiScene::new();
        scene.apply_delta([icon, path], []);
        assert!(
            scene.primitives().any(|primitive| {
                primitive.node == id(1)
                    && matches!(
                        primitive.kind,
                        ScenePrimitiveKind::Icon { icon, .. }
                            if icon == nana_ui_core::Icon::Search
                    )
            }),
            "icon root still paints the atlas glyph"
        );
        assert!(
            scene.primitives().all(|primitive| primitive.node != id(2)),
            "path children of an Icon visual must not paint as boxes"
        );
    }

    #[test]
    fn completion_and_hover_overlays_paint_above_editor_layers() {
        let mut input = node(1, None, &[]);
        input.standard_visual = Some(StandardVisual::TextInput {
            placeholder: Arc::from(""),
            size: nana_ui_core::ControlSize::Medium,
            secure: false,
            invalid: false,
            steppers: false,
            diagnostics: Arc::from([]),
            matches: Arc::from([]),
            line_numbers: false,
            indent_guides: None,
            folds: Arc::from([]),
            editor_options: nana_ui_runtime::TextEditorRenderOptions::default(),
        });
        let row_rect = |index: usize| LayoutBox {
            x: 10.0,
            y: 20.0 + index as f32 * 14.0,
            width: 120.0,
            height: 14.0,
        };
        let text_region = |content: &str, bounds: LayoutBox| ComponentTextRegion {
            bounds,
            content: Arc::from(content),
            color: Some([0.9, 0.9, 0.9, 1.0]),
            font_size: 12.0,
            font_weight: None,
        };
        input.component_geometry = Some(ComponentGeometry::TextInput {
            multiline: true,
            text: text_region("fn", LayoutBox::default()),
            selection: Vec::new(),
            caret: None,
            additional_carets: Vec::new(),
            additional_caret_color: [0.0; 4],
            preedit: Vec::new(),
            diagnostic_markers: Vec::new(),
            match_markers: Vec::new(),
            caret_line: None,
            bracket_markers: Vec::new(),
            indent_guides: Vec::new(),
            line_labels: Vec::new(),
            line_labels_color: [0.0; 4],
            line_labels_font_size: 11.0,
            folds: nana_ui_runtime::TextFoldGeometry::default(),
            completion_popup: Some(nana_ui_runtime::TextCompletionPopup {
                panel: LayoutBox {
                    x: 8.0,
                    y: 18.0,
                    width: 120.0,
                    height: 32.0,
                },
                selected: 1,
                first_row: 0,
                rows: (0..2)
                    .map(|index| nana_ui_runtime::TextCompletionRow {
                        bounds: row_rect(index),
                        label: text_region("label", row_rect(index)),
                        detail: None,
                        kind: Some(text_region("fn", row_rect(index))),
                    })
                    .collect(),
                background: [0.1, 0.1, 0.1, 1.0],
                border: [0.3, 0.3, 0.3, 1.0],
                selected_background: [0.2, 0.2, 0.2, 1.0],
                label_color: [1.0; 4],
                detail_color: [0.5; 4],
                kind_color: [0.4; 4],
            }),
            hover_popup: Some(nana_ui_runtime::TextHoverPopup {
                panel: LayoutBox {
                    x: 8.0,
                    y: 80.0,
                    width: 120.0,
                    height: 28.0,
                },
                title: text_region("hover", row_rect(0)),
                body_rows: vec![text_region("body", row_rect(1))],
                background: [0.1, 0.1, 0.1, 1.0],
                border: [0.3, 0.3, 0.3, 1.0],
                title_color: [1.0; 4],
                body_color: [0.6; 4],
            }),
            background: None,
            border: None,
            border_width: 0.0,
            focus_ring: None,
            selection_color: [0.0; 4],
            caret_color: [0.0; 4],
            preedit_color: [0.0; 4],
            occurrence_markers: Vec::new(),
            whitespace_marks: Vec::new(),
            whitespace_color: [0.0; 4],
            wrap_guides: Vec::new(),
            steppers: None,
            minimap: None,
        });
        let mut scene = UiScene::new();
        scene.apply_delta([input], []);

        let kind = |slot: u8| {
            scene
                .primitive(PrimitiveId { node: id(1), slot })
                .map(|primitive| primitive.kind.clone())
                .expect("overlay primitive")
        };
        // 面板底 + 选中行高亮 + 行文本（label/kind 各一层；detail 为空
        // 的候选不产生文本层）。
        assert!(matches!(kind(90), ScenePrimitiveKind::Quad { .. }));
        assert!(matches!(kind(91), ScenePrimitiveKind::Quad { .. }));
        assert!(matches!(kind(92), ScenePrimitiveKind::Text { content, .. } if content == "label"));
        assert!(matches!(kind(93), ScenePrimitiveKind::Text { content, .. } if content == "label"));
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 100
                })
                .is_none()
        );
        assert!(matches!(kind(108), ScenePrimitiveKind::Text { content, .. } if content == "fn"));
        // hover 浮窗：面板 + 标题 + 正文。
        assert!(matches!(kind(120), ScenePrimitiveKind::Quad { .. }));
        assert!(
            matches!(kind(121), ScenePrimitiveKind::Text { content, .. } if content == "hover")
        );
        assert!(matches!(kind(122), ScenePrimitiveKind::Text { content, .. } if content == "body"));
        // 两行之外没有多余文本层。
        assert!(
            scene
                .primitive(PrimitiveId {
                    node: id(1),
                    slot: 94
                })
                .is_none()
        );
    }
}
