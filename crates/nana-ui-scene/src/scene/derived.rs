//! Consolidated derived scene snapshot (`SceneDerived`).
//!
//! Replaces the six per-scene `Mutex` caches and three `OnceLock` fields with
//! one `Arc<SceneDerived>` guarded by a single mutex. Projection tables are
//! populated in one iterative pre-order pass over the node tree (safe up to
//! `MAX_TREE_DEPTH` on small thread stacks) and keyed by
//! `(placement_epoch, attribute_epoch, presentation_epoch)`.

use std::collections::HashMap;
use std::sync::Arc;

use super::ancestors::{
    Ancestor3d, AncestorFold, AncestorState, MAX_ANCESTOR_DEPTH, clip_chain_parents, clip_link,
    is_workspace_resize_handle,
};
use super::attributes::{DrawAttributes, inverse, same_drawn_transform};
use super::groups::{
    dest_group, empty_opacity_groups, inherited_visual_opacity_groups, is_opacity_group,
    local_opacity, visual_is_opacity_group,
};
use super::placement::record_on_page;
use super::visibility::VisibilityIndex;
use super::*;

/// Consolidated epoch counters tracked on [`UiScene`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct SceneEpochs {
    pub(super) placement: u64,
    pub(super) attribute: u64,
    pub(super) presentation: u64,
}

impl SceneEpochs {
    pub(super) const fn derived_key(self) -> SceneDerivedKey {
        SceneDerivedKey {
            placement_epoch: self.placement,
            attribute_epoch: self.attribute,
            presentation_epoch: self.presentation,
        }
    }
}

/// Version key for [`SceneProjectionTables`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct SceneDerivedKey {
    pub(super) placement_epoch: u64,
    pub(super) attribute_epoch: u64,
    pub(super) presentation_epoch: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct NodeGpuMotion {
    pub(super) transform_id: u32,
    pub(super) node_opacity: (u32, Option<f32>),
    pub(super) scrim_opacity: (u32, Option<f32>),
    pub(super) overlay_opacity: (u32, Option<f32>),
}

/// Immutable per-node & per-primitive tables derived in one iterative pre-order
/// pass over `UiScene::nodes` for [`Self::key`].
#[derive(Debug, Clone)]
pub(super) struct SceneProjectionTables {
    pub(super) key: SceneDerivedKey,
    pub(super) node_index: NodeMap<usize>,
    pub(super) placed_origins: Vec<[f32; 2]>,
    pub(super) draw_parent_states: Vec<AncestorState>,
    pub(super) draw_attributes: Vec<Option<DrawAttributes>>,
    pub(super) placed_clips: HashMap<PrimitiveId, Arc<[ClipRegion]>>,
    pub(super) node_opacity_groups: Vec<Arc<[OpacityGroup]>>,
    pub(super) inherited_node_opacity_groups: Vec<Arc<[OpacityGroup]>>,
    pub(super) overlay_opacity_groups: Vec<Option<Arc<[OpacityGroup]>>>,
    pub(super) node_layer_factor: Vec<f32>,
    pub(super) node_inherited_layer_factor: Vec<f32>,
    pub(super) overlay_layer_factor: Vec<f32>,
    pub(super) overlay_inherited_layer_factor: Vec<f32>,
    pub(super) gpu_motion: Vec<NodeGpuMotion>,
}

/// All derived scene data under `UiScene::derived`.
#[derive(Debug, Clone, Default)]
pub(super) struct SceneDerived {
    pub(super) projection: Option<Arc<SceneProjectionTables>>,
    pub(super) frame_plan: Option<Arc<FramePlan>>,
    pub(super) visibility: Option<VisibilityIndex>,
    pub(super) extended_color: Option<bool>,
}

impl SceneProjectionTables {
    pub(super) fn placed_origin(&self, id: StableNodeId) -> Option<[f32; 2]> {
        self.node_index
            .get(&id)
            .map(|&idx| self.placed_origins[idx])
    }

    pub(super) fn draw_parent_state(&self, id: StableNodeId) -> Option<&AncestorState> {
        self.node_index
            .get(&id)
            .map(|&idx| &self.draw_parent_states[idx])
    }

    pub(super) fn draw_attributes(&self, id: StableNodeId) -> Option<&DrawAttributes> {
        self.node_index
            .get(&id)
            .and_then(|&idx| self.draw_attributes[idx].as_ref())
    }

    pub(super) fn opacity_groups(&self, vn: VisualNode) -> Arc<[OpacityGroup]> {
        if self.node_opacity_groups.is_empty() {
            return empty_opacity_groups();
        }
        match vn {
            VisualNode::Node(id) => self
                .node_index
                .get(&id)
                .map_or_else(empty_opacity_groups, |&idx| {
                    Arc::clone(&self.node_opacity_groups[idx])
                }),
            VisualNode::Overlay(id) => self
                .node_index
                .get(&id)
                .and_then(|&idx| self.overlay_opacity_groups[idx].as_ref())
                .map_or_else(empty_opacity_groups, Arc::clone),
        }
    }

    pub(super) fn inherited_node_opacity_groups(&self, id: StableNodeId) -> Arc<[OpacityGroup]> {
        if self.inherited_node_opacity_groups.is_empty() {
            return empty_opacity_groups();
        }
        self.node_index
            .get(&id)
            .map_or_else(empty_opacity_groups, |&idx| {
                Arc::clone(&self.inherited_node_opacity_groups[idx])
            })
    }

    pub(super) fn layer_factor(&self, vn: VisualNode, own: bool) -> f32 {
        if self.node_layer_factor.is_empty() {
            return 1.0;
        }
        let Some(&idx) = self.node_index.get(&vn.node_id()) else {
            return 1.0;
        };
        match (vn, own) {
            (VisualNode::Node(_), true) => self.node_layer_factor[idx],
            (VisualNode::Node(_), false) => self.node_inherited_layer_factor[idx],
            (VisualNode::Overlay(_), true) => self.overlay_layer_factor[idx],
            (VisualNode::Overlay(_), false) => self.overlay_inherited_layer_factor[idx],
        }
    }

    pub(super) fn gpu_motion_ids(&self, vn: VisualNode, own_opacity: bool) -> (u32, u32) {
        if self.gpu_motion.is_empty() {
            return (0, 0);
        }
        let Some(&idx) = self.node_index.get(&vn.node_id()) else {
            return (0, 0);
        };
        let entry = self.gpu_motion[idx];
        match vn {
            VisualNode::Node(_) => {
                let op = if own_opacity {
                    entry.node_opacity.0
                } else {
                    entry.scrim_opacity.0
                };
                (entry.transform_id, op)
            }
            VisualNode::Overlay(_) => (
                0,
                if own_opacity {
                    entry.overlay_opacity.0
                } else {
                    0
                },
            ),
        }
    }

    pub(super) fn gpu_opacity_divisor(
        &self,
        vn: VisualNode,
        own_opacity: bool,
    ) -> (u32, Option<f32>) {
        if self.gpu_motion.is_empty() {
            return (0, None);
        }
        let Some(&idx) = self.node_index.get(&vn.node_id()) else {
            return (0, None);
        };
        let entry = self.gpu_motion[idx];
        match vn {
            VisualNode::Node(_) => {
                if own_opacity {
                    entry.node_opacity
                } else {
                    entry.scrim_opacity
                }
            }
            VisualNode::Overlay(_) => {
                if own_opacity {
                    entry.overlay_opacity
                } else {
                    (0, None)
                }
            }
        }
    }
}

impl UiScene {
    pub(super) fn take_derived(&mut self) -> Option<SceneDerived> {
        self.derived
            .get_mut()
            .expect("scene derived")
            .take()
            .map(|arc| Arc::unwrap_or_clone(arc))
    }

    pub(super) fn store_derived(&mut self, derived: SceneDerived) {
        let keep = derived.projection.is_some()
            || derived.frame_plan.is_some()
            || derived.visibility.is_some()
            || derived.extended_color.is_some();
        *self.derived.get_mut().expect("scene derived") = keep.then(|| Arc::new(derived));
    }

    pub(super) fn invalidate_derived_all(&mut self) {
        *self.derived.get_mut().expect("scene derived") = None;
    }

    #[cfg(test)]
    pub(super) fn has_visibility_index(&self) -> bool {
        self.derived
            .lock()
            .expect("scene derived")
            .as_ref()
            .is_some_and(|d| d.visibility.is_some())
    }

    pub(super) fn derived_frame_plan(&self) -> Option<Arc<FramePlan>> {
        self.derived
            .lock()
            .expect("scene derived")
            .as_ref()
            .and_then(|d| d.frame_plan.clone())
    }

    pub(super) fn set_derived_frame_plan(&self, plan: Arc<FramePlan>) {
        let mut guard = self.derived.lock().expect("scene derived");
        let derived = Arc::make_mut(guard.get_or_insert_with(|| Arc::new(SceneDerived::default())));
        derived.frame_plan = Some(plan);
    }

    pub(super) fn derived_visibility(&self) -> Option<VisibilityIndex> {
        self.derived
            .lock()
            .expect("scene derived")
            .as_ref()
            .and_then(|d| d.visibility.clone())
    }

    pub(super) fn set_derived_visibility(&self, visibility: VisibilityIndex) {
        let mut guard = self.derived.lock().expect("scene derived");
        let derived = Arc::make_mut(guard.get_or_insert_with(|| Arc::new(SceneDerived::default())));
        derived.visibility = Some(visibility);
    }

    pub(super) fn derived_extended_color(&self) -> Option<bool> {
        self.derived
            .lock()
            .expect("scene derived")
            .as_ref()
            .and_then(|d| d.extended_color)
    }

    pub(super) fn set_derived_extended_color(&self, extended: bool) {
        let mut guard = self.derived.lock().expect("scene derived");
        let derived = Arc::make_mut(guard.get_or_insert_with(|| Arc::new(SceneDerived::default())));
        derived.extended_color = Some(extended);
    }

    pub(super) fn derived_projection(&self) -> Arc<SceneProjectionTables> {
        let key = self.epochs.derived_key();
        {
            let guard = self.derived.lock().expect("scene derived");
            if let Some(tables) = guard
                .as_ref()
                .and_then(|d| d.projection.as_ref())
                .filter(|t| t.key == key)
            {
                return Arc::clone(tables);
            }
        }
        let built = Arc::new(self.build_projection_tables(key));
        let mut guard = self.derived.lock().expect("scene derived");
        if let Some(existing) = guard
            .as_ref()
            .and_then(|d| d.projection.as_ref())
            .filter(|t| t.key == key)
        {
            return Arc::clone(existing);
        }
        let derived = Arc::make_mut(guard.get_or_insert_with(|| Arc::new(SceneDerived::default())));
        derived.projection = Some(Arc::clone(&built));
        built
    }

    /// Build all per-node and per-primitive tables in one iterative pre-order pass.
    fn build_projection_tables(&self, key: SceneDerivedKey) -> SceneProjectionTables {
        let ordered_ids = collect_preorder_nodes(&self.nodes);
        let n = ordered_ids.len();
        let mut node_index = NodeMap::default();
        node_index.reserve(n);
        let mut nodes_by_idx: Vec<&ExtractedNode> = Vec::with_capacity(n);
        for (idx, &id) in ordered_ids.iter().enumerate() {
            node_index.insert(id, idx);
            nodes_by_idx.push(self.nodes[&id].as_ref());
        }

        // 1. Placed origins resolved iteratively along `layout_frame` chains.
        let placed_origins = self.compute_placed_origins(&nodes_by_idx, &node_index);

        // 2. Pre-order draw ancestor states + rebased DrawAttributes.
        let (draw_parent_states, draw_attributes) =
            self.compute_draw_states(&nodes_by_idx, &node_index, &placed_origins);

        // 3. Rebased/offset primitive clip chains (only entries differing from fast paths).
        let placed_clips =
            self.compute_placed_clips(&node_index, &placed_origins, &draw_attributes);

        // 4. Visual-hierarchy passes: opacity groups, compositor layer factors, GPU motion IDs.
        let needs_groups = self.dest_group_candidates != 0
            || !self.nodes.opacity_motion.is_empty()
            || !self.nodes.filter_motion.is_empty();
        let has_presentation = self.compositor.has_presentation();

        let (
            node_opacity_groups,
            inherited_node_opacity_groups,
            overlay_opacity_groups,
            node_layer_factor,
            node_inherited_layer_factor,
            overlay_layer_factor,
            overlay_inherited_layer_factor,
            gpu_motion,
        ) = if needs_groups || has_presentation {
            self.compute_visual_tables(
                &nodes_by_idx,
                &node_index,
                &placed_origins,
                &draw_parent_states,
                needs_groups,
                has_presentation,
            )
        } else {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        };

        SceneProjectionTables {
            key,
            node_index,
            placed_origins,
            draw_parent_states,
            draw_attributes,
            placed_clips,
            node_opacity_groups,
            inherited_node_opacity_groups,
            overlay_opacity_groups,
            node_layer_factor,
            node_inherited_layer_factor,
            overlay_layer_factor,
            overlay_inherited_layer_factor,
            gpu_motion,
        }
    }

    fn compute_placed_origins(
        &self,
        nodes_by_idx: &[&ExtractedNode],
        node_index: &NodeMap<usize>,
    ) -> Vec<[f32; 2]> {
        let n = nodes_by_idx.len();
        if !self.placement_live {
            return nodes_by_idx
                .iter()
                .map(|node| [node.layout.x, node.layout.y])
                .collect();
        }
        let mut origins: Vec<Option<[f32; 2]>> = vec![None; n];
        let mut chain: Vec<usize> = Vec::new();
        for start in 0..n {
            if origins[start].is_some() {
                continue;
            }
            chain.clear();
            let mut cur = start;
            let mut depth = 0usize;
            let mut origin = loop {
                if let Some(resolved) = origins[cur] {
                    break resolved;
                }
                let node = nodes_by_idx[cur];
                if super::placement::is_document(node) || depth >= MAX_ANCESTOR_DEPTH {
                    origins[cur] = Some([0.0, 0.0]);
                    break [0.0, 0.0];
                }
                let frame = match node.layout_frame {
                    nana_ui_runtime::LayoutFrame::Parent => node.parent,
                    nana_ui_runtime::LayoutFrame::Block(block) => Some(block),
                    nana_ui_runtime::LayoutFrame::Page => None,
                };
                match frame.and_then(|id| node_index.get(&id).copied()) {
                    None => {
                        let base = if frame.is_none() {
                            [node.local_layout.x, node.local_layout.y]
                        } else {
                            [node.layout.x, node.layout.y]
                        };
                        origins[cur] = Some(base);
                        break base;
                    }
                    Some(frame_idx) => {
                        let frame_node = nodes_by_idx[frame_idx];
                        if super::placement::is_document(frame_node) {
                            let base = [node.local_layout.x, node.local_layout.y];
                            origins[cur] = Some(base);
                            break base;
                        }
                        if frame_idx == cur || chain.contains(&frame_idx) {
                            let base = [node.local_layout.x, node.local_layout.y];
                            origins[cur] = Some(base);
                            break base;
                        }
                        chain.push(cur);
                        cur = frame_idx;
                        depth += 1;
                    }
                }
            };
            for &idx in chain.iter().rev() {
                let node = nodes_by_idx[idx];
                origin = [
                    origin[0] + node.local_layout.x,
                    origin[1] + node.local_layout.y,
                ];
                origins[idx] = Some(origin);
            }
        }
        origins
            .into_iter()
            .map(|o| o.unwrap_or([0.0, 0.0]))
            .collect()
    }

    fn compute_draw_states(
        &self,
        nodes_by_idx: &[&ExtractedNode],
        node_index: &NodeMap<usize>,
        placed_origins: &[[f32; 2]],
    ) -> (Vec<AncestorState>, Vec<Option<DrawAttributes>>) {
        let n = nodes_by_idx.len();
        let empty_clips: Arc<[ClipRegion]> = Arc::from(Vec::new());
        let identity_state = || {
            (
                AffineTransform::IDENTITY,
                1.0,
                Arc::clone(&empty_clips),
                Ancestor3d::default(),
            )
        };
        let mut parent_states: Vec<AncestorState> = Vec::with_capacity(n);
        let mut handed_down: Vec<Option<AncestorState>> = vec![None; n];
        let mut attributes: Vec<Option<DrawAttributes>> = Vec::with_capacity(n);

        for (idx, &node) in nodes_by_idx.iter().enumerate() {
            let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
            let visual_parent = effective_visual_parent(&self.nodes, node);
            let parent_idx = node.parent.and_then(|id| node_index.get(&id).copied());
            let parent_node = parent_idx.map(|pi| nodes_by_idx[pi]);

            // Fast path: plain child inherits its parent's precomputed `handed_down` in O(1).
            let state = if let (Some(pi), Some(pnode)) = (parent_idx, parent_node)
                && !fixed
                && visual_parent.is_none()
                && !is_triggered_menu(pnode)
                && clip_link(node) == Some(pnode.id)
                && !is_workspace_resize_handle(node)
                && let Some(handed) = handed_down[pi].as_ref()
            {
                handed.clone()
            } else if node.parent.is_none() && !fixed && visual_parent.is_none() {
                identity_state()
            } else {
                self.compute_ancestor_state_with_origins(
                    node,
                    true,
                    fixed,
                    node_index,
                    placed_origins,
                )
                .0
            };

            // Compute what `node` hands down to its plain children.
            let child_follows = if fixed {
                clip_link(node).is_none()
            } else {
                clip_link(node) == node.parent
            };
            if visual_parent.is_none() && !is_triggered_menu(node) {
                let base_for_child = if fixed {
                    Some((
                        self.hanging_branch_transform_with(
                            node,
                            Some((node_index, placed_origins)),
                        ),
                        state.1,
                        Arc::clone(&empty_clips),
                        Ancestor3d::default(),
                    ))
                } else if child_follows {
                    Some(state.clone())
                } else {
                    None
                };
                if let Some(base) = base_for_child {
                    handed_down[idx] = self.fold_node_for_child(
                        base,
                        node,
                        true,
                        fixed,
                        placed_origins[idx],
                        &empty_clips,
                        Some((node_index, placed_origins)),
                    );
                } else {
                    let ancestors = super::ancestors::ancestor_nodes(&self.nodes, node.id)
                        .map(|(_, anc)| anc)
                        .collect::<Vec<_>>();
                    let mut hung = vec![node.id];
                    hung.extend(clip_chain_parents(&self.nodes, node));
                    let (folded, dep) = self.fold_ancestor_chain_with_origins(
                        ancestors,
                        &hung,
                        None,
                        false,
                        true,
                        node_index,
                        placed_origins,
                    );
                    if !dep {
                        handed_down[idx] = Some(folded);
                    }
                }
            }

            let origin = if record_on_page(node) {
                [0.0, 0.0]
            } else {
                placed_origins[idx]
            };
            let attr = self
                .projections
                .get(&node.id)
                .filter(|&&(epoch, _, _)| epoch != self.epochs.attribute)
                .map(|&(_, base_transform, _)| {
                    let (ref parent, _, ref parent_clips, ctx_3d) = state;
                    let local = self.resolved_local_transform_at(
                        node,
                        placed_origins[idx],
                        ctx_3d.blocks_3d,
                        ctx_3d.parent_perspective,
                        Some((node_index, placed_origins)),
                    );
                    let current = parent.then(local);
                    let (delta, rebased) = if same_drawn_transform(current, base_transform) {
                        (AffineTransform::IDENTITY, None)
                    } else {
                        inverse(base_transform).map_or((AffineTransform::IDENTITY, None), |inv| {
                            (current.then(inv), Some((base_transform, current)))
                        })
                    };
                    DrawAttributes {
                        delta,
                        rebased,
                        parent_clips: Arc::clone(parent_clips),
                        origin,
                    }
                });

            parent_states.push(state);
            attributes.push(attr);
        }

        (parent_states, attributes)
    }

    #[allow(clippy::too_many_arguments)]
    fn fold_node_for_child(
        &self,
        base: AncestorState,
        node: &ExtractedNode,
        visual: bool,
        is_fixed: bool,
        placed_origin: [f32; 2],
        empty_clips: &Arc<[ClipRegion]>,
        precomputed: Option<(&NodeMap<usize>, &[[f32; 2]])>,
    ) -> Option<AncestorState> {
        let (transform, mut opacity, inherited, ctx_3d) = base;
        if !is_opacity_group(&self.nodes, node) {
            opacity *= if visual {
                self.resolved_local_opacity(node)
            } else {
                local_opacity(node)
            };
        }
        let mut fold = AncestorFold {
            transform,
            opacity,
            clips: Vec::new(),
            ctx_3d,
        };
        let geometry_origin = if record_on_page(node) {
            [0.0, 0.0]
        } else {
            placed_origin
        };
        let placed = LayoutBox {
            x: placed_origin[0],
            y: placed_origin[1],
            ..node.layout
        };
        if self.fold_ancestor_at(
            &mut fold,
            node,
            None,
            visual,
            true,
            true,
            placed,
            geometry_origin,
            precomputed,
        ) {
            return None;
        }
        let clips = if is_fixed {
            if fold.clips.is_empty() {
                Arc::clone(empty_clips)
            } else {
                fold.clips.into()
            }
        } else if fold.clips.is_empty() {
            inherited
        } else {
            inherited.iter().cloned().chain(fold.clips).collect()
        };
        Some((fold.transform, fold.opacity, clips, fold.ctx_3d))
    }

    fn compute_placed_clips(
        &self,
        node_index: &NodeMap<usize>,
        placed_origins: &[[f32; 2]],
        draw_attributes: &[Option<DrawAttributes>],
    ) -> HashMap<PrimitiveId, Arc<[ClipRegion]>> {
        let mut clips_map = HashMap::new();
        for (&id, held) in &self.primitives {
            let primitive = &held.primitive;
            if super::is_triggered_overlay_surface(primitive.id) {
                continue;
            }
            let Some(&idx) = node_index.get(&primitive.node) else {
                continue;
            };
            let proj = self.projections.get(&primitive.node);
            let is_retained = proj.is_none_or(|&(epoch, _, _)| epoch == self.epochs.attribute);
            if is_retained {
                let own_from = held.inherited_clips;
                let own = &primitive.clips[own_from..];
                if own.is_empty() {
                    continue;
                }
                let built_at = held.origin;
                if built_at == [0.0, 0.0] {
                    continue;
                }
                let mut chain = primitive.clips[..own_from].to_vec();
                chain.extend(own.iter().cloned().map(|mut clip| {
                    clip.bounds = offset_rect(clip.bounds, built_at);
                    clip
                }));
                clips_map.insert(id, chain.into());
            } else if let Some(attributes) = &draw_attributes[idx] {
                let &(_, _, parent_clip_count) = proj.expect("projection present");
                let own_from = parent_clip_count.min(primitive.clips.len());
                let own = &primitive.clips[own_from..];
                if own.is_empty() {
                    continue;
                }
                let rebase = (attributes.delta != AffineTransform::IDENTITY
                    || attributes.rebased.is_some())
                .then_some(attributes);
                let mut chain = attributes.parent_clips.to_vec();
                chain.extend(own.iter().cloned().map(|mut clip| {
                    if let Some(attrs) = rebase {
                        clip.transform = attrs.rebase(clip.transform);
                    }
                    clip.bounds = offset_rect(clip.bounds, attributes.origin);
                    clip
                }));
                clips_map.insert(id, chain.into());
            } else {
                let _ = placed_origins;
            }
        }
        clips_map
    }

    #[allow(clippy::type_complexity)]
    fn compute_visual_tables(
        &self,
        nodes_by_idx: &[&ExtractedNode],
        node_index: &NodeMap<usize>,
        placed_origins: &[[f32; 2]],
        draw_parent_states: &[AncestorState],
        needs_groups: bool,
        has_presentation: bool,
    ) -> (
        Vec<Arc<[OpacityGroup]>>,
        Vec<Arc<[OpacityGroup]>>,
        Vec<Option<Arc<[OpacityGroup]>>>,
        Vec<f32>,
        Vec<f32>,
        Vec<f32>,
        Vec<f32>,
        Vec<NodeGpuMotion>,
    ) {
        let n = nodes_by_idx.len();
        let total = 2 * n;
        let mut state: Vec<u8> = vec![0; total]; // 0 = unvisited, 1 = on stack, 2 = done

        let mut node_groups = if needs_groups {
            vec![empty_opacity_groups(); n]
        } else {
            Vec::new()
        };
        let mut inherited_node_groups = if needs_groups {
            vec![empty_opacity_groups(); n]
        } else {
            Vec::new()
        };
        let mut overlay_groups: Vec<Option<Arc<[OpacityGroup]>>> = if needs_groups {
            vec![None; n]
        } else {
            Vec::new()
        };

        let mut node_factor = if has_presentation {
            vec![1.0; n]
        } else {
            Vec::new()
        };
        let mut node_inherited_factor = if has_presentation {
            vec![1.0; n]
        } else {
            Vec::new()
        };
        let mut overlay_factor = if has_presentation {
            vec![1.0; n]
        } else {
            Vec::new()
        };
        let mut overlay_inherited_factor = if has_presentation {
            vec![1.0; n]
        } else {
            Vec::new()
        };

        let mut gpu_motion = if has_presentation {
            vec![NodeGpuMotion::default(); n]
        } else {
            Vec::new()
        };
        // Propagated active GPU opacity descriptor `(opacity_id, layer_opacity)` per visual slot.
        let mut active_gpu_opacity: Vec<(u32, Option<f32>)> = if has_presentation {
            vec![(0, None); total]
        } else {
            Vec::new()
        };

        let slot_to_vn = |slot: usize| -> VisualNode {
            if slot < n {
                VisualNode::Node(nodes_by_idx[slot].id)
            } else {
                VisualNode::Overlay(nodes_by_idx[slot - n].id)
            }
        };
        let vn_to_slot = |vn: VisualNode| -> Option<usize> {
            let idx = *node_index.get(&vn.node_id())?;
            Some(match vn {
                VisualNode::Node(_) => idx,
                VisualNode::Overlay(_) => idx + n,
            })
        };

        // Precompute `Node` transform IDs directly from `draw_parent_states`.
        if has_presentation {
            for (idx, &node) in nodes_by_idx.iter().enumerate() {
                let mut transform_id = if super::is_triggered_menu(node) {
                    0
                } else {
                    self.gpu_motion_id_for(node.id, 0)
                };
                if transform_id != 0 {
                    let layout = &node.source_style.layout;
                    let ctx_3d = draw_parent_states[idx].3;
                    if (ctx_3d.blocks_3d && layout.transforms.transform_3d.is_some())
                        || ctx_3d.parent_perspective.is_some()
                        || ctx_3d.backface_hidden
                        || layout.transforms.backface_visibility
                            == nana_ui_core::BackfaceVisibility::Hidden
                    {
                        transform_id = 0;
                    }
                }
                gpu_motion[idx].transform_id = transform_id;
            }
        }

        // Resolve all `2 * N` visual slots in iterative visual-parent post-order.
        let mut stack: Vec<(usize, Option<usize>)> = Vec::new();
        for start in 0..total {
            if state[start] == 2 {
                continue;
            }
            // Skip `Overlay(idx)` unless triggered menu or referenced by an overlay/opacity motion.
            if start >= n {
                let node = nodes_by_idx[start - n];
                if !super::is_triggered_menu(node)
                    && !self.nodes.opacity_motion.contains(&node.id)
                    && !self.nodes.filter_motion.contains(&node.id)
                    && self.compositor_layer(node.id).is_none()
                {
                    state[start] = 2;
                    continue;
                }
            }
            stack.clear();
            let mut cur = Some(start);
            let mut depth = 0usize;
            while let Some(slot) = cur {
                if state[slot] == 2 {
                    break;
                }
                if state[slot] == 1 || depth >= MAX_ANCESTOR_DEPTH {
                    break;
                }
                state[slot] = 1;
                let vn = slot_to_vn(slot);
                let parent_slot = super::visual_parent_of(&self.nodes, vn).and_then(vn_to_slot);
                stack.push((slot, parent_slot));
                cur = parent_slot.filter(|&ps| state[ps] == 0);
                depth += 1;
            }

            while let Some((slot, parent_slot)) = stack.pop() {
                let vn = slot_to_vn(slot);
                let idx = if slot < n { slot } else { slot - n };
                let node = nodes_by_idx[idx];

                if needs_groups {
                    let base = match parent_slot {
                        Some(ps) if state[ps] == 2 => {
                            let parent_vn = slot_to_vn(ps);
                            let pgroups = if ps < n {
                                Arc::clone(&node_groups[ps])
                            } else {
                                overlay_groups[ps - n]
                                    .as_ref()
                                    .map_or_else(empty_opacity_groups, Arc::clone)
                            };
                            inherited_visual_opacity_groups(&self.nodes, parent_vn, vn, pgroups)
                        }
                        _ => empty_opacity_groups(),
                    };
                    if slot < n {
                        inherited_node_groups[idx] = Arc::clone(&base);
                    }
                    let own = self.visual_node_own_opacity_group_at(vn, placed_origins[idx]);
                    let full = if let Some(own) = own {
                        let mut groups = base.to_vec();
                        groups.push(self.within_filter_depth(&base, own));
                        Arc::from(groups)
                    } else {
                        base
                    };
                    if slot < n {
                        node_groups[idx] = full;
                    } else {
                        overlay_groups[idx] = Some(full);
                    }
                }

                if has_presentation {
                    let inherited_factor = match parent_slot {
                        Some(ps) if state[ps] == 2 => {
                            if ps < n {
                                node_factor[ps]
                            } else {
                                overlay_factor[ps - n]
                            }
                        }
                        _ => 1.0,
                    };
                    let own_factor = self.visual_layer_opacity_factor(vn);
                    let total_factor = inherited_factor * own_factor;
                    if slot < n {
                        node_inherited_factor[idx] = inherited_factor;
                        node_factor[idx] = total_factor;
                    } else {
                        overlay_inherited_factor[idx] = inherited_factor;
                        overlay_factor[idx] = total_factor;
                    }

                    let isolates = visual_is_opacity_group(&self.nodes, vn);
                    let own_op = self.own_gpu_opacity_pair(vn, node);
                    let anc_op = if isolates {
                        (0, None)
                    } else {
                        match parent_slot {
                            Some(ps) if state[ps] == 2 => {
                                let pvn = slot_to_vn(ps);
                                if visual_is_opacity_group(&self.nodes, pvn) {
                                    (0, None)
                                } else {
                                    active_gpu_opacity[ps]
                                }
                            }
                            _ => (0, None),
                        }
                    };
                    let with_own = if isolates {
                        (0, None)
                    } else if own_op.0 != 0 {
                        own_op
                    } else {
                        anc_op
                    };
                    active_gpu_opacity[slot] = with_own;
                    if slot < n {
                        gpu_motion[idx].node_opacity = with_own;
                        gpu_motion[idx].scrim_opacity = anc_op;
                    } else {
                        gpu_motion[idx].overlay_opacity = with_own;
                    }
                }

                state[slot] = 2;
            }
        }

        (
            node_groups,
            inherited_node_groups,
            overlay_groups,
            node_factor,
            node_inherited_factor,
            overlay_factor,
            overlay_inherited_factor,
            gpu_motion,
        )
    }

    fn visual_node_own_opacity_group_at(
        &self,
        vn: VisualNode,
        placed_origin: [f32; 2],
    ) -> Option<OpacityGroup> {
        match vn {
            VisualNode::Node(id) => {
                let candidate = self.nodes.get(&id)?;
                is_opacity_group(&self.nodes, candidate).then(|| {
                    let placed = LayoutBox {
                        x: placed_origin[0],
                        y: placed_origin[1],
                        ..candidate.layout
                    };
                    let mut group = dest_group(&self.nodes, id, candidate, placed);
                    let presents = !super::is_triggered_menu(candidate);
                    let filter_motion_id = if presents {
                        self.gpu_motion_id_for(id, 6)
                    } else {
                        0
                    };
                    let opacity_motion_id = if presents {
                        self.gpu_motion_id_for(id, 1)
                    } else {
                        0
                    };
                    group.filter_motion_id = filter_motion_id;
                    group.opacity_motion_id = opacity_motion_id;
                    group.opacity = if opacity_motion_id != 0 {
                        1.0
                    } else {
                        self.resolved_local_opacity(candidate)
                    };
                    if let Some(filter) = self.compositor.effect(id).filter(|_| presents) {
                        group.filter = filter;
                    }
                    group.filter = group.filter.with_blur_cap(ColorFilter::blur_cap(
                        candidate.source_style.layout.paint.wide_filter_blur,
                    ));
                    if let Some(streak) = self.compositor.motion_streak(id).filter(|_| presents) {
                        group.motion_streak = streak;
                    }
                    group
                })
            }
            VisualNode::Overlay(id) => self.visual_node_overlay_opacity_group(id),
        }
    }

    fn own_gpu_opacity_pair(&self, vn: VisualNode, node: &ExtractedNode) -> (u32, Option<f32>) {
        let triggered = super::is_triggered_menu(node);
        let id = match vn {
            VisualNode::Node(id) if !triggered => self.gpu_motion_id_for(id, 1),
            VisualNode::Overlay(id) if triggered => self.gpu_motion_id_for(id, 1),
            _ => 0,
        };
        if id == 0 {
            return (0, None);
        }
        let layer_opacity = self.compositor_layer(node.id).map(|l| l.opacity);
        (id, layer_opacity)
    }
}

/// Collect all nodes in iterative parent-before-child pre-order (root order +
/// detached nodes sorted deterministically by `StableNodeId`).
fn collect_preorder_nodes(nodes: &SceneNodes) -> Vec<StableNodeId> {
    let mut roots: Vec<StableNodeId> = nodes
        .values()
        .filter(|node| node.parent.is_none_or(|p| !nodes.contains_key(&p)))
        .map(|node| node.id)
        .collect();
    roots.sort_unstable();

    let mut visited = NodeSet::default();
    visited.reserve(nodes.len());
    let mut order = Vec::with_capacity(nodes.len());
    let mut stack = Vec::new();

    for root in roots {
        if visited.insert(root) {
            stack.push(root);
            while let Some(id) = stack.pop() {
                order.push(id);
                if let Some(node) = nodes.get(&id) {
                    for &child in node.children.iter().rev() {
                        if nodes.contains_key(&child) && visited.insert(child) {
                            stack.push(child);
                        }
                    }
                }
            }
        }
    }
    if order.len() < nodes.len() {
        let mut detached: Vec<StableNodeId> = nodes
            .keys()
            .copied()
            .filter(|id| !visited.contains(id))
            .collect();
        detached.sort_unstable();
        for start in detached {
            if visited.insert(start) {
                stack.push(start);
                while let Some(id) = stack.pop() {
                    order.push(id);
                    if let Some(node) = nodes.get(&id) {
                        for &child in node.children.iter().rev() {
                            if nodes.contains_key(&child) && visited.insert(child) {
                                stack.push(child);
                            }
                        }
                    }
                }
            }
        }
    }
    order
}
