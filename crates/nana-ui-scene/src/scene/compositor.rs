//! Compositor layers: presentation transform/opacity over retained primitives.
//!
//! [`OpacityGroup`] / [`FilterGroup`] stay dest-isolation groups. They are not
//! renamed into this type. Layout-class properties (width/height) never promote.

use std::time::Duration;

use nana_ui_core::motion::{
    AnimatableProperty, AnimationClass, MotionDescriptorStore, MotionGpuDescriptor,
    MotionGpuKeyframe, MotionHandle, MotionInspectorEntry, MotionTargetId, MotionTrackId,
    MotionValue, MotionWorkCounters, PresentationStore,
};
use nana_ui_core::{ColorFilter, PaintTransform};
use nana_ui_runtime::{ExtractedNode, NodeMap, NodeSet, StableNodeId};

use super::{AffineTransform, ClipRegion, UiScene, local_opacity, node_scene_transform};

/// Candidate must stay compositor-eligible this long before a layer is created.
/// Sub-frame overlay flashes never promote.
pub const LAYER_PROMOTE_HOLD: Duration = Duration::from_millis(16);
/// After becoming ineligible, the layer stays for this long so brief overlay
/// gaps (hover flicker) reuse the same identity instead of demote/promote.
pub const LAYER_DEMOTE_HOLD: Duration = Duration::from_millis(120);

const COMPOSITOR_PROPERTIES: [AnimatableProperty; 4] = [
    AnimatableProperty::Transform,
    AnimatableProperty::Opacity,
    AnimatableProperty::Clip,
    AnimatableProperty::ShaderParameter,
];

/// Stable layer identity. Equals the promoting node's [`StableNodeId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompositorLayerId(u64);

impl CompositorLayerId {
    pub fn from_node(node: StableNodeId) -> Self {
        Self(node.get())
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn node(self) -> Option<StableNodeId> {
        StableNodeId::new(self.0)
    }
}

/// Compact Motion IR binding on a layer. `index`/`generation` are Workstream
/// D's [`MotionHandle`]; `generation == 0` means no live descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositorMotionBinding {
    pub track_id: MotionTrackId,
    pub index: u32,
    pub generation: u32,
}

impl CompositorMotionBinding {
    pub fn from_track(
        track_id: MotionTrackId,
        descriptors: Option<&MotionDescriptorStore>,
    ) -> Self {
        let handle = descriptors
            .and_then(|store| store.handle_for(track_id))
            .unwrap_or(MotionHandle::NULL);
        Self {
            track_id,
            index: handle.index(),
            generation: handle.generation(),
        }
    }

    pub fn handle(self) -> MotionHandle {
        MotionHandle::from_parts(self.index, self.generation)
    }
}

/// Per-primitive compositor encode. GPU ids are only valid for kinds that
/// evaluate in shader; others keep CPU [`SceneDraw`] presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositorPaintEncode {
    pub transform: AffineTransform,
    /// Scene-space pivot of the GPU transform overlay: the node's resolved
    /// `transform-origin`. Meaningless without a transform id.
    pub transform_origin: [f32; 2],
    pub opacity: f32,
    pub motion_ids: (u32, u32),
}

/// Why this subtree is a compositor layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositorLayerKind {
    /// Transform and/or opacity presentation.
    TransformOpacity,
    /// Clip and/or effect contract. Filter evaluation is not a dest `FilterGroup`.
    FilterClip,
}

/// Retained subtree plus presentation transform/opacity. Primitives stay in
/// logical geometry; the painter CPU-applies [`Self::transform`] / [`Self::opacity`].
#[derive(Debug, Clone, PartialEq)]
pub struct CompositorLayer {
    pub id: CompositorLayerId,
    pub node: StableNodeId,
    pub parent: Option<CompositorLayerId>,
    pub kind: CompositorLayerKind,
    pub transform: AffineTransform,
    pub opacity: f32,
    pub clip: Option<ClipRegion>,
    pub effect: Option<ColorFilter>,
    pub bindings: Vec<CompositorMotionBinding>,
    /// Increments when topology or primitive structure of the covered subtree
    /// changes. Motion timestamp must not bump this.
    pub cache_generation: u64,
    /// Last observed surface/device generation. Hosts call
    /// [`UiScene::set_surface_generation`] after device loss (Workstream H
    /// owns the real GPU rebuild).
    pub surface_generation: u64,
    pub z_index: i32,
}

#[derive(Debug, Clone)]
enum LayerPhase {
    PendingPromote { since: Duration },
    Active,
    PendingDemote { since: Duration },
}

#[derive(Debug, Clone, Default)]
pub(super) struct CompositorRegistry {
    layers: NodeMap<CompositorLayer>,
    phases: NodeMap<LayerPhase>,
    requested: NodeSet,
    presentation_epoch: u64,
    surface_generation: u64,
    last_promoted: usize,
    last_demoted: usize,
    gpu: MotionGpuPack,
}

#[derive(Debug, Clone)]
struct MotionGpuPack {
    source: u64,
    structure_epoch: u64,
    slot_capacity: usize,
    now: Duration,
    descriptors: Vec<MotionGpuDescriptor>,
    keyframes: Vec<MotionGpuKeyframe>,
}

impl Default for MotionGpuPack {
    fn default() -> Self {
        Self {
            source: 0,
            structure_epoch: u64::MAX,
            slot_capacity: 0,
            now: Duration::ZERO,
            descriptors: vec![MotionGpuDescriptor::vacant(0)],
            keyframes: vec![MotionGpuKeyframe::dummy()],
        }
    }
}

impl CompositorRegistry {
    fn layer(&self, node: StableNodeId) -> Option<&CompositorLayer> {
        self.layers.get(&node)
    }

    fn is_active(&self, node: StableNodeId) -> bool {
        matches!(
            self.phases.get(&node),
            Some(LayerPhase::Active | LayerPhase::PendingDemote { .. })
        ) && self.layers.contains_key(&node)
    }

    fn sync_gpu_pack(&mut self, store: &MotionDescriptorStore, now: Duration) {
        self.gpu.now = now;
        if self.gpu.source == store.source()
            && self.gpu.structure_epoch == store.structure_epoch()
            && self.gpu.slot_capacity == store.slot_capacity()
        {
            return;
        }
        let (descriptors, keyframes) = store.pack_gpu();
        self.gpu.source = store.source();
        self.gpu.structure_epoch = store.structure_epoch();
        self.gpu.slot_capacity = store.slot_capacity();
        self.gpu.descriptors = descriptors;
        self.gpu.keyframes = keyframes;
    }
}

impl UiScene {
    pub fn compositor_layer(&self, node: StableNodeId) -> Option<&CompositorLayer> {
        self.compositor.layer(node)
    }

    pub fn compositor_layers(&self) -> impl Iterator<Item = &CompositorLayer> {
        self.compositor.layers.values()
    }

    pub fn compositor_layer_count(&self) -> usize {
        self.compositor.layers.len()
    }

    pub fn compositor_work_counters(&self) -> MotionWorkCounters {
        MotionWorkCounters {
            compositor_layers_active: self.compositor.layers.len(),
            compositor_layers_promoted: self.compositor.last_promoted,
            compositor_layers_demoted: self.compositor.last_demoted,
            compositor_cache_bytes: 0,
            ..MotionWorkCounters::default()
        }
    }

    /// Fill layer / promotion / GPU-evaluator fields on Runtime inspector rows.
    pub fn annotate_motion_inspector(&self, entries: &mut [MotionInspectorEntry]) {
        for entry in entries {
            let Some(node) = StableNodeId::new(entry.node) else {
                continue;
            };
            if let Some(layer) = self.compositor.layer(node) {
                entry.layer = Some(layer.id.get());
                entry.layer_promotion_reason = Some(match layer.kind {
                    CompositorLayerKind::TransformOpacity => "transform/opacity presentation",
                    CompositorLayerKind::FilterClip => "clip/effect presentation",
                });
                if self.compositor.requested.contains(&node) && layer.bindings.is_empty() {
                    entry.layer_promotion_reason = Some("explicit request_compositor_layer");
                }
                if entry.evaluator == nana_ui_core::motion::MotionEvaluatorBackend::Gpu {
                    let ids = self.compositor_gpu_motion_ids(node);
                    if ids == (0, 0) {
                        entry.evaluator = nana_ui_core::motion::MotionEvaluatorBackend::Cpu;
                        if entry.cpu_fallback_reason.is_none() {
                            entry.cpu_fallback_reason = Some(
                                "CPU SceneDraw: primitive kind does not evaluate compositor motion on GPU."
                                    .to_string(),
                            );
                        }
                    }
                }
            } else if matches!(
                self.compositor.phases.get(&node),
                Some(LayerPhase::PendingPromote { .. })
            ) {
                entry.layer_promotion_reason = Some("pending promote hysteresis (16ms)");
            }
        }
    }

    pub fn compositor_needs_tick(&self) -> bool {
        !self.compositor.layers.is_empty()
            || !self.compositor.phases.is_empty()
            || !self.compositor.requested.is_empty()
    }

    pub fn compositor_layer_requested(&self, node: StableNodeId) -> bool {
        self.compositor.requested.contains(&node)
    }

    /// Cache key fragment for painters: presentation may change without
    /// [`Self::instance_id`].
    pub fn presentation_epoch(&self) -> u64 {
        self.compositor.presentation_epoch
    }

    pub fn surface_generation(&self) -> u64 {
        self.compositor.surface_generation
    }

    pub fn motion_gpu_now(&self) -> Duration {
        self.compositor.gpu.now
    }

    pub fn motion_gpu_structure_epoch(&self) -> u64 {
        self.compositor.gpu.structure_epoch
    }

    /// Descriptor store the packed table came from; see
    /// `MotionDescriptorStore::source`.
    pub fn motion_gpu_source(&self) -> u64 {
        self.compositor.gpu.source
    }

    pub fn motion_gpu_descriptors(&self) -> &[MotionGpuDescriptor] {
        &self.compositor.gpu.descriptors
    }

    pub fn motion_gpu_keyframes(&self) -> &[MotionGpuKeyframe] {
        &self.compositor.gpu.keyframes
    }

    /// Advanced request: promote `node` once hysteresis allows, even without overlay.
    pub fn request_compositor_layer(&mut self, node: StableNodeId) {
        self.compositor.requested.insert(node);
    }

    pub fn clear_compositor_layer_request(&mut self, node: StableNodeId) {
        self.compositor.requested.remove(&node);
    }

    /// Host hook after surface/device generation changes. Layers record the new
    /// generation so GPU resources can be rebuilt; this does not extract.
    pub fn set_surface_generation(&mut self, generation: u64) {
        if self.compositor.surface_generation == generation {
            return;
        }
        self.compositor.surface_generation = generation;
        for layer in self.compositor.layers.values_mut() {
            layer.surface_generation = generation;
        }
        self.compositor.presentation_epoch = self.compositor.presentation_epoch.wrapping_add(1);
    }

    /// Bind compositor overlays from B's presentation store. Does not rebuild
    /// primitives; only motion timestamp must not invalidate [`CompositorLayer::cache_generation`].
    ///
    /// A layer that moves does move the retained visibility index, though, and
    /// that is re-derived here: no extraction reaches this path, so nothing
    /// else will.
    pub fn apply_presentation(
        &mut self,
        store: &PresentationStore,
        now: Duration,
        descriptors: Option<&MotionDescriptorStore>,
    ) {
        if let Some(store) = descriptors {
            self.compositor.sync_gpu_pack(store, now);
        } else {
            self.compositor.gpu.now = now;
        }
        self.compositor.last_promoted = 0;
        self.compositor.last_demoted = 0;
        let mut candidates = self.compositor.requested.clone();
        for overlay in store.overlays() {
            if overlay.track.property.animation_class() != AnimationClass::Compositor {
                continue;
            }
            if let Some(node) = StableNodeId::new(overlay.track.target.get())
                && self.nodes.contains_key(&node)
            {
                candidates.insert(node);
            }
        }
        for node in self.compositor.layers.keys().copied() {
            candidates.insert(node);
        }
        for node in self.compositor.phases.keys().copied() {
            candidates.insert(node);
        }

        if candidates.is_empty() {
            return;
        }

        let surface_generation = self.compositor.surface_generation;
        let mut changed = false;
        let mut moved = false;

        for node in candidates {
            let Some(extracted) = self.nodes.get(&node) else {
                self.compositor.phases.remove(&node);
                if self.compositor.layers.remove(&node).is_some() {
                    self.compositor.last_demoted = self.compositor.last_demoted.saturating_add(1);
                    changed = true;
                    moved = true;
                }
                continue;
            };
            // All a layer hands the visibility index: the transform
            // `resolved_local_transform` presents for this node, and so for
            // every descendant inheriting its projection. `None` is a node no
            // layer speaks for, whose transform only an extraction can move —
            // and none reaches this path. Read either side of the transition
            // rather than inferred from its arms, which all reach it and can
            // all leave it be.
            let presented = |compositor: &CompositorRegistry| {
                compositor
                    .layer(node)
                    .filter(|_| compositor.is_active(node))
                    .map(|layer| transform_bits(layer.transform))
            };
            let before = presented(&self.compositor);
            let snapshot = layer_snapshot(extracted, store, now, descriptors);
            let eligible = snapshot.eligible || self.compositor.requested.contains(&node);
            let previous = self.compositor.phases.get(&node).cloned();
            match (eligible, previous) {
                (true, None) => {
                    let since = snapshot.eligible_since.unwrap_or(now);
                    if hold_elapsed(now, since, LAYER_PROMOTE_HOLD) {
                        let z_index = extracted.z_index;
                        self.compositor.layers.insert(
                            node,
                            make_layer(extracted.id, z_index, snapshot, surface_generation),
                        );
                        self.compositor.phases.insert(node, LayerPhase::Active);
                        self.compositor.last_promoted =
                            self.compositor.last_promoted.saturating_add(1);
                        changed = true;
                    } else {
                        self.compositor
                            .phases
                            .insert(node, LayerPhase::PendingPromote { since });
                    }
                }
                (true, Some(LayerPhase::PendingPromote { since })) => {
                    if hold_elapsed(now, since, LAYER_PROMOTE_HOLD) {
                        let z_index = extracted.z_index;
                        self.compositor.layers.insert(
                            node,
                            make_layer(extracted.id, z_index, snapshot, surface_generation),
                        );
                        self.compositor.phases.insert(node, LayerPhase::Active);
                        self.compositor.last_promoted =
                            self.compositor.last_promoted.saturating_add(1);
                        changed = true;
                    } else {
                        self.compositor
                            .phases
                            .insert(node, LayerPhase::PendingPromote { since });
                    }
                }
                (true, Some(LayerPhase::Active) | Some(LayerPhase::PendingDemote { .. })) => {
                    if let Some(layer) = self.compositor.layers.get_mut(&node) {
                        let visual_changed = layer.transform != snapshot.transform
                            || layer.opacity != snapshot.opacity
                            || layer.kind != snapshot.kind
                            || layer.bindings != snapshot.bindings
                            || layer.clip != snapshot.clip
                            || layer.effect != snapshot.effect
                            || layer.z_index != extracted.z_index;
                        layer.transform = snapshot.transform;
                        layer.opacity = snapshot.opacity;
                        layer.kind = snapshot.kind;
                        layer.bindings = snapshot.bindings;
                        layer.clip = snapshot.clip;
                        layer.effect = snapshot.effect;
                        layer.z_index = extracted.z_index;
                        changed |= visual_changed;
                    } else {
                        let z_index = extracted.z_index;
                        self.compositor.layers.insert(
                            node,
                            make_layer(extracted.id, z_index, snapshot, surface_generation),
                        );
                        self.compositor.last_promoted =
                            self.compositor.last_promoted.saturating_add(1);
                        changed = true;
                    }
                    self.compositor.phases.insert(node, LayerPhase::Active);
                }
                (false, Some(LayerPhase::Active)) => {
                    self.compositor
                        .phases
                        .insert(node, LayerPhase::PendingDemote { since: now });
                }
                (false, Some(LayerPhase::PendingDemote { since })) => {
                    if hold_elapsed(now, since, LAYER_DEMOTE_HOLD) {
                        self.compositor.layers.remove(&node);
                        self.compositor.phases.remove(&node);
                        self.compositor.last_demoted =
                            self.compositor.last_demoted.saturating_add(1);
                        changed = true;
                    }
                }
                (false, Some(LayerPhase::PendingPromote { .. }) | None) => {
                    self.compositor.phases.remove(&node);
                }
            }
            moved |= before != presented(&self.compositor);
        }

        let active_nodes: NodeSet = self.compositor.layers.keys().copied().collect();
        for layer in self.compositor.layers.values_mut() {
            let parent = nearest_layer_parent(&self.nodes, layer.node, &active_nodes);
            if layer.parent != parent {
                layer.parent = parent;
                changed = true;
                // A layer's parent contributes to every descendant's
                // presented transform. The layer's own transform may be
                // unchanged, so `before != presented` above does not detect
                // this topology-only movement. Rebuild all retained bounds
                // before the next visibility query.
                moved = true;
            }
        }

        if changed {
            self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
            self.compositor.presentation_epoch = self.compositor.presentation_epoch.wrapping_add(1);
        }
        // A layer that moved moved its retained descendants' projections
        // with it — what an ancestor transform does in `apply_delta`, except
        // that nothing here re-extracted, so there is no changed set to
        // refresh from and the index would keep culling against where the
        // subtree was when it was last derived. Re-derive from the plan it
        // already holds; a plan is structural, and no layer is in one.
        if moved && let Some(mut visibility) = self.visibility.take() {
            visibility.refresh_bounds(self);
            let _ = self.visibility.set(visibility);
        }
        #[cfg(debug_assertions)]
        self.audit_retained_projection();
    }

    pub(super) fn forget_compositor_node(&mut self, id: StableNodeId) {
        self.compositor.layers.remove(&id);
        self.compositor.phases.remove(&id);
        self.compositor.requested.remove(&id);
    }

    pub(super) fn invalidate_compositor_cache(&mut self, id: StableNodeId) {
        if self.compositor.layers.is_empty() {
            return;
        }
        for node in super::ancestor_ids(&self.nodes, id).collect::<Vec<_>>() {
            if let Some(layer) = self.compositor.layers.get_mut(&node) {
                layer.cache_generation = layer.cache_generation.saturating_add(1);
            }
        }
    }

    pub(super) fn retain_compositor_requests(&mut self, node: &ExtractedNode) {
        if node.compositor.request_layer {
            self.compositor.requested.insert(node.id);
        } else {
            self.compositor.requested.remove(&node.id);
        }
    }

    pub(super) fn resolved_local_transform(
        &self,
        node: &ExtractedNode,
        block_3d: bool,
    ) -> AffineTransform {
        // A closed 3D context refuses this node's transform, and a layer does
        // not reopen it. `layer_snapshot` cannot see the rule — it is handed
        // one node, and the answer is about the ancestors above it — so the
        // refusal is made here too, or promoting a layer (an opacity fade is
        // enough) hands back the projection the ancestor walk denied.
        if block_3d && node.source_style.layout.transform_3d.is_some() {
            return AffineTransform::IDENTITY;
        }
        if self.compositor.is_active(node.id) {
            return self
                .compositor
                .layers
                .get(&node.id)
                .map(|layer| layer.transform)
                .unwrap_or_else(|| {
                    node_scene_transform(node.source_style.layout.as_ref(), node.layout, block_3d)
                });
        }
        node_scene_transform(node.source_style.layout.as_ref(), node.layout, block_3d)
    }

    pub(super) fn resolved_local_opacity(&self, node: &ExtractedNode) -> f32 {
        if self.compositor.is_active(node.id) {
            return self
                .compositor
                .layers
                .get(&node.id)
                .map(|layer| layer.opacity)
                .unwrap_or_else(|| local_opacity(node));
        }
        local_opacity(node)
    }

    /// Logical primitive opacity multiplied by compositor layer factors.
    pub fn compositor_paint_opacity(&self, node: StableNodeId, logical_opacity: f32) -> f32 {
        if self.compositor.layers.is_empty() {
            return logical_opacity.clamp(0.0, 1.0);
        }
        // `draw_primitive` asks this for every primitive, so the ancestor half
        // is memoized the same way dest groups are: a container's factor is
        // its parent's times its own. The queried node's own factor is not
        // stored — every primitive names a different node.
        let own = self.layer_opacity_factor(node);
        let inherited = match self.nodes.get(&node).and_then(|node| node.parent) {
            Some(parent) => {
                let mut cache = self
                    .layer_factor_cache
                    .lock()
                    .expect("scene compositor factors");
                self.ancestor_layer_factor(&mut cache, parent, 0)
            }
            None => 1.0,
        };
        (logical_opacity * own * inherited).clamp(0.0, 1.0)
    }

    /// What this node's own layer does to the opacity below it: the presented
    /// opacity divided by the logical one it was derived from.
    fn layer_opacity_factor(&self, id: StableNodeId) -> f32 {
        let Some(layer) = self.compositor.layers.get(&id) else {
            return 1.0;
        };
        let Some(extracted) = self.nodes.get(&id) else {
            return 1.0;
        };
        let logical = local_opacity(extracted);
        if logical.abs() < 1e-8 {
            layer.opacity
        } else {
            layer.opacity / logical
        }
    }

    fn ancestor_layer_factor(
        &self,
        cache: &mut super::LayerFactorCache,
        id: StableNodeId,
        depth: usize,
    ) -> f32 {
        if depth >= super::MAX_ANCESTOR_DEPTH {
            return 1.0;
        }
        let stamp = (self.instance_id(), self.attribute_epoch);
        if let Some((held, factor)) = cache.get(&id)
            && *held == stamp
        {
            return *factor;
        }
        let Some(extracted) = self.nodes.get(&id) else {
            return 1.0;
        };
        let inherited = match extracted.parent {
            Some(parent) => self.ancestor_layer_factor(cache, parent, depth + 1),
            None => 1.0,
        };
        let factor = inherited * self.layer_opacity_factor(id);
        cache.insert(id, (stamp, factor));
        factor
    }

    /// Nested compositor opacity: product of ancestor (and self) layer opacities.
    pub fn composed_layer_opacity(&self, node: StableNodeId) -> f32 {
        if self.compositor.layers.is_empty() {
            return 1.0;
        }
        let mut opacity = 1.0;
        for id in super::ancestor_ids(&self.nodes, node) {
            if let Some(layer) = self.compositor.layers.get(&id) {
                opacity *= layer.opacity;
            }
        }
        opacity.clamp(0.0, 1.0)
    }

    /// Nested compositor transform: outer layers then inner.
    pub fn composed_layer_transform(&self, node: StableNodeId) -> AffineTransform {
        if self.compositor.layers.is_empty() {
            return AffineTransform::IDENTITY;
        }
        let mut chain = Vec::new();
        for id in super::ancestor_ids(&self.nodes, node) {
            if let Some(layer) = self.compositor.layers.get(&id) {
                chain.push(layer.transform);
            }
        }
        chain
            .into_iter()
            .rev()
            .fold(AffineTransform::IDENTITY, |acc, local| acc.then(local))
    }

    /// Handle indices for the GPU storage buffer (`0` = none). Shader
    /// evaluate looks up [`Self::motion_gpu_descriptors`]. Transform ids are
    /// only the node's own layer so the painter can strip that overlay without
    /// double-applying ancestor presentation. Opacity walks ancestors.
    pub fn compositor_gpu_motion_ids(&self, node: StableNodeId) -> (u32, u32) {
        let mut transform = self.gpu_motion_id_for(node, 0);
        // The shader evaluates the overlay in the node's own place, so it owes
        // the same refusal [`Self::resolved_local_transform`] makes, or the
        // GPU hands back what the CPU gave up. The ancestor walk sits behind
        // two reads false for every node without both a `matrix3d` and an
        // overlay of its own.
        if transform != 0
            && let Some(extracted) = self.nodes.get(&node)
            && extracted.source_style.layout.transform_3d.is_some()
            && self.draw_ancestor_state(extracted).3
        {
            transform = 0;
        }
        let mut opacity = self.gpu_motion_id_for(node, 1);
        if opacity == 0
            && let Some(parent) = self.nodes.get(&node).and_then(|node| node.parent)
        {
            for id in super::ancestor_ids(&self.nodes, parent) {
                opacity = self.gpu_motion_id_for(id, 1);
                if opacity != 0 {
                    break;
                }
            }
        }
        (transform, opacity)
    }

    /// World transform and pivot encoded for GPU evaluate. The transform is
    /// the node's parent: the shader applies the overlay around
    /// `transform-origin` in its place, as the CPU layer snapshot does. The
    /// parent is resolved directly, not by inverting the layer, so a collapsed
    /// (zero-scale) overlay still encodes.
    pub fn compositor_gpu_encode_transform(
        &self,
        node: StableNodeId,
        composed: AffineTransform,
    ) -> (AffineTransform, [f32; 2]) {
        let (transform_id, _) = self.compositor_gpu_motion_ids(node);
        let Some(extracted) = self.nodes.get(&node).filter(|_| transform_id != 0) else {
            return (composed, [0.0, 0.0]);
        };
        let (parent, _, _, _) = self.draw_ancestor_state(extracted);
        let layout = extracted.layout;
        let [ox, oy] = extracted
            .source_style
            .layout
            .resolved_transform_origin(layout.width, layout.height);
        (parent, [layout.x + ox, layout.y + oy])
    }

    /// Paint opacity with the GPU-evaluated overlay factored out.
    pub fn compositor_gpu_encode_opacity(&self, node: StableNodeId, paint_opacity: f32) -> f32 {
        let (_, opacity_id) = self.compositor_gpu_motion_ids(node);
        if opacity_id == 0 {
            return paint_opacity;
        }
        let index = opacity_id - 1;
        for id in super::ancestor_ids(&self.nodes, node) {
            if let Some(layer) = self.compositor.layer(id)
                && layer
                    .bindings
                    .iter()
                    .any(|binding| binding.index == index && binding.generation != 0)
            {
                if layer.opacity.abs() < 1e-8 {
                    return 0.0;
                }
                return (paint_opacity / layer.opacity).clamp(0.0, 1.0);
            }
        }
        paint_opacity
    }

    /// Encode compositor motion for one primitive. Only kinds that run
    /// shader `evaluate()` strip the overlay and receive GPU ids. Others
    /// keep CPU presentation so Text / Icon / Mesh / HostTexture stay faded
    /// and transformed with the layer.
    pub fn compositor_paint_encode(
        &self,
        node: StableNodeId,
        kind: &super::ScenePrimitiveKind,
        composed: AffineTransform,
        paint_opacity: f32,
    ) -> CompositorPaintEncode {
        if kind.evaluates_compositor_motion_on_gpu() {
            let (transform, transform_origin) =
                self.compositor_gpu_encode_transform(node, composed);
            CompositorPaintEncode {
                transform,
                transform_origin,
                opacity: self.compositor_gpu_encode_opacity(node, paint_opacity),
                motion_ids: self.compositor_gpu_motion_ids(node),
            }
        } else {
            CompositorPaintEncode {
                transform: composed,
                transform_origin: [0.0, 0.0],
                opacity: paint_opacity,
                motion_ids: (0, 0),
            }
        }
    }

    fn gpu_motion_id_for(&self, node: StableNodeId, property: u32) -> u32 {
        let Some(layer) = self.compositor.layer(node) else {
            return 0;
        };
        for binding in &layer.bindings {
            if binding.generation == 0 {
                continue;
            }
            let Some(descriptor) = self.compositor.gpu.descriptors.get(binding.index as usize)
            else {
                continue;
            };
            if descriptor.generation != binding.generation || !descriptor.is_live() {
                continue;
            }
            if descriptor.property == property {
                return binding.index.saturating_add(1);
            }
        }
        0
    }
}

fn make_layer(
    node: StableNodeId,
    z_index: i32,
    snapshot: LayerSnapshot,
    surface_generation: u64,
) -> CompositorLayer {
    CompositorLayer {
        id: CompositorLayerId::from_node(node),
        node,
        parent: None,
        kind: snapshot.kind,
        transform: snapshot.transform,
        opacity: snapshot.opacity,
        clip: snapshot.clip,
        effect: snapshot.effect,
        bindings: snapshot.bindings,
        cache_generation: 1,
        surface_generation,
        z_index,
    }
}

/// A transform as the bits it is made of: the visibility index is audited
/// against a fresh build bit for bit, and `==` calls `0.0` and `-0.0` the same
/// transform though they do not always project a bound to the same bits.
fn transform_bits(transform: AffineTransform) -> ([u32; 6], [u32; 2]) {
    (transform.0.map(f32::to_bits), transform.1.map(f32::to_bits))
}

fn hold_elapsed(now: Duration, since: Duration, hold: Duration) -> bool {
    now.saturating_sub(since) >= hold
}

fn nearest_layer_parent(
    nodes: &super::SceneNodes,
    node: StableNodeId,
    active: &NodeSet,
) -> Option<CompositorLayerId> {
    let parent = nodes.get(&node).and_then(|node| node.parent)?;
    super::ancestor_ids(nodes, parent)
        .find(|id| active.contains(id))
        .map(CompositorLayerId::from_node)
}

#[derive(Clone)]
struct LayerSnapshot {
    eligible: bool,
    eligible_since: Option<Duration>,
    kind: CompositorLayerKind,
    transform: AffineTransform,
    opacity: f32,
    clip: Option<ClipRegion>,
    effect: Option<ColorFilter>,
    bindings: Vec<CompositorMotionBinding>,
}

fn layer_snapshot(
    node: &ExtractedNode,
    store: &PresentationStore,
    now: Duration,
    descriptors: Option<&MotionDescriptorStore>,
) -> LayerSnapshot {
    let target = MotionTargetId::new(node.id.get());
    let mut bindings: Vec<CompositorMotionBinding> = Vec::new();
    let mut has_clip = false;
    let mut presentation_transform = None;
    let mut presentation_opacity = None;
    let mut eligible_since: Option<Duration> = None;
    if let Some(target) = target {
        for property in COMPOSITOR_PROPERTIES {
            let Some(track_id) = store.winning_track_id(target, property, now) else {
                continue;
            };
            if property.animation_class() != AnimationClass::Compositor {
                continue;
            }
            if let Some(overlay) = store.get(track_id) {
                let start = overlay.track.timing.start;
                eligible_since = Some(eligible_since.map_or(start, |prev| prev.min(start)));
            }
            if !bindings.iter().any(|binding| binding.track_id == track_id) {
                bindings.push(CompositorMotionBinding::from_track(track_id, descriptors));
            }
            match property {
                AnimatableProperty::Transform => {
                    if let Some(MotionValue::Transform(value)) =
                        store.applied_value(target, property, now)
                    {
                        presentation_transform = Some(value);
                    }
                }
                AnimatableProperty::Opacity => {
                    if let Some(MotionValue::Scalar(value)) =
                        store.applied_value(target, property, now)
                    {
                        presentation_opacity = Some(value.clamp(0.0, 1.0));
                    }
                }
                AnimatableProperty::Clip => has_clip = true,
                AnimatableProperty::ShaderParameter => {}
                _ => {}
            }
        }
    }
    for track_id in &node.compositor.bindings {
        if !bindings.iter().any(|binding| binding.track_id == *track_id) {
            bindings.push(CompositorMotionBinding::from_track(*track_id, descriptors));
        }
    }
    let eligible = !bindings.is_empty()
        || node.compositor.request_layer
        || presentation_transform.is_some()
        || presentation_opacity.is_some()
        || has_clip;
    let kind = if has_clip {
        CompositorLayerKind::FilterClip
    } else {
        CompositorLayerKind::TransformOpacity
    };
    let transform = match presentation_transform {
        Some(value) => affine_from_paint(node, value),
        None => node_scene_transform(node.source_style.layout.as_ref(), node.layout, false),
    };
    let opacity = presentation_opacity.unwrap_or_else(|| local_opacity(node));
    LayerSnapshot {
        eligible,
        eligible_since,
        kind,
        transform,
        opacity,
        clip: None,
        effect: None,
        bindings,
    }
}

fn affine_from_paint(node: &ExtractedNode, transform: PaintTransform) -> AffineTransform {
    let mut layout = (*node.source_style.layout).clone();
    layout.transform = Some(transform);
    node_scene_transform(&layout, node.layout, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UiScene;
    use nana_ui_core::motion::{
        AnimationPlayback, Easing, MotionCurve, MotionTiming, MotionTo, MotionTrack, MotionValue,
    };
    use nana_ui_core::{LayoutStyle, PaintTransform};
    use nana_ui_runtime::{ComputedStyle, LayoutBox, NodeKind, NodeStyle};
    use std::sync::Arc;

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn node(value: u64, parent: Option<u64>, children: &[u64]) -> ExtractedNode {
        ExtractedNode {
            custom_paint: None,
            chrome_radii: nana_ui_core::ChromeRadii::default(),
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
            style: Arc::new(ComputedStyle {
                background: Some([0.2, 0.3, 0.4, 1.0]),
                ..ComputedStyle::default()
            }),
            text: None,
            text_metrics: None,
            text_layout: None,
            text_preserve_lines: false,
            z_index: 0,
            focused: false,
            editable: false,
            text_spans: Vec::new(),
            standard_visual: None,
            component_geometry: None,
            standard_visual_foreground: None,
            custom_render: None,
            drop_hover: None,
            document_text_selection: Vec::new(),
            document_text_selection_color: [0.0; 4],
            compositor: Default::default(),
        }
    }

    fn opacity_track(
        track: u64,
        target: u64,
        start_ms: u64,
        duration_ms: u64,
        from: f32,
        to: f32,
    ) -> MotionTrack {
        MotionTrack::transition(
            MotionTrackId::new(track).unwrap(),
            MotionTargetId::new(target).unwrap(),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(from),
            MotionValue::Scalar(to),
            MotionTiming::new(
                Duration::from_millis(start_ms),
                Duration::from_millis(duration_ms),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    fn transform_track(
        track: u64,
        target: u64,
        start_ms: u64,
        duration_ms: u64,
        from: PaintTransform,
        to: PaintTransform,
    ) -> MotionTrack {
        MotionTrack::transition(
            MotionTrackId::new(track).unwrap(),
            MotionTargetId::new(target).unwrap(),
            AnimatableProperty::Transform,
            MotionValue::Transform(from),
            MotionValue::Transform(to),
            MotionTiming::new(
                Duration::from_millis(start_ms),
                Duration::from_millis(duration_ms),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    fn width_track(track: u64, target: u64) -> MotionTrack {
        MotionTrack::transition(
            MotionTrackId::new(track).unwrap(),
            MotionTargetId::new(target).unwrap(),
            AnimatableProperty::Width,
            MotionValue::Scalar(10.0),
            MotionValue::Scalar(40.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    fn cache_gen(scene: &UiScene, node: StableNodeId) -> u64 {
        scene.compositor_layer(node).unwrap().cache_generation
    }

    #[test]
    fn opacity_overlay_promotes_after_hold_and_keeps_logical_style() {
        let mut scene = UiScene::new();
        let mut overlay = node(1, None, &[]);
        overlay.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(1.0),
            background: Some([0.1, 0.2, 0.3, 1.0]),
            ..LayoutStyle::default()
        });
        scene.apply_delta([overlay.clone()], []);
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(7, 1, 0, 100, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, Duration::ZERO, None);
        assert!(
            scene.compositor_layer(id(1)).is_none(),
            "promote hold is {}ms",
            LAYER_PROMOTE_HOLD.as_millis()
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        let layer = scene
            .compositor_layer(id(1))
            .expect("layer after promote hold");
        assert_eq!(layer.id, CompositorLayerId::from_node(id(1)));
        assert!((layer.opacity - 0.16).abs() > 0.0);
        assert_eq!(
            scene.nodes[&id(1)].source_style.layout.opacity,
            Some(1.0),
            "logical extracted style stays the target"
        );
        match store.applied_value(
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Opacity,
            LAYER_PROMOTE_HOLD,
        ) {
            Some(MotionValue::Scalar(value)) => assert!((value - layer.opacity).abs() < 1e-5),
            other => panic!("expected presentation opacity, got {other:?}"),
        }
    }

    #[test]
    fn time_only_frames_keep_layer_identity_and_primitive_cache() {
        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        let mut store = PresentationStore::new();
        store.insert(
            transform_track(
                3,
                1,
                0,
                200,
                PaintTransform::default(),
                PaintTransform {
                    e: 40.0,
                    ..PaintTransform::default()
                },
            ),
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        let first_id = scene.compositor_layer(id(1)).unwrap().id;
        let cache = scene.compositor_layer(id(1)).unwrap().cache_generation;
        let prims = scene.primitive_count();
        let first_e = scene.compositor_layer(id(1)).unwrap().transform.0[4];
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(16), None);
        let second = scene.compositor_layer(id(1)).unwrap();
        assert_eq!(second.id, first_id);
        assert_eq!(second.cache_generation, cache);
        assert_eq!(scene.primitive_count(), prims);
        assert_ne!(first_e, second.transform.0[4]);
    }

    #[test]
    fn short_overlay_does_not_promote_and_gap_reuses_identity() {
        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(1, 1, 0, 8, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, Duration::from_millis(8), None);
        assert!(
            scene.compositor_layer(id(1)).is_none(),
            "overlay shorter than promote hold {}ms must not create a layer",
            LAYER_PROMOTE_HOLD.as_millis()
        );

        store = PresentationStore::new();
        store.insert(
            opacity_track(2, 1, 0, 400, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        let identity = scene.compositor_layer(id(1)).unwrap().id;
        store = PresentationStore::new();
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(50), None);
        assert_eq!(
            scene.compositor_layer(id(1)).unwrap().id,
            identity,
            "first empty apply only starts PendingDemote; identity stays"
        );
        store.insert(
            opacity_track(3, 1, 80, 400, 0.2, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(50), None);
        assert_eq!(scene.compositor_layer(id(1)).unwrap().id, identity);

        scene.apply_presentation(
            &PresentationStore::new(),
            LAYER_PROMOTE_HOLD + Duration::from_millis(51),
            None,
        );
        assert!(scene.compositor_layer(id(1)).is_some());
        scene.apply_presentation(
            &PresentationStore::new(),
            LAYER_PROMOTE_HOLD
                + Duration::from_millis(51)
                + LAYER_DEMOTE_HOLD
                + Duration::from_millis(1),
            None,
        );
        assert!(scene.compositor_layer(id(1)).is_none());
    }

    #[test]
    fn nested_opacity_and_transform_compose() {
        let mut scene = UiScene::new();
        let mut parent = node(1, None, &[2]);
        let mut child = node(2, Some(1), &[]);
        parent.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(1.0),
            ..LayoutStyle::default()
        });
        child.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(1.0),
            ..LayoutStyle::default()
        });
        scene.apply_delta([parent, child], []);
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(1, 1, 0, 200, 0.5, 0.5),
            MotionValue::Scalar(1.0),
        );
        store.insert(
            opacity_track(2, 2, 0, 200, 0.5, 0.5),
            MotionValue::Scalar(1.0),
        );
        store.insert(
            transform_track(
                3,
                1,
                0,
                200,
                PaintTransform {
                    e: 10.0,
                    ..PaintTransform::default()
                },
                PaintTransform {
                    e: 10.0,
                    ..PaintTransform::default()
                },
            ),
            MotionValue::Transform(PaintTransform::default()),
        );
        store.insert(
            transform_track(
                4,
                2,
                0,
                200,
                PaintTransform {
                    f: 5.0,
                    ..PaintTransform::default()
                },
                PaintTransform {
                    f: 5.0,
                    ..PaintTransform::default()
                },
            ),
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        assert!(scene.compositor_layer(id(1)).is_some());
        assert!(scene.compositor_layer(id(2)).is_some());
        assert_eq!(
            scene.compositor_layer(id(2)).unwrap().parent,
            Some(CompositorLayerId::from_node(id(1)))
        );
        assert!((scene.composed_layer_opacity(id(2)) - 0.25).abs() < 1e-5);
        let composed = scene.composed_layer_transform(id(2));
        assert!(
            (composed.0[4] - 10.0).abs() < 1e-3,
            "parent translate X, got {:?}",
            composed.0
        );
        assert!(
            (composed.0[5] - 5.0).abs() < 1e-3,
            "child translate Y, got {:?}",
            composed.0
        );
    }

    #[test]
    fn a_layer_parent_change_refreshes_retained_visibility_bounds() {
        let mut scene = UiScene::new();
        let mut parent = node(1, None, &[2]);
        let mut child = node(2, Some(1), &[]);
        parent.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(1.0),
            ..LayoutStyle::default()
        });
        child.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(1.0),
            ..LayoutStyle::default()
        });
        scene.apply_delta([parent, child], []);
        let mut store = PresentationStore::new();
        store.insert(
            transform_track(
                1,
                2,
                0,
                200,
                PaintTransform {
                    e: 40.0,
                    ..PaintTransform::default()
                },
                PaintTransform {
                    e: 40.0,
                    ..PaintTransform::default()
                },
            ),
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        let viewport = crate::SceneRect {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 120.0,
        };
        let _ = scene.visible_operations(viewport);

        store.insert(
            transform_track(
                2,
                1,
                0,
                200,
                PaintTransform {
                    e: 30.0,
                    ..PaintTransform::default()
                },
                PaintTransform {
                    e: 30.0,
                    ..PaintTransform::default()
                },
            ),
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        assert_eq!(
            scene.compositor_layer(id(2)).unwrap().parent,
            Some(CompositorLayerId::from_node(id(1)))
        );
        assert!(
            scene.visible_operations(viewport).is_ok(),
            "a layer parent change must leave a usable visibility index"
        );
    }

    #[test]
    fn topology_change_invalidates_cache_time_does_not() {
        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(1, 1, 0, 400, 0.2, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        let before = cache_gen(&scene, id(1));
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(32), None);
        assert_eq!(cache_gen(&scene, id(1)), before);

        let child = node(2, Some(1), &[]);
        let mut parent = node(1, None, &[2]);
        parent.compositor.bindings = scene
            .compositor_layer(id(1))
            .unwrap()
            .bindings
            .iter()
            .map(|b| b.track_id)
            .collect();
        scene.apply_delta([parent, child], []);
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(32), None);
        assert!(
            cache_gen(&scene, id(1)) > before,
            "adding a child is topology; cache must bump"
        );
        let after_topo = cache_gen(&scene, id(1));
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD + Duration::from_millis(48), None);
        assert_eq!(cache_gen(&scene, id(1)), after_topo);
    }

    #[test]
    fn extract_binds_compositor_overlay_without_baking_presentation() {
        use nana_ui_runtime::{AnimationId, AnimationSpec, DocumentId, MutationQueue, UiWorld};
        let mut world = UiWorld::new();
        let node = id(1);
        let mut queue = MutationQueue::new();
        queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node,
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(1.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::ZERO);
        let extracted = world.extract_nodes(&[node]);
        assert_eq!(
            extracted[0].source_style.layout.opacity, None,
            "logical style stays the target (unset = 1.0)"
        );
        assert!(!extracted[0].compositor.bindings.is_empty());
        let mut scene = UiScene::new();
        scene.apply_delta(extracted, []);
        scene.apply_presentation(
            world.presentation_store(),
            LAYER_PROMOTE_HOLD,
            Some(world.motion_descriptors()),
        );
        let layer = scene.compositor_layer(node).expect("promoted overlay");
        let expected = world
            .motion_descriptors()
            .handle_for(layer.bindings[0].track_id)
            .unwrap_or(MotionHandle::NULL);
        assert_eq!(layer.bindings[0].generation, expected.generation());
        assert_eq!(layer.bindings[0].index, expected.index());
        assert!(layer.opacity < 1.0);
    }

    #[test]
    fn layout_width_overlay_does_not_promote() {
        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        let mut store = PresentationStore::new();
        store.insert(width_track(9, 1), MotionValue::Scalar(40.0));
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        assert!(scene.compositor_layer(id(1)).is_none());
    }

    #[test]
    fn surface_generation_is_recorded_on_layers() {
        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        scene.request_compositor_layer(id(1));
        scene.apply_presentation(&PresentationStore::new(), Duration::ZERO, None);
        scene.apply_presentation(&PresentationStore::new(), LAYER_PROMOTE_HOLD, None);
        assert_eq!(scene.compositor_layer(id(1)).unwrap().surface_generation, 0);
        scene.set_surface_generation(7);
        assert_eq!(scene.compositor_layer(id(1)).unwrap().surface_generation, 7);
        assert_eq!(scene.surface_generation(), 7);
    }

    #[test]
    fn hysteresis_locks_16ms_promote_and_120ms_demote() {
        assert_eq!(LAYER_PROMOTE_HOLD, Duration::from_millis(16));
        assert_eq!(LAYER_DEMOTE_HOLD, Duration::from_millis(120));

        let mut scene = UiScene::new();
        scene.apply_delta([node(1, None, &[])], []);
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(1, 1, 0, 400, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, Duration::from_millis(15), None);
        assert!(
            scene.compositor_layer(id(1)).is_none(),
            "15ms < 16ms promote hold must not create a layer"
        );
        scene.apply_presentation(&store, Duration::from_millis(16), None);
        let identity = scene
            .compositor_layer(id(1))
            .expect("layer at exactly 16ms")
            .id;

        scene.apply_presentation(&PresentationStore::new(), Duration::from_millis(16), None);
        assert_eq!(scene.compositor_layer(id(1)).unwrap().id, identity);
        scene.apply_presentation(&PresentationStore::new(), Duration::from_millis(18), None);
        assert!(
            scene.compositor_layer(id(1)).is_some(),
            "2ms after empty apply is still inside the 120ms demote hold"
        );
        scene.apply_presentation(&PresentationStore::new(), Duration::from_millis(135), None);
        assert!(
            scene.compositor_layer(id(1)).is_some(),
            "119ms after empty apply must keep the layer; DEMOTE=2ms would have dropped it"
        );
        scene.apply_presentation(&PresentationStore::new(), Duration::from_millis(136), None);
        assert!(
            scene.compositor_layer(id(1)).is_none(),
            "120ms after empty apply must demote"
        );
    }

    #[test]
    fn compositor_request_clear_demotes_and_stops_tick() {
        use nana_ui_runtime::{DocumentId, MutationQueue, UiWorld};
        let mut world = UiWorld::new();
        let node = id(1);
        let mut queue = MutationQueue::new();
        queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
        world.commit(queue).unwrap();
        world.request_compositor_layer(node);
        let mut scene = UiScene::new();
        scene.apply_delta(world.extract_nodes(&[node]), []);
        assert!(scene.compositor_layer_requested(node));
        scene.apply_presentation(
            world.presentation_store(),
            Duration::ZERO,
            Some(world.motion_descriptors()),
        );
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(16),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_layer(node).is_some());

        world.clear_compositor_layer_request(node);
        scene.apply_delta(world.extract_nodes(&[node]), []);
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(16),
            Some(world.motion_descriptors()),
        );
        assert!(
            !scene.compositor_layer_requested(node),
            "extract with request_layer=false must drop the scene request"
        );
        assert!(
            scene.compositor_layer(node).is_some(),
            "clear starts demote hold; the layer stays through 120ms"
        );
        assert!(scene.compositor_needs_tick());

        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(18),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_layer(node).is_some());
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(135),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_layer(node).is_some());
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(136),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_layer(node).is_none());
        assert!(
            !scene.compositor_needs_tick(),
            "requested and phases must drain after demote"
        );
    }

    /// The packed tables are cached on the store's epoch and capacity, both of
    /// which start the same in every store. Re-pointing a scene at another
    /// document's motion would otherwise keep serving the first one's slab.
    #[test]
    fn repointing_a_scene_at_another_store_repacks_the_slab() {
        use nana_ui_runtime::{AnimationId, AnimationSpec, DocumentId, MutationQueue, UiWorld};
        let animated = |to: f32| {
            let node = id(1);
            let mut world = UiWorld::new();
            let mut queue = MutationQueue::new();
            queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
            queue.start_animation(
                AnimationSpec::new(
                    AnimationId::new(1).unwrap(),
                    node,
                    Duration::ZERO,
                    Duration::from_millis(400),
                    Duration::from_millis(16),
                    Easing::Linear,
                )
                .with_property(AnimatableProperty::Opacity)
                .with_range(
                    MotionValue::Scalar(0.0),
                    MotionTo::Value(MotionValue::Scalar(to)),
                ),
            );
            world.commit(queue).unwrap();
            world.advance_animations(Duration::ZERO);
            world
        };
        let first = animated(1.0);
        let second = animated(0.25);
        assert_eq!(
            first.motion_descriptors().structure_epoch(),
            second.motion_descriptors().structure_epoch(),
            "independent documents do reach the same epoch"
        );

        let mut scene = UiScene::new();
        scene.apply_delta(first.extract_nodes(&[id(1)]), []);
        scene.apply_presentation(
            first.presentation_store(),
            Duration::from_millis(16),
            Some(first.motion_descriptors()),
        );
        let packed = scene.motion_gpu_descriptors().to_vec();

        scene.apply_presentation(
            second.presentation_store(),
            Duration::from_millis(16),
            Some(second.motion_descriptors()),
        );
        assert_eq!(
            scene.motion_gpu_source(),
            second.motion_descriptors().source()
        );
        assert_ne!(
            scene.motion_gpu_descriptors(),
            packed.as_slice(),
            "the other store's descriptors must replace the packed slab"
        );
    }

    #[test]
    fn flush_steady_compositor_frames_do_not_rebuild_descriptor_slab() {
        use nana_ui_runtime::{AnimationId, AnimationSpec, DocumentId, MutationQueue, UiWorld};
        let mut world = UiWorld::new();
        let node = id(1);
        let mut queue = MutationQueue::new();
        queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node,
                Duration::ZERO,
                Duration::from_millis(400),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(1.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::ZERO);
        let mut scene = UiScene::new();
        scene.apply_delta(world.extract_nodes(&[node]), []);
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(16),
            Some(world.motion_descriptors()),
        );
        let layer = scene.compositor_layer(node).expect("promoted");
        let epoch = world.motion_descriptors().structure_epoch();
        let capacity = world.motion_descriptors().slot_capacity();
        let generation = layer.bindings[0].generation;
        let binding_index = layer.bindings[0].index;
        let cache = layer.cache_generation;
        let identity = layer.id;

        world.advance_animations(Duration::from_millis(32));
        scene.apply_presentation(
            world.presentation_store(),
            world.animation_now(),
            Some(world.motion_descriptors()),
        );
        world.advance_animations(Duration::from_millis(48));
        scene.apply_presentation(
            world.presentation_store(),
            world.animation_now(),
            Some(world.motion_descriptors()),
        );
        let second = scene.compositor_layer(node).expect("steady layer");
        assert_eq!(second.id, identity);
        assert_eq!(second.cache_generation, cache);
        assert_eq!(second.bindings[0].generation, generation);
        assert_eq!(world.motion_descriptors().structure_epoch(), epoch);
        assert_eq!(world.motion_descriptors().slot_capacity(), capacity);
        assert_eq!(
            scene.motion_gpu_structure_epoch(),
            world.motion_descriptors().structure_epoch()
        );
        let packed = scene.motion_gpu_descriptors();
        assert!(packed[binding_index as usize].is_live());
        scene.apply_presentation(
            world.presentation_store(),
            world.animation_now(),
            Some(world.motion_descriptors()),
        );
        assert_eq!(
            scene.motion_gpu_structure_epoch(),
            world.motion_descriptors().structure_epoch(),
            "steady timestamps must not rebuild the GPU pack"
        );
        let (_, opacity_id) = scene.compositor_gpu_motion_ids(node);
        assert_eq!(opacity_id, binding_index.saturating_add(1));
        let paint = scene.compositor_paint_opacity(node, 1.0);
        let encoded = scene.compositor_gpu_encode_opacity(node, paint);
        let layer = scene.compositor_layer(node).expect("layer");
        assert!((encoded * layer.opacity - paint).abs() < 1e-5);
        assert_eq!(
            layer.bindings[0].handle(),
            MotionHandle::from_parts(binding_index, generation)
        );
    }

    #[test]
    fn non_quad_keeps_cpu_overlay_under_ancestor_opacity() {
        use nana_ui_runtime::{
            AnimationId, AnimationSpec, DocumentId, MutationQueue, TextContent, UiWorld,
        };
        let parent = id(1);
        let child = id(2);
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(parent, DocumentId::new(1).unwrap(), NodeKind::Document);
        queue.create(
            child,
            DocumentId::new(1).unwrap(),
            NodeKind::Element { tag: "span".into() },
        );
        queue.insert(parent, child, None);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                parent,
                Duration::ZERO,
                Duration::from_millis(400),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(1.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::ZERO);
        let mut extracted = world.extract_nodes(&[parent, child]);
        for node in &mut extracted {
            if node.id == parent {
                node.source_style.layout = Arc::new(LayoutStyle {
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    ..LayoutStyle::default()
                });
                node.style = Arc::new(ComputedStyle {
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    ..ComputedStyle::default()
                });
                node.layout = LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 80.0,
                    height: 80.0,
                };
            }
            if node.id == child {
                node.text = Some(TextContent { value: "Hi".into() });
                node.style = Arc::new(ComputedStyle {
                    color: Some([1.0, 1.0, 1.0, 1.0]),
                    font_size: 16.0,
                    ..ComputedStyle::default()
                });
                node.layout = LayoutBox {
                    x: 8.0,
                    y: 8.0,
                    width: 64.0,
                    height: 24.0,
                };
            }
        }
        let mut scene = UiScene::new();
        scene.apply_delta(extracted, []);
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(200),
            Some(world.motion_descriptors()),
        );
        let layer = scene.compositor_layer(parent).expect("promoted");
        assert!(
            layer.opacity > 0.2 && layer.opacity < 0.8,
            "mid-run overlay {}",
            layer.opacity
        );

        let mut saw_quad = false;
        let mut saw_text = false;
        for primitive in scene.primitives() {
            let draw = scene.draw_primitive(primitive.id).expect("draw");
            let encode = scene.compositor_paint_encode(
                draw.node,
                &draw.kind,
                draw.transform,
                draw.paint_opacity,
            );
            match &draw.kind {
                crate::ScenePrimitiveKind::Text { .. } => {
                    saw_text = true;
                    assert!(
                        (encode.opacity - draw.paint_opacity).abs() < 1e-5,
                        "Text must keep CPU overlay {}, got {}",
                        draw.paint_opacity,
                        encode.opacity
                    );
                    assert_eq!(encode.motion_ids, (0, 0));
                    assert!(
                        (draw.paint_opacity - layer.opacity).abs() < 0.05,
                        "drawn text opacity {} must follow presentation {}",
                        draw.paint_opacity,
                        layer.opacity
                    );
                    assert!(
                        (encode.opacity - 1.0).abs() > 0.2,
                        "stripping overlay for Text would pin encode at logical 1.0"
                    );
                }
                crate::ScenePrimitiveKind::Quad { .. } => {
                    saw_quad = true;
                    assert_ne!(encode.motion_ids.1, 0, "Quad must receive GPU opacity id");
                    assert!(
                        (encode.opacity - draw.paint_opacity).abs() > 0.05,
                        "Quad must strip CPU overlay for shader evaluate"
                    );
                }
                _ => {}
            }
        }
        assert!(saw_quad && saw_text, "parent quad and child text required");
    }

    #[test]
    fn a_painted_node_follows_its_compositor_animation_like_built_in_paint() {
        use nana_ui_runtime::{
            AnimationId, AnimationSpec, DocumentId, MutationQueue, PaintOp, PaintPath,
            PaintRecording, ResolvedPaint, UiWorld,
        };
        let node = id(1);
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node,
                Duration::ZERO,
                Duration::from_millis(400),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(1.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::ZERO);
        let mut extracted = world.extract_nodes(&[node]);
        // A triangle stays a path mesh (a rectangle would be drawn as a quad).
        let mut square = PaintPath::new();
        square
            .move_to(0.0, 0.0)
            .line_to(20.0, 0.0)
            .line_to(0.0, 20.0)
            .close();
        extracted[0].layout = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 80.0,
        };
        extracted[0].style = Arc::new(ComputedStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..ComputedStyle::default()
        });
        extracted[0].custom_paint = Some(Arc::new(PaintRecording {
            behind_children: vec![
                PaintOp::DrawDefault,
                PaintOp::FillPath {
                    path: Arc::new(square),
                    paint: ResolvedPaint::Solid([0.0, 0.0, 1.0, 1.0]),
                },
            ],
            over_children: Vec::new(),
        }));
        let mut scene = UiScene::new();
        scene.apply_delta(extracted, []);
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(200),
            Some(world.motion_descriptors()),
        );
        let layer = scene.compositor_layer(node).expect("promoted");
        let mut saw_path = false;
        for primitive in scene.primitives() {
            let draw = scene.draw_primitive(primitive.id).expect("draw");
            let encode = scene.compositor_paint_encode(
                draw.node,
                &draw.kind,
                draw.transform,
                draw.paint_opacity,
            );
            if let crate::ScenePrimitiveKind::Path { .. } = &draw.kind {
                saw_path = true;
                assert_eq!(encode.motion_ids, (0, 0), "a mesh keeps CPU presentation");
                assert!(
                    (encode.opacity - layer.opacity).abs() < 0.05,
                    "painted {} must follow the animation at {}",
                    encode.opacity,
                    layer.opacity
                );
            }
        }
        assert!(saw_path);
    }

    #[test]
    fn opacity_groups_are_not_compositor_layers() {
        let mut scene = UiScene::new();
        let mut parent = node(1, None, &[2]);
        parent.source_style.layout = Arc::new(LayoutStyle {
            opacity: Some(0.5),
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..LayoutStyle::default()
        });
        scene.apply_delta([parent, node(2, Some(1), &[])], []);
        assert!(!scene.opacity_groups(id(2)).is_empty());
        assert!(scene.compositor_layers().next().is_none());
    }

    #[test]
    fn a_deeper_tree_does_not_change_what_a_group_query_answers() {
        // The memo is what keeps `opacity_groups` off the per-primitive
        // ancestor walk. It may not change the answer, and it must notice when
        // a style below it did.
        let mut scene = UiScene::new();
        let mut chain = Vec::new();
        for level in 1..=12u64 {
            let children: Vec<u64> = if level == 12 { vec![] } else { vec![level + 1] };
            let mut entry = node(level, (level > 1).then(|| level - 1), &children);
            if level == 1 {
                entry.source_style.layout = Arc::new(LayoutStyle {
                    opacity: Some(0.5),
                    background: Some([1.0, 0.0, 0.0, 1.0]),
                    ..LayoutStyle::default()
                });
            }
            chain.push(entry);
        }
        scene.apply_delta(chain, []);
        let deep = scene.opacity_groups(id(12));
        assert_eq!(deep.len(), 1, "one translucent ancestor, however deep");
        assert_eq!(deep[0].node, id(1));
        // Asked twice, answered from the memo the second time, and the same.
        assert_eq!(scene.opacity_groups(id(12)).to_vec(), deep.to_vec());
        // The ancestor stops being a group; the memo must not outlive that.
        let mut opaque = node(1, None, &[2]);
        opaque.source_style.layout = Arc::new(LayoutStyle {
            background: Some([1.0, 0.0, 0.0, 1.0]),
            ..LayoutStyle::default()
        });
        scene.apply_delta([opaque], []);
        assert!(
            scene.opacity_groups(id(12)).is_empty(),
            "a style change upstream has to reach every node under it"
        );
    }

    #[test]
    fn compositor_needs_tick_does_not_require_cpu_frame_interval() {
        use nana_ui_runtime::{AnimationId, AnimationSpec, DocumentId, MutationQueue, UiWorld};
        let mut world = UiWorld::new();
        let node = id(1);
        let mut queue = MutationQueue::new();
        queue.create(node, DocumentId::new(1).unwrap(), NodeKind::Document);
        queue.start_animation(
            AnimationSpec::new(
                AnimationId::new(1).unwrap(),
                node,
                Duration::ZERO,
                Duration::from_millis(400),
                Duration::from_millis(16),
                Easing::Linear,
            )
            .with_property(AnimatableProperty::Opacity)
            .with_range(
                MotionValue::Scalar(0.0),
                MotionTo::Value(MotionValue::Scalar(1.0)),
            ),
        );
        world.commit(queue).unwrap();
        world.advance_animations(Duration::ZERO);
        let mut scene = UiScene::new();
        scene.apply_delta(world.extract_nodes(&[node]), []);
        scene.apply_presentation(
            world.presentation_store(),
            Duration::from_millis(16),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_needs_tick());
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(400))
        );
        world.sync_presentation_clock(Duration::from_millis(80));
        scene.apply_presentation(
            world.presentation_store(),
            world.animation_now(),
            Some(world.motion_descriptors()),
        );
        assert!(scene.compositor_needs_tick());
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(400)),
            "host compositor ticks must not invent CPU frame-interval deadlines"
        );
        assert_eq!(
            world
                .advance_animations(Duration::from_millis(80))
                .animation_deadlines_scanned,
            0
        );
    }

    /// Three nodes, the middle one carrying a compositor transform animation,
    /// so the leaf under it is a retained descendant that moves without ever
    /// being re-extracted.
    fn animated_subtree(store: &PresentationStore, now: Duration) -> UiScene {
        let mut scene = UiScene::new();
        scene.apply_delta(
            [
                node(1, None, &[2]),
                node(2, Some(1), &[3]),
                node(3, Some(2), &[]),
            ],
            [],
        );
        scene.apply_presentation(store, now, None);
        scene
    }

    /// Slide `target` down one scene unit per millisecond, so the time a tick
    /// names is also the y the subtree is at.
    fn slide_down(track: u64, target: u64, distance: f32) -> MotionTrack {
        transform_track(
            track,
            target,
            0,
            distance as u64,
            PaintTransform::default(),
            PaintTransform {
                f: distance,
                ..PaintTransform::default()
            },
        )
    }

    /// What answers for a viewport band too thin to hold the 80-unit nodes at
    /// rest, so its contents are only what the animation carried into it.
    fn band(scene: &UiScene, y: f32) -> Vec<crate::RenderOperation> {
        let band = crate::SceneRect {
            x: 0.0,
            y,
            width: 100.0,
            height: 20.0,
        };
        scene.visible_operations(band).expect("index")
    }

    fn everything(scene: &UiScene) -> Vec<crate::RenderOperation> {
        let all = crate::SceneRect {
            x: 0.0,
            y: 0.0,
            width: 10_000.0,
            height: 10_000.0,
        };
        scene.visible_operations(all).expect("index")
    }

    /// Whether the scroll fast path's shift is still the last thing to have
    /// touched these bounds — false as soon as anything re-derives them.
    fn translated(scene: &UiScene) -> bool {
        scene
            .visibility
            .get()
            .expect("the index survives a scroll")
            .translated()
    }

    /// A compositor layer moves its own projection and every retained
    /// descendant's, and nothing around it is re-extracted — so the index gets
    /// no changed set to refresh from, and no delta to be audited by either.
    /// It still has to answer where the animation put the subtree: a bound
    /// left behind culls a subtree that is on screen, one left ahead draws one
    /// that is not, and a demote drops the whole animation at once.
    #[test]
    fn an_animated_layer_culls_its_subtree_where_the_animation_put_it() {
        let ms = Duration::from_millis;
        let mut store = PresentationStore::new();
        store.insert(
            slide_down(9, 2, 400.0),
            MotionValue::Transform(PaintTransform::default()),
        );

        // Derived while the subtree is near the top, then animated down into a
        // band it was never derived against.
        let mut scene = animated_subtree(&store, ms(16));
        let plan = scene.frame_plan().expect("plan");
        assert_eq!(everything(&scene).len(), 3);
        assert!(
            band(&scene, 150.0).is_empty(),
            "the band was meant to be empty before the animation reaches it"
        );
        scene.apply_presentation(&store, ms(116), None);
        assert!(
            Arc::ptr_eq(&plan, &scene.frame_plan().expect("plan")),
            "a presentation tick rebuilt the frame plan"
        );

        let arrived = animated_subtree(&store, ms(116));
        assert_eq!(
            band(&arrived, 150.0).len(),
            2,
            "the band was meant to hold the animated node and its leaf"
        );
        assert_eq!(
            band(&scene, 150.0),
            band(&arrived, 150.0),
            "a retained index culled the subtree where the animation left it"
        );
        assert_eq!(everything(&scene), everything(&arrived));

        // An ordinary repaint keeps the index and audits it against a fresh
        // build; mid-animation the two have to agree bit for bit.
        let mut repainted = node(3, Some(2), &[]);
        repainted.style = Arc::new(ComputedStyle {
            background: Some([0.9, 0.1, 0.1, 1.0]),
            ..ComputedStyle::default()
        });
        scene.apply_delta([repainted], []);
        assert!(
            scene.visibility.get().is_some(),
            "a repaint dropped the visibility index"
        );

        // Past the band: a bound the animation has left must stop answering.
        scene.apply_presentation(&store, ms(316), None);
        assert!(
            band(&scene, 150.0).is_empty(),
            "a retained index drew the subtree where the animation no longer is"
        );

        // A demote drops the presented transform in one step rather than
        // moving it, and the subtree snaps back to its logical box.
        scene.apply_presentation(&store, ms(600), None);
        scene.apply_presentation(&store, ms(600) + LAYER_DEMOTE_HOLD, None);
        assert!(
            scene.compositor_layer(id(2)).is_none(),
            "the layer was meant to demote once the animation ended"
        );
        assert_eq!(
            band(&scene, 0.0).len(),
            3,
            "a demoted layer left its subtree's bounds where the animation was"
        );
        assert!(band(&scene, 150.0).is_empty());
    }

    /// The refresh follows what a layer *presents*, not the fact that a layer
    /// changed at all: an overlay that only fades rewrites its layer every
    /// frame and moves no bound, and re-deriving the whole scene for it is the
    /// cost the retained index exists to avoid. Promoting and demoting do
    /// re-derive — they change *which* transform `resolved_local_transform`
    /// answers with, and a node under a closed 3D context is where those two
    /// are not the same projection at all — but hysteresis makes them once per
    /// animation rather than once per frame.
    #[test]
    fn the_bounds_refresh_follows_what_a_layer_presents() {
        let scrolled = |y: f32| {
            let mut container = node(1, None, &[2]);
            container.scroll_offset.y = y;
            container
        };
        // Derive the index and scroll it, so `translated` reads true until
        // something re-derives it again.
        let mark = |scene: &mut UiScene, y: f32| {
            everything(scene);
            scene.apply_delta([scrolled(y)], []);
            assert!(translated(scene), "the scroll fast path did not run");
        };
        let mut scene = UiScene::new();
        scene.apply_delta([scrolled(0.0), node(2, Some(1), &[])], []);
        // Opacity only: this layer carries the node's own logical transform,
        // so nothing below is caught by comparing transforms.
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(4, 1, 0, 400, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );

        mark(&mut scene, 10.0);
        scene.apply_presentation(&store, Duration::from_millis(16), None);
        assert!(
            scene.compositor_layer(id(1)).is_some(),
            "the fade did not promote a layer"
        );
        assert!(
            !translated(&scene),
            "a promote left the retained bounds alone"
        );

        mark(&mut scene, 20.0);
        for tick in [32, 48, 64] {
            scene.apply_presentation(&store, Duration::from_millis(tick), None);
            assert!(
                translated(&scene),
                "a steady fade re-derived every bound in the scene"
            );
        }

        // The same layer, now moving: that one does owe a re-derivation.
        store.insert(
            slide_down(5, 1, 400.0),
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, Duration::from_millis(80), None);
        assert!(
            !translated(&scene),
            "a moving layer left the retained bounds where they were"
        );

        // And back off the layer, which is a move of its own.
        mark(&mut scene, 30.0);
        let gone = Duration::from_millis(500);
        scene.apply_presentation(&store, gone, None);
        assert!(
            translated(&scene),
            "an ineligible frame is not yet a demote"
        );
        scene.apply_presentation(&store, gone + LAYER_DEMOTE_HOLD, None);
        assert!(
            scene.compositor_layer(id(1)).is_none(),
            "the layer did not demote"
        );
        assert!(
            !translated(&scene),
            "a demote left the retained bounds alone"
        );
    }

    /// `perspective` / `preserve-3d` on an ancestor fails the descendant's
    /// `matrix3d` closed — the engine refuses the projection rather than
    /// approximating it. A compositor layer must not be a way back in: the
    /// snapshot behind it is built from one node, with no view of the
    /// ancestors the rule is about, and a fade alone is enough to promote one.
    #[test]
    fn a_layer_does_not_reopen_a_closed_3d_context() {
        use nana_ui_core::{LayoutStyle, PaintMat4};
        let mut parent = node(1, None, &[2]);
        parent.source_style.layout = Arc::new(LayoutStyle {
            css_perspective: Some(800.0),
            ..LayoutStyle::default()
        });
        let mut child = node(2, Some(1), &[]);
        child.source_style.layout = Arc::new(LayoutStyle {
            transform_3d: Some(
                PaintMat4::perspective(800.0)
                    .unwrap()
                    .then(PaintMat4::rotate_y(30_f32.to_radians())),
            ),
            ..LayoutStyle::default()
        });
        let mut scene = UiScene::new();
        scene.apply_delta([parent, child], []);
        let primitive = scene.primitives().find(|p| p.node == id(2)).unwrap().id;
        let closed = scene.draw_primitive(primitive).unwrap().transform;
        assert_eq!(
            closed,
            AffineTransform::IDENTITY,
            "the closed context was meant to refuse the 3D transform outright"
        );

        // An opacity overlay, which carries no transform of its own.
        let mut store = PresentationStore::new();
        store.insert(
            opacity_track(11, 2, 0, 400, 0.0, 1.0),
            MotionValue::Scalar(1.0),
        );
        scene.apply_presentation(&store, LAYER_PROMOTE_HOLD, None);
        assert!(
            scene.compositor_layer(id(2)).is_some(),
            "the fade did not promote a layer"
        );
        assert_eq!(
            scene.draw_primitive(primitive).unwrap().transform,
            closed,
            "a promoted layer reopened a closed 3D context"
        );

        // And one that does carry a transform: refused on the CPU and on the
        // GPU, which applies the overlay in the node's place off these ids
        // alone and would otherwise put the projection back a frame later.
        let sliding = slide_down(12, 2, 400.0);
        let mut descriptors = MotionDescriptorStore::new();
        descriptors.bind(&sliding).expect("descriptor slot");
        store.insert(sliding, MotionValue::Transform(PaintTransform::default()));
        let moving = LAYER_PROMOTE_HOLD + Duration::from_millis(64);
        scene.apply_presentation(&store, moving, Some(&descriptors));
        let draw = scene.draw_primitive(primitive).expect("draw");
        assert!(
            draw.kind.evaluates_compositor_motion_on_gpu(),
            "this node was meant to be one the shader evaluates"
        );
        assert_eq!(
            draw.transform, closed,
            "a presented transform reopened a closed 3D context"
        );
        let encode = scene.compositor_paint_encode(
            draw.node,
            &draw.kind,
            draw.transform,
            draw.paint_opacity,
        );
        assert_eq!(
            encode.motion_ids.0, 0,
            "the shader was handed a transform the closed context refused"
        );
        assert_eq!(encode.transform, closed);

        // A sibling with no `matrix3d` of its own is refused nothing: the rule
        // is about this node's own 3D transform, not about the ancestor.
        let mut plain = node(3, Some(1), &[]);
        plain.source_style.layout = Arc::new(LayoutStyle::default());
        scene.apply_delta([plain], []);
        let sliding_plain = slide_down(13, 3, 400.0);
        descriptors.bind(&sliding_plain).expect("descriptor slot");
        store.insert(
            sliding_plain,
            MotionValue::Transform(PaintTransform::default()),
        );
        scene.apply_presentation(&store, moving, Some(&descriptors));
        assert_ne!(
            scene.compositor_gpu_motion_ids(id(3)).0,
            0,
            "an ordinary node lost its GPU transform to the 3D rule"
        );
    }
}
