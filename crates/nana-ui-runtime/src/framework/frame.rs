//! AppContext frame operations.

use super::*;

impl AppContext {
    /// Drain deterministic work scheduled since the previous frame.
    pub fn take_system_work(&mut self) -> crate::SystemWork {
        self.world.take_system_work()
    }

    /// Algorithm-level counters from the last drained system batch.
    pub fn last_work_counters(&self) -> crate::WorkCounters {
        self.world.last_work_counters()
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
        let result = self.world.resolve_styles(ids).map_err(FrameworkError::from);
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
        result
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
        result
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
        result
    }

    /// Compute and atomically publish canonical Runtime layout for one window.
    ///
    /// Full pass: recomputes every box and rebuilds the retained layout cache.
    /// Used when viewport semantics changed or a complete layout is required.
    pub fn layout_document(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        self.layout_document_impl(document, viewport, &[], true)
    }

    /// [`Self::layout_document`] restricted to the ancestor closure of
    /// `dirty`. Clean subtrees reuse the retained cache, so the cost scales
    /// with the change, not the document. [`Self::take_last_layout_scope`]
    /// reports the recomputed set for scoped text re-shaping.
    pub fn layout_document_scoped(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
        dirty: &[StableNodeId],
    ) -> Result<crate::CommitReport, FrameworkError> {
        let mut dirty = dirty.to_vec();
        dirty.sort_unstable();
        dirty.dedup();
        self.layout_document_impl(document, viewport, &dirty, false)
    }

    /// Relayout after a viewport change. Document roots plus any live
    /// `position: fixed` / `vw` / `vh` boxes are dirty; unchanged subtrees keep
    /// the retained cache.
    pub fn layout_document_for_viewport(
        &mut self,
        document: DocumentId,
        viewport: crate::LayoutViewport,
    ) -> Result<crate::CommitReport, FrameworkError> {
        let mut dirty = self.world.document_roots(document);
        dirty.extend(self.world.viewport_basis_ids());
        self.layout_document_impl(document, viewport, &dirty, false)
    }

    /// Nodes recomputed by the most recent layout pass; drains on read.
    pub fn take_last_layout_scope(&mut self) -> Vec<StableNodeId> {
        std::mem::take(&mut self.last_layout_scope)
    }

    /// Nodes carrying an undrained LAYOUT-dirty bit (e.g. set by a shaping
    /// pass after the work drain). Sorted for determinism.
    pub fn pending_layout_dirty(&mut self) -> Vec<StableNodeId> {
        self.world.pending_layout_dirty()
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
        self.layout_invocations += 1;
        if force_full {
            self.layout_full_invocations += 1;
        }
        let started = self.stage_clock();
        self.component_lifecycle
            .viewports
            .insert(document, viewport);
        let result = (|| {
            self.position_open_tooltips(document)?;
            let layouts = crate::RuntimeLayoutEngine.layout_document_scoped(
                &self.world,
                document,
                viewport,
                dirty,
                &mut self.layout_cache,
                force_full,
            )?;
            let mut mutations = MutationQueue::new();
            let mut scope = Vec::with_capacity(layouts.len());
            for (id, layout) in layouts {
                scope.push(id);
                if self.world.layout_box(id) != Some(layout) {
                    mutations.write_layout(id, layout);
                }
            }
            self.last_layout_scope = scope;
            let report = self.commit_mutations(mutations)?;
            self.publish_document_scroll_metrics(document)?;
            Ok(report)
        })();
        self.record_stage(FrameStage::Layout, started);
        result
    }

    pub(super) fn publish_document_scroll_metrics(
        &mut self,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let updates = self
            .world
            .document_order(document)
            .into_iter()
            .filter(|id| self.is_scroll_view(*id))
            .filter_map(|id| {
                let metrics = self.scroll_metrics_from_layout(id)?;
                (self.world.scroll_metrics(id) != Some(metrics))
                    .then_some((Entity::<ScrollView>::from_stable_id(id), metrics))
            })
            .collect::<Vec<_>>();
        for (entity, metrics) in updates {
            self.set_scroll_metrics(entity, metrics)?;
        }
        Ok(())
    }

    pub(super) fn scroll_metrics_from_layout(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        let viewport = self.world.layout_box(id)?;
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return None;
        }
        let mut content_width = viewport.width;
        let mut content_height = viewport.height;
        let mut stack = self
            .world
            .node(id)
            .map(|node| node.children)
            .unwrap_or_default();
        while let Some(child) = stack.pop() {
            if self
                .world
                .node_style(child)
                .is_some_and(|style| style.layout.omits_box())
            {
                continue;
            }
            if let Some(bounds) = self.world.layout_box(child) {
                content_width = content_width.max(bounds.x + bounds.width - viewport.x);
                content_height = content_height.max(bounds.y + bounds.height - viewport.y);
            }
            if let Some(node) = self.world.node(child) {
                stack.extend(node.children);
            }
        }
        Some(ScrollMetrics {
            viewport_width: viewport.width,
            viewport_height: viewport.height,
            content_width: content_width.max(0.0),
            content_height: content_height.max(0.0),
        })
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
        let loading_deadline = self
            .component_lifecycle
            .loading
            .keys()
            .any(|target| self.world.is_mounted(*target))
            .then_some(self.component_lifecycle.next_loading_frame)
            .flatten();
        let workspace_deadline = self
            .component_lifecycle
            .workspace_transitions
            .keys()
            .any(|target| self.world.is_mounted(*target))
            .then_some(self.component_lifecycle.next_workspace_frame)
            .flatten();
        self.world
            .next_animation_deadline()
            .into_iter()
            .chain(loading_deadline)
            .chain(workspace_deadline)
            .chain(
                self.component_lifecycle
                    .tooltips
                    .iter()
                    .filter(|(target, _)| self.world.is_mounted(**target))
                    .filter_map(|(_, tooltip)| tooltip.show_at),
            )
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
        if self
            .component_lifecycle
            .next_loading_frame
            .is_some_and(|deadline| deadline <= now)
        {
            let phase = (now.as_secs_f32() / LOADING_CYCLE.as_secs_f32()).rem_euclid(1.0);
            let loading = self
                .component_lifecycle
                .loading
                .iter()
                .filter(|(target, _)| self.world.is_mounted(**target))
                .map(|(&target, &kind)| (target, kind))
                .collect::<Vec<_>>();
            for (target, kind) in loading {
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
                        .update_component(
                            Entity::<crate::Card>::from_stable_id(target),
                            |card, _| {
                                card.loading_phase = phase;
                            },
                        )
                        .is_ok(),
                };
                if changed {
                    frame.component_updates.push(target);
                }
            }
            self.component_lifecycle.next_loading_frame = self
                .component_lifecycle
                .loading
                .keys()
                .any(|target| self.world.is_mounted(*target))
                .then(|| now.checked_add(COMPONENT_FRAME_INTERVAL))
                .flatten();
        }
        let section_targets = frame
            .samples
            .iter()
            .map(|sample| sample.target)
            .filter(|target| {
                self.views
                    .get(target)
                    .is_some_and(|view| view.is::<SidebarSection>())
            })
            .collect::<Vec<_>>();
        for target in section_targets {
            if self
                .update_component(
                    Entity::<SidebarSection>::from_stable_id(target),
                    |section, _| {
                        section.animation_progress = section.state.expansion(now);
                    },
                )
                .is_ok()
            {
                frame.component_updates.push(target);
            }
        }
        let skeleton_targets = frame
            .samples
            .iter()
            .map(|sample| sample.target)
            .filter(|target| {
                self.views
                    .get(target)
                    .is_some_and(|view| view.is::<crate::Skeleton>())
            })
            .collect::<Vec<_>>();
        for target in skeleton_targets {
            let Some(phase) = frame
                .samples
                .iter()
                .find(|sample| sample.target == target)
                .map(|sample| sample.progress)
            else {
                continue;
            };
            if self
                .update_component(
                    Entity::<crate::Skeleton>::from_stable_id(target),
                    |skeleton, _| {
                        skeleton.pulse = phase;
                    },
                )
                .is_ok()
            {
                frame.component_updates.push(target);
            }
        }
        // 工作区折叠/展开过渡：结算前的每一帧采样一次模型并重投影，
        // 过渡全部结束后停止帧调度。
        if self
            .component_lifecycle
            .next_workspace_frame
            .is_some_and(|deadline| deadline <= now)
        {
            let targets = self
                .component_lifecycle
                .workspace_transitions
                .keys()
                .copied()
                .filter(|id| self.world.is_mounted(*id))
                .collect::<Vec<_>>();
            let mut any_transitioning = false;
            for id in targets {
                let mut still_transitioning = false;
                let advanced = self
                    .update_component(Entity::<Workspace>::from_stable_id(id), |workspace, _| {
                        let changed = workspace.apply(WorkspaceMutation::AdvanceAnimations, now);
                        still_transitioning = workspace.model.has_active_transitions();
                        changed
                    })
                    .unwrap_or(false);
                any_transitioning |= still_transitioning;
                if advanced {
                    frame.component_updates.push(id);
                }
            }
            self.component_lifecycle.next_workspace_frame = if any_transitioning {
                Some(now.checked_add(COMPONENT_FRAME_INTERVAL).unwrap_or(now))
            } else {
                None
            };
        }
        frame.next_deadline = self.next_animation_deadline();
        frame
    }
}
