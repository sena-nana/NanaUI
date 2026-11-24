mod attributes;
mod composition;
mod compositor;
pub use attributes::SceneDraw;
use compositor::CompositorRegistry;
pub use compositor::{
    CompositorLayer, CompositorLayerId, CompositorLayerKind, CompositorMotionBinding,
    CompositorPaintEncode, LAYER_DEMOTE_HOLD, LAYER_PROMOTE_HOLD,
};
mod sticky;
mod visibility;
pub use composition::FramePlan;
use visibility::VisibilityIndex;
mod custom_paint;
pub use custom_paint::{PathMesh, PathVertex};
mod order;
mod placement;
use placement::{offset_rect, record_on_page};
mod primitives;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

#[cfg(test)]
use nana_ui_core::DrawerSide;
use nana_ui_core::{
    BackgroundImage, BorderImageSpec, ClipPath, ColorFilter, DirSpec, FontFeatureSetting,
    FontKerningSpec, FontVariationSetting, Icon, LineBreakSpec, LineHeightSpec, MixBlendMode,
    WordBreakSpec, WritingModeSpec, icon_y_on_text_glyph_center,
};
use nana_ui_runtime::scene_contract::{
    ComponentElevation, ComponentTextRegion, ExtractedMove, ExtractedNode,
};
use nana_ui_runtime::{
    CustomRenderNode, LayoutBox, NodeKind, NodeMap, NodeSet, StableNodeId, TextHorizontalAlignment,
    TextShaping, TextVerticalAlignment,
};

use crate::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};

mod ancestors;
mod audit;
mod component_text;
mod delta;
mod derived;
mod geometry;
mod groups;
mod kinds;
mod paint_order;
mod presentation;
mod retained;
mod slots;
mod text_effects;
mod text_primitives;
mod visual_quads;

use ancestors::*;
use audit::*;
use component_text::*;
use delta::*;
use derived::*;
use geometry::*;
use groups::*;
use paint_order::*;
use slots::*;
use text_primitives::*;
use visual_quads::*;

pub use delta::{SceneDelta, SceneDeltaStats};
pub use geometry::{AffineTransform, ClipRegion, SceneRect};
pub use groups::{EdgeFadeMask, FilterGroup, InsetShadowOverlay, OpacityGroup};
pub use kinds::{
    LayerMask, LayerMaskMode, NodeMask, QuadSurfacePaint, ScenePrimitive, ScenePrimitiveKind,
    StrokeCap, StrokePattern,
};
pub use slots::PrimitiveId;
#[allow(unused_imports)]
pub(crate) use slots::TRIGGERED_OVERLAY_SURFACE_SLOT;
pub use text_effects::{
    SceneGlyphPresentation, SceneRichPaint, SceneRichRun, SceneTextEffects, SceneTextOpenType,
    SceneTextSpan, css_text_effects,
};
pub use text_primitives::rich_fill_spans;

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
    /// Where the node's origin stood when this was built: where it is drawn
    /// while its projection is retained, which no move has touched since.
    origin: [f32; 2],
    /// How many of [`ScenePrimitive::clips`] the node inherited; the rest
    /// are its own, in its own coordinates.
    inherited_clips: usize,
}

#[derive(Debug)]
pub struct UiScene {
    derived: std::sync::Mutex<Option<Arc<SceneDerived>>>,
    epochs: SceneEpochs,
    projections: NodeMap<(u64, AffineTransform, usize)>,
    /// Projections that cannot be adjusted by an inverse delta. Usually empty;
    /// ordinary scrolling must not scan every retained descendant.
    unadjustable_projections: NodeSet,
    sticky_nodes: nana_ui_runtime::StickyDependencies,
    /// Nodes that hang from a clip parent other than their parent (#201): a
    /// scroll above them that they escape must not move their culling bounds.
    escaped_nodes: NodeSet,
    /// Set by the first layout move the scene keeps ([`ExtractedMove`]):
    /// from then on a record may stand elsewhere than where it was
    /// extracted, and where it stands is summed down its frame chain
    /// ([`UiScene::placed_origin`]). Until then every record stands at its
    /// [`ExtractedNode::layout`], which a fresh record's frame chain gives to
    /// the bit.
    placement_live: bool,
    /// Nodes whose last build read where they stand -- a transform a painter
    /// applies about the node's corner, an icon centred on the text of a
    /// parent it is not measured from -- and so are built again when they
    /// or a box above them move.
    origin_bound: NodeSet,
    /// Nodes whose last build read their own offset from their parent and
    /// nothing else that a move changes -- an icon centred on its parent's
    /// text, in its parent's coordinates -- and so are built again when they
    /// are the root of a move, not when a box above them moves.
    offset_bound: NodeSet,
    /// Nodes measured from a frame other than their parent, or sticking to
    /// a scroller: what a move of a box above them does not carry along.
    unfollowing: NodeSet,
    /// Scratch reused strictly during `&mut self` primitive rebuild passes.
    rebuild_scratch: RebuildScratch,
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
    /// Whether an image URL or host-texture slot summary on `FramePlan` may
    /// have changed in place across rebuilt primitives.
    frame_plan_resources_changed: bool,
    /// An extraction changed retained geometry while compositor layers may
    /// still be presenting the previous projection. The compositor pass
    /// clears this after rebasing the visibility index for the current layer.
    compositor_projection_dirty: bool,
    compositor: CompositorRegistry,
    /// Geometry built from each custom-painted node's recording, reused while
    /// the runtime hands back the same recording. Empty unless a node has a
    /// `Painter`.
    custom_paint: NodeMap<custom_paint::BuiltPaint>,
    /// Nodes that [`may_be_dest_group`] admits. Paint asks
    /// [`UiScene::opacity_groups`] once per primitive, and even the memoized
    /// walk behind it has to read a cold `ExtractedNode` and its style for the
    /// queried node itself; a zero here proves no walk can find a group, so no
    /// frame of a scene without isolation pays for it at all.
    dest_group_candidates: usize,
    /// Nodes already reported for a filter past the chain's depth
    /// (Issue #324), so the diagnostic is one per node, not one per frame.
    filter_depth_reported: std::sync::Mutex<NodeSet>,
    /// The document's frame interval, which an open shutter exposes a
    /// fraction of (Issue #323). Set by the host before each
    /// [`UiScene::apply_presentation`].
    presentation_frame_interval: std::time::Duration,
    /// Text nodes presenting per glyph, and until when what they draw keeps
    /// changing (`None`: a looping effect). What keeps frames coming while a
    /// reveal plays and stops them once it has finished.
    glyph_live: NodeMap<Option<std::time::Duration>>,
    /// Charts and inline custom-GPU payloads in motion, by node: until when
    /// their marks or emphasis move.
    gpu_payload_live: NodeMap<std::time::Duration>,
    /// Vector recordings emitted via [`ExtractedNode::widget_paint`], reused per node.
    widget_vector_paint: NodeMap<Arc<[custom_paint::BuiltPaint]>>,
    /// Inline objects of presenting text nodes, as laid out, so each
    /// compositor tick can move, scale and fade them the way the glyphs
    /// around them are (the text shader cannot reach them: they are images).
    glyph_objects: NodeMap<(Arc<SceneGlyphPresentation>, Arc<[GlyphObject]>)>,
    /// Identity that changes on node-changing mutation and on Clone.
    /// In-place [`UiScene::apply_delta`] that updates or removes nodes also
    /// gets a fresh value, because product flush mutates a unique `Arc` in
    /// place after the first paint. Painters key a validated op stream on
    /// this id. Never zero: two freshly created scenes must not share an
    /// identity.
    instance: u64,
    /// How every rounded corner of this scene is shaped: the installed
    /// theme's, set by the document that extracts into the scene. See
    /// [`UiScene::corner_shape`].
    corner_shape: nana_ui_core::CornerShape,
    /// The installed theme's focus ring, read when primitives are built. See
    /// [`UiScene::focus_ring`].
    focus_ring: nana_ui_core::FocusRingMetrics,
}

impl Default for UiScene {
    fn default() -> Self {
        Self {
            derived: std::sync::Mutex::new(None),
            epochs: SceneEpochs::default(),
            projections: NodeMap::default(),
            unadjustable_projections: NodeSet::default(),
            sticky_nodes: nana_ui_runtime::StickyDependencies::default(),
            escaped_nodes: NodeSet::default(),
            placement_live: false,
            origin_bound: NodeSet::default(),
            offset_bound: NodeSet::default(),
            unfollowing: NodeSet::default(),
            rebuild_scratch: RebuildScratch::default(),
            nodes: SceneNodes::default(),
            node_order: NodeMap::default(),
            primitives: BTreeMap::new(),
            ordered: BTreeSet::new(),
            build: next_primitive_revision(),
            structure_changed: false,
            frame_plan_resources_changed: false,
            compositor_projection_dirty: false,
            compositor: CompositorRegistry::default(),
            custom_paint: NodeMap::default(),
            widget_vector_paint: NodeMap::default(),
            dest_group_candidates: 0,
            filter_depth_reported: std::sync::Mutex::new(NodeSet::default()),
            presentation_frame_interval: std::time::Duration::from_millis(16),
            glyph_live: NodeMap::default(),
            gpu_payload_live: NodeMap::default(),
            glyph_objects: NodeMap::default(),
            instance: next_scene_instance(),
            corner_shape: nana_ui_core::CornerShape::Round,
            focus_ring: nana_ui_core::FOCUS_RING_METRICS,
        }
    }
}

impl Clone for UiScene {
    fn clone(&self) -> Self {
        Self {
            derived: std::sync::Mutex::new(self.derived.lock().expect("scene derived").clone()),
            epochs: self.epochs,
            projections: self.projections.clone(),
            unadjustable_projections: self.unadjustable_projections.clone(),
            sticky_nodes: self.sticky_nodes.clone(),
            escaped_nodes: self.escaped_nodes.clone(),
            placement_live: self.placement_live,
            origin_bound: self.origin_bound.clone(),
            offset_bound: self.offset_bound.clone(),
            unfollowing: self.unfollowing.clone(),
            rebuild_scratch: RebuildScratch::default(),
            nodes: self.nodes.clone(),
            node_order: self.node_order.clone(),
            primitives: self.primitives.clone(),
            ordered: self.ordered.clone(),
            build: self.build,
            structure_changed: self.structure_changed,
            frame_plan_resources_changed: self.frame_plan_resources_changed,
            compositor_projection_dirty: self.compositor_projection_dirty,
            compositor: self.compositor.clone(),
            custom_paint: self.custom_paint.clone(),
            widget_vector_paint: self.widget_vector_paint.clone(),
            dest_group_candidates: self.dest_group_candidates,
            filter_depth_reported: std::sync::Mutex::new(NodeSet::default()),
            presentation_frame_interval: self.presentation_frame_interval,
            glyph_live: self.glyph_live.clone(),
            gpu_payload_live: self.gpu_payload_live.clone(),
            glyph_objects: self.glyph_objects.clone(),
            instance: next_scene_instance(),
            corner_shape: self.corner_shape,
            focus_ring: self.focus_ring,
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

    /// What an answer derived from this scene's projections, transforms and
    /// clip chains stays valid for.
    ///
    /// `instance` moves whenever the scene's nodes do, `attribute_epoch`
    /// whenever what a node inherits does — including a compositor
    /// presentation tick, which moves geometry without touching the node set.
    /// An idle flush moves neither, which is what lets a host mirroring this
    /// geometry into a platform compositor skip the scan entirely rather than
    /// rebuilding the same answer every frame.
    pub const fn projection_revision(&self) -> (u64, u64) {
        (self.instance, self.epochs.attribute)
    }

    /// How every rounded corner in this scene is shaped. Primitives carry
    /// only their radii; a painter reads the shape once per frame.
    pub const fn corner_shape(&self) -> nana_ui_core::CornerShape {
        self.corner_shape
    }

    /// Shape every rounded corner. The document that extracts into the scene
    /// sets its installed theme's shape here. A change is a new
    /// [`Self::instance_id`], so a painter does not reuse a frame it painted
    /// with the old shape.
    pub fn set_corner_shape(&mut self, shape: nana_ui_core::CornerShape) {
        if self.corner_shape != shape {
            self.corner_shape = shape;
            self.instance = next_scene_instance();
        }
    }

    /// The focus ring geometry primitives are built with.
    pub const fn focus_ring(&self) -> nana_ui_core::FocusRingMetrics {
        self.focus_ring
    }

    /// Set the focus ring geometry. The document that extracts into the scene
    /// sets its installed theme's ring here before it applies a delta; a
    /// theme install re-extracts every node, so the change reaches every
    /// ring in the same frame. A change is a new [`Self::instance_id`].
    pub fn set_focus_ring(&mut self, ring: nana_ui_core::FocusRingMetrics) {
        if self.focus_ring != ring {
            self.focus_ring = ring;
            self.instance = next_scene_instance();
        }
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

    /// The resource of every custom-render primitive the scene paints, in
    /// paint order: the host textures it needs this frame. A node may paint
    /// several (a menu's row pictures), so a host keeping textures alive
    /// reads them here rather than from each node's own custom render.
    pub fn custom_resources(&self) -> impl Iterator<Item = &str> {
        self.primitives()
            .filter_map(|primitive| match &primitive.kind {
                ScenePrimitiveKind::Custom { node, .. } => Some(node.resource.as_ref()),
                _ => None,
            })
    }

    /// The record the scene holds for `id`: the one its last delta gave it.
    /// A record a layout move kept ([`ExtractedMove`]), and every record
    /// under it, keeps the [`ExtractedNode::layout`] it was extracted with;
    /// [`Self::node_layout_bounds`] is where its box stands now.
    pub fn extracted_node(&self, id: StableNodeId) -> Option<&ExtractedNode> {
        self.nodes.get(&id).map(Arc::as_ref)
    }

    /// Where `id`'s box stands on the page now, before any scroll or
    /// transform: the same as [`Self::node_layout_bounds`].
    /// [`Self::draw_node_rect`] maps this space to where it is drawn.
    pub fn node_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        self.node_layout_bounds(id)
    }

    pub fn is_node_in_subtree(&self, root: StableNodeId, candidate: StableNodeId) -> bool {
        ancestor_ids(&self.nodes, candidate).any(|id| id == root)
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&ScenePrimitive> {
        self.primitives.get(&id).map(|held| &held.primitive)
    }

    /// The primitive, the rebuild that last wrote it ([`SceneDraw::revision`]),
    /// where it was built ([`RetainedPrimitive::origin`]) and how many of its
    /// clips it inherited.
    fn primitive_placed_at(
        &self,
        id: PrimitiveId,
    ) -> Option<(&ScenePrimitive, u64, [f32; 2], usize)> {
        self.primitives.get(&id).map(|held| {
            (
                &held.primitive,
                held.build,
                held.origin,
                held.inherited_clips,
            )
        })
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
        self.invalidate_derived_all();
        self.instance = next_scene_instance();
        true
    }

    /// Returns `true` when any retained primitive or isolating group in this
    /// scene carries wide-gamut or HDR authoring colors (`< 0` or `> 1` in
    /// linear scRGB) that require a floating-point working target to avoid
    /// hardware `[0, 1]` clamping. Cached across frames until a scene delta
    /// modifies or removes primitives.
    pub fn has_extended_color(&self) -> bool {
        if let Some(extended) = self.derived_extended_color() {
            return extended;
        }
        let extended = self
            .primitives
            .values()
            .any(|held| held.primitive.kind.has_extended_color())
            || self.nodes.values().any(|node| {
                let paint = &node.source_style.layout.paint;
                node.style.paint_colors.has_extended_srgb()
                    || paint
                        .filter
                        .as_ref()
                        .is_some_and(kinds::color_filter_has_extended_color)
                    || paint
                        .backdrop_filter
                        .as_ref()
                        .is_some_and(|bf| bf.saturate > 1.0 + 1.0e-3)
                    || paint.box_shadows.iter().any(|s| {
                        s.paint_color
                            .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
                    })
            })
            || self.compositor.presents_extended_filter();
        self.set_derived_extended_color(extended);
        extended
    }
}

/// The scene's nodes.
///
/// `Arc` rather than the node itself: rebuilding a node's primitives needs an
/// owned handle while `&mut self` inserts them, and an `ExtractedNode` is 784
/// bytes. A container style change rebuilds every descendant, so that clone
/// used to be most of a megabyte of memmove per frame.
#[derive(Debug, Clone, Default)]
pub(super) struct SceneNodes {
    retained: NodeMap<Arc<ExtractedNode>>,
    /// Presentation isolation is scene metadata, never a logical style write.
    /// Keeping it beside the retained nodes lets ordering, projection and paint
    /// agree on the same boundaries, including before a layer is promoted.
    opacity_motion: NodeSet,
    /// Nodes whose `filter` the compositor presents (Issue #312): each is a
    /// dest group for as long as a filter track applies, whatever its
    /// logical filter, and its group takes the sampled filter.
    filter_motion: NodeSet,
}

impl std::ops::Deref for SceneNodes {
    type Target = NodeMap<Arc<ExtractedNode>>;

    fn deref(&self) -> &Self::Target {
        &self.retained
    }
}

impl std::ops::DerefMut for SceneNodes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.retained
    }
}

#[cfg(test)]
impl FromIterator<(StableNodeId, Arc<ExtractedNode>)> for SceneNodes {
    fn from_iter<T: IntoIterator<Item = (StableNodeId, Arc<ExtractedNode>)>>(iter: T) -> Self {
        Self {
            retained: iter.into_iter().collect(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests;
