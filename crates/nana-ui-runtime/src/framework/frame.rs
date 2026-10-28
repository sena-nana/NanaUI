//! AppContext frame operations.

use super::*;

impl AppContext {
    /// Drain deterministic work scheduled since the previous frame.
    ///
    /// This is the canonical application/frame drain. It polls local tasks
    /// and flushes the reactive host before delegating to
    /// [`crate::UiWorld::take_system_work`]. Callers with an [`AppContext`]
    /// should use this method instead of draining `compat_world_mut()`
    /// directly, because the latter bypasses those runtime steps.
    pub fn take_system_work(&mut self) -> crate::SystemWork {
        // Signal writes outside input (timers, program updates, other
        // contexts) land here, before the frame reads the world.
        crate::view::poll_tasks();
        if self.flush_reactive().is_err() {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FLUSH_FAILED);
        }
        self.world.take_system_work()
    }

    /// [`Self::take_system_work`] for `document`'s frame: the world-wide work
    /// drains as usual, and of the layout seeds only `document`'s. Another
    /// document's edits stay pending for its own frame. A host that draws
    /// several documents from one context drains each frame through this.
    pub fn take_system_work_for(&mut self, document: DocumentId) -> crate::SystemWork {
        crate::view::poll_tasks();
        if self.flush_reactive().is_err() {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::FLUSH_FAILED);
        }
        self.world.take_system_work_for(document)
    }

    /// Consume the host cursor-sync hint after a successful document frame.
    #[doc(hidden)]
    pub fn take_window_cursor_dirty(&mut self) -> bool {
        self.world.take_window_cursor_dirty()
    }

    /// Algorithm-level counters from the last drained system batch.
    pub fn last_work_counters(&self) -> crate::WorkCounters {
        self.world.last_work_counters()
    }

    /// The presentation host records actual encode/submit work after flush.
    /// This replaces the previous output observation, including on idle reuse.
    pub fn record_output_work(&mut self, work: nana_ui_core::OutputWorkObservation) {
        self.world.record_output_work(work);
    }

    /// Replace the last presentation's actual painter observation. `None`
    /// clears unsupported or skipped observations instead of retaining old work.
    pub fn record_presentation_gpu_work(&mut self, work: Option<nana_ui_core::GpuWorkObservation>) {
        self.world.record_presentation_gpu_work(work);
    }

    /// Record extract output onto the last drained work counters.
    pub fn record_extract(&mut self, extracted: &[crate::ExtractedNode]) {
        self.world.record_extract(extracted);
    }

    /// Open a product-frame profiler and work-counter accumulator.
    pub fn begin_frame_profile(&mut self) {
        self.frame_profiler = FrameProfiler::new();
        self.frame_profiler.mark_runtime_unsupported();
        self.profiling = true;
        self.world.begin_frame_counters();
    }

    pub fn finish_frame_profile(&mut self) {
        self.world.end_frame_counters();
        self.profiling = false;
        let profile = std::mem::take(&mut self.frame_profiler).finish();
        // Match last_work_counters: an idle flush (no stage ran) must not wipe
        // the last non-empty product profile.
        if profile.any_stage_ran() {
            record_frame_diagnostics(&profile);
            self.last_profile = profile;
        }
    }

    pub fn last_frame_profile(&self) -> &FrameProfile {
        &self.last_profile
    }

    pub(super) fn stage_clock(&self) -> Option<Instant> {
        self.profiling.then(Instant::now)
    }

    pub(super) fn record_stage(&mut self, stage: FrameStage, started: Option<Instant>) {
        if let Some(started) = started {
            self.frame_profiler.record(stage, started.elapsed());
        }
    }

    /// Record a stage duration while a product frame is open.
    pub fn time_stage_duration(&mut self, stage: FrameStage, duration: Duration) {
        if self.profiling {
            self.frame_profiler.record(stage, duration);
        }
    }

    /// Return a drained system batch to the scheduler after a canonical frame
    /// fails. Frame drivers should restore every consumed batch before retry.
    pub fn restore_system_work(&mut self, work: crate::SystemWork) {
        self.world.restore_system_work(work);
    }

    /// Resolve inherited style for the supplied dirty nodes.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), FrameworkError> {
        let started = self.stage_clock();
        let previous_focus = ids
            .iter()
            .filter_map(|id| self.world.document_of(*id))
            .filter_map(|document| self.world.focused(document).map(|node| (document, node)))
            .collect::<HashMap<_, _>>();
        let result = self.world.resolve_styles(ids).map_err(FrameworkError::from);
        if result.is_ok() {
            for (document, previous) in previous_focus {
                self.seal_blurred_editor_history(document, Some(previous));
            }
        }
        self.record_stage(FrameStage::Style, started);
        result
    }

    /// Derive registered text presentations for scheduled nodes.
    pub fn resolve_presentations(&mut self, ids: &[StableNodeId]) -> Result<(), FrameworkError> {
        self.world
            .resolve_presentations(ids)
            .map_err(FrameworkError::from)
    }

    /// Shape only scheduled text through the host's real text backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl crate::TextShaper,
    ) -> Result<(), FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text(ids, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        result?;
        self.reproject_text_backend_views()?;
        self.refresh_shaped_text_area_layouts()?;
        // Clamps are announced from the layout-aware passes: before layout
        // a text has no width to clamp against.
        Ok(())
    }

    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text_for_layout(document, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        let reshaped = result?;
        let reprojected =
            self.reproject_text_backend_views()? | self.refresh_shaped_text_area_layouts()?;
        self.announce_text_clamps()?;
        Ok(reprojected || reshaped)
    }

    /// [`Self::shape_text_for_layout`] restricted to `ids` (the last layout
    /// scope): nodes outside it keep shapes matching their unchanged boxes.
    pub fn shape_text_for_layout_scoped(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text_for_layout_scoped(ids, shaper)
            .map_err(FrameworkError::from);
        self.record_stage(FrameStage::TextShape, started);
        let reshaped = result?;
        let reprojected =
            self.reproject_text_backend_views()? | self.refresh_shaped_text_area_layouts()?;
        self.announce_text_clamps()?;
        Ok(reprojected || reshaped)
    }

    /// Compute and atomically publish canonical Runtime layout for one window.
    ///
    /// Full pass: recomputes every box and rebuilds the retained layout cache.
    /// The product frame never calls it: edits go through
    /// [`Self::layout_document_with_frontier`] and viewport changes through
    /// [`Self::layout_document_for_viewport`]. It is for a layout outside the
    /// frame driver (tests, tools, a first layout) and counts as a
    /// `layout_full_invocations`, not as a `layout_full_document_fallbacks`.
    pub fn layout_document(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_document_impl(document, viewport, &[], true)
    }

    /// Incremental layout driven by typed invalidation seeds. Clean subtrees
    /// reuse the retained cache, so the cost scales with the dependency
    /// frontier rather than the document. [`Self::take_last_layout_scope`]
    /// reports the recomputed set for scoped text re-shaping.
    pub fn layout_document_with_frontier(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        seeds: &[crate::LayoutFrontierSeed],
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_document_impl_with_seeds(document, viewport, &[], false, Some(seeds))
    }

    /// Relayout after a viewport change, with `seeds` the frame drained for
    /// its other edits. The roots lay out again against the new viewport,
    /// and of their descendants only the boxes that consume the axis it
    /// moved on, and the `position: fixed` / `vw` / `vh` boxes that resolve
    /// against it; unchanged subtrees keep the retained cache.
    pub fn layout_document_for_viewport(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        seeds: &[crate::LayoutFrontierSeed],
    ) -> Result<crate::CommitReport, FrameworkError> {
        // The engine plants the resize's seeds against the viewport its cache
        // was laid out at. A document laid out for the first time has none:
        // it lays out whole.
        let mut typed = if self.world.document_viewport(document).is_none() {
            self.world.viewport_resize_seeds(document, None, viewport)
        } else {
            Vec::new()
        };
        typed.extend_from_slice(seeds);
        let dirty = typed.iter().map(|seed| seed.node).collect::<Vec<_>>();
        self.layout_document_impl_with_seeds(document, viewport, &dirty, false, Some(&typed))
    }

    /// Nodes recomputed by the most recent layout pass; drains on read.
    pub fn take_last_layout_scope(&mut self) -> Vec<StableNodeId> {
        std::mem::take(&mut self.last_layout_scope)
    }

    /// Frontier counters of the most recent layout pass for `document`,
    /// including the size of the dependency graph it built.
    pub fn layout_frontier_stats(&self, document: DocumentId) -> crate::LayoutFrontierStats {
        self.layout_cache.frontier_stats(document)
    }

    /// What the retained layout cache holds for `document`.
    #[cfg(test)]
    pub(crate) fn retained_layout_footprint(
        &self,
        document: DocumentId,
    ) -> crate::layout_engine::RetainedLayoutFootprint {
        self.layout_cache.footprint(document)
    }

    /// Drain typed layout seeds emitted after the current work batch, such as
    /// intrinsic changes discovered while shaping text.
    pub fn take_layout_frontier_seeds(
        &mut self,
        document: DocumentId,
    ) -> Vec<crate::LayoutFrontierSeed> {
        self.world.take_layout_frontier_seeds(document)
    }

    /// Full (`force_full`) layout passes, which discard the retained cache.
    pub fn layout_full_invocations(&self) -> usize {
        self.layout_full_invocations
    }

    /// Layout passes, scoped and full.
    pub fn layout_invocations(&self) -> usize {
        self.layout_invocations
    }

    pub(super) fn layout_document_impl(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
        force_full: bool,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_document_impl_with_seeds(document, viewport, dirty, force_full, None)
    }

    fn layout_document_impl_with_seeds(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
        force_full: bool,
        supplied_seeds: Option<&[crate::LayoutFrontierSeed]>,
    ) -> Result<crate::CommitReport, FrameworkError> {
        let mut typed_seeds = if force_full {
            self.world.clear_layout_frontier_seeds(document);
            Vec::new()
        } else {
            supplied_seeds
                .map(|seeds| seeds.to_vec())
                .unwrap_or_default()
        };
        if !force_full {
            typed_seeds.extend(self.world.take_layout_frontier_seeds(document));
        }
        #[cfg(not(feature = "benchmark"))]
        let result = self.layout_document_observed(
            document,
            viewport,
            dirty,
            &typed_seeds,
            force_full,
            |_| {},
        );
        // Time the production path itself rather than a benchmark-only
        // reimplementation of it: the sub-stage clocks are the same
        // `completed` hook the full-layout benchmark already uses.
        #[cfg(feature = "benchmark")]
        let result = {
            let mut substages = [Duration::ZERO; 4];
            let mut started = Instant::now();
            let result = self.layout_document_observed(
                document,
                viewport,
                dirty,
                &typed_seeds,
                force_full,
                |stage| {
                    substages[stage] = started.elapsed();
                    started = Instant::now();
                },
            );
            for (total, elapsed) in self.layout_substage_totals.iter_mut().zip(substages) {
                *total += elapsed;
            }
            result
        };
        if result.is_err() {
            self.world.restore_layout_frontier_seeds(&typed_seeds);
        }
        result
    }

    /// Drain the per-sub-stage totals accumulated inside the Layout stage:
    /// `[position_open_tooltips, engine, writeback + commit, scroll metrics]`.
    #[cfg(feature = "benchmark")]
    pub fn take_layout_substage_totals(&mut self) -> [Duration; 4] {
        std::mem::take(&mut self.layout_substage_totals)
    }

    /// Diagnostic timings for the canonical full layout path. The production
    /// entry points use the same operations with no per-stage clock callbacks.
    #[cfg(feature = "benchmark")]
    pub fn benchmark_layout_document(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<[Duration; 4], FrameworkError> {
        let mut timings = [Duration::ZERO; 4];
        let mut started = Instant::now();
        self.layout_document_observed(document, viewport, &[], &[], true, |stage| {
            timings[stage] = started.elapsed();
            started = Instant::now();
        })?;
        Ok(timings)
    }

    fn layout_document_observed(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
        typed_seeds: &[crate::LayoutFrontierSeed],
        force_full: bool,
        mut completed: impl FnMut(usize),
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_invocations += 1;
        if force_full {
            self.layout_full_invocations += 1;
        }
        let started = self.stage_clock();
        // Diagnostics time layout even outside a profiled frame.
        let diagnostics_clock =
            started.or_else(|| nana_diagnostics::metrics_enabled().then(Instant::now));
        self.world.set_document_viewport(document, viewport);
        // A seed of another document is its own frame's to lay out; this pass
        // would only drop it, and the edit with it.
        let own_seeds: Vec<crate::LayoutFrontierSeed>;
        let typed_seeds = if typed_seeds
            .iter()
            .all(|seed| self.world.document_of(seed.node) == Some(document))
        {
            typed_seeds
        } else {
            let (own, theirs): (Vec<_>, Vec<_>) = typed_seeds
                .iter()
                .copied()
                .partition(|seed| self.world.document_of(seed.node) == Some(document));
            self.world.requeue_layout_seeds(theirs);
            own_seeds = own;
            &own_seeds
        };
        let result = (|| {
            self.position_open_tooltips(document)?;
            completed(0);
            let layouts = crate::RuntimeLayoutEngine.layout_document_with_frontier(
                &self.world,
                document,
                viewport,
                typed_seeds,
                &mut self.layout_cache,
                force_full,
            )?;
            let intrinsic_counters = self.layout_cache.take_intrinsic_counters();
            self.world
                .record_intrinsic_measure_counters(intrinsic_counters);
            let frontier = self.layout_cache.frontier_stats(document);
            let execution = self.layout_cache.execution_stats(document);
            self.world.record_layout_frontier(frontier);
            self.world.record_layout_execution(execution);
            if execution.dynamic.contexts_considered > 0 {
                use nana_diagnostics::{framework::layout, metric};
                let dynamic = execution.dynamic;
                metric!(layout::DYNAMIC_SOLVES, dynamic.solver_runs);
                metric!(layout::DYNAMIC_ENVELOPE_REBUILDS, dynamic.envelope_rebuilds);
                metric!(layout::DYNAMIC_BUDGET_FALLBACKS, dynamic.budget_fallbacks);
            }
            if frontier.constraint.seeds > 0 {
                self.world.record_resize_context_solves(
                    execution.plan_misses + execution.containers_uncacheable,
                );
            }
            completed(1);
            #[cfg(feature = "benchmark")]
            let mut phase = crate::layout_engine::plan_stats::PhaseClock::start();
            let mut mutations = MutationQueue::new();
            let mut scope = Vec::with_capacity(layouts.len());
            let mut context_children = Vec::new();
            for (id, layout) in layouts {
                scope.push(id);
                let padding_changed = self
                    .layout_cache
                    .used_padding(document, id)
                    .is_some_and(|padding| self.world.write_layout_padding(id, padding));
                // A turned scroll origin re-measures even when no box moved.
                let far_start_changed = self
                    .layout_cache
                    .far_start(document, id)
                    .is_some_and(|far| self.world.write_layout_far_start(id, far));
                // A different context places the children differently but can
                // leave every box where it was; the publish below rebuilds the
                // fragments under the new kind either way.
                if let Some(context) = self.layout_cache.formatting_context(document, id)
                    && self.world.write_formatting_context(id, context)
                {
                    context_children.extend_from_slice(self.world.child_ids(id));
                }
                if padding_changed
                    || far_start_changed
                    || self.world.component_layout_box(id) != Some(layout)
                {
                    mutations.write_layout(id, layout);
                }
            }
            #[cfg(feature = "benchmark")]
            phase.lap(10);
            // Participation is published even when a context transition leaves
            // a fixed child's border box unchanged and placement reuses it.
            if !context_children.is_empty() {
                let mut included: crate::NodeSet = scope.iter().copied().collect();
                for child in context_children {
                    if included.insert(child) {
                        scope.push(child);
                    }
                }
            }
            let mut settings_rows = Vec::new();
            let mut terminal_sizes = Vec::new();
            let mut adaptive_editors = Vec::new();
            for id in &scope {
                let Some(view) = self.views.get(id) else {
                    continue;
                };
                if view
                    .downcast_ref::<TextArea>()
                    .is_some_and(TextArea::uses_adaptive_footer)
                {
                    adaptive_editors.push(*id);
                }
                if view.is::<crate::TerminalView>() {
                    terminal_sizes.push(*id);
                } else if view
                    .downcast_ref::<crate::SettingsRow>()
                    .is_some_and(|row| row.stack_below.is_some() && !row.stacked)
                {
                    settings_rows.push(*id);
                }
            }
            #[cfg(feature = "benchmark")]
            phase.lap(11);
            let report = self.commit_mutations(mutations)?;
            #[cfg(feature = "benchmark")]
            phase.lap(12);
            // Publish one coherent canonical Result snapshot only after every
            // sibling box in the scope has been committed. Interim LayoutBox
            // writes published during commit are replaced with the
            // RuntimeLayout provenance here.
            self.world
                .publish_layout_results(&scope, crate::LayoutResultSource::RuntimeLayout);
            let inline_objects = self.layout_cache.take_inline_objects(document);
            self.world.sync_inline_metrics(
                &scope,
                |id| self.layout_cache.intrinsic_baseline(document, id),
                inline_objects,
            );
            #[cfg(feature = "benchmark")]
            phase.lap(13);
            self.last_layout_scope = scope;
            for id in adaptive_editors {
                self.refresh_text_area_compact_layout(id)?;
            }
            for id in settings_rows {
                self.refresh_settings_row_layout(id)?;
            }
            for id in terminal_sizes {
                if let Some(bounds) = self.world.component_layout_box(id) {
                    self.resize_terminal_view(
                        Entity::from_stable_id(id),
                        bounds.width,
                        bounds.height,
                    )?;
                }
            }
            completed(2);
            self.apply_document_scroll_retention(document, force_full)?;
            // After scroll retention: a clamped offset moves the room too.
            self.refresh_select_menu_rooms(document)?;
            self.announce_size_changes(document)?;
            self.play_pending_flips(document);
            // A toast stack's new toasts were measured by this pass: what
            // shows follows from their heights.
            self.reconcile_toast_stacks(document)?;
            completed(3);
            Ok(report)
        })();
        self.record_stage(FrameStage::Layout, started);
        if let Some(clock) = diagnostics_clock {
            use nana_diagnostics::{framework::layout, metric};
            metric!(layout::PASS_NS, clock.elapsed());
            metric!(layout::INVOCATIONS);
            if force_full {
                metric!(layout::FULL_INVOCATIONS);
            }
            metric!(layout::DIRTY_ROOTS, dirty.len());
            if result.is_ok() {
                metric!(layout::BOXES_LAID_OUT, self.last_layout_scope.len());
            }
        }
        result
    }

    /// After a layout pass, whose commit already re-measured the scroll
    /// containers: announce the `ScrollView` offsets it clamped, then apply
    /// follow-end / anchor retention.
    pub(super) fn apply_document_scroll_retention(
        &mut self,
        document: DocumentId,
        force_full: bool,
    ) -> Result<(), FrameworkError> {
        for id in self.world.take_scroll_reclamped() {
            if !self.is_scroll_view(id) {
                continue;
            }
            let offset = self.world.scroll_offset(id).unwrap_or_default();
            self.update(Entity::<ScrollView>::from_stable_id(id), |_, cx| {
                cx.emit(ScrollChanged { offset });
            })?;
        }
        let asked: Vec<_> = self
            .laid_out_notices
            .iter()
            .copied()
            .filter(|id| {
                self.world
                    .node(*id)
                    .is_some_and(|node| node.document == document)
            })
            .collect();
        for id in asked {
            self.laid_out_notices.remove(&id);
            if self.is_scroll_view(id) {
                self.update(Entity::<ScrollView>::from_stable_id(id), |_, cx| {
                    cx.emit(ScrollLaidOut);
                })?;
            }
        }
        for id in self.world.take_scroll_resized() {
            let Some(metrics) = self.world.scroll_metrics(id) else {
                continue;
            };
            if !self.is_scroll_view(id) {
                continue;
            }
            let (width, height) = (metrics.viewport_width, metrics.viewport_height);
            self.update(Entity::<ScrollView>::from_stable_id(id), |_, cx| {
                cx.emit(ScrollViewportChanged { width, height });
            })?;
        }
        let targets = if force_full {
            self.world
                .document_order(document)
                .into_iter()
                .filter(|id| self.is_scroll_view(*id))
                .collect()
        } else {
            self.scoped_scroll_retention_targets(document)
        };
        for id in targets {
            self.apply_scroll_retention(Entity::<ScrollView>::from_stable_id(id))?;
        }
        Ok(())
    }

    pub(super) fn scoped_scroll_retention_targets(
        &self,
        document: DocumentId,
    ) -> Vec<StableNodeId> {
        let mut visited = HashSet::new();
        let mut targets = HashSet::new();
        for &id in &self.last_layout_scope {
            if self.world.document_of(id) != Some(document) {
                continue;
            }
            let mut cursor = Some(id);
            while let Some(id) = cursor {
                if !visited.insert(id) {
                    break;
                }
                if self.is_scroll_view(id) && self.world.presence_live(id) {
                    targets.insert(id);
                }
                cursor = self.world.parent_id(id);
            }
        }
        if targets.len() <= 1 {
            return targets.into_iter().collect();
        }
        // Preserve document order without descending into unrelated subtrees.
        let mut branches = HashSet::new();
        let mut branch_parents = HashSet::new();
        for &id in &targets {
            let mut cursor = Some(id);
            while let Some(id) = cursor {
                if !branches.insert(id) {
                    break;
                }
                cursor = self.world.parent_id(id);
                if let Some(parent) = cursor {
                    branch_parents.insert(parent);
                }
            }
        }
        let mut ordered = Vec::with_capacity(targets.len());
        let mut stack = self.world.document_roots(document);
        stack.reverse();
        while let Some(id) = stack.pop() {
            if !branches.contains(&id) {
                continue;
            }
            if targets.contains(&id) {
                ordered.push(id);
            }
            if ordered.len() == targets.len() {
                break;
            }
            if branch_parents.contains(&id)
                && let Some(node) = self.world.node(id)
            {
                stack.extend(
                    node.children
                        .into_iter()
                        .rev()
                        .filter(|id| branches.contains(id)),
                );
            }
        }
        ordered
    }

    /// Re-queue LAYOUT after a host drained a frame without measuring.
    pub fn defer_layout(&mut self, ids: &[StableNodeId]) {
        for &id in ids {
            self.world.mark_layout(id);
        }
    }

    /// Rebuild the compact hit index for one document after layout or input
    /// work. The retained hierarchy remains private to this context.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let started = self.stage_clock();
        self.world.rebuild_hit_test(document);
        self.record_stage(FrameStage::HitTest, started);
    }

    /// Patch only the subtrees covering `dirty`, falling back to a full document
    /// rebuild when the change is structural. See
    /// [`UiWorld::rebuild_hit_test_scoped`].
    pub fn rebuild_hit_test_for(&mut self, document: DocumentId, dirty: &[StableNodeId]) {
        let started = self.stage_clock();
        if !self.world.rebuild_hit_test_scoped(document, dirty) {
            self.world.rebuild_hit_test(document);
        }
        self.record_stage(FrameStage::HitTest, started);
    }

    /// Drain recorded scroll deltas for the in-place hit-index patch.
    pub fn take_scroll_hit_updates(&mut self) -> Vec<(StableNodeId, [f32; 2])> {
        self.world.take_scroll_hit_updates()
    }

    /// See [`UiWorld::hit_test_work_is_scroll_only`].
    pub fn hit_test_work_is_scroll_only(
        &self,
        input: &[StableNodeId],
        updates: &[(StableNodeId, [f32; 2])],
    ) -> bool {
        self.world.hit_test_work_is_scroll_only(input, updates)
    }

    /// Pre-compose a scroll translation onto the scroller subtree's hit
    /// entries instead of rebuilding the document index.
    pub fn update_hit_test_scroll(
        &mut self,
        document: DocumentId,
        scroller: StableNodeId,
        delta: [f32; 2],
    ) {
        let started = self.stage_clock();
        self.world.update_hit_test_scroll(document, scroller, delta);
        self.record_stage(FrameStage::HitTest, started);
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.world
            .next_animation_deadline()
            .into_iter()
            .chain(
                self.component_lifecycle
                    .tooltips
                    .iter()
                    .filter(|(target, _)| self.world.is_mounted(**target))
                    .filter_map(|(_, tooltip)| tooltip.show_at),
            )
            .chain(
                self.component_lifecycle
                    .hover_cards
                    .iter()
                    .filter(|(target, _)| self.world.is_mounted(**target))
                    .filter_map(|(_, lifecycle)| lifecycle.show_at.or(lifecycle.close_at)),
            )
            .chain(self.auto_overlay_deadline())
            .chain(self.scrollbar_hold_deadline())
            .chain(self.wheel_scroll_deadline())
            .chain(self.touch_scroll_deadline())
            .chain(self.caret_blink_deadline())
            .chain(self.rich_drag_deadline())
            .chain(self.plain_drag_deadline())
            .min()
    }

    /// Whether a split handle hover probe may run now for this document;
    /// records the probe when true. Split-pane pointermove handling gates the
    /// probe to one per frame interval because a probe outside every handle
    /// slop walks the whole document.
    pub(crate) fn begin_split_hover_probe(&mut self, document: DocumentId, now: Duration) -> bool {
        self.component_lifecycle
            .begin_split_hover_probe(document, now)
    }

    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        self.component_lifecycle.now = now;
        let mut frame = self.world.advance_animations(now);
        self.advance_wheel_scrolls(now, &mut frame);
        self.advance_touch_scrolls(now, &mut frame);
        self.advance_caret_blink(now, &mut frame);
        if let Some(id) = self.advance_rich_drag(now) {
            frame.component_updates.push(id);
        }
        if let Some(id) = self.advance_plain_drag(now) {
            frame.component_updates.push(id);
        }
        let finished_surfaces = frame
            .samples
            .iter()
            .filter(|sample| {
                sample.finished
                    && (crate::component_animation_id(
                        crate::component_animation_kinds::SURFACE,
                        sample.target,
                    ) == Some(sample.id)
                        || crate::component_animation_id(
                            crate::component_animation_kinds::SURFACE_POP,
                            sample.target,
                        ) == Some(sample.id)
                        || crate::component_animation_id(
                            crate::component_animation_kinds::SURFACE_SCRIM,
                            sample.target,
                        ) == Some(sample.id))
                    && !self.world.surface_closing(sample.target)
            })
            .map(|sample| sample.target)
            .collect::<Vec<_>>();
        for target in finished_surfaces {
            let _ = self.finish_surface_exit(target);
        }
        self.finish_leaves(&frame.events);
        let tooltip_targets = self
            .component_lifecycle
            .tooltips
            .iter()
            .filter_map(|(&target, tooltip)| {
                if !self.world.is_mounted(target) {
                    return None;
                }
                tooltip.show_at.filter(|deadline| *deadline <= now)?;
                Some(target)
            })
            .collect::<Vec<_>>();
        for target in tooltip_targets {
            if self.open_tooltip(target).unwrap_or(false) {
                frame.component_updates.push(target);
            }
        }
        for bar in self.drive_due_auto_overlays() {
            if !frame.component_updates.contains(&bar) {
                frame.component_updates.push(bar);
            }
        }
        for scroll in self.release_scrollbar_holds(now) {
            if !frame.component_updates.contains(&scroll) {
                frame.component_updates.push(scroll);
            }
        }
        self.sweep_hover_cards();
        let hover_targets = self
            .component_lifecycle
            .hover_cards
            .iter()
            .filter(|(target, _)| self.world.is_mounted(**target))
            .filter_map(|(&target, lifecycle)| {
                if lifecycle.show_at.is_some_and(|deadline| deadline <= now) {
                    return Some((target, true));
                }
                if lifecycle.close_at.is_some_and(|deadline| deadline <= now) {
                    return Some((target, false));
                }
                None
            })
            .collect::<Vec<_>>();
        for (target, open) in hover_targets {
            let changed = if open {
                self.open_hover_card(target).unwrap_or(false)
            } else {
                self.close_hover_card(target).unwrap_or(false)
            };
            if changed {
                frame.component_updates.push(target);
            }
        }
        let loading_targets = frame
            .samples
            .iter()
            .filter(|sample| {
                crate::component_animation_id(
                    crate::component_animation_kinds::LOADING,
                    sample.target,
                ) == Some(sample.id)
            })
            .map(|sample| (sample.target, sample.progress))
            .collect::<Vec<_>>();
        for (target, phase) in loading_targets {
            let Some(kind) = self.component_lifecycle.loading.get(&target).copied() else {
                continue;
            };
            let changed = match kind {
                LoadingComponent::Button => self
                    .update_component(Entity::<Button>::from_stable_id(target), |button, _| {
                        button.loading_phase = phase;
                    })
                    .is_ok(),
                LoadingComponent::Switch => self
                    .update_component(Entity::<Switch>::from_stable_id(target), |switch, _| {
                        switch.loading_phase = phase;
                    })
                    .is_ok(),
                LoadingComponent::Card => self
                    .update_component(Entity::<crate::Card>::from_stable_id(target), |card, _| {
                        card.loading_phase = phase;
                    })
                    .is_ok(),
            };
            if changed && !frame.component_updates.contains(&target) {
                frame.component_updates.push(target);
            }
        }
        let section_targets = frame
            .samples
            .iter()
            .filter(|sample| {
                crate::component_animation_id(
                    crate::component_animation_kinds::SIDEBAR,
                    sample.target,
                ) == Some(sample.id)
            })
            .map(|sample| (sample.target, sample.progress, sample.value))
            .collect::<Vec<_>>();
        for (target, progress, value) in section_targets {
            let expansion = match value {
                crate::MotionValue::Scalar(value) => value,
                _ => progress,
            };
            if self
                .update_component(
                    Entity::<SidebarSection>::from_stable_id(target),
                    |section, _| {
                        section.animation_progress = expansion.clamp(0.0, 1.0);
                    },
                )
                .is_ok()
                // `advance_animations` already pushed this target for the
                // SIDEBAR sample, and the host reprojects once per entry:
                // without the guard its sibling loops all carry, a toggling
                // section is uploaded twice every frame of its transition.
                && !frame.component_updates.contains(&target)
            {
                frame.component_updates.push(target);
            }
        }
        let spinner_targets = frame
            .samples
            .iter()
            .filter(|sample| {
                crate::component_animation_id(
                    crate::component_animation_kinds::SPINNER,
                    sample.target,
                ) == Some(sample.id)
            })
            .map(|sample| (sample.target, sample.progress))
            .collect::<Vec<_>>();
        for (target, phase) in spinner_targets {
            if self
                .update_component(
                    Entity::<crate::Spinner>::from_stable_id(target),
                    |spinner, _| {
                        spinner.phase = phase;
                    },
                )
                .is_ok()
                && !frame.component_updates.contains(&target)
            {
                frame.component_updates.push(target);
            }
        }
        let expired_toasts = frame
            .samples
            .iter()
            .filter(|sample| {
                sample.finished
                    && crate::component_animation_id(
                        crate::component_animation_kinds::TOAST,
                        sample.target,
                    ) == Some(sample.id)
            })
            .map(|sample| sample.target)
            .collect::<Vec<_>>();
        for target in expired_toasts {
            if self.advance_toast_timer(target, now).unwrap_or(false) {
                let _ = self
                    .update_component(Entity::<crate::Toast>::from_stable_id(target), |_, cx| {
                        cx.emit(crate::ToastDismissed)
                    });
            }
        }
        let flashes = frame
            .samples
            .iter()
            .filter(|sample| {
                sample.finished
                    && crate::component_animation_id(
                        crate::component_animation_kinds::ATTENTION,
                        sample.target,
                    ) == Some(sample.id)
            })
            .map(|sample| (sample.target, sample.property))
            .collect::<Vec<_>>();
        for (target, property) in flashes {
            let _ = self.advance_settings_row_flash(target, property, now);
        }
        let workspace_targets = frame
            .samples
            .iter()
            .filter(|sample| {
                crate::component_animation_id(
                    crate::component_animation_kinds::WORKSPACE,
                    sample.target,
                ) == Some(sample.id)
            })
            .map(|sample| sample.target)
            .collect::<Vec<_>>();
        for id in workspace_targets {
            let advanced = self
                .update_component(Entity::<Workspace>::from_stable_id(id), |workspace, _| {
                    workspace.apply(WorkspaceMutation::AdvanceAnimations, now)
                })
                .unwrap_or(false);
            if advanced && !frame.component_updates.contains(&id) {
                frame.component_updates.push(id);
            }
        }
        frame.next_deadline = self.next_animation_deadline();
        frame
    }
}

/// Aggregate a finished frame profile into the process diagnostics (Issue
/// #227). Reuses the profiler's measurements; nothing is timed twice.
fn record_frame_diagnostics(profile: &FrameProfile) {
    use nana_diagnostics::MetricValue;
    use nana_diagnostics::framework::runtime as rt;
    if !nana_diagnostics::metrics_enabled() {
        return;
    }
    rt::FLUSHES.record(1);
    rt::FRAME_CPU_NS.record(profile.cpu_total.to_metric());
    for timing in &profile.stages {
        if timing.status != crate::StageStatus::Ran {
            continue;
        }
        let metric = match timing.stage {
            FrameStage::Input => &rt::STAGE_INPUT_NS,
            FrameStage::Reconcile => &rt::STAGE_RECONCILE_NS,
            FrameStage::Style => &rt::STAGE_STYLE_NS,
            FrameStage::TextShape => &rt::STAGE_TEXT_SHAPE_NS,
            FrameStage::Layout => &rt::STAGE_LAYOUT_NS,
            FrameStage::HitTest => &rt::STAGE_HIT_TEST_NS,
            FrameStage::Accessibility => &rt::STAGE_ACCESSIBILITY_NS,
            FrameStage::Animation => &rt::STAGE_ANIMATION_NS,
            FrameStage::Extract => &rt::STAGE_EXTRACT_NS,
            // GPU-host stages are recorded by the host that times them.
            FrameStage::Batch | FrameStage::GpuUpload | FrameStage::Encode | FrameStage::Submit => {
                continue;
            }
        };
        metric.record(timing.duration.to_metric());
    }
}

#[cfg(test)]
mod output_observation_tests {
    use super::*;

    #[test]
    fn presentation_gpu_work_replaces_and_clears_previous_frame_counters() {
        let mut context = AppContext::new();
        assert_eq!(context.last_work_counters().draw_calls, None);
        context.record_presentation_gpu_work(Some(nana_ui_core::GpuWorkObservation {
            draw_calls: 3,
            draw_batches: 2,
            batch_rebuilds: 1,
            gpu_upload_bytes: 64,
            gpu_buffer_reallocations: 1,
        }));
        let work = context.last_work_counters();
        assert_eq!(work.draw_calls, Some(3));
        assert_eq!(work.draw_batches, Some(2));
        assert_eq!(work.batch_rebuilds, Some(1));
        assert_eq!(work.gpu_upload_bytes, Some(64));
        assert_eq!(work.gpu_buffer_reallocations, Some(1));
        context.record_presentation_gpu_work(Some(Default::default()));
        let work = context.last_work_counters();
        assert_eq!(work.draw_calls, Some(0));
        assert_eq!(work.gpu_upload_bytes, Some(0));
        context.record_presentation_gpu_work(None);
        let work = context.last_work_counters();
        assert_eq!(work.draw_calls, None);
        assert_eq!(work.draw_batches, None);
        assert_eq!(work.batch_rebuilds, None);
        assert_eq!(work.gpu_upload_bytes, None);
        assert_eq!(work.gpu_buffer_reallocations, None);
    }

    #[test]
    fn idle_host_output_observation_replaces_the_previous_rendered_frame() {
        let mut context = AppContext::new();
        assert_eq!(context.last_work_counters().output_ui_renders, None);
        context.record_output_work(nana_ui_core::OutputWorkObservation {
            ui_renders: 2,
            submissions: 1,
            gpu_copies: 1,
            ..Default::default()
        });
        assert_eq!(context.last_work_counters().output_ui_renders, Some(2));
        context.record_output_work(nana_ui_core::OutputWorkObservation {
            idle_reuse_frames: 1,
            ..Default::default()
        });
        let work = context.last_work_counters();
        assert_eq!(work.output_ui_renders, Some(0));
        assert_eq!(work.output_submissions, Some(0));
        assert_eq!(work.output_gpu_copies, Some(0));
        assert_eq!(work.output_idle_reuse_frames, Some(1));
    }
}
