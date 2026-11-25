//! Topology, layout placement, scroll, interaction, and visual mutation application.

use super::super::*;
use super::restyle::markdown_blocks_changed;

impl UiWorld {
    pub(super) fn apply_topology_or_visual_mutation(
        &mut self,
        mutation: &UiMutation,
        report: &mut CommitReport,
    ) {
        // Invalidate before topology writes so old ancestors also lose stale
        // maxima. Insert's new parent is invalidated independently below.
        match mutation {
            UiMutation::WriteLayout { id, .. } | UiMutation::PlaceLayout { id, .. } => {
                // Marked with the commit's other writes when it ends.
                self.defer_scroll_content(*id);
                self.scroll.layout_touched = true;
            }
            UiMutation::Detach { id }
            | UiMutation::ParkSubtree { root: id }
            | UiMutation::DespawnSubtree { root: id } => {
                self.invalidate_scroll_topology(*id, None);
                self.scroll.layout_touched = true;
            }
            UiMutation::Insert { parent, child, .. } => {
                self.invalidate_scroll_topology(*child, Some(*parent));
                self.scroll.layout_touched = true;
            }
            UiMutation::SetWidgetVisual { id, visual } => {
                let layout = &self.record(*id).style.layout;
                let current = self.nodes.widget_visual(*id);
                let was = scroll_container(layout, current);
                let now = scroll_container(layout, visual.as_deref());
                let contains_changed = overflow_contained_axes(layout, current)
                    != overflow_contained_axes(layout, visual.as_deref());
                let triggered_changed = current != visual.as_deref()
                    && (current.is_some_and(|w| w.triggered_menu_overlay().is_some())
                        || visual
                            .as_ref()
                            .is_some_and(|w| w.triggered_menu_overlay().is_some()));
                if was != now {
                    self.track_scroll_container(*id, now);
                }
                if contains_changed {
                    self.invalidate_scroll_content(*id);
                    self.scroll.layout_touched = true;
                }
                // An open triggered menu places its items `fixed`, out of
                // every scroller's range.
                if triggered_changed {
                    self.scroll.content_bounds.borrow_mut().relist(self, *id);
                    self.scroll.layout_touched = true;
                }
            }
            _ => {}
        }
        match mutation {
            UiMutation::Create { id, document, kind } => {
                let kind_arc = self.nodes.intern_kind(kind);
                let mut record = NodeRecord::with_kind(
                    *document,
                    kind_arc,
                    self.nodes.default_style(),
                    initial_interaction(kind),
                );
                record.render_full_epoch = self.invalidation.render_epoch.get();
                self.nodes.insert(*id, record);
                self.record_topology_invalidation(*id);
                self.invalidation.dirty_entities.insert(*id);
                self.invalidation.pending_work_revision =
                    self.invalidation.pending_work_revision.saturating_add(1);
                self.tree.spawned_since_drain += 1;
                report.created += 1;
                self.refresh_root_membership(*id);
                self.recompute_node_theme_deps(*id);
            }
            UiMutation::Insert {
                parent,
                child,
                before,
            } => {
                let old_parent = self
                    .identity_and_parent(*child)
                    .expect("validated child must exist")
                    .1;
                if old_parent == Some(*parent) && *before == Some(*child) {
                    return;
                }
                self.record_child_list_invalidation(*parent);
                if old_parent != Some(*parent) {
                    self.record_topology_invalidation(*child);
                }
                self.clear_layout_results_subtree(*child);
                if let Some(old_parent) = old_parent {
                    self.remove_child(old_parent, *child);
                }
                let parent_hierarchy = self.hierarchy_mut(*parent);
                let siblings = Arc::make_mut(&mut parent_hierarchy.children);
                let index = before
                    .and_then(|before| siblings.iter().position(|id| *id == before))
                    .unwrap_or(siblings.len());
                siblings.insert(index, *child);
                let after = siblings.len() - index - 1;
                #[cfg(test)]
                {
                    self.tree.child_list_steps += after as u64;
                }
                self.renumber_children_between(*parent, index, after);
                self.hierarchy_mut(*child).parent = Some(*parent);
                let below = self.record(*parent).depth.saturating_add(1);
                self.set_subtree_depth(*child, below);
                self.link_block_extent_reads(*parent, *child, true);
                if old_parent != Some(*parent) {
                    self.responsive_reparented(*child);
                    self.i18n_reparented(*child, old_parent);
                    self.rebind_sticky_subtree(*child);
                    self.theme_scope_reparented(*child, old_parent);
                    if old_parent.is_none() {
                        self.recompute_node_theme_deps(*child);
                    }
                }
                let parent_mount = self.record(*parent).mount;
                if self.record(*child).mount != parent_mount {
                    self.set_subtree_mount_state(*child, parent_mount);
                    if parent_mount == MountState::Parked {
                        self.withdraw_parked_hits(*child);
                    }
                }
                if old_parent.is_some() && old_parent != Some(*parent) {
                    // A moved subtree inherits the new parent's
                    // `pointer-events` before the frame re-projects it.
                    self.withdraw_pointer_events_none(*child);
                }
                if old_parent == Some(*parent) {
                    // Retained-order moves carry the entire subtree; descendants
                    // keep their inherited state and local geometry until layout
                    // writeback identifies actual changes.
                    self.mark(
                        *child,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                } else {
                    self.mark_subtree(*child, DirtyMask::ALL);
                }
                self.mark_ancestors(
                    *parent,
                    DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
                self.record_child_list_invalidation(*parent);
                if let Some(old_parent) = old_parent {
                    self.mark_ancestors(
                        old_parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                    self.record_child_list_invalidation(old_parent);
                }
                if old_parent.is_some() {
                    report.reparented += 1;
                } else {
                    report.inserted += 1;
                }
                self.tree.detached.remove(child);
                self.tree.detached_mounted.remove(child);
                self.sync_subtree_presence(*child);
                self.refresh_root_membership(*child);
                self.note_structural_change(*parent);
                if let Some(old_parent) = old_parent {
                    self.note_structural_change(old_parent);
                }
            }
            UiMutation::ReorderChildren { parent, order } => {
                self.reorder_children(*parent, order, report);
            }
            UiMutation::Detach { id } => {
                let old_parent = self.parent_id(*id);
                self.capture_shared_elements_in_subtree(*id);
                self.record_topology_invalidation(*id);
                self.clear_layout_results_subtree(*id);
                if self.unlink_from_parent(*id) {
                    report.detached += 1;
                }
                self.leave_live_document(*id);
                self.rebind_sticky_subtree(*id);
                self.refresh_root_membership(*id);
                self.theme_scope_reparented(*id, old_parent);
                self.recompute_node_theme_deps(*id);
                self.sync_active_focus_traps();
            }
            UiMutation::ParkSubtree { root } => {
                let old_parent = self.parent_id(*root);
                self.capture_shared_elements_in_subtree(*root);
                self.record_topology_invalidation(*root);
                self.clear_layout_results_subtree(*root);
                self.unlink_from_parent(*root);
                self.set_subtree_mount_state(*root, MountState::Parked);
                self.leave_live_document(*root);
                self.rebind_sticky_subtree(*root);
                self.refresh_root_membership(*root);
                self.theme_scope_reparented(*root, old_parent);
                self.sync_active_focus_traps();
            }
            UiMutation::DespawnSubtree { root } => {
                self.capture_shared_elements_in_subtree(*root);
                self.record_topology_invalidation(*root);
                let root_snapshot = self.node(*root).expect("validated root must exist");
                if let Some(parent) = root_snapshot.parent {
                    self.record_child_list_invalidation(parent);
                    self.remove_child(parent, *root);
                    self.mark_ancestors(
                        parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                    self.note_structural_change(parent);
                }
                let mut stack = vec![*root];
                while let Some(id) = stack.pop() {
                    let snapshot = self.node(id).expect("validated subtree must exist");
                    stack.extend(snapshot.children.iter().rev().copied());
                    self.forget_visual_presence(id);
                    self.forget_shared_element_node(id);
                    self.scroll
                        .content_bounds
                        .get_mut()
                        .remove(id, snapshot.parent);
                    self.invalidation.paint_recordings.get_mut().remove(&id);
                    self.invalidation
                        .modal_surface_recordings
                        .get_mut()
                        .remove(&id);
                    if !self.invalidation.painter_overrides.is_empty() {
                        self.invalidation.painter_overrides.remove(&id);
                    }
                    if !self.invalidation.media_time_mappings.is_empty() {
                        self.invalidation.media_time_mappings.remove(&id);
                    }
                    self.write_overlay_host(id, None);
                    self.reindex_component(id, None);
                    self.forget_labelled_by(id);
                    self.set_live(id, None);
                    self.set_selected_override(id, None);
                    self.set_hit_test_behavior_override(id, None);
                    let _removed = self.nodes.remove(id);
                    self.invalidation.theme_state.pending_theme_eval.remove(&id);
                    self.invalidation.theme_state.theme_scopes.remove_owner(id);
                    self.input.focus_scopes.retain(|root, target| {
                        if *target == Some(id) {
                            *target = None;
                        }
                        *root != id
                    });
                    self.input.focus_policies.remove(&id);
                    self.input.directional_scopes.remove(&id);
                    self.input.focus_neighbors.remove(&id);
                    if !self.input.focus_neighbors.is_empty() {
                        self.input.focus_neighbors.retain(|_, neighbors| {
                            if neighbors.up == Some(id) {
                                neighbors.up = None;
                            }
                            if neighbors.down == Some(id) {
                                neighbors.down = None;
                            }
                            if neighbors.left == Some(id) {
                                neighbors.left = None;
                            }
                            if neighbors.right == Some(id) {
                                neighbors.right = None;
                            }
                            !neighbors.is_empty()
                        });
                    }
                    self.sync_focus_trap(id);
                    self.invalidation.dirty_entities.remove(&id);
                    self.hit_index.non_scroll_hit_dirty.remove(&id);
                    // A cause queued for a node that is gone seeds nothing,
                    // and a drain for one document keeps what it cannot place
                    // in it: left here, every node ever removed stayed in
                    // the table and was looked at again each frame.
                    self.invalidation.pending_layout_invalidations.remove(&id);
                    self.remove_document_root(snapshot.document, id);
                    if self.input.focused.get(&snapshot.document) == Some(&id) {
                        self.input.focused.remove(&snapshot.document);
                        self.input.focus_counters.focus_changes += 1;
                    }
                    if let Some(index) = self.hit_index.hit_test_index.get_mut(&snapshot.document) {
                        retain_hit_tree(index, id);
                    }
                    let released = self
                        .input
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            (target == id).then_some((document, pointer_id))
                        })
                        .collect::<Vec<_>>();
                    for key @ (_, pointer_id) in released {
                        self.input.pointer_captures.remove(&key);
                        self.input
                            .pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target: id,
                                captured: false,
                            });
                    }
                    self.input.pointer_hover.retain(|_, target| *target != id);
                    self.input.pointer_press.retain(|_, target| *target != id);
                    self.cancel_animations_for_removed(id);
                    self.remove_surface_presence(id);
                    #[cfg(feature = "controls")]
                    self.forget_reorder_preview(id);
                    self.motion.hover_transitions.remove(&id);
                    self.invalidation.intent_nodes.remove(&id);
                    self.forget_replaced(id);
                    self.overlays_a11y.sticky_nodes.remove(id);
                    self.forget_responsive(id);
                    self.forget_i18n(id, snapshot.document);
                    self.clear_overlay_references(id);
                    self.overlays_a11y.overlay_host_nodes.remove(&id);
                    self.overlays_a11y.drop_targets.remove(&id);
                    // Every other per-node side table is cleared here; this one
                    // grew for the life of the session on any view that mounts
                    // and unmounts promoted nodes.
                    self.motion.compositor_layer_requests.remove(&id);
                    if self
                        .overlays_a11y
                        .drop_hover
                        .is_some_and(|(hover, _)| hover == id)
                    {
                        self.overlays_a11y.drop_hover = None;
                    }
                    self.tree.detached.remove(&id);
                    self.tree.detached_mounted.remove(&id);
                    self.tree.retired.insert(id);
                    self.invalidation.pending_render_removals.push(id);
                    self.invalidation.pending_accessibility_removals.push(id);
                    report.despawned += 1;
                }
            }
            UiMutation::SetExplicitRect { id, rect } => {
                self.apply_explicit_rect(*id, rect.as_deref().copied());
            }
            UiMutation::WriteLayout { id, layout } => {
                self.invalidation
                    .pending_document_layouts
                    .push((*id, *layout));
            }
            UiMutation::PlaceLayout { id, layout, frame } => {
                self.write_local_layout(*id, *layout, **frame);
            }
            UiMutation::PatchPlacement { id, top, height } => {
                let Some(current) = self.node_style(*id).cloned() else {
                    return;
                };
                if placement_already(current.layout.as_ref(), *top, *height) {
                    return;
                }
                let height_changed = match (current.layout.height, *height) {
                    (None, None) => false,
                    (Some(nana_ui_core::LengthSpec::Px(current_height)), Some(next)) => {
                        current_height.to_bits() != next.to_bits()
                    }
                    _ => true,
                };
                let mut style = current;
                {
                    let layout = std::sync::Arc::make_mut(&mut style.layout);
                    layout.offset_top = Some(nana_ui_core::LengthSpec::Px(*top));
                    layout.height = height.map(nana_ui_core::LengthSpec::Px);
                }
                self.write_node_style(*id, style);
                self.record_layout_invalidation(*id, placement_invalidation(height_changed));
                self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                self.mark_subtree(
                    *id,
                    DirtyMask::INPUT | DirtyMask::ACCESSIBILITY | DirtyMask::RENDER,
                );
                if let Some(parent) = self.parent_id(*id) {
                    self.mark_ancestors(parent, DirtyMask::RENDER);
                }
            }
            UiMutation::PatchTransform { id, transform } => {
                let Some(current) = self.node_style(*id).cloned() else {
                    return;
                };
                if current.layout.transforms.transform == *transform {
                    return;
                }
                let previous_resolved = Arc::clone(&self.record(*id).resolved_layout);
                let mut style = current;
                std::sync::Arc::make_mut(&mut style.layout)
                    .transforms
                    .transform = *transform;
                self.write_node_style(*id, style);
                let resolved = Arc::clone(&self.record(*id).resolved_layout);
                let facts = self
                    .invalidation
                    .layout_pairs
                    .facts(&previous_resolved, &resolved);
                let stacking_changed = facts
                    .changed
                    .intersects(nana_ui_core::LayoutStyleChange::STACKING);
                if !stacking_changed && facts.keeps_space {
                    self.mark_transform_kept(*id);
                } else {
                    self.mark(
                        *id,
                        DirtyMask::TRANSFORM | DirtyMask::INPUT | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::SetScrollOffset { id, offset } => {
                // Defer clamping until after `remeasure_scroll_containers()` so
                // an offset restored alongside newly grown content clamps once.
                self.queue_scroll_request(*id, *offset);
            }
            UiMutation::SetScrollOverscroll { id, offset } => {
                self.cancel_scroll_request(*id);
                self.scroll_to_offset(*id, *offset);
            }
            UiMutation::SetScrollMetrics { id, metrics } => {
                let changed = self.nodes.scroll_metrics(*id).copied() != *metrics;
                self.store_scroll_metrics(*id, *metrics);
                if changed && self.has_local_layout_result(*id) {
                    self.mark_moved_by_layout(
                        *id,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                        false,
                    );
                }
            }
            UiMutation::SetInteraction { id, interaction } => {
                self.record_mut(*id).interaction = *interaction;
                if !interaction.pointer_events {
                    self.input.pointer_hover.retain(|_, target| target != id);
                    self.input.pointer_press.retain(|_, target| target != id);
                    self.withdraw_pointer_interaction(*id);
                }
                self.mark(
                    *id,
                    DirtyMask::STATE
                        | DirtyMask::INPUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
                self.recompute_node_theme_deps(*id);
            }
            UiMutation::SetHitTestBehavior { id, behavior } => {
                if self.set_hit_test_behavior_override(*id, *behavior) {
                    self.mark(*id, DirtyMask::INPUT);
                }
            }
            UiMutation::SetFocusPolicy { id, policy } => {
                match policy {
                    Some(policy) => {
                        self.input.focus_policies.insert(*id, *policy);
                    }
                    None => {
                        self.input.focus_policies.remove(id);
                    }
                }
                let document = self.record(*id).document;
                if self.input.focused.get(&document) == Some(id)
                    && !self.focus_policy(*id).allows_programmatic()
                {
                    self.input.focused.remove(&document);
                    self.release_focus(*id);
                    self.input.focus_counters.focus_changes += 1;
                }
                self.mark(
                    *id,
                    DirtyMask::STATE
                        | DirtyMask::INPUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetFocusNeighbors { id, neighbors } => {
                if neighbors.is_empty() {
                    self.input.focus_neighbors.remove(id);
                } else {
                    self.input.focus_neighbors.insert(*id, **neighbors);
                }
                self.mark(*id, DirtyMask::STATE | DirtyMask::INPUT);
            }
            UiMutation::SetDirectionalFocus { id, scope } => match scope {
                Some(scope) => {
                    self.input.directional_scopes.insert(*id, *scope);
                }
                None => {
                    self.input.directional_scopes.remove(id);
                }
            },
            UiMutation::SetCustomRender { id, content } => {
                let previous = self.nodes.custom_render(*id);
                let same_resource = match (previous, content.as_ref()) {
                    (Some(previous), Some(next)) => {
                        previous.renderer == next.renderer && previous.resource == next.resource
                    }
                    _ => false,
                };
                let seeds_before = self.invalidation.layout_seeds_created;
                self.nodes
                    .set_custom_render(*id, content.as_deref().cloned());
                if same_resource {
                    // A frame, a texture generation, a fit: what the node shows
                    // moved and its size did not. Paint.
                    self.invalidation
                        .counters
                        .pending_drain_counts
                        .replaced_content_updates += 1;
                } else {
                    self.reindex_replaced(*id);
                }
                self.mark(*id, DirtyMask::RENDER);
                if same_resource {
                    self.invalidation
                        .counters
                        .pending_drain_counts
                        .replaced_content_only_layout_invalidations +=
                        (self.invalidation.layout_seeds_created - seeds_before) as usize;
                }
            }
            UiMutation::SetMediaTime { id, mapping } => {
                let changed = match mapping {
                    Some(mapping) => self
                        .invalidation
                        .media_time_mappings
                        .insert(*id, **mapping)
                        .is_none_or(|previous| previous != **mapping),
                    None => self.invalidation.media_time_mappings.remove(id).is_some(),
                };
                // The node's box and paint are unchanged; the frame it shows
                // is the host's to replace. Marking render asks for that frame.
                if changed {
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetPainter { id, painter } => {
                let changed = match painter {
                    Some(painter) => self
                        .invalidation
                        .painter_overrides
                        .insert(*id, (**painter).clone())
                        .is_none_or(|previous| previous != **painter),
                    None => self.invalidation.painter_overrides.remove(id).is_some(),
                };
                if changed && self.node_painter(*id).is_none() {
                    self.invalidation.paint_recordings.get_mut().remove(id);
                }
                if changed {
                    self.mark(*id, DirtyMask::RENDER | DirtyMask::INPUT);
                    self.recompute_node_theme_deps(*id);
                }
            }
            UiMutation::SetEventListener { id, event, enabled } => {
                let mut listeners = self.nodes.event_listeners(*id).cloned().unwrap_or_default();
                let before = listeners.clone();
                listeners.set(event.to_string(), *enabled);
                if listeners != before {
                    self.hit_index.region_epoch = self.hit_index.region_epoch.wrapping_add(1);
                }
                if listeners.is_empty() {
                    self.nodes.set_event_listeners(*id, None);
                } else {
                    self.nodes.set_event_listeners(*id, Some(listeners));
                }
            }
            UiMutation::SetComponentType { id, type_id } => {
                let current = self.nodes.component_type(*id);
                if current != type_id.as_ref() {
                    self.reindex_component(*id, type_id.as_ref());
                    self.nodes.set_component_type(*id, type_id.clone());
                }
            }
            UiMutation::SetWidgetVisual { id, visual } => {
                if self.nodes.widget_visual(*id) != visual.as_deref() {
                    #[cfg(feature = "controls")]
                    let reorder_paints_same = match (
                        self.nodes
                            .widget_visual(*id)
                            .and_then(|visual| visual.downcast_ref::<crate::ReorderListVisual>()),
                        visual
                            .as_ref()
                            .and_then(|visual| visual.downcast_ref::<crate::ReorderListVisual>()),
                    ) {
                        (Some(previous), next) => {
                            let previous = previous.clone();
                            self.sync_reorder_list_motion(*id, Some(&previous), next);
                            super::reorder_motion::reorder_list_paints_same(
                                self,
                                *id,
                                Some(&previous),
                                next,
                            )
                        }
                        (None, Some(next)) => {
                            self.sync_reorder_list_motion(*id, None, Some(next));
                            false
                        }
                        (None, None) => false,
                    };
                    #[cfg(not(feature = "controls"))]
                    let reorder_paints_same = false;

                    if let Some(next) = visual
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<crate::ScrollbarVisual>())
                    {
                        let previous = self
                            .nodes
                            .widget_visual(*id)
                            .and_then(|v| v.downcast_ref::<crate::ScrollbarVisual>())
                            .map(crate::ScrollbarVisual::targets);
                        self.sync_scrollbar_motion(*id, previous, next);
                    }
                    let switch_transition = match (
                        self.nodes
                            .widget_visual(*id)
                            .and_then(|v| v.downcast_ref::<crate::SwitchVisual>()),
                        visual
                            .as_ref()
                            .and_then(|v| v.downcast_ref::<crate::SwitchVisual>()),
                    ) {
                        (Some(old), Some(next)) if old.checked != next.checked => {
                            Some((old.thumb_progress, next.checked))
                        }
                        _ => None,
                    };
                    if let Some((thumb_progress, next)) = switch_transition {
                        let node_theme = self.node_theme(*id);
                        let duration = node_theme.duration(nana_ui_core::MotionRole::OverlayFade);
                        let easing = node_theme
                            .motion()
                            .easing(nana_ui_core::EasingRole::Standard);
                        self.start_component_track(
                            *id,
                            crate::component_animation_kinds::SWITCH,
                            duration,
                            easing,
                            crate::AnimatableProperty::Progress,
                            crate::MotionValue::Scalar(thumb_progress),
                            crate::MotionValue::Scalar(f32::from(next)),
                            crate::MotionInterrupt::Retarget,
                            None,
                        );
                    }

                    let prev = self.nodes.widget_visual(*id).cloned();
                    let tree_accessibility_changed =
                        super::tree_view_accessibility_changed(prev.as_ref(), visual.as_deref());
                    let select_accessibility_changed =
                        super::select_accessibility_changed(prev.as_ref(), visual.as_deref());
                    let menu_accessibility_changed =
                        super::menu_surface_accessibility_changed(prev.as_ref(), visual.as_deref());
                    let prev_select = prev
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<crate::SelectVisual>());
                    let next_select = visual
                        .as_deref()
                        .and_then(|v| v.downcast_ref::<crate::SelectVisual>());
                    let prev_menu = prev
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<crate::MenuSurfaceVisual>());
                    let next_menu = visual
                        .as_deref()
                        .and_then(|v| v.downcast_ref::<crate::MenuSurfaceVisual>());
                    let prev_modal = prev
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<crate::ModalFrameVisual>());
                    let next_modal = visual
                        .as_deref()
                        .and_then(|v| v.downcast_ref::<crate::ModalFrameVisual>());
                    let prev_text_input = prev
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<crate::TextInputVisual>());
                    let next_text_input = visual
                        .as_deref()
                        .and_then(|v| v.downcast_ref::<crate::TextInputVisual>());
                    let text_input_presentation_changed =
                        prev_text_input.is_some() || next_text_input.is_some();
                    let text_folds_changed = match (prev_text_input, next_text_input) {
                        (Some(old), Some(next)) => old.folds != next.folds,
                        (Some(_), None) => true,
                        (None, Some(next)) => !next.folds.is_empty(),
                        (None, None) => false,
                    };
                    let modal_state_changed = matches!(
                        (prev_modal, next_modal),
                        (Some(old), Some(next))
                            if old.busy != next.busy || old.danger != next.danger
                    );
                    let modal_overlay_mount_changed = prev_modal.is_some() != next_modal.is_some();
                    let menu_state_changed = prev_menu.map(|m| m.open) != next_menu.map(|m| m.open);
                    let menu_overlay_mount_changed = matches!(
                        (prev_menu, next_menu),
                        (Some(old), Some(next))
                            if old.open
                                && next.open
                                && old
                                    .overlay
                                    .zip(next.overlay)
                                    .is_some_and(|(a, b)| a.overlay_mount != b.overlay_mount)
                    );
                    let select_fit_changed = match (prev_select, next_select) {
                        (Some(old), Some(next)) => old.fitted_options_changed(next),
                        (Some(old), None) => old.fit_options,
                        (None, Some(next)) => next.fit_options,
                        (None, None) => false,
                    };
                    let select_hit_changed = prev_select
                        .and_then(crate::SelectVisual::menu_hit_shape)
                        != next_select.and_then(crate::SelectVisual::menu_hit_shape);
                    let select_overlay_mount_changed = matches!(
                        (prev_select, next_select),
                        (Some(old), Some(next))
                            if old.opened && next.opened && old.overlay_mount != next.overlay_mount
                    );
                    let markdown_changed =
                        markdown_blocks_changed(prev.as_ref(), visual.as_deref());
                    let content_layout_changed = match (&prev, visual.as_deref()) {
                        (_, Some(next)) => next.content_layout_changed(prev.as_ref()),
                        (Some(old), None) => old.clearing_changes_content_layout(),
                        (None, None) => false,
                    };
                    let parts_placement_only = matches!(
                        (&prev, visual.as_deref()),
                        (Some(old), Some(next)) if next.parts_placement_only(old)
                    );
                    let owns_intrinsic_text = (prev
                        .as_ref()
                        .is_some_and(crate::WidgetVisualHandle::owns_intrinsic_text)
                        || visual
                            .as_deref()
                            .is_some_and(crate::WidgetVisualHandle::owns_intrinsic_text))
                        && prev.as_ref() != visual.as_deref();
                    let text_path_changed = prev.as_ref().and_then(|v| v.text_visual_key())
                        != visual.as_ref().and_then(|v| v.text_visual_key());
                    let card_text_style_changed = prev
                        .as_ref()
                        .is_some_and(|v| v.uses_card_title_typography())
                        != visual
                            .as_ref()
                            .is_some_and(|v| v.uses_card_title_typography());
                    let invalidates_text_constraint = visual
                        .as_ref()
                        .is_some_and(|v| v.invalidates_node_text_constraint())
                        || prev
                            .as_ref()
                            .is_some_and(|v| v.invalidates_node_text_constraint());
                    self.nodes
                        .set_widget_visual(*id, visual.as_deref().cloned());
                    self.recompute_node_theme_deps(*id);
                    if !visual
                        .as_ref()
                        .is_some_and(|v| v.downcast_ref::<crate::EmptyStateVisual>().is_some())
                    {
                        self.nodes.set_empty_state_text(*id, None);
                    }
                    if next_modal.is_none() {
                        self.nodes.set_modal_text(*id, None);
                    }
                    if card_text_style_changed {
                        self.nodes
                            .invalidate_text(*id, crate::text_node::TextDirty::SHAPE_STYLE);
                        self.mark(*id, DirtyMask::TEXT);
                    }
                    if content_layout_changed && invalidates_text_constraint {
                        self.nodes
                            .invalidate_text(*id, crate::text_node::TextDirty::CONSTRAINT);
                        self.mark(*id, DirtyMask::TEXT);
                    }
                    if select_fit_changed {
                        self.nodes
                            .invalidate_text(*id, crate::text_node::TextDirty::CONTENT);
                        self.mark(*id, DirtyMask::TEXT);
                    }
                    if text_path_changed || owns_intrinsic_text || text_input_presentation_changed {
                        self.mark(*id, DirtyMask::TEXT);
                    }
                    if owns_intrinsic_text
                        || select_fit_changed
                        || menu_state_changed
                        || text_input_presentation_changed
                    {
                        self.invalidate_layout_result(*id);
                    }
                    if prev_text_input.is_some() && next_text_input.is_none() {
                        self.nodes.set_text_input_presentation(*id, None);
                        self.nodes.set_text_viewport_pin(*id, None);
                    }
                    if text_folds_changed {
                        let offered = next_text_input
                            .filter(|v| !v.folds.is_empty())
                            .map(|v| Arc::clone(&v.folds));
                        self.reconcile_text_fold_offered(*id, offered);
                    }
                    self.sync_node_presence(*id);
                    if markdown_changed
                        || content_layout_changed
                        || owns_intrinsic_text
                        || select_fit_changed
                        || text_input_presentation_changed
                    {
                        let (kind, fields, footprint, reason) = if parts_placement_only
                            && !markdown_changed
                            && !owns_intrinsic_text
                            && !select_fit_changed
                        {
                            (
                                InvalidationKind::PLACEMENT,
                                LayoutStyleChange::FLOW,
                                LayoutDependencyFootprint::NONE,
                                InvalidationReason::TEXT,
                            )
                        } else {
                            (
                                InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                                LayoutStyleChange::INTRINSIC.union(LayoutStyleChange::FLOW),
                                LayoutDependencyFootprint::intrinsic_container(),
                                InvalidationReason::CONTEXT,
                            )
                        };
                        self.record_layout_invalidation(
                            *id,
                            LayoutInvalidation::new(
                                LayoutInvalidationSource::Component,
                                reason,
                                kind,
                                fields,
                                footprint,
                            ),
                        );
                    }
                    if menu_state_changed {
                        self.record_layout_invalidation(
                            *id,
                            LayoutInvalidation::new(
                                LayoutInvalidationSource::Component,
                                InvalidationReason::CONTEXT,
                                InvalidationKind::TOPOLOGY
                                    .union(InvalidationKind::MEASURE)
                                    .union(InvalidationKind::PLACEMENT),
                                LayoutStyleChange::FLOW,
                                LayoutDependencyFootprint::ALL,
                            ),
                        );
                    }
                    if tree_accessibility_changed {
                        self.mark(*id, DirtyMask::ACCESSIBILITY);
                    }
                    if select_accessibility_changed
                        || (menu_accessibility_changed && !menu_state_changed)
                    {
                        self.mark_subtree(*id, DirtyMask::ACCESSIBILITY);
                    }
                    if select_hit_changed {
                        self.mark(*id, DirtyMask::INPUT);
                    }
                    if modal_state_changed {
                        self.mark_subtree(
                            *id,
                            DirtyMask::STYLE
                                | DirtyMask::INPUT
                                | DirtyMask::FOCUS_IME
                                | DirtyMask::ACCESSIBILITY
                                | DirtyMask::RENDER,
                        );
                    }
                    if menu_state_changed {
                        self.mark_subtree(
                            *id,
                            DirtyMask::STYLE
                                | DirtyMask::INPUT
                                | DirtyMask::FOCUS_IME
                                | DirtyMask::ACCESSIBILITY
                                | DirtyMask::RENDER,
                        );
                        self.mark_ancestors(*id, DirtyMask::RENDER);
                    } else if select_overlay_mount_changed
                        || menu_overlay_mount_changed
                        || modal_overlay_mount_changed
                    {
                        self.mark_subtree(
                            *id,
                            DirtyMask::STYLE | DirtyMask::INPUT | DirtyMask::RENDER,
                        );
                    }
                    if !reorder_paints_same {
                        self.mark(*id, DirtyMask::RENDER);
                    }
                }
            }
            UiMutation::SetLabelledBy { id, label } => {
                if self.labelled_by(*id) != *label {
                    self.set_labelled_by(*id, *label);
                    self.mark(*id, DirtyMask::ACCESSIBILITY);
                }
            }
            UiMutation::SetLive { id, live } => {
                if self.live(*id) != *live {
                    self.set_live(*id, *live);
                    self.mark(*id, DirtyMask::ACCESSIBILITY);
                }
            }
            UiMutation::SetSelected { id, selected } => {
                if self.selected_override(*id) != *selected {
                    self.set_selected_override(*id, *selected);
                    self.mark(*id, DirtyMask::ACCESSIBILITY);
                    if !self.record(*id).style.interaction.is_empty() {
                        self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    if self.node_painter(*id).is_some() {
                        self.mark_repaint(*id);
                    }
                }
            }
            UiMutation::SetOverlayMount { id, mount } => {
                if self.nodes.overlay_mount(*id) != mount.as_deref() {
                    self.nodes.set_overlay_mount(*id, mount.as_deref().copied());
                    self.mark_subtree(*id, DirtyMask::RENDER | DirtyMask::INPUT);
                }
            }
            UiMutation::SetAccessibility { id, accessibility } => {
                let previous = self.nodes.accessibility(*id);
                let hidden_changed = previous.hidden != accessibility.hidden;
                let interaction_style_changed = previous.disabled != accessibility.disabled
                    || previous.checked != accessibility.checked
                    || previous.selected != accessibility.selected
                    || previous.mixed != accessibility.mixed;
                // A numeric field's spinner is drawn from these: which halves
                // are live follows `editable` and the value against the range.
                let spinner_changed = previous.editable != accessibility.editable
                    || previous.numeric_value != accessibility.numeric_value
                    || previous.numeric_minimum != accessibility.numeric_minimum
                    || previous.numeric_maximum != accessibility.numeric_maximum;
                self.nodes.set_accessibility(*id, accessibility);
                self.mark_inline_text_owners(*id);
                if hidden_changed {
                    // `aria-hidden` changes the projected structure for the
                    // whole subtree. Dirty descendants so an incremental
                    // host receives removals/reinsertions, and dirty
                    // ancestors so their child lists are updated as well.
                    self.mark_subtree(*id, DirtyMask::ACCESSIBILITY);
                    if let Some(parent) = self.parent_id(*id) {
                        self.mark_ancestors(parent, DirtyMask::ACCESSIBILITY);
                    }
                } else {
                    self.mark(*id, DirtyMask::ACCESSIBILITY);
                }
                if spinner_changed {
                    self.mark(*id, DirtyMask::RENDER);
                }
                if interaction_style_changed && !self.record(*id).style.interaction.is_empty() {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                if interaction_style_changed && self.node_painter(*id).is_some() {
                    self.mark_repaint(*id);
                }
                if accessibility.modal || self.input.active_focus_traps.contains(id) {
                    self.sync_focus_trap(*id);
                }
                if accessibility.disabled {
                    let document = self.record(*id).document;
                    if let Some(focused) = self.input.focused.get(&document).copied()
                        && self.is_descendant_or_self(focused, *id)
                        && !self.focus_policy(focused).allows_programmatic()
                    {
                        self.input.focused.remove(&document);
                        self.release_focus(focused);
                        self.input.focus_counters.focus_changes += 1;
                    }
                }
            }
            UiMutation::SetSurfaceOpen { id, open, menu } => {
                self.set_surface_open(*id, *open, *menu);
                self.sync_focus_trap(*id);
            }
            UiMutation::SetOverlayHost { host, state } => {
                let previous = self.nodes.overlay_host(*host).copied();
                if previous == Some(**state) {
                    return;
                }
                self.write_overlay_host(*host, Some(**state));
                self.mark(*host, DirtyMask::ACCESSIBILITY);
                if let Some(inactive) = previous
                    .and_then(|previous| previous.active)
                    .filter(|active| Some(*active) != state.active)
                {
                    self.clear_surface_pointer_interactions(inactive);
                }
                let changed_roots = previous
                    .and_then(|previous| previous.active)
                    .into_iter()
                    .chain(state.active)
                    .collect::<HashSet<_>>();
                for root in changed_roots {
                    self.invalidate_layout_result(root);
                    self.record_layout_invalidation(
                        root,
                        LayoutInvalidation::new(
                            LayoutInvalidationSource::Component,
                            InvalidationReason::CONTEXT,
                            InvalidationKind::TOPOLOGY
                                .union(InvalidationKind::MEASURE)
                                .union(InvalidationKind::PLACEMENT),
                            LayoutStyleChange::FLOW,
                            LayoutDependencyFootprint::ALL,
                        ),
                    );
                    self.mark_subtree(
                        root,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::CapturePointer { pointer_id, target } => {
                let document = self.record(*target).document;
                let previous = self
                    .input
                    .pointer_captures
                    .insert((document, *pointer_id), *target);
                if previous == Some(*target) {
                    return;
                }
                if let Some(previous) = previous {
                    self.input
                        .pending_pointer_capture_changes
                        .push(PointerCaptureChange {
                            pointer_id: *pointer_id,
                            target: previous,
                            captured: false,
                        });
                }
                self.input
                    .pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: true,
                    });
            }
            UiMutation::ReleasePointer { pointer_id, target } => {
                let document = self.record(*target).document;
                self.input.pointer_captures.remove(&(document, *pointer_id));
                self.input
                    .pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: false,
                    });
            }
            UiMutation::StartAnimation { animation } => {
                self.install_animation((**animation).clone());
            }
            UiMutation::ReplaceAnimation { animation } => {
                self.replace_animation((**animation).clone());
            }
            UiMutation::StopAnimation { id } => {
                self.cancel_animation(*id);
            }
            UiMutation::FinishAnimation { id } => {
                self.finish_animation(*id);
            }
            UiMutation::ReverseAnimation { id } => {
                self.reverse_animation(*id);
            }
            UiMutation::PauseAnimation { id } => {
                self.pause_animation(*id);
            }
            UiMutation::ResumeAnimation { id } => {
                self.resume_animation(*id);
            }
            UiMutation::SetSharedElement { target, key } => {
                self.register_shared_element_node(*target, key.as_deref().cloned());
            }
            UiMutation::CaptureSharedElement { target, key } => {
                let _ = self.capture_shared_element_snapshot(*target, key.clone());
            }
            UiMutation::ClaimSharedElement { target, claim } => {
                self.motion.pending_shared_element_claims.push((
                    *target,
                    claim.key.clone(),
                    claim.transition,
                    claim.now,
                ));
            }
            UiMutation::RequestFocus { document, target } => {
                if let Some(target) = target {
                    self.remember_scope_focus(*target);
                }
                let from_pointer = self.input.pointer_modality.contains(document);
                if from_pointer {
                    self.input.focus_from_pointer.insert(*document);
                } else {
                    self.input.focus_from_pointer.remove(document);
                }
                let old = match target {
                    Some(target) => self.input.focused.insert(*document, *target),
                    None => self.input.focused.remove(document),
                };
                if old != *target {
                    self.input.focus_counters.focus_changes += 1;
                }
                if let Some(old) = old.filter(|old| Some(*old) != *target) {
                    self.release_focus(old);
                    // A rich editor draws its caret only while focused.
                    if self.nodes.rich_editor_marks(old).is_some() {
                        self.mark(old, DirtyMask::RENDER);
                    }
                }
                if let Some(target) = target
                    && self.nodes.rich_editor_marks(*target).is_some()
                {
                    self.mark(*target, DirtyMask::RENDER);
                }
                if let Some(target) = target {
                    self.mark(*target, DirtyMask::STATE);
                    if !self.record(*target).style.interaction.focused.is_empty() {
                        if self.record(*target).hierarchy.children.is_empty() {
                            self.mark(*target, DirtyMask::STYLE | DirtyMask::RENDER);
                        } else {
                            self.mark_subtree(*target, DirtyMask::STYLE | DirtyMask::RENDER);
                        }
                    }
                    self.mark_focus_changed(*target);
                }
            }
            UiMutation::RestoreFocusWithin { root } => {
                let target = super::validate::ValidationPlan::new(self)
                    .restorable_scope_focus(*root)
                    .ok()
                    .flatten();
                if let Some((document, target)) = target {
                    self.apply(
                        &UiMutation::RequestFocus {
                            document,
                            target: Some(target),
                        },
                        report,
                    );
                }
            }
            _ => unreachable!("dispatched to restyle or editor handler"),
        }
    }
}

impl UiWorld {
    /// filter and one renumbering from the first of them, where one each per
    /// child made clearing a long list from the front quadratic (#389).
    /// Returns how many mutations the run spans; a lone removal keeps its own
    /// edit.
    pub(super) fn unlink_removal_run(&mut self, mutations: &[UiMutation]) -> usize {
        let removal_root = |mutation: &UiMutation| match mutation {
            UiMutation::Detach { id }
            | UiMutation::ParkSubtree { root: id }
            | UiMutation::DespawnSubtree { root: id } => Some(*id),
            _ => None,
        };
        let Some(parent) = mutations
            .first()
            .and_then(removal_root)
            .and_then(|root| self.parent_id(root))
        else {
            return 1;
        };
        // Siblings of one parent: none is in another's subtree, so taking
        // them out first leaves each mutation the same subtree to retire.
        let mut leaving = NodeSet::default();
        let mut roots = Vec::new();
        for mutation in mutations {
            match removal_root(mutation) {
                Some(root) if self.parent_id(root) == Some(parent) && leaving.insert(root) => {
                    roots.push(root);
                }
                _ => break,
            }
        }
        if roots.len() < 2 {
            return 1;
        }
        for &root in &roots {
            self.link_block_extent_reads(parent, root, false);
        }
        let hierarchy = self.hierarchy_mut(parent);
        #[cfg(test)]
        let filtered = hierarchy.children.len() as u64;
        let children = Arc::make_mut(&mut hierarchy.children);
        // How many children stay before the first one leaving, and after
        // the last.
        let (mut head, mut tail) = (None, 0);
        let mut seen = 0;
        children.retain(|child| {
            let leaves = leaving.contains(child);
            if leaves {
                head.get_or_insert(seen);
                tail = 0;
            } else {
                tail += 1;
            }
            seen += 1;
            !leaves
        });
        let head = head.expect("a run's roots are their parent's children");
        intern_empty_children(&mut hierarchy.children);
        #[cfg(test)]
        {
            self.tree.child_list_steps += filtered;
        }
        self.renumber_children_between(parent, head, tail);
        self.tree.unlinked_ahead = leaving;
        roots.len()
    }

    /// Move `id` to `offset` clamped to its scrolling area.
    pub(super) fn scroll_to_clamped(&mut self, id: StableNodeId, offset: ScrollOffset) {
        self.scroll_to_offset(id, self.clamp_scroll_offset(id, offset));
    }

    fn queue_scroll_request(&mut self, id: StableNodeId, offset: ScrollOffset) {
        if let Some(existing) = self
            .scroll
            .requested
            .iter_mut()
            .find(|(existing_id, _)| *existing_id == id)
        {
            existing.1 = offset;
        } else {
            self.scroll.requested.push((id, offset));
        }
    }

    fn cancel_scroll_request(&mut self, id: StableNodeId) {
        self.scroll
            .requested
            .retain(|(existing_id, _)| *existing_id != id);
    }

    /// Move `id` to `offset` as given: a touch overscroll, or a clamped one.
    pub(super) fn scroll_to_offset(&mut self, id: StableNodeId, offset: ScrollOffset) {
        let previous = self.nodes.scroll_offset(id);
        if previous != offset {
            self.nodes.set_scroll_offset(id, offset);
            self.input.focus_nav_epoch = self.input.focus_nav_epoch.wrapping_add(1);
            // Hit-index patch + Scene extract of this scroller only.
            // Descendants keep LayoutBox; paint uses scroll_offset.
            self.scroll
                .hit_updates
                .push((id, [previous.x - offset.x, previous.y - offset.y]));
            self.mark_scrolled(id);
        }
    }

    /// Store `id`'s scrolling area and clamp its offset into it.
    pub(super) fn store_scroll_metrics(
        &mut self,
        id: StableNodeId,
        metrics: Option<ScrollMetrics>,
    ) -> bool {
        if scroll_viewport_size(self.nodes.scroll_metrics(id).copied())
            != scroll_viewport_size(metrics)
        {
            self.invalidation
                .counters
                .pending_drain_counts
                .scroll_viewport_updates += 1;
        }
        self.nodes.set_scroll_metrics(id, metrics);
        let current = self.nodes.scroll_offset(id);
        let clamped = self.clamp_scroll_offset(id, current);
        if current == clamped {
            return false;
        }
        self.nodes.set_scroll_offset(id, clamped);
        self.scroll
            .hit_updates
            .push((id, [current.x - clamped.x, current.y - clamped.y]));
        self.mark_scrolled(id);
        true
    }

    /// Re-measure every scroll container whose content this commit moved,
    /// so none keeps a scrolling area its layout has left behind — whoever
    /// wrote the boxes. The content index already knows which: a container
    /// something under it moved is dirty there, so this costs one check per
    /// scroll container, not a walk per written box. A text editor's area
    /// follows its shaped value instead.
    pub(super) fn remeasure_scroll_containers(&mut self) {
        let touched = std::mem::take(&mut self.scroll.layout_touched);
        if !touched && self.scroll.restyled.is_empty() {
            return;
        }
        let mut stale = std::mem::take(&mut self.scroll.restyled);
        if touched {
            self.scroll
                .containers
                .retain(|id| self.nodes.get(*id).is_some());
            let index = self.scroll.content_bounds.borrow();
            // All of them before any is measured: measuring an outer
            // container refreshes the inner ones' entries with it.
            stale.extend(
                self.scroll
                    .containers
                    .iter()
                    .copied()
                    .filter(|id| index.stale(*id)),
            );
        }
        for id in stale {
            if !self.contains(id) {
                continue;
            }
            let scrolls = self.is_scroll_container(id);
            if scrolls && self.text_scroll_metrics(id).is_some() {
                continue;
            }
            // Nothing to measure before a box is laid out: keep what was
            // published. A container that stopped scrolling drops its area.
            let metrics = if scrolls {
                match self.layout_scroll_metrics(id) {
                    Some(metrics) => Some(metrics),
                    None => continue,
                }
            } else {
                None
            };
            let previous = self.nodes.scroll_metrics(id).copied();
            if previous == metrics {
                continue;
            }
            if scroll_viewport_size(previous) != scroll_viewport_size(metrics) {
                self.scroll.resized.insert(id);
            }
            if self.store_scroll_metrics(id, metrics) {
                self.scroll.reclamped.insert(id);
            }
        }
    }

    /// `id`'s effective layout moved from `previous` to `next`, by a style
    /// write or a responsive variant: a box that appears or disappears, or
    /// starts or stops keeping its overflow in, changes its scroll ancestors'
    /// content, and a container that starts or stops scrolling, or restyles,
    /// is re-measured even when no box moved.
    pub(super) fn track_scroll_style(
        &mut self,
        id: StableNodeId,
        previous: &Arc<nana_ui_core::LayoutStyle>,
        next: &Arc<nana_ui_core::LayoutStyle>,
    ) {
        // A paint-only restyle shares the layout style it replaces.
        if Arc::ptr_eq(previous, next) {
            return;
        }
        let visual = self.nodes.widget_visual(id);
        let was = scroll_container(previous, visual);
        let now = scroll_container(next, visual);
        let contains =
            overflow_contained_axes(previous, visual) != overflow_contained_axes(next, visual);
        // By value: a `ScrollView` re-projects an equal style in a fresh
        // `Arc` on every update.
        let restyled = (was || now) && **previous != **next;
        if previous.omits_box() != next.omits_box() || contains {
            self.invalidate_scroll_content(id);
            self.scroll.layout_touched = true;
        }
        if restyled {
            self.track_scroll_container(id, now);
        }
    }

    pub(super) fn track_scroll_container(&mut self, id: StableNodeId, scrolls: bool) {
        let changed = if scrolls {
            self.scroll.containers.insert(id)
        } else {
            self.scroll.containers.remove(&id)
        };
        if changed {
            // A scroll container keeps its content out of the area above it:
            // a scrollbar visual coming or going moves what it adds there.
            self.invalidate_scroll_content(id);
            self.scroll.layout_touched = true;
        }
        self.scroll.restyled.push(id);
    }

    /// Scroll containers whose viewport size a re-measure changed since the
    /// last call, still in the world.
    pub(crate) fn take_scroll_resized(&mut self) -> Vec<StableNodeId> {
        let mut ids = std::mem::take(&mut self.scroll.resized)
            .into_iter()
            .filter(|id| self.contains(*id))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// Scroll containers whose offset a re-measure clamped since the last
    /// call, still in the world.
    pub(crate) fn take_scroll_reclamped(&mut self) -> Vec<StableNodeId> {
        let mut ids = std::mem::take(&mut self.scroll.reclamped)
            .into_iter()
            .filter(|id| self.contains(*id))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }
}

fn scroll_viewport_size(metrics: Option<ScrollMetrics>) -> Option<(f32, f32)> {
    metrics.map(|metrics| (metrics.viewport_width, metrics.viewport_height))
}

impl UiWorld {
    /// The document boxes the commit wrote (`WriteLayout`), each kept
    /// measured from its node's frame once the commit's tree is in place:
    /// parents first, so a box under one written with it is measured from
    /// where that one now stands, whatever order they were written in. A box
    /// not written moves with its parent.
    pub(super) fn place_document_layouts(&mut self) {
        let mut written = std::mem::take(&mut self.invalidation.pending_document_layouts);
        if written.is_empty() {
            return;
        }
        // The last write of a box wins.
        let mut seen = NodeSet::default();
        written.reverse();
        written.retain(|(id, _)| seen.insert(*id) && self.nodes.contains(*id));
        let depth = |world: &Self, id: StableNodeId| {
            std::iter::successors(world.parent_id(id), |id| world.parent_id(*id)).count()
        };
        let mut ordered: Vec<(usize, StableNodeId, LayoutBox)> = written
            .into_iter()
            .map(|(id, layout)| (depth(self, id), id, layout))
            .collect();
        ordered.compact_sort_by_key(|&(depth, id, _)| (depth, id));
        for (_, id, layout) in ordered {
            let frame = self.layout_frame_from_styles(id);
            let [x, y] = self.frame_origin_of(self.record(id), frame);
            let local = LayoutBox {
                x: layout.x - x,
                y: layout.y - y,
                ..layout
            };
            self.write_local_layout(id, local, frame);
        }
    }

    /// Every box under a node whose position changed, measured from the
    /// frame its styles now give it, where it stands on the page: what a box
    /// is measured from follows its styles even before layout runs again.
    pub(super) fn reframe_layouts(&mut self) {
        let roots = std::mem::take(&mut self.invalidation.layout_reframe_roots);
        let mut seen = NodeSet::default();
        let mut open = roots;
        while let Some(id) = open.pop() {
            if !seen.insert(id) || !self.nodes.contains(id) {
                continue;
            }
            open.extend(self.child_ids(id).iter().copied());
            let frame = self.layout_frame_from_styles(id);
            if self.nodes.layout_frame(id) == frame {
                continue;
            }
            let [x, y] = self.layout_origin(id).unwrap_or_default();
            let [frame_x, frame_y] = self.frame_origin_of(self.record(id), frame);
            let geom = self.nodes.geometry_entry_mut(id);
            geom.set_frame(frame);
            geom.local.x = x - frame_x;
            geom.local.y = y - frame_y;
            self.nodes.note_node_space(id);
            self.defer_scroll_content(id);
        }
    }

    /// Write `id`'s box, `layout` measured from `frame`'s origin, and mark
    /// what reads it.
    pub(super) fn write_local_layout(
        &mut self,
        id: StableNodeId,
        layout: LayoutBox,
        frame: crate::LayoutFrame,
    ) {
        let prev_local = self.nodes.local_layout(id);
        let prev_frame = self.nodes.layout_frame(id);
        // A box that only moved leaves its text's constraints alone; one
        // that changed size is a new container for it.
        let resized = prev_local.width.to_bits() != layout.width.to_bits()
            || prev_local.height.to_bits() != layout.height.to_bits();
        let moved = prev_local.x.to_bits() != layout.x.to_bits()
            || prev_local.y.to_bits() != layout.y.to_bits()
            || prev_frame != frame;
        let omission_changed = (prev_local.width <= 0.0 || prev_local.height <= 0.0)
            != (layout.width <= 0.0 || layout.height <= 0.0);
        let reframed = prev_frame != frame;
        let geom = self.nodes.geometry_entry_mut(id);
        geom.local = layout;
        geom.set_frame(frame);
        if reframed {
            self.nodes.note_node_space(id);
        }
        if omission_changed && self.inline_parent(id).is_some() {
            self.mark_subtree(
                id,
                DirtyMask::RENDER | DirtyMask::ACCESSIBILITY | DirtyMask::INPUT,
            );
        }

        // A parent that sizes its surface from this box (a modal frame's
        // body, a popover's items): when the box changes, the parent's clip,
        // hit area and paint change with it although its own box did not.
        if (resized || moved)
            && let Some(parent) = self.parent_id(id)
            && self
                .widget_visual_ref(parent)
                .is_some_and(|visual| visual.paint_depends_on_child(id))
        {
            self.mark(
                parent,
                DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
        }
        if resized {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::CONSTRAINT);
            self.note_responsive_resize(id);
        }
        // Padding and fragment changes that keep this border box are
        // scheduled when their result publishes. Accessible bounds are the
        // offset from the parent's: a box that moved changed them.
        let mut bits = DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY;
        if moved {
            // What it holds moved with it, unmarked (`settle_layout_moves`).
            bits |= DirtyMask::MOVED;
        }
        if resized || moved {
            bits |= DirtyMask::RESHAPED;
            self.invalidation.layout_written.insert(id);
        }
        if resized {
            self.mark_moved_by_layout(id, bits, false);
        } else if moved {
            // Its hit entry is built again: the write that moved it may have
            // come with a sibling coming or going, which its entry reads.
            self.mark_moved_by_layout(id, bits, true);
        }
        if moved {
            self.invalidation.layout_moved_roots.push((id, !resized));
        }
    }

    /// The boxes the commit moved, each the root of what moves with it:
    /// nothing under one is marked. Every consumer keeps what a moved box
    /// holds in that box's coordinates -- hit entries, accessible bounds
    /// (relative to the parent's) and scene records -- and finds by itself
    /// what under it reads more than its own offset (pinned hit entries,
    /// [`UiWorld::project_accessibility_delta`]'s walk, the scene's frame
    /// chain). A root that only moved may keep its extracted record; a node
    /// below one whose visual's geometry reads the page
    /// ([`crate::WidgetVisual::geometry_reads_page`]) is extracted again: a
    /// renderer that places what paints where it stands now needs the
    /// geometry a cold extraction gives.
    pub(super) fn settle_layout_moves(&mut self) {
        let written = std::mem::take(&mut self.invalidation.layout_written);
        let roots = std::mem::take(&mut self.invalidation.layout_moved_roots);
        if roots.is_empty() {
            return;
        }
        let epoch = self.invalidation.render_epoch.get();
        for &(root, only_moved) in &roots {
            // Nothing but where it stands changed: the record extraction
            // made of it last may be reused with its new box.
            if only_moved && written.contains(&root) && self.nodes.contains(root) {
                self.record_mut(root).render_moved_epoch = epoch;
            }
        }
        // A component's geometry is in its own coordinates and moves with it,
        // except for one that reads the page (an open menu surface, which
        // places its items there): that is extracted again under a moved box.
        // An open select's menu is fitted to its room in the field's own
        // coordinates, which a move above it changes: extraction asks whether
        // its surface still lands where the record has it. The store keeps
        // those few apart (`NodeSpace::page_bound`), so no other visual is
        // looked at.
        if self.nodes.has_page_bound() {
            // Whether a box is a moved root or under one, remembered for
            // every box an answer passed.
            let moved_roots: NodeSet = roots.iter().map(|&(root, _)| root).collect();
            let mut under: NodeMap<bool> = moved_roots.iter().map(|&root| (root, true)).collect();
            let mut chain = Vec::new();
            let mut below_moves = |world: &Self, id: StableNodeId| {
                // A moved root's own record is decided where it is extracted.
                if moved_roots.contains(&id) {
                    return false;
                }
                let mut cursor = world.parent_id(id);
                let moved = loop {
                    let Some(at) = cursor else {
                        break false;
                    };
                    if let Some(&known) = under.get(&at) {
                        break known;
                    }
                    chain.push(at);
                    cursor = world.parent_id(at);
                };
                for at in chain.drain(..) {
                    under.insert(at, moved);
                }
                moved
            };
            let (mut reread, mut menus) = (Vec::new(), Vec::new());
            for id in self.nodes.page_bound() {
                if !below_moves(self, id) {
                    continue;
                }
                let space = self.record(id).space;
                if space.contains(crate::node_space::NodeSpace::READS_PAGE) {
                    reread.push(id);
                }
                if space.contains(crate::node_space::NodeSpace::MENU_ROOM) {
                    menus.push(id);
                }
            }
            for id in reread {
                self.mark(id, DirtyMask::RENDER);
            }
            for id in menus {
                self.nodes.set_render_room_epoch(id, epoch);
                self.mark_keeping_render_reason(id, DirtyMask::RENDER, MarkOrigin::General, false);
            }
        }
        self.bump_last_counters(|counters| {
            counters.record_layout_translations(roots.len(), 0);
        });
    }
}

pub(super) fn placement_already(
    style: &nana_ui_core::LayoutStyle,
    top: f32,
    height: Option<f32>,
) -> bool {
    let top_matches = matches!(
        style.offset_top,
        Some(nana_ui_core::LengthSpec::Px(current)) if current.to_bits() == top.to_bits()
    );
    let height_matches = match (style.height, height) {
        (None, None) => true,
        (Some(nana_ui_core::LengthSpec::Px(current)), Some(next)) => {
            current.to_bits() == next.to_bits()
        }
        _ => false,
    };
    top_matches && height_matches
}

/// Same field groups a style write emits when only the absolute top, or the
/// top and the imposed height, move.
fn placement_invalidation(height_changed: bool) -> nana_ui_core::LayoutInvalidation {
    let mut fields = nana_ui_core::LayoutStyleChange::POSITION;
    let mut kind = nana_ui_core::InvalidationKind::PLACEMENT;
    let mut footprint = nana_ui_core::LayoutDependencyFootprint::DEPENDS_ON_CHILD_METRICS
        .union(nana_ui_core::LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE)
        .union(nana_ui_core::LayoutDependencyFootprint::EXPORTS_INTRINSIC_BLOCK)
        .union(nana_ui_core::LayoutDependencyFootprint::DEPENDS_ON_CONTAINING_BLOCK);
    if height_changed {
        fields = fields.union(nana_ui_core::LayoutStyleChange::SIZING);
        kind = kind.union(nana_ui_core::InvalidationKind::MEASURE);
        footprint = footprint
            .union(nana_ui_core::LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT)
            .union(nana_ui_core::LayoutDependencyFootprint::CONSUMES_PARENT_BLOCK_CONSTRAINT)
            .union(nana_ui_core::LayoutDependencyFootprint::DEPENDS_ON_SIBLING_PREFIX)
            .union(nana_ui_core::LayoutDependencyFootprint::CONTEXT_LOCAL_COUPLING);
    }
    nana_ui_core::LayoutInvalidation::new(
        nana_ui_core::LayoutInvalidationSource::Author,
        nana_ui_core::InvalidationReason::STYLE,
        kind,
        fields,
        footprint,
    )
}
