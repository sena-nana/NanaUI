//! AppContext frame operations.

use super::*;

impl AppContext {
    /// Drain deterministic work scheduled since the previous frame.
    ///
    /// This is the canonical application/frame drain. It polls local tasks
    /// and flushes the reactive host before delegating to
    /// [`crate::UiWorld::take_system_work`].
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

    pub fn record_extract_reusing(
        &mut self,
        extracted: &[crate::ExtractedNode],
        moved: &[crate::ExtractedMove],
    ) {
        self.world.record_extract_reusing(extracted, moved);
    }

    /// Open a product-frame profiler and work-counter accumulator.
    pub fn begin_frame_profile(&mut self) {
        self.layout.frame_layout_spent = (0, None);
        self.layout.frame_profiler = FrameProfiler::new();
        self.layout.frame_profiler.mark_runtime_unsupported();
        self.layout.profiling = true;
        self.world.begin_frame();
    }

    pub fn finish_frame_profile(&mut self) {
        self.world.end_frame();
        self.layout.profiling = false;
        let profile = std::mem::take(&mut self.layout.frame_profiler).finish();
        // Match last_work_counters: an idle flush (no stage ran) must not wipe
        // the last non-empty product profile.
        if profile.any_stage_ran() {
            record_frame_diagnostics(&profile);
            self.layout.last_profile = profile;
        }
    }

    pub fn last_frame_profile(&self) -> &FrameProfile {
        &self.layout.last_profile
    }

    pub(super) fn stage_clock(&self) -> Option<Instant> {
        self.layout.profiling.then(Instant::now)
    }

    pub(super) fn record_stage(&mut self, stage: FrameStage, started: Option<Instant>) {
        if let Some(started) = started {
            self.layout.frame_profiler.record(stage, started.elapsed());
        }
    }

    /// Record a stage duration while a product frame is open.
    pub fn time_stage_duration(&mut self, stage: FrameStage, duration: Duration) {
        if self.layout.profiling {
            self.layout.frame_profiler.record(stage, duration);
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

    fn finish_text_shape(
        &mut self,
        started: Option<Instant>,
        result: Result<bool, FrameworkError>,
    ) -> Result<bool, FrameworkError> {
        self.record_stage(FrameStage::TextShape, started);
        let reshaped = result?;
        let reprojected =
            self.reproject_text_backend_views()? | self.refresh_shaped_text_area_layouts()?;
        self.announce_text_clamps()?;
        Ok(reprojected || reshaped)
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
        self.finish_text_shape(started, result)
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
        self.finish_text_shape(started, result)
    }

    /// [`Self::shape_text_for_layout`] restricted to [`Self::last_layout_scope`],
    /// without draining that scope.
    pub fn shape_text_for_last_layout_scope(
        &mut self,
        shaper: &mut impl crate::TextShaper,
    ) -> Result<bool, FrameworkError> {
        let started = self.stage_clock();
        let result = self
            .world
            .shape_text_for_layout_scoped(&self.layout.last_layout_scope, shaper)
            .map_err(FrameworkError::from);
        self.finish_text_shape(started, result)
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
        std::mem::take(&mut self.layout.last_layout_scope)
    }

    /// Nodes recomputed by the most recent layout pass, without draining
    /// them: a laid-out node outside it kept its cached layout.
    /// `RuntimeDocument::flush` preserves this set across post-layout shaping;
    /// it is emptied only if a caller explicitly calls
    /// [`Self::take_last_layout_scope`], or replaced by the next layout pass.
    pub fn last_layout_scope(&self) -> &[StableNodeId] {
        &self.layout.last_layout_scope
    }

    /// Frontier counters of the most recent layout pass for `document`,
    /// including the size of the dependency graph it built.
    pub fn layout_frontier_stats(&self, document: DocumentId) -> crate::LayoutFrontierStats {
        self.layout.layout_cache.frontier_stats(document)
    }

    /// Let a frame's layout place up to `budget` and leave what is out of
    /// view to later frames, or (`None`, the default) place everything.
    ///
    /// Measurement is never sliced: every box keeps the size its content
    /// gives it. The boundaries in view are always placed, then the others
    /// nearest the viewport first until the budget is spent, at least one a
    /// pass. Only a subtree with nothing placed under it yet is left (a
    /// first layout, a subtree just mounted): it has no boxes until a later
    /// frame places all of it. While [`Self::layout_pending`] holds, a host
    /// keeps running frames; each one's layout continues where the last
    /// stopped. The budget counts per frame (from
    /// [`Self::begin_frame_profile`]); outside one, per layout call.
    pub fn set_layout_slice_budget(&mut self, budget: Option<crate::LayoutSliceBudget>) {
        self.layout.layout_slice_budget = budget;
    }

    pub fn layout_slice_budget(&self) -> Option<crate::LayoutSliceBudget> {
        self.layout.layout_slice_budget
    }

    /// Whether a sliced layout of `document` left subtrees to place: the
    /// next layout pass places them, with or without new edits.
    pub fn layout_pending(&self, document: DocumentId) -> bool {
        self.layout.layout_cache.layout_pending(document)
    }

    /// What is left of the frame's slice budget for the next pass.
    fn layout_slice_limit(&mut self) -> Option<crate::layout_engine::SliceLimit> {
        let budget = self.layout.layout_slice_budget?;
        if !self.layout.profiling {
            self.layout.frame_layout_spent = (0, None);
        }
        let placed = self.layout.frame_layout_spent.0;
        let started = *self
            .layout
            .frame_layout_spent
            .1
            .get_or_insert_with(Instant::now);
        Some(crate::layout_engine::SliceLimit {
            placement_nodes: budget
                .placement_nodes
                .map(|nodes| nodes.saturating_sub(placed)),
            deadline: budget.time.map(|time| started + time),
        })
    }

    /// Work the most recent layout pass for `document` ran.
    #[cfg(test)]
    pub(crate) fn layout_execution_stats(
        &self,
        document: DocumentId,
    ) -> crate::layout_engine::LayoutExecutionStats {
        self.layout.layout_cache.execution_stats(document)
    }

    /// What the retained layout cache holds for `document`.
    #[cfg(test)]
    pub(crate) fn retained_layout_footprint(
        &self,
        document: DocumentId,
    ) -> crate::layout_engine::RetainedLayoutFootprint {
        self.layout.layout_cache.footprint(document)
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
        self.layout.layout_full_invocations
    }

    /// Layout passes, scoped and full.
    pub fn layout_invocations(&self) -> usize {
        self.layout.layout_invocations
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
            for (total, elapsed) in self.layout.layout_substage_totals.iter_mut().zip(substages) {
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
        std::mem::take(&mut self.layout.layout_substage_totals)
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
        self.layout.layout_invocations += 1;
        if force_full {
            self.layout.layout_full_invocations += 1;
        }
        let started = self.stage_clock();
        // Diagnostics time layout even outside a profiled frame.
        let diagnostics_clock =
            started.or_else(|| nana_diagnostics::metrics_enabled().then(Instant::now));
        let prev_media_env = self.world.media_env();
        self.world.set_document_viewport(document, viewport);
        if self.world.media_env() != prev_media_env {
            self.notify_media_env_signals();
        }
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
            let slice = self.layout_slice_limit();
            self.layout.layout_cache.set_slice_limit(slice);
            let layouts = crate::RuntimeLayoutEngine.layout_document_with_frontier_local(
                &self.world,
                document,
                viewport,
                typed_seeds,
                &mut self.layout.layout_cache,
                force_full,
            )?;
            let intrinsic_counters = self.layout.layout_cache.take_intrinsic_counters();
            self.world
                .record_intrinsic_measure_counters(intrinsic_counters);
            let frontier = self.layout.layout_cache.frontier_stats(document);
            let execution = self.layout.layout_cache.execution_stats(document);
            self.layout.frame_layout_spent.0 = self
                .layout
                .frame_layout_spent
                .0
                .saturating_add(execution.placement_nodes);
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
            let mut placements = Vec::new();
            let mut scope = Vec::with_capacity(layouts.len());
            let mut context_children = Vec::new();
            for (id, layout) in layouts {
                scope.push(id);
                let padding_changed = self
                    .layout
                    .layout_cache
                    .used_padding(document, id)
                    .is_some_and(|padding| self.world.write_layout_padding(id, padding));
                // A turned scroll origin re-measures even when no box moved.
                let far_start_changed = self
                    .layout
                    .layout_cache
                    .far_start(document, id)
                    .is_some_and(|far| self.world.write_layout_far_start(id, far));
                // A different context places the children differently but can
                // leave every box where it was; the publish below rebuilds the
                // fragments under the new kind either way. A node that lost
                // its box keeps no context, as in a cold layout (#392).
                if self.world.write_formatting_context(
                    id,
                    self.layout.layout_cache.formatting_context(document, id),
                ) {
                    context_children.extend_from_slice(self.world.child_ids(id));
                }
                let frame = self.layout.layout_cache.frame(document, id);
                if padding_changed
                    || far_start_changed
                    || self.world.local_layout_box(id) != Some(layout)
                    || self.world.layout_frame(id) != Some(frame)
                {
                    placements.push(crate::mutation::LayoutPlacement { id, layout, frame });
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
            let mut post_layout_nodes = Vec::new();
            for &id in &scope {
                if let Some(post_layout) = self.behavior(id).and_then(|b| b.post_layout) {
                    post_layout_nodes.push((id, post_layout));
                }
            }
            #[cfg(feature = "benchmark")]
            phase.lap(11);
            // Every written box is in `scope`, which is published just below:
            // the commit leaves their results to that publish.
            self.world.set_pass_publishes_written_boxes(true);
            let report = self.commit_layout_placements(&placements);
            drop(placements);
            self.world.set_pass_publishes_written_boxes(false);
            let report = report?;
            #[cfg(feature = "benchmark")]
            phase.lap(12);
            // Publish one coherent canonical Result snapshot only after every
            // sibling box in the scope has been committed.
            self.world
                .publish_layout_results(&scope, crate::LayoutResultSource::RuntimeLayout);
            let inline_objects = self.layout.layout_cache.take_inline_objects(document);
            self.world.sync_inline_metrics(
                &scope,
                |id, first_only| {
                    self.layout
                        .layout_cache
                        .intrinsic_baseline(document, id, first_only)
                },
                inline_objects,
                viewport,
            );
            #[cfg(feature = "benchmark")]
            phase.lap(13);
            self.layout.last_layout_scope = scope;
            for (id, post_layout) in post_layout_nodes {
                post_layout(self, id)?;
            }
            completed(2);
            self.apply_document_scroll_retention(document, force_full)?;
            // After scroll retention: a clamped offset moves the room too.
            self.refresh_select_menu_rooms(document)?;
            self.announce_size_changes(document)?;
            self.play_pending_flips(document);
            self.play_pending_shared_elements(document);
            self.play_layout_moves(document);
            // A toast stack's new toasts were measured by this pass: what
            // shows follows from their heights.
            for i in 0..self.capabilities.post_layout_docs.len() {
                let hook = self.capabilities.post_layout_docs[i];
                hook(self, document)?;
            }
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
            let dirty_roots = if typed_seeds.is_empty() {
                dirty.len()
            } else {
                typed_seeds.len()
            };
            metric!(layout::DIRTY_ROOTS, dirty_roots);
            if result.is_ok() {
                let execution = self.layout.layout_cache.execution_stats(document);
                metric!(layout::BOXES_LAID_OUT, self.layout.last_layout_scope.len());
                metric!(layout::MEASURE_NODES, execution.measure_nodes);
                metric!(layout::PLACEMENT_NODES, execution.placement_nodes);
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
            .lifecycle
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
            self.lifecycle.laid_out_notices.remove(&id);
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
        // A first layout's scope is the whole document: sets hashed by id.
        let mut visited = crate::NodeSet::default();
        let mut targets = crate::NodeSet::default();
        for &id in &self.layout.last_layout_scope {
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
        let mut branches = crate::NodeSet::default();
        let mut branch_parents = crate::NodeSet::default();
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

    /// Drain whether style resolution touched a hit-test-relevant property (#106).
    pub fn take_style_hit_changed(&mut self) -> bool {
        self.world.take_style_hit_changed()
    }

    /// Theme/style work counters from the last style pass or theme install (#106).
    pub fn last_theme_work_counters(&self) -> crate::ThemeWorkCounters {
        self.world.last_theme_work_counters()
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

    /// The next host time a wake is due: the document's animation deadline
    /// mapped through its presentation clock, and the framework's own
    /// wall-clock timers (tooltips, wheel and touch scrolling, caret blink).
    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.world
            .next_presentation_wakeup()
            .into_iter()
            .chain(
                self.component_lifecycle
                    .tooltips
                    .iter()
                    .filter(|(target, _)| {
                        self.world.is_mounted(**target) && self.world.is_overlay_reachable(**target)
                    })
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
            .chain(self.menu_aim_deadline())
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
        if let Some(id) = self.advance_plain_drag(now) {
            frame.component_updates.push(id);
        }
        self.settle_animation_frame(now, frame)
    }

    /// The document's presentation playback (Issue #300).
    pub fn presentation_playback(&self) -> crate::PresentationPlayback {
        self.world.presentation_playback()
    }

    /// Play the document's presentation time at a rate, or hold it. Only
    /// this document is affected. Input, wheel and touch scrolling, caret
    /// blink, tooltips and other timers keep the host's clock.
    pub fn set_presentation_playback(&mut self, playback: crate::PresentationPlayback) {
        self.world.set_presentation_playback(playback);
    }

    /// Set the document's presentation time: every running track is sampled
    /// at `to` and the compositor reads `to`, in one step (see
    /// [`crate::UiWorld::seek_presentation`]). The framework's own consumers
    /// of the samples (surfaces, toasts, sidebar sections, spinners) react as
    /// on a frame; ends the seek reached are delivered by the next advance.
    /// The returned frame carries the samples and component updates.
    pub fn seek_presentation(&mut self, to: Duration) -> AnimationFrame {
        let frame = self.world.seek_presentation(to);
        let now = self.component_lifecycle.now;
        self.settle_animation_frame(now, frame)
    }

    /// Advance the presentation time by `interval` and sample there; that
    /// interval is the frame interval until the document plays again.
    pub fn step_presentation(&mut self, interval: Duration) -> AnimationFrame {
        let frame = self.world.step_presentation(interval);
        let now = self.component_lifecycle.now;
        self.settle_animation_frame(now, frame)
    }

    /// Set the presentation time to `to` as one frame of `interval`: the
    /// sampling of [`Self::seek_presentation`], with `interval` the frame
    /// interval until the document plays again (see
    /// [`crate::UiWorld::seek_presentation_frame`]).
    pub fn seek_presentation_frame(&mut self, to: Duration, interval: Duration) -> AnimationFrame {
        let frame = self.world.seek_presentation_frame(to, interval);
        let now = self.component_lifecycle.now;
        self.settle_animation_frame(now, frame)
    }

    /// The interval the current frame stands for (display interval while
    /// playing, or the last step).
    pub fn presentation_frame_interval(&self) -> Duration {
        self.world.presentation_frame_interval()
    }

    /// Host hook: the display interval, the frame interval while playing.
    pub fn set_presentation_display_interval(&mut self, interval: Duration) {
        self.world.set_presentation_display_interval(interval);
    }

    /// Synchronize the world presentation clock to `now`.
    pub fn sync_presentation_clock(&mut self, now: Duration) {
        self.world.sync_presentation_clock(now);
    }

    /// Notify the runtime of the active text shaper before scene layout.
    pub fn observe_text_shaper(&mut self, host: &(impl crate::TextShaper + ?Sized)) {
        self.world.observe_text_shaper(host);
    }

    /// Reconcile focus and IME targets after nodes changed.
    pub fn reconcile_focus(&mut self, ids: &[StableNodeId]) {
        self.world.reconcile_focus(ids);
    }

    /// Update the active document-wide text selection projection.
    pub fn set_document_text_selection(
        &mut self,
        document: DocumentId,
        selection: Option<crate::DocumentTextSelection>,
    ) {
        self.world.set_document_text_selection(document, selection);
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "benchmark"))]
    pub fn begin_frame_counters(&mut self) {
        self.world.begin_frame_counters();
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "benchmark"))]
    pub fn end_frame_counters(&mut self) {
        self.world.end_frame_counters();
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "benchmark"))]
    pub fn scheduled_ui_frames(&mut self, ticks: usize) -> usize {
        self.world.scheduled_ui_frames(ticks)
    }

    /// React to a frame's samples and events on the framework's side. `now`
    /// is host time, for the wall-clock timers this also runs.
    fn settle_animation_frame(
        &mut self,
        now: Duration,
        mut frame: AnimationFrame,
    ) -> AnimationFrame {
        // Runs the samples start or finish are stamped on the document's
        // presentation time, as the samples were.
        let presented = self.world.animation_now();
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
        for i in 0..self.capabilities.frame_tickers.len() {
            let tick = self.capabilities.frame_tickers[i];
            for updated in tick(self, now) {
                if !frame.component_updates.contains(&updated) {
                    frame.component_updates.push(updated);
                }
            }
        }
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
            let changed = if let Some(overlay) = self.behavior(target).and_then(|b| b.overlay) {
                if open {
                    overlay
                        .open_hover_card
                        .is_some_and(|f| f(self, target).unwrap_or(false))
                } else {
                    overlay
                        .close_hover_card
                        .is_some_and(|f| f(self, target).unwrap_or(false))
                }
            } else {
                false
            };
            if changed {
                frame.component_updates.push(target);
            }
        }
        for sample in &frame.samples {
            let target = sample.target;
            let sample_time =
                if crate::component_animation_id(crate::component_animation_kinds::TOAST, target)
                    == Some(sample.id)
                {
                    now
                } else {
                    presented
                };
            let is_loading =
                crate::component_animation_id(crate::component_animation_kinds::LOADING, target)
                    == Some(sample.id);
            if is_loading
                && self.component_lifecycle.loading.get(&target).copied()
                    == Some(LoadingComponent::Button)
            {
                let changed = self
                    .update_component(Entity::<Button>::from_stable_id(target), |button, _| {
                        button.loading_phase = sample.progress;
                    })
                    .is_ok();
                if changed && !frame.component_updates.contains(&target) {
                    frame.component_updates.push(target);
                }
                continue;
            }
            let wants_sample = is_loading
                || crate::component_animation_id(crate::component_animation_kinds::SIDEBAR, target)
                    == Some(sample.id)
                || crate::component_animation_id(crate::component_animation_kinds::SPINNER, target)
                    == Some(sample.id)
                || crate::component_animation_id(
                    crate::component_animation_kinds::WORKSPACE,
                    target,
                ) == Some(sample.id)
                || (sample.finished
                    && (crate::component_animation_id(
                        crate::component_animation_kinds::TOAST,
                        target,
                    ) == Some(sample.id)
                        || crate::component_animation_id(
                            crate::component_animation_kinds::ATTENTION,
                            target,
                        ) == Some(sample.id)));
            if wants_sample
                && let Some(apply) = self.behavior(target).and_then(|b| b.animation_sample)
                && apply(self, target, sample, sample_time)
                && !frame.component_updates.contains(&target)
            {
                frame.component_updates.push(target);
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
