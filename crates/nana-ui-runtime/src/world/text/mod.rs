//! Text shaping, editor presentation and display mappings.

mod editor_presentation;
mod plain;
mod shaper;

pub(crate) use editor_presentation::TextDisplayView;
pub(in crate::world) use editor_presentation::*;
pub(in crate::world) use plain::*;
pub(in crate::world) use shaper::*;

use std::{collections::HashSet, mem::size_of, sync::Arc};

use crate::text_node::TextBackendEpoch;
use crate::world::{
    DirtyMask, METRIC_EXPORTS, TextInputPresentation, text_intrinsic_changed, validate_text_metrics,
};
use crate::{
    DocumentId, NodeKind, StableNodeId, TextContent, TextMetrics, TextShaper, UiWorld, UiWorldError,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShapePassKind {
    Layout,
    Scheduled,
}

struct ShapedNodeOutcome {
    id: StableNodeId,
    metrics: TextMetrics,
    natural: Option<f32>,
    presentation: Option<TextInputPresentation>,
    resolved_before: bool,
}

impl UiWorld {
    /// Text-shaping pipeline shared by scheduled (`shape_text`) and post-layout
    /// (`shape_text_for_layout_impl`) passes.
    fn shape_node_batch<S: TextShaper>(
        &mut self,
        pass: ShapePassKind,
        ids: &[StableNodeId],
        host: &mut S,
    ) -> Result<bool, UiWorldError> {
        let shown = if pass == ShapePassKind::Scheduled {
            self.resolve_presentations(ids)?;
            let shown = self.take_shown_text(ids);
            if !ids.is_empty() {
                self.record_hot_path_allocation(
                    1,
                    ids.len().saturating_mul(size_of::<(
                        StableNodeId,
                        TextMetrics,
                        Option<TextInputPresentation>,
                    )>()),
                );
            }
            shown
        } else {
            Vec::new()
        };
        let backend = PlainTextBackend::of(host);
        self.paint_text_engine = backend.engine.clone();
        self.observe_text_backend(backend.epoch);
        let mut work = nana_text::TextWorkCounters::default();
        let mut shaper = CountingShaper::new(host);
        let mut shaped: Vec<ShapedNodeOutcome> = if pass == ShapePassKind::Scheduled {
            Vec::with_capacity(ids.len())
        } else {
            Vec::new()
        };
        let mut adaptive_updates = Vec::new();
        let mut empty_shaped = Vec::new();
        let mut modal_shaped = Vec::new();
        let mut resolved = Vec::new();
        let mut stopped = 0usize;
        let mut max_text_id: Option<StableNodeId> = None;
        let mut text_count = 0usize;
        for &id in ids.iter().chain(&shown) {
            if let Some(record) = self.nodes.get(id)
                && (record.has_text || matches!(record.kind.as_ref(), NodeKind::Text))
            {
                text_count += 1;
                max_text_id = Some(max_text_id.map_or(id, |current| current.max(id)));
            }
        }
        if let Some(highest) = max_text_id {
            self.nodes
                .reserve_text_nodes_through(highest, self.nodes.text_nodes_len() + text_count);
        }
        #[cfg(any(test, feature = "benchmark"))]
        crate::text_shape_stats::note_scope(ids.len());
        let outcome = (|| -> Result<(), UiWorldError> {
            for &id in ids.iter().chain(&shown) {
                if pass == ShapePassKind::Scheduled && !self.contains(id) {
                    return Err(UiWorldError::MissingNode(id));
                }
                if self.plain_text_is_current(id, backend.epoch, &mut work) {
                    continue;
                }
                if self
                    .widget_visual_ref(id)
                    .is_some_and(crate::WidgetVisualHandle::owns_intrinsic_text)
                {
                    self.nodes.release_text_layout(id);
                }
                match pass {
                    ShapePassKind::Layout => {
                        let computed = self.record(id).resolved.0.as_ref();
                        if let Some(empty_visual) =
                            self.widget_visual_as::<crate::EmptyStateVisual>(id)
                        {
                            if computed.visible {
                                let Some(layout) = self.component_layout_box(id) else {
                                    continue;
                                };
                                let width = self
                                    .component_content_box(id)
                                    .map_or(layout.width, |content| content.width);
                                let runs = shaper.runs;
                                let typography = self.node_theme(id).typography();
                                let intrinsic = shape_empty_state_text(
                                    id,
                                    empty_visual,
                                    computed,
                                    Some(width),
                                    &mut shaper,
                                    typography,
                                );
                                work.record_text_pass(1, usize::from(shaper.runs > runs));
                                validate_text_metrics(id, intrinsic.title)?;
                                if let Some(message) = intrinsic.message {
                                    validate_text_metrics(id, message)?;
                                }
                                empty_shaped.push((id, intrinsic));
                            }
                            continue;
                        }
                        if let Some(modal_visual) =
                            self.widget_visual_as::<crate::ModalFrameVisual>(id)
                        {
                            if computed.visible {
                                let Some(root) = self.component_layout_box(id) else {
                                    continue;
                                };
                                let parts = self.modal_text_parts(id, root);
                                let width = |kind| {
                                    crate::layout_engine::parts::part_bounds(&parts, kind)
                                        .map(|part| part.width)
                                };
                                let Some(wrap_width) = width(crate::LayoutPartKind::Header) else {
                                    continue;
                                };
                                let body_width =
                                    width(crate::LayoutPartKind::Note).unwrap_or(wrap_width);
                                let runs = shaper.runs;
                                let node_theme = self.node_theme(id);
                                let intrinsic = shape_modal_text(
                                    id,
                                    modal_visual,
                                    computed,
                                    node_theme.recipes().dialog(),
                                    Some(wrap_width),
                                    Some(body_width),
                                    &mut shaper,
                                    node_theme.typography(),
                                );
                                work.record_text_pass(1, usize::from(shaper.runs > runs));
                                validate_text_metrics(id, intrinsic.title)?;
                                if let Some(description) = intrinsic.description {
                                    validate_text_metrics(id, description)?;
                                }
                                if let Some(body) = intrinsic.body {
                                    validate_text_metrics(id, body)?;
                                }
                                if self.nodes.modal_text(id) != Some(&intrinsic) {
                                    modal_shaped.push((id, intrinsic));
                                }
                            }
                            continue;
                        }
                        if !computed.visible {
                            continue;
                        }
                    }
                    ShapePassKind::Scheduled => {}
                }
                let presentation = self
                    .widget_visual_ref(id)
                    .is_some_and(crate::WidgetVisualHandle::is_text_input)
                    .then(|| self.text_input_presentation_source(id))
                    .flatten();
                let empty = presentation.as_ref().map_or_else(
                    || self.nodes.text_content(id).value.is_empty(),
                    |source| source.text.value.is_empty(),
                );
                if pass == ShapePassKind::Layout && presentation.is_some() {
                    self.nodes.release_text_layout(id);
                }
                match pass {
                    ShapePassKind::Layout if empty && presentation.is_none() => {
                        if !matches!(self.record(id).kind.as_ref(), NodeKind::Text) {
                            resolved.push(PlainResolution::nothing(id));
                        }
                        continue;
                    }
                    ShapePassKind::Scheduled
                        if presentation.is_none()
                            && !self.record(id).has_text
                            && !matches!(self.record(id).kind.as_ref(), NodeKind::Text)
                            && !self
                                .widget_visual_ref(id)
                                .is_some_and(crate::WidgetVisualHandle::owns_intrinsic_text) =>
                    {
                        let resolved_before = self.text_resolved_before(id);
                        resolved.push(PlainResolution::nothing(id));
                        shaped.push(ShapedNodeOutcome {
                            id,
                            metrics: TextMetrics::default(),
                            natural: None,
                            presentation: None,
                            resolved_before,
                        });
                        continue;
                    }
                    _ => {}
                }
                let style = Arc::clone(&self.record(id).resolved.0);
                #[cfg(any(test, feature = "benchmark"))]
                if presentation.as_ref().map_or_else(
                    || self.record(id).has_text,
                    |source| !source.text.value.is_empty(),
                ) {
                    crate::text_shape_stats::note_nonempty();
                }
                let mut stampable = true;
                if pass == ShapePassKind::Scheduled {
                    if let Some(empty_visual) = self.widget_visual_as::<crate::EmptyStateVisual>(id)
                    {
                        stampable = false;
                        let runs = shaper.runs;
                        let typography = self.node_theme(id).typography();
                        let intrinsic = shape_empty_state_text(
                            id,
                            empty_visual,
                            &style,
                            None,
                            &mut shaper,
                            typography,
                        );
                        work.record_text_pass(1, usize::from(shaper.runs > runs));
                        validate_text_metrics(id, intrinsic.title)?;
                        if let Some(message) = intrinsic.message {
                            validate_text_metrics(id, message)?;
                        }
                        empty_shaped.push((id, intrinsic));
                    }
                    if let Some(modal_visual) = self.widget_visual_as::<crate::ModalFrameVisual>(id)
                    {
                        stampable = false;
                        let runs = shaper.runs;
                        let node_theme = self.node_theme(id);
                        let intrinsic = shape_modal_text(
                            id,
                            modal_visual,
                            &style,
                            node_theme.recipes().dialog(),
                            None,
                            None,
                            &mut shaper,
                            node_theme.typography(),
                        );
                        work.record_text_pass(1, usize::from(shaper.runs > runs));
                        validate_text_metrics(id, intrinsic.title)?;
                        if let Some(description) = intrinsic.description {
                            validate_text_metrics(id, description)?;
                        }
                        if let Some(body) = intrinsic.body {
                            validate_text_metrics(id, body)?;
                        }
                        modal_shaped.push((id, intrinsic));
                    }
                }
                let constraints =
                    self.text_shape_constraints_for(id, presentation.as_ref().map(|s| s.multiline));
                let Some(presentation) = presentation else {
                    let PlainText {
                        metrics,
                        natural,
                        layout,
                        provisional,
                        host_unbroken,
                    } = self.resolve_plain_text(
                        id,
                        constraints,
                        backend.engine.as_ref(),
                        &mut shaper,
                        &mut work,
                    );
                    validate_text_metrics(id, metrics)?;
                    let resolved_before = self.text_resolved_before(id);
                    if stampable {
                        resolved.push(PlainResolution::text(
                            id,
                            constraints,
                            layout,
                            provisional,
                            host_unbroken,
                        ));
                    }
                    if pass == ShapePassKind::Scheduled
                        || self.nodes.text_metrics(id) != metrics
                        || self.text_natural_width(id) != natural
                    {
                        shaped.push(ShapedNodeOutcome {
                            id,
                            metrics,
                            natural,
                            presentation: None,
                            resolved_before,
                        });
                    } else if resolved_before {
                        stopped += 1;
                    }
                    continue;
                };
                if pass == ShapePassKind::Scheduled {
                    self.nodes.release_text_layout(id);
                }
                let text = clone_shaped_text(self, id, Some(&presentation));
                work.text_source_clones += 1;
                let runs = shaper.runs;
                let metrics = shaper.shape(id, &text, &style, constraints);
                work.record_text_pass(1, usize::from(shaper.runs > runs));
                validate_text_metrics(id, metrics)?;
                let previous_overlays = self
                    .nodes
                    .text_input_presentation(id)
                    .map(|stored| stored.overlay_metrics.clone())
                    .unwrap_or_default();
                if presentation.editor.adaptive_footer {
                    adaptive_updates.push(id);
                }
                let presentation = shape_text_input_presentation_in_theme(
                    id,
                    presentation,
                    &style,
                    constraints,
                    &previous_overlays,
                    self.node_theme(id),
                    &mut shaper,
                );
                let resolved_before = self.text_resolved_before(id);
                if pass == ShapePassKind::Scheduled {
                    shaped.push(ShapedNodeOutcome {
                        id,
                        metrics,
                        natural: None,
                        presentation: Some(presentation),
                        resolved_before,
                    });
                } else {
                    let metrics_held = self.nodes.text_metrics(id) == metrics;
                    if metrics_held && resolved_before {
                        stopped += 1;
                    }
                    if !metrics_held
                        || self.nodes.text_input_presentation(id) != Some(&presentation)
                    {
                        shaped.push(ShapedNodeOutcome {
                            id,
                            metrics,
                            natural: None,
                            presentation: Some(presentation),
                            resolved_before,
                        });
                    }
                }
            }
            Ok(())
        })();
        let runs = shaper.runs;
        let wrap_layouts = shaper.wrap_layouts;
        if let Err(err) = outcome {
            let _ = host.take_text_work();
            if pass == ShapePassKind::Scheduled {
                self.text.text_shown.extend(shown);
            }
            return Err(err);
        }
        self.text
            .text_input_adaptive_updates
            .extend(adaptive_updates);
        let (mut scaled, mut unbreakable_moved, inline_placement_changed) =
            self.apply_plain_resolutions(resolved, backend.epoch, &mut work);
        let mut changed = false;
        let mut scale_parent_reflows = 0usize;
        if pass == ShapePassKind::Layout {
            unbreakable_moved.retain(|id| !shaped.iter().any(|item| item.id == *id));
            self.note_text_propagations_stopped(stopped.saturating_sub(unbreakable_moved.len()));
            changed = !shaped.is_empty()
                || !modal_shaped.is_empty()
                || !unbreakable_moved.is_empty()
                || inline_placement_changed;
            for id in &unbreakable_moved {
                self.note_layout_source_change();
                if self.propagate_layout_from_node(*id) && scaled.contains(id) {
                    scale_parent_reflows += 1;
                }
            }
        }
        let mut scheduled_stopped = 0usize;
        for item in shaped {
            let ShapedNodeOutcome {
                id,
                metrics,
                natural,
                presentation,
                resolved_before,
            } = item;
            let previous = self.nodes.text_metrics(id);
            let previous_natural = self.text_natural_width(id);
            let width_moved = pass == ShapePassKind::Scheduled && unbreakable_moved.contains(&id);
            if previous != metrics || previous_natural != natural || width_moved {
                self.note_layout_source_change();
            }
            self.nodes.set_text_metrics(id, metrics);
            self.nodes.set_text_natural_width(id, natural);
            if let Some(presentation) = presentation {
                if pass == ShapePassKind::Scheduled && self.nodes.text_viewport_pin(id).is_some() {
                    let moved = match self.nodes.text_input_presentation(id) {
                        Some(previous_presentation) => {
                            (previous_presentation.caret_x, previous_presentation.caret_y)
                                != (presentation.caret_x, presentation.caret_y)
                        }
                        None => true,
                    };
                    if moved {
                        self.nodes.set_text_viewport_pin(id, None);
                    }
                }
                self.nodes
                    .set_text_input_presentation(id, Some(presentation));
                if self.nodes.take_text_scale_pending(id) {
                    scaled.insert(id);
                }
            }
            if text_intrinsic_changed(previous, metrics)
                || previous_natural != natural
                || width_moved
            {
                if self.propagate_layout_from_node(id) && scaled.contains(&id) {
                    scale_parent_reflows += 1;
                }
            } else if pass == ShapePassKind::Scheduled && resolved_before {
                scheduled_stopped += 1;
            }
        }
        if pass == ShapePassKind::Scheduled {
            self.note_text_propagations_stopped(scheduled_stopped);
        }
        self.note_text_scale_relayouts(scaled.len(), scale_parent_reflows);
        for (id, presentation) in empty_shaped {
            let updated = self.apply_empty_state_text_presentation(id, presentation);
            if pass == ShapePassKind::Layout {
                changed |= updated;
            }
        }
        for (id, presentation) in modal_shaped {
            self.nodes.set_modal_text(id, Some(presentation));
            if pass == ShapePassKind::Layout {
                self.record_layout_invalidation(
                    id,
                    nana_ui_core::LayoutInvalidation::new(
                        nana_ui_core::LayoutInvalidationSource::Text,
                        nana_ui_core::InvalidationReason::TEXT,
                        nana_ui_core::InvalidationKind::MEASURE
                            .union(nana_ui_core::InvalidationKind::PLACEMENT),
                        nana_ui_core::LayoutStyleChange::INTRINSIC,
                        METRIC_EXPORTS,
                    ),
                );
                self.mark(id, DirtyMask::RENDER);
            }
        }
        if pass == ShapePassKind::Scheduled {
            self.resolve_block_struts(ids, host);
        }
        self.refresh_document_text_highlights(host);
        work.accumulate(host.take_text_work());
        if pass == ShapePassKind::Layout {
            let (relayouts, reshapes) = (work.constraint_only_relayouts, work.text_nodes_shaped);
            self.bump_last_counters(|counters| counters.record_resize_text(relayouts, reshapes));
        }
        self.finish_text_pass(work, runs, wrap_layouts);
        Ok(changed)
    }

    pub(in crate::world) fn shape_text_for_layout_impl(
        &mut self,
        ids: Vec<StableNodeId>,
        host: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        self.shape_node_batch(ShapePassKind::Layout, &ids, host)
    }

    /// Shape text for nodes whose layout or presentation needs resolution.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        host: &mut impl TextShaper,
    ) -> Result<(), UiWorldError> {
        self.shape_node_batch(ShapePassKind::Scheduled, ids, host)
            .map(|_| ())
    }
}

impl UiWorld {
    /// Counts what a typography scale change cost this pass's text: the text
    /// it reached that laid out again, and of that the text whose metrics
    /// moved its parent.
    fn note_text_scale_relayouts(&mut self, relayouts: usize, parent_reflows: usize) {
        if relayouts > 0 {
            self.bump_last_counters(|counters| {
                counters.record_typography_scale_text(0, relayouts, parent_reflows);
            });
        }
    }

    /// Folds one pass's counters into the frame.
    fn finish_text_pass(
        &mut self,
        work: nana_text::TextWorkCounters,
        runs: usize,
        wrap_layouts: usize,
    ) {
        self.bump_last_counters(|counters| {
            counters.record_text_shape(
                runs,
                work.layout_cache_hits.unwrap_or(0),
                work.layout_cache_misses.unwrap_or(0),
                wrap_layouts,
                work.constraint_only_relayouts,
            );
        });
        record_text_diagnostics(&work);
        self.record_text_work(work);
    }

    /// Notes the backend plain text resolves against. A different font set
    /// than the last pass saw makes every resolved text node stale; nodes this
    /// pass does not visit are scheduled so none keeps metrics from the old
    /// fonts.
    fn observe_text_backend(&mut self, epoch: TextBackendEpoch) {
        let previous = self.text.text_backend.replace(epoch);
        if previous != Some(epoch) {
            self.text.text_backend_changed = true;
            self.note_layout_source_change();
        }
        if previous.is_none_or(|previous| previous == epoch) {
            return;
        }
        // So does every block's strut: a face that serves its font now, or
        // stopped serving it, moves its lines.
        let struts = self.nodes.strut_holders().collect::<Vec<_>>();
        self.text.strut_candidates.extend(struts);
        // Every resolved text shapes again against the new fonts. Layout hears
        // only from the text whose exported metrics moved, as after any other
        // reshape: a font registration that changes no text's metrics lays
        // nothing out.
        let stale = self.nodes.text_bearing_nodes().collect::<Vec<_>>();
        for id in stale {
            self.nodes
                .invalidate_text(id, crate::text_node::TextDirty::FONT);
            self.mark(id, DirtyMask::TEXT | DirtyMask::RENDER);
        }
        // A painter that measured text re-records against the new backend;
        // one that did not is untouched.
        let mut painted = self
            .invalidation
            .paint_recordings
            .get_mut()
            .iter()
            .filter(|(_, held)| held.measured_text.is_some())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        painted.extend(
            self.invalidation
                .modal_surface_recordings
                .get_mut()
                .iter()
                .filter(|(_, held)| held.measured_text.is_some())
                .map(|(id, _)| *id),
        );
        for id in painted {
            self.mark_repaint(id);
        }
    }
}

impl UiWorld {
    /// Text-bearing nodes style resolution turned visible since the last
    /// scheduled pass and not already in `ids`, for this pass to re-resolve.
    fn take_shown_text(&mut self, ids: &[StableNodeId]) -> Vec<StableNodeId> {
        if self.text.text_shown.is_empty() {
            return Vec::new();
        }
        let scheduled: HashSet<StableNodeId> = ids.iter().copied().collect();
        let mut shown = std::mem::take(&mut self.text.text_shown);
        shown.sort_unstable();
        shown.dedup();
        shown.retain(|id| {
            // Editor, EmptyState and ModalFrame text is re-measured whenever a
            // layout scope reaches it and is never stamped; showing it again
            // is not a reason to measure it here as well.
            !scheduled.contains(id)
                && !self
                    .widget_visual_ref(*id)
                    .is_some_and(crate::WidgetVisualHandle::is_text_input)
                && !self
                    .widget_visual_ref(*id)
                    .is_some_and(crate::WidgetVisualHandle::owns_intrinsic_text)
                && self.nodes.get(*id).is_some_and(|record| {
                    matches!(record.kind.as_ref(), NodeKind::Text) || record.has_text
                })
        });
        shown
    }

    /// Schedules every text the host's current font set may measure
    /// differently than the one this world last resolved against. A frame
    /// driver calls this before draining work, so registering a font makes
    /// a static document settle on the new fonts instead of waiting for
    /// unrelated work to reach a text pass.
    pub fn observe_text_shaper(&mut self, host: &(impl TextShaper + ?Sized)) {
        let backend = PlainTextBackend::of(host);
        self.paint_text_engine = backend.engine.clone();
        self.observe_text_backend(backend.epoch);
        self.observe_engine_fallback_language(backend.fallback_language);
    }

    /// Text revisions of `id` (Issue #95): which classes of change its text
    /// has seen. Paint and compositor changes never move `content`, `shape`
    /// or `constraint`.
    pub fn text_revisions(&self, id: StableNodeId) -> Option<crate::TextRevisions> {
        self.nodes
            .text_node(id)
            .map(|text| text.revisions)
            .or_else(|| self.nodes.contains(id).then(crate::TextRevisions::default))
    }

    /// The `nana-text` layout plain text node `id` retains, with its handle.
    /// `None` when the node's text resolved through the host shaper, is empty,
    /// or is an editor's.
    pub fn text_layout(
        &self,
        id: StableNodeId,
    ) -> Option<(nana_text::TextLayoutId, &Arc<nana_text::TextLayout>)> {
        let handle = self.nodes.text_node(id)?.layout;
        self.nodes
            .text_layouts()
            .get(handle)
            .map(|layout| (handle, layout))
    }

    /// Layouts retained across every plain text node of this world.
    pub fn retained_text_layouts(&self) -> usize {
        self.nodes.text_layouts().len()
    }
}

impl UiWorld {
    /// [`Self::shape_text_for_layout`] restricted to `ids` (typically the
    /// relayout scope plus nodes whose published box changed). Nodes outside
    /// the scope keep their previous shape, which already matches their
    /// unchanged constraints.
    pub fn shape_text_for_layout_scoped(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        let mut scope = ids.to_vec();
        scope.sort_unstable();
        scope.dedup();
        scope.retain(|id| self.nodes.contains(*id));
        self.shape_text_for_layout_impl(scope, shaper)
    }
}

impl UiWorld {
    /// Re-shape visible text against its resolved content box after the first
    /// layout pass. This closes wrapping/ellipsis height measurement without
    /// moving layout ownership into the renderer adapter.
    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        self.shape_text_for_layout_impl(self.document_order(document), shaper)
    }
}

impl UiWorld {
    /// Whether a pass already resolved `id`'s text, so this pass changes text
    /// that had exported metrics rather than laying it out for the first time.
    fn text_resolved_before(&self, id: StableNodeId) -> bool {
        self.nodes
            .text_node(id)
            .is_some_and(crate::text_node::TextNodeState::resolved_before)
            || self.nodes.text_input_presentation(id).is_some()
    }

    /// Text that kept its exported metrics stops the change there. The
    /// frontier never sees it, so the stop is counted where it happens.
    fn note_text_propagations_stopped(&mut self, stopped: usize) {
        if stopped > 0 {
            self.bump_last_counters(|counters| {
                counters.layout_propagations_stopped =
                    counters.layout_propagations_stopped.saturating_add(stopped);
                counters.record_text_metrics(0, stopped, 0);
            });
        }
    }
}

/// Aggregate one text pass into the process diagnostics (Issue #227). Pure
/// counter adds; nothing when diagnostics are off.
fn record_text_diagnostics(work: &nana_text::TextWorkCounters) {
    use nana_diagnostics::framework::text;
    if !nana_diagnostics::metrics_enabled() {
        return;
    }
    let pairs = [
        (&text::SHAPE_HITS, work.shape_cache_hits),
        (&text::SHAPE_MISSES, work.shape_cache_misses),
        (&text::LAYOUT_HITS, work.layout_cache_hits),
        (&text::LAYOUT_MISSES, work.layout_cache_misses),
        (&text::GLYPHS_RESOLVED, work.glyphs_resolved),
    ];
    for (metric, value) in pairs {
        if let Some(value) = value.filter(|v| *v > 0) {
            metric.record(value as u64);
        }
    }
}
