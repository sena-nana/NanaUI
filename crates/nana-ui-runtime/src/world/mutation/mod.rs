//! Atomic mutation validation and application.

mod apply_editor;
mod apply_topology;
mod restyle;
mod validate;

pub(super) use validate::ValidationPlan;

use super::*;

impl UiWorld {
    pub(super) fn apply(&mut self, mutation: &UiMutation, report: &mut CommitReport) {
        if self.apply_restyle_or_text_mutation(mutation) {
            return;
        }
        if self.apply_editor_mutation(mutation) {
            return;
        }
        self.apply_topology_or_visual_mutation(mutation, report);
    }
}

impl UiWorld {
    /// Borrowing variant of [`commit`]: validate-then-apply against a queue
    /// the caller still owns. Validation runs fully before the apply loop, so
    /// a rejected batch never lands partially and the caller may replay it.
    pub fn commit_ref(&mut self, queue: &MutationQueue) -> Result<CommitReport, UiWorldError> {
        let publish_written_deferred = self.pass_publishes_written_boxes();
        self.commit_applying(queue, &[], None, publish_written_deferred)
    }

    /// `commit_ref` that also collects the root of every `Insert` and
    /// `ParkSubtree` it actually applied into `roots`.
    ///
    /// `placements` are a layout pass's writeback, committed exactly as the
    /// same boxes queued as [`UiMutation::PlaceLayout`] after `queue`'s
    /// mutations would be: each is validated as one and goes through the one
    /// loop below as one. A batch is either mutations or placements.
    fn commit_applying(
        &mut self,
        queue: &MutationQueue,
        placements: &[crate::mutation::LayoutPlacement],
        mut roots: Option<&mut Vec<StableNodeId>>,
        publish_written_deferred: bool,
    ) -> Result<CommitReport, UiWorldError> {
        debug_assert!(
            queue.is_empty() || placements.is_empty(),
            "a batch is either mutations or a layout pass's placements"
        );
        let mut report =
            CommitReport::unchanged(self.invalidation.generation, queue.len() + placements.len());
        if queue.is_empty() && placements.is_empty() {
            return Ok(report);
        }
        let mut validated = Ok(());
        // Placements are a layout pass's writeback, validated as
        // `validate_simple_mutation` validates one.
        for placement in placements {
            if !self.contains(placement.id) {
                return Err(UiWorldError::MissingNode(placement.id));
            }
            if !validate::layout_is_valid(&placement.layout) {
                return Err(UiWorldError::InvalidLayout(placement.id));
            }
        }
        let caps = queue.capabilities();
        let mut scanned = 0usize;
        if !queue.is_empty() && !self.validate_simple_mutation(queue.as_slice())? {
            let mut plan = validate::ValidationPlan::new(self);
            validated = plan.validate_with_caps(queue.as_slice(), caps);
            scanned = plan.scanned;
            #[cfg(test)]
            let steps = plan.child_list_steps;
            #[cfg(test)]
            {
                self.tree.child_list_steps += steps;
            }
        }
        self.invalidation.validation_nodes_scanned = self
            .invalidation
            .validation_nodes_scanned
            .saturating_add(scanned);
        validated?;
        // A skipped no-op changes nothing, so the generation moves with the
        // first mutation that does; a batch of nothing but no-ops keeps it.
        let mut applied = false;
        let mut layout_inputs_changed = false;
        let mut layout_result_ids = Vec::new();
        let mutations = queue.as_slice();
        let mut max_create_id: Option<StableNodeId> = None;
        let mut create_count = 0usize;
        if caps.intersects(crate::mutation::BatchCapabilities::TOPOLOGY) {
            for mutation in mutations {
                if let UiMutation::Create { id, .. } = mutation {
                    create_count += 1;
                    max_create_id = Some(max_create_id.map_or(*id, |current| current.max(*id)));
                }
            }
        }
        if let Some(highest) = max_create_id {
            self.nodes
                .reserve_nodes_through(highest, self.nodes.len() + create_count);
        }
        let mut run_end = 0;
        self.tree.renumber_later = Some(Default::default());
        for index in 0..mutations.len() + placements.len() {
            let placed;
            let mutation = match mutations.get(index) {
                Some(mutation) => mutation,
                None => {
                    let placement = placements[index - mutations.len()];
                    placed = UiMutation::PlaceLayout {
                        id: placement.id,
                        layout: placement.layout,
                        frame: Box::new(placement.frame),
                    };
                    &placed
                }
            };
            if caps.intersects(crate::mutation::BatchCapabilities::TOPOLOGY) && index >= run_end {
                // A placement removes nothing: it is a run of its own.
                run_end = index
                    + mutations
                        .get(index..)
                        .filter(|rest| !rest.is_empty())
                        .map_or(1, |rest| self.unlink_removal_run(rest));
            }
            if self.is_structural_noop(mutation) {
                self.note_noop_text_scale(mutation);
                continue;
            }
            if !applied {
                applied = true;
                self.invalidation.generation = self.invalidation.generation.wrapping_add(1);
                report.generation = self.invalidation.generation;
            }
            if !matches!(
                mutation,
                UiMutation::WriteLayout { .. } | UiMutation::PlaceLayout { .. }
            ) {
                layout_inputs_changed = true;
            }
            let layout_result_id = match mutation {
                UiMutation::WriteLayout { .. } | UiMutation::PlaceLayout { .. }
                    if publish_written_deferred =>
                {
                    None
                }
                UiMutation::WriteLayout { id, .. }
                | UiMutation::PlaceLayout { id, .. }
                | UiMutation::SetScrollOffset { id, .. }
                | UiMutation::SetScrollOverscroll { id, .. }
                | UiMutation::SetScrollMetrics { id, .. } => Some(*id),
                _ => None,
            };
            let seeds_before = self.invalidation.layout_seeds_created;
            self.apply(mutation, &mut report);
            if matches!(
                mutation,
                UiMutation::RequestFocus { .. }
                    | UiMutation::SetInteraction { .. }
                    | UiMutation::SetAccessibility { .. }
            ) {
                self.note_state_seeds(seeds_before);
            }
            if let Some(id) = layout_result_id {
                layout_result_ids.push(id);
            }
            if let Some(roots) = roots.as_deref_mut() {
                match mutation {
                    UiMutation::ParkSubtree { root } => roots.push(*root),
                    UiMutation::Insert { child, .. } => roots.push(*child),
                    _ => {}
                }
            }
        }
        // Results are published below.
        self.renumber_children_left();
        debug_assert!(
            self.tree.unlinked_ahead.is_empty(),
            "a removal run left children its mutations never reached"
        );
        if !applied {
            return Ok(report);
        }
        self.reframe_layouts();
        self.place_document_layouts();
        self.flush_scroll_content();
        self.remeasure_scroll_containers();
        for (id, offset) in std::mem::take(&mut self.scroll.requested) {
            if self.contains(id) {
                self.scroll_to_clamped(id, offset);
            }
        }
        // Containers this commit resized read their rules now: a variant
        // that changed is a style change the frame's next round lays out.
        self.evaluate_responsive();
        self.settle_layout_moves();
        self.publish_layout_results(&layout_result_ids, crate::LayoutResultSource::RuntimeLayout);
        if layout_inputs_changed {
            self.note_layout_source_change();
        }
        #[cfg(debug_assertions)]
        self.audit_hit_eligibility("commit");
        #[cfg(any(test, feature = "layout-verify"))]
        self.nodes.check_node_spaces();
        Ok(report)
    }
}

impl UiWorld {
    pub fn commit(&mut self, queue: MutationQueue) -> Result<CommitReport, UiWorldError> {
        self.commit_ref(&queue)
    }

    /// Commit through the normal atomic validator, then derive lifecycle work
    /// from the final topology. A descendant extracted from a parked ancestor
    /// must not inherit that ancestor's suspension bookkeeping.
    pub(crate) fn commit_with_mount_lifecycle(
        &mut self,
        queue: MutationQueue,
        placements: &[crate::mutation::LayoutPlacement],
    ) -> Result<(CommitReport, NodeSet, NodeSet), UiWorldError> {
        // A root put back where it already was mounts or parks nothing, so
        // only the ones actually applied count.
        let mut roots = Vec::new();
        let publish_written_deferred = self.pass_publishes_written_boxes();
        let mut report = self.commit_applying(
            &queue,
            placements,
            Some(&mut roots),
            publish_written_deferred,
        )?;
        let mut parked = NodeSet::default();
        let mut inserted = NodeSet::default();
        let mut visited = NodeSet::default();
        while let Some(id) = roots.pop() {
            if !self.contains(id) || !visited.insert(id) {
                continue;
            }
            // Insert roots can be nested in the final tree; visit their union
            // once instead of rescanning each ancestor's entire subtree.
            roots.extend(self.record(id).hierarchy.children.iter().copied());
            if self.mount_state(id) == Some(MountState::Parked) {
                parked.insert(id);
            } else if self.is_mounted(id) {
                inserted.insert(id);
            }
        }
        for &id in &parked {
            #[cfg(feature = "charts")]
            if let Some(spec) = self
                .widget_visual_as::<crate::ChartVisual>(id)
                .filter(|visual| visual.hover != crate::ChartHoverVisual::default())
                .map(|visual| Arc::clone(&visual.spec))
            {
                // A parked chart shows nothing hovered.
                self.apply(
                    &UiMutation::SetWidgetVisual {
                        id,
                        visual: Some(Box::new(crate::WidgetVisualHandle::new(
                            crate::ChartVisual {
                                spec,
                                hover: crate::ChartHoverVisual::default(),
                                selection: None,
                            },
                        ))),
                    },
                    &mut report,
                );
                report.mutations += 1;
            }
            let Some(mut visual) = self.widget_visual_as::<crate::IconVisual>(id).cloned() else {
                continue;
            };
            if let Some(tooltip) = visual.tooltip.as_mut()
                && tooltip.open
            {
                tooltip.open = false;
                // This is a derived write to a validated, surviving node. Use
                // normal mutation application (dirty/extract accounting) within
                // the same generation, without a second observable commit.
                self.apply(
                    &UiMutation::SetWidgetVisual {
                        id,
                        visual: Some(Box::new(crate::WidgetVisualHandle::new(visual))),
                    },
                    &mut report,
                );
                report.mutations += 1;
            }
        }
        Ok((report, parked, inserted))
    }
}
