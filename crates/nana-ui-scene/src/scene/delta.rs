//! Applying an extraction delta: removals, moves, rebases and re-extraction.

use super::*;

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

impl UiScene {
    /// Apply Runtime's dirty extraction and tombstone stream atomically.
    /// Updating or removing nodes refreshes [`Self::instance_id`]; an empty
    /// no-op keeps the current instance.
    pub fn apply_delta(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        self.apply_delta_with_moves(extracted, std::iter::empty(), removals)
    }

    /// [`Self::apply_delta`] with the nodes layout only moved
    /// (`UiWorld::extract_nodes_reusing`): each keeps the record the scene
    /// holds, given its new box and result.
    pub fn apply_delta_with_moves(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        moved: impl IntoIterator<Item = ExtractedMove>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        let mut delta = SceneDelta::default();
        self.structure_changed = false;
        let mut removed_nodes = 0;
        let mut changed = Vec::new();
        let mut hierarchy_changed = false;
        let mut inherited_roots = HashSet::new();
        let mut custom_paint_changed = false;
        let mut rebind_sticky = Vec::new();
        for id in removals {
            if let Some(old) = self.nodes.remove(&id) {
                self.dest_group_candidates -= usize::from(may_be_dest_group(&old));
                delta.removed.push(id);
                self.projections.remove(&id);
                self.unadjustable_projections.remove(&id);
                self.origin_bound.remove(&id);
                self.offset_bound.remove(&id);
                self.unfollowing.remove(&id);
                self.placed_origins
                    .get_mut()
                    .expect("scene placements")
                    .remove(&id);
                rebind_sticky.extend(self.sticky_nodes.affected(id));
                self.sticky_nodes.remove(id);
                self.escaped_nodes.remove(&id);
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
                let opacity_cache = self
                    .opacity_group_cache
                    .get_mut()
                    .expect("scene opacity groups");
                if !opacity_cache.is_empty() {
                    if self.dest_group_candidates == 0 {
                        opacity_cache.clear();
                    } else {
                        opacity_cache.remove(&id);
                    }
                }
                if !self.custom_paint.is_empty() {
                    self.custom_paint.remove(&id);
                }
                if !self.modal_surface_paint.is_empty() {
                    self.modal_surface_paint.remove(&id);
                }
                if !self.markdown_diagram_paint.is_empty() {
                    self.markdown_diagram_paint.remove(&id);
                }
            }
        }
        let mut updated_nodes = 0;
        // Roots whose retained descendants have to be rebuilt because what
        // they inherit changed. A scrolled projective subtree cannot be
        // re-projected, and a group that stops isolating hands its opacity
        // down to primitives that baked it.
        let mut subtree_rebuild = Vec::new();
        // Boxes that only moved and paint nothing: a rebuild would give them
        // no primitive again.
        let mut paintless_moves = Vec::new();
        let mut scroll_translations = Vec::new();
        let mut stacking_changed = false;
        let mut inherited_geometry_changed = !inherited_roots.is_empty();
        // A root layout only moved keeps its record, primitives and all, and
        // so does everything under it: what a record holds is in its node's
        // own coordinates and is drawn at the node's origin, summed down its
        // frames when it is drawn (`placement.rs`). Nothing is rebuilt; what
        // the subtree projects is rebased, and its culling bounds move by the
        // root's shift.
        let mut layout_moves: Vec<StableNodeId> = Vec::new();
        let moved: Vec<ExtractedMove> = moved.into_iter().collect();
        let extracted: Vec<ExtractedNode> = extracted.into_iter().collect();
        // Where each record that may move stands before anything in this
        // delta is written, when there are culling bounds to move: a root's
        // shift is measured against it.
        let track_shifts = self.visibility.get().is_some();
        let stood: NodeMap<[f32; 2]> = if let Some(visibility) = self.visibility.get() {
            let ids: Vec<StableNodeId> = moved
                .iter()
                .map(|moved| moved.id)
                .chain(extracted.iter().filter_map(|node| {
                    let old = self.nodes.get(&node.id)?;
                    (old.layout != node.layout).then_some(node.id)
                }))
                .filter(|&id| visibility.paints_under(id))
                .collect();
            ids.into_iter()
                .filter_map(|id| Some((id, self.placed_origin_mut(id)?)))
                .collect()
        } else {
            NodeMap::default()
        };
        for moved in moved {
            let id = moved.id;
            let Some(held) = self.nodes.get(&id) else {
                debug_assert!(false, "{id:?} moved, but the scene holds no record of it");
                continue;
            };
            let bits = |layout: &LayoutBox| {
                [layout.x, layout.y, layout.width, layout.height].map(f32::to_bits)
            };
            let same_result = same_layout_result(&held.layout_result, &moved.layout_result);
            if bits(&held.local_layout) == bits(&moved.local_layout)
                && held.layout_frame == moved.layout_frame
                && same_result
            {
                continue;
            }
            // A record of another size or result paints other primitives.
            let resized = held.layout.width.to_bits() != moved.layout.width.to_bits()
                || held.layout.height.to_bits() != moved.layout.height.to_bits()
                || !same_result;
            let frame_changed = held.layout_frame != moved.layout_frame;
            let node = Arc::make_mut(self.nodes.get_mut(&id).expect("held above"));
            node.local_layout = moved.local_layout;
            node.layout_frame = moved.layout_frame;
            node.layout_result = moved.layout_result;
            if resized {
                node.layout.width = moved.layout.width;
                node.layout.height = moved.layout.height;
                changed.push(id);
            }
            if frame_changed {
                if node.layout_frame == nana_ui_runtime::LayoutFrame::Parent
                    && node.source_style.layout.position != nana_ui_core::PositionSpec::Sticky
                {
                    self.unfollowing.remove(&id);
                } else {
                    self.unfollowing.insert(id);
                }
            }
            self.placement_live = true;
            self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
            delta.transforms.push(id);
            delta.clips.push(id);
            delta.paint.push(id);
            layout_moves.push(id);
            updated_nodes += 1;
        }
        #[cfg(debug_assertions)]
        let mut fresh = Vec::new();
        for node in extracted {
            let previous = self.nodes.get(&node.id);
            if previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.source_style.layout.position != node.source_style.layout.position
                    || old.source_style.layout.overflow_x != node.source_style.layout.overflow_x
                    || old.source_style.layout.overflow_y != node.source_style.layout.overflow_y
            }) {
                rebind_sticky.extend(self.sticky_nodes.affected(node.id));
                rebind_sticky.push(node.id);
            }
            // Gaining or losing a painter changes which primitives a node
            // owns without necessarily touching the frame plan, so the
            // retained index's slots for it cannot be trusted. A new
            // recording on the same painter is ordinary: the node is rebuilt
            // below and `VisibilityIndex::update` re-derives its slots from
            // the new primitives, keeping the index for everything else.
            custom_paint_changed |= previous
                .is_some_and(|old| old.custom_paint.is_some() != node.custom_paint.is_some());
            let sticky_geometry_changed = self.sticky_nodes.has_dependents(node.id)
                && previous.is_some_and(|old| {
                    let facts = |node: &ExtractedNode| {
                        node.layout_result.as_ref().map(|result| {
                            (result.content_box, result.padding_box, result.scroll_extent)
                        })
                    };
                    facts(old) != facts(&node)
                });
            // A node extracted again that only moved keeps its primitives,
            // as a move the runtime sent would: they are in its own
            // coordinates.
            let kept_move = previous
                .filter(|old| !record_on_page(old) && !record_on_page(&node))
                .is_some_and(|old| moved_in_place(old, &node));
            // Whatever else changed, what it holds and was not extracted moves
            // with it, as under a move the runtime sent.
            if previous.is_some_and(|old| old.layout != node.layout) {
                self.placement_live = true;
                layout_moves.push(node.id);
            }
            let inherited_changed = previous.map_or(!node.children.is_empty(), |old| {
                sticky_geometry_changed
                    || old.parent != node.parent
                    || clip_link(old) != clip_link(&node)
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            });
            if inherited_changed {
                // Retained descendants may not be extracted when an ancestor's
                // clip or transform changes. Refresh their inherited projection
                // without rebuilding invertible retained geometry.
                self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
                inherited_geometry_changed |= sticky_geometry_changed
                    || previous.is_none_or(|old| inherited_transform_changed(old, &node));
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
                    || clip_link(old) != clip_link(&node)
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            }) {
                delta.clips.push(node.id);
            }
            delta.paint.push(node.id);
            // Where the clip chain hangs a node is part of the culling
            // index's structure: which scrolls move its bounds (#201).
            hierarchy_changed |= previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.children != node.children
                    || old.source_style.layout.position != node.source_style.layout.position
                    || clip_link(old) != clip_link(&node)
            });
            if escapes_clip_parent(&node) {
                self.escaped_nodes.insert(node.id);
            } else {
                self.escaped_nodes.remove(&node.id);
            }
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
            // What it hands its children is derived from the node map, not
            // from its primitives, and its projection is read only to draw
            // primitives of its own: a box that only moved and has none keeps
            // nothing a rebuild would change.
            if self.node_primitive_count(node.id) == 0
                && previous.is_some_and(|old| moved_only(old, &node))
            {
                paintless_moves.push(node.id);
            } else if !kept_move {
                changed.push(node.id);
            }
            if node.layout_frame == nana_ui_runtime::LayoutFrame::Parent
                && node.source_style.layout.position != nana_ui_core::PositionSpec::Sticky
            {
                self.unfollowing.remove(&node.id);
            } else {
                self.unfollowing.insert(node.id);
            }
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
            #[cfg(debug_assertions)]
            fresh.push(node.id);
            let candidate = may_be_dest_group(&node);
            let replaced = self.nodes.insert(node.id, Arc::new(node));
            self.dest_group_candidates -=
                usize::from(replaced.as_deref().is_some_and(may_be_dest_group));
            self.dest_group_candidates += usize::from(candidate);
            updated_nodes += 1;
        }
        // Every record is in place: placements read from here on see them.
        if updated_nodes != 0 || removed_nodes != 0 {
            self.placement_epoch = self.placement_epoch.wrapping_add(1);
            self.placed_clips
                .get_mut()
                .expect("scene placed clips")
                .clear();
        }
        // A fresh record stands where its frames place it, to the bit, or a
        // move kept above it would place it off by the difference.
        #[cfg(debug_assertions)]
        if self.placement_live && self.nodes.len() <= RETAINED_AUDIT_LIMIT {
            for id in fresh {
                let node = self.nodes.get(&id).expect("inserted above");
                // The document is the page: its origin is the page's.
                if matches!(node.kind.as_ref(), NodeKind::Document) {
                    continue;
                }
                let placed = self.placed_origin(node);
                assert_eq!(
                    placed.map(f32::to_bits),
                    [node.layout.x, node.layout.y].map(f32::to_bits),
                    "{id:?}: a fresh record is not where its frames place it"
                );
            }
        }
        // What a move does not carry: a node that built from where it stands
        // is built again where it stands now, and the culling bounds of what
        // does not follow its root are derived again.
        let moved_set: NodeSet = layout_moves.iter().copied().collect();
        let under_move = |nodes: &SceneNodes, id: StableNodeId, inclusive: bool| {
            let mut cursor = if inclusive {
                Some(id)
            } else {
                nodes.get(&id).and_then(|node| node.parent)
            };
            let mut depth = 0;
            while let Some(at) = cursor {
                if moved_set.contains(&at) {
                    return true;
                }
                depth += 1;
                if depth > MAX_ANCESTOR_DEPTH {
                    return false;
                }
                cursor = nodes.get(&at).and_then(|node| node.parent);
            }
            false
        };
        if !moved_set.is_empty() {
            for &id in &self.origin_bound {
                if under_move(&self.nodes, id, true) {
                    changed.push(id);
                }
            }
            if !self.offset_bound.is_empty() {
                changed.extend(
                    layout_moves
                        .iter()
                        .copied()
                        .filter(|id| self.offset_bound.contains(id)),
                );
            }
        }
        let unfollowing_moved: Vec<StableNodeId> = if moved_set.is_empty() || !track_shifts {
            Vec::new()
        } else {
            self.unfollowing
                .iter()
                .copied()
                .filter(|&id| under_move(&self.nodes, id, false))
                .collect()
        };
        // How far each root moved, less what the nearest moved root above it
        // already moves it by in the culling index.
        let layout_shifts: Vec<(StableNodeId, Option<[f32; 2]>)> = {
            let total: NodeMap<[f32; 2]> = stood
                .iter()
                .filter_map(|(&id, &stood)| {
                    let [x, y] = self.placed_origin_mut(id)?;
                    Some((id, [x - stood[0], y - stood[1]]))
                })
                .collect();
            total
                .iter()
                .map(|(&id, &[dx, dy])| {
                    let mut cursor = self.nodes.get(&id).and_then(|node| {
                        (node.source_style.layout.position != nana_ui_core::PositionSpec::Fixed)
                            .then_some(node.parent)
                            .flatten()
                    });
                    let mut above = [0.0, 0.0];
                    while let Some(at) = cursor {
                        if let Some(&shift) = total.get(&at) {
                            above = shift;
                            break;
                        }
                        cursor = self.nodes.get(&at).and_then(|node| {
                            (node.source_style.layout.position != nana_ui_core::PositionSpec::Fixed)
                                .then_some(node.parent)
                                .flatten()
                        });
                    }
                    // The shift is on the page, before the transforms above
                    // the root: its subtree's drawn bounds move by it
                    // through their linear part. A root's own transform is
                    // about where it stands, so it moves with it and does
                    // not take part. Under a projection the bounds do not
                    // move by any one offset: they are derived again.
                    let (dx, dy) = (dx - above[0], dy - above[1]);
                    let inherited = self
                        .nodes
                        .get(&id)
                        .map_or(AffineTransform::IDENTITY, |node| {
                            self.draw_ancestor_state(node).0
                        });
                    if inherited.is_projective() {
                        return (id, None);
                    }
                    let [a, b, c, d, _, _] = inherited.0;
                    let shift = if [a, b, c, d] == [1.0, 0.0, 0.0, 1.0] {
                        [dx, dy]
                    } else {
                        [a * dx + c * dy, b * dx + d * dy]
                    };
                    (id, Some(shift))
                })
                .collect()
        };
        rebind_sticky.sort_unstable();
        rebind_sticky.dedup();
        for id in rebind_sticky {
            if self.nodes.get(&id).is_some_and(|node| {
                node.source_style.layout.position == nana_ui_core::PositionSpec::Sticky
            }) {
                let mut cursor = self.nodes.get(&id).and_then(|node| node.parent);
                let parents: Vec<_> = std::iter::from_fn(|| {
                    let id = cursor?;
                    let node = self.nodes.get(&id)?;
                    let style = &node.source_style.layout;
                    cursor = if style.overflow_x.clips()
                        || style.overflow_y.clips()
                        || style.position == nana_ui_core::PositionSpec::Fixed
                    {
                        None
                    } else {
                        node.parent
                    };
                    Some(id)
                })
                .collect();
                self.sticky_nodes.bind(id, parents.into_iter());
            } else {
                self.sticky_nodes.remove(id);
            }
        }
        let order_rebuilt = (updated_nodes != 0 || removed_nodes != 0)
            && (hierarchy_changed || self.node_order.len() != self.nodes.len());
        let mut rebuilt_primitives = 0;
        if updated_nodes != 0 || removed_nodes != 0 {
            self.compositor_projection_dirty =
                self.compositor.has_presentation() && scroll_translations.is_empty();
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
                    && (inherited_roots
                        .iter()
                        .any(|root| self.node_descends_from(id, *root))
                        || (!moved_set.is_empty() && under_move(&self.nodes, id, true)))
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
            #[cfg(debug_assertions)]
            if self.nodes.len() <= RETAINED_AUDIT_LIMIT {
                for &id in &paintless_moves {
                    assert_eq!(
                        self.rebuild_node_primitives(id),
                        0,
                        "{id:?} moved without paint, yet a rebuild paints it"
                    );
                }
            }
            // Rebuilt nodes re-enter `ordered` at their own key, so a reorder is
            // only needed when keys the delta did not touch also moved.
            if order_rebuilt || stacking_changed {
                self.sort_primitives();
            }
            if !rebuild.is_empty() || removed_nodes != 0 {
                self.extended_color.take();
            }
            if order_rebuilt || stacking_changed || removed_nodes != 0 || self.structure_changed {
                self.frame_plan.take();
                self.visibility.take();
            }
            if custom_paint_changed {
                self.visibility.take();
            }
            if let Some(mut visibility) = self.visibility.take() {
                // Every translation goes in before any bound is derived again:
                // a derived bound is already where it stands, and a shift
                // after it would move it twice.
                //
                // A moved subtree's bounds move with its root, less what a
                // moved root above it already moved them by.
                for &(root, shift) in &layout_shifts {
                    if let Some(shift) = shift
                        && shift != [0.0, 0.0]
                    {
                        visibility.translate_subtree(root, shift);
                    }
                }
                for &(root, offset) in &scroll_translations {
                    visibility.translate_subtree(root, offset);
                }
                // Sticky branches do not follow the ancestor's uniform shift.
                // Refresh their retained culling bounds, leaving unrelated
                // primitives and the compiled frame plan in place.
                for &(root, _) in &scroll_translations {
                    for id in self.sticky_nodes.affected(root) {
                        visibility.update(self, &[id]);
                        visibility.refresh_subtree(self, id);
                    }
                    // Nor do boxes that escape the scroller (#201).
                    for &id in &self.escaped_nodes {
                        if id != root && self.node_descends_from(id, root) {
                            visibility.update(self, &[id]);
                            visibility.refresh_subtree(self, id);
                        }
                    }
                }
                // A moved root's own primitives are not under it, and what is
                // under it measured from a frame above it (or sticking to a
                // scroller above it) did not move with it.
                if !layout_moves.is_empty() {
                    visibility.update(self, &layout_moves);
                    for &id in &unfollowing_moved {
                        visibility.update(self, &[id]);
                        visibility.refresh_subtree(self, id);
                    }
                }
                for &(root, shift) in &layout_shifts {
                    if shift.is_none() {
                        visibility.refresh_subtree(self, root);
                    }
                }
                if inherited_geometry_changed {
                    // An ancestor's projection moved, and its retained
                    // descendants are not in `rebuild`, so their bounds have to
                    // be re-derived — but only under the roots that moved, and
                    // from the plan this index already holds rather than a new
                    // one. Which primitives sit under those roots did not
                    // change, and that is the half rebuilding would pay for
                    // again.
                    for &root in &inherited_roots {
                        visibility.refresh_subtree(self, root);
                    }
                }
                visibility.update(self, &rebuild);
                let _ = self.visibility.set(visibility);
            }
            self.instance = next_scene_instance();
        }
        #[cfg(debug_assertions)]
        if !layout_moves.is_empty() {
            self.audit_moved_primitives(&layout_moves);
        }
        #[cfg(debug_assertions)]
        self.audit_retained_projection();
        // The counter is what lets `opacity_groups` answer without touching the
        // node map, so a path that edits it without maintaining the counter
        // would drop isolation groups from paint and show nothing else. There
        // are two such paths today; this catches a third being added.
        // Rescanning is linear in the scene, so it is bounded to the small
        // trees unit tests build — a new mutation site will be reached by one
        // of those long before it is reached by a scene big enough for the
        // bound to matter.
        debug_assert!(
            self.nodes.len() > RETAINED_AUDIT_LIMIT
                || self.dest_group_candidates
                    == self
                        .nodes
                        .iter()
                        .filter(|(_, node)| may_be_dest_group(node))
                        .count(),
            "dest_group_candidates drifted from the node map"
        );
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
pub(super) enum InheritedComponentClip {
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

/// Whether re-extracting this node can move a *retained descendant's* projected
/// transform, which is the only way one node's change reaches another node's
/// visibility bound.
///
/// `project_ancestor_state` builds that transform from each ancestor's
/// `position` (where the chain starts), its local scene transform, and whether
/// it closes a 3D context — and nothing else. Everything else a style carries
/// still bumps `attribute_epoch`, because the *clip* chain paint reads is wider
/// than this; it just does not move a bound, so it must not cost a pass over
/// every primitive in the scene. A fade is the case that matters: it rewrites
/// one node's style every frame and moves nothing.
///
/// Where the node stands is not part of it either: a descendant's primitives
/// are drawn at the descendant's own origin, and a box that moved carries its
/// subtree's culling bounds with it (`layout_moves`). Where its transform
/// pivots is, and is compared below.
fn inherited_transform_changed(old: &ExtractedNode, node: &ExtractedNode) -> bool {
    if old.parent != node.parent
        || old.source_style.layout.position != node.source_style.layout.position
        || old.source_style.layout.fails_closed_3d_context()
            != node.source_style.layout.fails_closed_3d_context()
    {
        return true;
    }
    if node.source_style.layout.position == nana_ui_core::PositionSpec::Sticky {
        let before = &old.source_style.layout;
        let after = &node.source_style.layout;
        if (
            before.offset_top,
            before.offset_right,
            before.offset_bottom,
            before.offset_left,
            before.dir,
            before.writing_mode,
            old.style.font_size,
        ) != (
            after.offset_top,
            after.offset_right,
            after.offset_bottom,
            after.offset_left,
            after.dir,
            after.writing_mode,
            node.style.font_size,
        ) {
            return true;
        }
    }
    [false, true].into_iter().any(|blocks_3d| {
        node_scene_transform(old.source_style.layout.as_ref(), old.layout, blocks_3d)
            != node_scene_transform(node.source_style.layout.as_ref(), node.layout, blocks_3d)
    })
}

/// What a node's own paint style contributes to the paint-order keys of its
/// subtree — the reason a change to it costs a reorder.
///
/// The *value* of `opacity` is not in it: [`is_opacity_group`] only asks
/// whether the node is translucent at all, so fading a container from 0.35 to
/// 0.37 must not re-sort the scene. Neither is an identity filter, which
/// [`dest_filter_applies`] already reads as no filter.
#[derive(PartialEq)]
pub(super) struct PaintOrderFacts {
    pub(super) z_index: i32,
    pub(super) translucent: bool,
    pub(super) filter: Option<ColorFilter>,
    pub(super) mix_blend: MixBlendMode,
    stacking_context: bool,
    pub(super) custom_paint: bool,
    /// A scroll view whose bars can show opens a group for them.
    scrollbar_chrome: bool,
    pub(super) scroll_edge_fade: bool,
}

/// [`moved_only`], with the same result: what the node paints is in its own
/// coordinates, so it paints the same primitives wherever it stands.
fn moved_in_place(old: &ExtractedNode, new: &ExtractedNode) -> bool {
    same_layout_result(&old.layout_result, &new.layout_result) && moved_only(old, new)
}

/// Whether two records hold the same layout result: the same allocation,
/// as a node that only moved shares it, or one equal to it.
fn same_layout_result(
    old: &Option<Arc<nana_ui_runtime::LayoutResult>>,
    new: &Option<Arc<nana_ui_runtime::LayoutResult>>,
) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => Arc::ptr_eq(old, new) || old == new,
        (None, None) => true,
        _ => false,
    }
}

/// Whether `new` is `old` moved: the same node in every respect but where
/// its box stands, at the same size, and the result that publishes it.
fn moved_only(old: &ExtractedNode, new: &ExtractedNode) -> bool {
    // A box that stayed is not a move; one that changed size is more than
    // one. Both are settled before the whole node is compared, which reads
    // a wide container's result placement by placement.
    if old.layout == new.layout
        || old.layout.width.to_bits() != new.layout.width.to_bits()
        || old.layout.height.to_bits() != new.layout.height.to_bits()
    {
        return false;
    }
    let mut probe = new.clone();
    probe.layout = old.layout;
    probe.local_layout = old.local_layout;
    probe.layout_frame = old.layout_frame;
    probe.layout_result = old.layout_result.clone();
    probe == *old
}

fn paint_order_facts(node: &ExtractedNode) -> PaintOrderFacts {
    let paint = &node.source_style.layout.paint;
    PaintOrderFacts {
        z_index: node.z_index,
        translucent: is_translucent(node),
        filter: paint.filter.filter(|filter| !filter.is_identity()),
        mix_blend: paint.mix_blend,
        stacking_context: node.source_style.layout.creates_paint_stacking_context(),
        custom_paint: node.custom_paint.is_some(),
        scrollbar_chrome: has_scrollbar_chrome(node),
        scroll_edge_fade: has_scroll_edge_fade(node),
    }
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

/// Whether `node` hangs from a clip parent other than its parent.
fn escapes_clip_parent(node: &ExtractedNode) -> bool {
    node.source_style.layout.position != nana_ui_core::PositionSpec::Fixed
        && clip_link(node) != node.parent
}

/// Two answers the primitive-rebuild pass keeps asking for: what a node
/// inherits from the chain above it, and where that chain puts it in paint
/// order. A container style change rebuilds every descendant, and a node owns
/// several primitives, so both are asked many times for the same parent.
///
/// The answers about one node are single entries: the pass walks the subtree
/// in order, so the next question almost always has the same answer as the
/// last, and comparing two words beats hashing. The answers about the chain
/// above a node are kept per ancestor instead, because a chain of only
/// children asks a different parent every time and a one-entry cache would
/// walk the whole chain again for each of them.
///
/// Deliberately not stamped and not always on. It is filled only while
/// `apply_delta` rebuilds primitives — where `self.nodes` and `self.node_order`
/// are already final and cannot move under it — and emptied on the way out.
#[derive(Default, Debug)]
pub(super) struct RebuildScratch {
    pub(super) active: bool,
    pub(super) ancestor_state: Option<(AncestorStateKey, AncestorState)>,
    /// The same answer for the draw pass, which has no bracket to reset it:
    /// siblings ask for it one after another while the painter walks paint
    /// order, so one entry carries a whole container. Stamped, because nothing
    /// clears it. See [`UiScene::draw_stamp`].
    pub(super) drawn_ancestor_state: Option<(AncestorStateKey, DrawStamp, AncestorState)>,
    /// The paint-order stack of the node being rebuilt, shared by all of its
    /// primitives.
    pub(super) order_stack: Option<(StableNodeId, GroupPrefix)>,
    /// Every node's paint-order prefix the pass has asked for: siblings share
    /// their parent's, and a node's is its parent's plus its own entry. See
    /// [`group_prefix_memo`].
    pub(super) group_prefixes: NodeMap<GroupPrefix>,
    /// Whether a node is inside a rasterized SVG, or in flow inside an icon,
    /// for every node the pass has walked through. See [`ancestor_answer`].
    pub(super) in_rasterized_svg: NodeMap<bool>,
    pub(super) in_icon_visual: NodeMap<bool>,
    /// The nodes an [`ancestor_answer`] walk passed, reused across walks.
    pub(super) walk: Vec<StableNodeId>,
    /// What each parent hands a plain child, logical then presented. See
    /// [`UiScene::handed_down`].
    pub(super) handed_down: [NodeMap<Option<AncestorState>>; 2],
}

impl RebuildScratch {
    pub(super) fn begin(&mut self) {
        self.active = true;
        self.ancestor_state = None;
        self.order_stack = None;
        self.group_prefixes.clear();
        self.in_rasterized_svg.clear();
        self.in_icon_visual.clear();
        self.handed_down.iter_mut().for_each(NodeMap::clear);
    }

    pub(super) fn end(&mut self) {
        self.active = false;
        self.ancestor_state = None;
        self.order_stack = None;
        self.group_prefixes = NodeMap::default();
        self.in_rasterized_svg = NodeMap::default();
        self.in_icon_visual = NodeMap::default();
        self.walk = Vec::new();
        self.handed_down = Default::default();
    }
}
