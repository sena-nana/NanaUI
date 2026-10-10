//! Algorithm-level work counters and named frame stages for the Performance
//! Contract (Issue #8).
//!
//! Runtime fills these from dirty system work. GPU upload bytes are omitted
//! rather than estimated: this crate does not observe renderer uploads.

/// Writes `accumulate` for a counter struct from how each field folds:
/// `latest` takes the other snapshot's value, `max` keeps the larger,
/// `nested` folds a counter struct, `sum` adds, saturating, and `optional`
/// adds when either side recorded one. `other` is destructured whole, so a
/// field left off the lists does not compile.
macro_rules! accumulate_counters {
    (
        $(#[$doc:meta])*
        $ty:ident {
            $(latest: $($latest:ident),+;)?
            $(max: $($max:ident),+;)?
            $(nested: $($nested:ident),+;)?
            sum: $($sum:ident),+;
            $(optional: $($optional:ident),+;)?
        }
    ) => {
        impl $ty {
            $(#[$doc])*
            pub fn accumulate(&mut self, other: Self) {
                let Self {
                    $($($latest,)+)? $($($max,)+)? $($($nested,)+)? $($sum,)+ $($($optional,)+)?
                } = other;
                $($(self.$latest = $latest;)+)?
                $($(self.$max = self.$max.max($max);)+)?
                $($(self.$nested.accumulate($nested);)+)?
                $(self.$sum = self.$sum.saturating_add($sum);)+
                $($(fold_optional_count(&mut self.$optional, $optional);)+)?
            }
        }
    };
}

/// Per-frame algorithm counts. Timing stays on the Runtime profiler; these
/// fields are the stable CI signals.
///
/// Runtime dirty-bit mapping (Issue #8 §6.2). PAINT stays folded into RENDER.
/// STATE and TRANSFORM are independent Runtime mask bits:
///
/// - STATE → Runtime `SystemWork::state` (hover/press/focus/`SetInteraction`).
///   STYLE is added only when interaction paints need resolving.
/// - STYLE → `style_processed`
/// - TEXT → `text_shaped` (scheduled text nodes) plus `text_shaped_runs` /
///   `text_layout_cache_*` recorded on the shaping hot path
/// - LAYOUT → `layout_nodes`
/// - TRANSFORM → Runtime `SystemWork::transform` (`PaintTransform`). INPUT and
///   RENDER are added because hit-test and extract consume the matrix; LAYOUT
///   is not. STYLE is not set for transform-only `SetStyle`.
/// - INPUT → `hit_test_candidates`
/// - FOCUS_IME is tracked on Runtime `SystemWork::focus_ime`, not a dedicated
///   counter field
/// - RENDER → `render_nodes_extracted` / `render_nodes_changed`. **PAINT is folded
///   into RENDER**; paint-only mutations (hover color, opacity) schedule RENDER
///   without LAYOUT
/// - ACCESSIBILITY → `accessibility_nodes_updated`
///
/// STATE and TRANSFORM follow FOCUS_IME: they are scheduled on Runtime
/// `SystemWork`, not dedicated `WorkCounters` fields.
///
/// `input_targets` counts live pointer hover/press/capture plus focus.
///
/// `allocations` / `allocated_bytes` are **CPU hot-path** Vec/slot/string
/// clones Runtime can observe (drain lists, layout input children, text-shape
/// temps). They are not a process-wide malloc hook.
///
/// `text_layout_cache_*` come from Runtime `TextLayoutCache` lookup/insert.
/// `glyph_cache_*` are `None` until a shaping pass consults Runtime
/// `GlyphCache` (hosts without a glyph backend never do). `cache_eviction`
/// is `Some` after a shaping pass that consulted the layout cache (including
/// 0). GPU upload / draw-batch are `None` until a renderer that actually
/// encodes/submits records them.
///
/// `validation_nodes_scanned`, `hit_test_nodes_rebuilt`, and
/// `gpu_buffer_reallocations` are the incremental-work sentinels: each one is a
/// place where a full-world implementation can silently cancel the incremental
/// contract while every other counter still looks correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkCounters {
    pub entities_total: usize,
    pub entities_changed: usize,
    pub entities_spawned: usize,
    pub entities_despawned: usize,
    pub style_processed: usize,
    pub text_shaped: usize,
    pub layout_nodes: usize,
    /// Typed layout seeds handed to the dependency-aware frontier builder.
    pub layout_frontier_seeds: usize,
    /// Seeds merged because they reached the same node in one frame.
    pub layout_frontier_seed_merges: usize,
    /// Nodes that entered the measure frontier. A metric delta that has to be
    /// measured again is admitted here: Issue #255's
    /// `layout_metric_deltas_measure` is this field.
    pub layout_frontier_nodes_measure: usize,
    /// Nodes that entered the placement frontier. Issue #255's
    /// `layout_metric_deltas_placement` is this field.
    pub layout_frontier_nodes_placement: usize,
    /// Formatting contexts scheduled for a local solve.
    /// `layout_context_local_solves` is this field.
    pub layout_frontier_contexts: usize,
    /// Nodes whose writing context changed: a mode or direction delta reached
    /// them. Issue #255's `layout_metric_deltas_writing` is this field.
    pub layout_frontier_nodes_writing: usize,
    /// Dependency edges inspected while building/propagating the frontier.
    /// Each visit reads one compact footprint: Issue #255's
    /// `layout_dependency_footprints_read` is this field.
    pub layout_dependency_edges_visited: usize,
    /// Propagation stopped because the exported metric delta was stable.
    /// Issue #255's `layout_metric_deltas_none` is this field.
    pub layout_propagations_stopped: usize,
    /// Bounded local subtree fallbacks used for an unsupported context.
    pub layout_local_subtree_fallbacks: usize,
    /// Full-document fallbacks. Normal product paths should keep this zero.
    pub layout_full_document_fallbacks: usize,
    /// Nodes that computed a used size. Retained used-size and measure-plan hits do not count.
    pub layout_measure_nodes: usize,
    /// Retained used-size and measure-plan hits. Intrinsic-metric hits stay on `intrinsic_measure_cache_hits`.
    pub layout_measure_cache_hits: usize,
    /// Scoped lookups that missed retained used size or a measure plan, then computed a size.
    pub layout_measure_cache_misses: usize,
    /// Nodes whose placement pass wrote a box. Frontier membership is `layout_frontier_nodes_placement`.
    pub layout_placement_nodes: usize,
    /// Published results kept because geometry still matched.
    pub layout_result_reused: usize,
    /// Published results replaced because geometry changed.
    pub layout_result_changed: usize,
    /// Placement writes that moved the origin and kept the measured size.
    pub layout_origin_only_updates: usize,
    /// Publish batches that wrote at least one changed layout result.
    pub layout_delta_commits: usize,
    /// Children result publication looked at: the covered children a held
    /// result was checked against, and every child of a result it rebuilt.
    /// A local edit keeps this the same at any document size; only a
    /// container whose own placements changed pays for its child count.
    pub layout_result_children_visited: usize,
    /// Containers whose placement ran a different formatting context than
    /// the one recorded for them (Issue #197). A transition replaces the
    /// context record and the children's participation; it never creates,
    /// despawns or resets a child. Static and paint-only frames record 0.
    pub layout_context_transitions: usize,
    /// Layout mutations whose effective value changed and queued a typed
    /// invalidation.
    pub layout_invalidations_created: usize,
    /// Layout classifications that came out empty: the write changed no
    /// field layout reads, so nothing was queued.
    pub layout_invalidations_zero_delta: usize,
    /// Style writes equal in effect to the style already there. They queue
    /// nothing and reach no frontier.
    pub layout_equivalent_mutations_skipped: usize,
    /// Containers placed from their retained plan, without a walk of their
    /// children (Issue #259 `layout_plan_hits`, placement side).
    pub layout_placement_plans_reused: usize,
    /// Containers measured from a retained measure plan, without a walk of
    /// their children (`layout_plan_hits`, measure side).
    pub layout_measure_plans_reused: usize,
    /// Sequential containers that kept their prefix and replayed only the
    /// children from the first changed one on.
    pub layout_suffixes_replayed: usize,
    /// Children a container walk measured, in its measure and its placement.
    /// A plan hit walks none; this is the scan a plan miss pays.
    pub layout_children_measured: usize,
    /// Containers that placed their children but could not record a plan,
    /// so a later pass walks them again.
    pub layout_containers_uncacheable: usize,
    /// Containers that had a retained plan for the pass's inputs and walked
    /// their children anyway (`layout_plan_misses`). Plan queries are the
    /// two `*_plans_reused` hits plus these.
    pub layout_plan_misses: usize,
    /// Plans a walk recorded over an earlier plan for the same container
    /// and constraint (`layout_plan_rebuilds`).
    pub layout_plan_rebuilds: usize,
    /// Retained-cache sweeps that dropped despawned ids.
    pub layout_retain_sweeps: usize,
    /// Adjacency entries of the dependency graphs layout built this frame:
    /// the scratch a pass allocates for its closure and drops after it.
    pub layout_scratch_entries: usize,
    /// Bytes those entries take.
    pub layout_scratch_bytes: usize,
    pub hit_test_candidates: usize,
    /// Unique live pointer hover, press, capture, and focus nodes this drain.
    pub input_targets: usize,
    pub accessibility_nodes_updated: usize,
    /// Nodes whose render extraction was invalidated this drain.
    pub render_nodes_changed: usize,
    /// Nodes actually produced by extract. Zero on last-work snapshots until
    /// Runtime `record_extract` runs.
    pub render_nodes_extracted: usize,
    /// Theme-resolved text spans on extracted nodes. Zero until extract is
    /// recorded; not a GPU batch count.
    pub extracted_text_spans: usize,
    /// Observed CPU hot-path heap events this drain/frame (Issue #8 §5 / §7).
    pub allocations: usize,
    /// Payload bytes of those observed events. Not allocator slack, not VRAM.
    pub allocated_bytes: usize,
    /// `TextShaper::shape` invocations (Issue #8 §3.5 shaping calls/frame).
    pub text_shaped_runs: usize,
    /// `TextLayoutCache::lookup` hits. Not “metrics left unchanged”.
    pub text_layout_cache_hits: usize,
    /// `TextLayoutCache::insert` after a lookup miss.
    pub text_layout_cache_misses: usize,
    /// Shape calls that requested wrapping (`TextShapeConstraints.wrap`).
    pub text_wrap_layouts: usize,
    /// Typed layout seeds text queued once it shaped: its own cause, and the
    /// one it hands its parent (Issue #260).
    pub text_reflow_seeds: usize,
    /// Text whose exported metrics (width, height, baseline, natural width)
    /// moved when it shaped again.
    pub text_external_metric_changes: usize,
    /// Text shaped again whose exported metrics held: layout stopped at it.
    pub text_external_metric_unchanged: usize,
    /// Parents text asked to measure again because its metrics moved.
    pub text_parent_reflows: usize,
    /// Text laid out again for a new box from runs it had already shaped.
    pub text_constraint_relayouts: usize,
    /// Text a language change reached: its computed language moved.
    pub text_language_scope_invalidations: usize,
    /// Text a language change shaped again although its language held. Zero
    /// unless an invalidation is coarser than the language text depends on.
    pub text_literal_nodes_invalidated_by_language: usize,
    /// Text a typography scale change reached: its computed scale moved.
    pub typography_scale_dependents_notified: usize,
    /// Of those, the text a pass then laid out again at the new size.
    pub typography_scale_text_relayouts: usize,
    /// Of those, the text whose metrics moved its parent's layout.
    pub typography_scale_parent_reflows: usize,
    /// Nodes a scale change visited inside its scope. Nothing outside the
    /// scope is visited.
    pub typography_scale_scope_nodes_scanned: usize,
    /// Scale changes that set the scale a scope already had: nothing visited.
    pub typography_scale_equivalent_skips: usize,
    /// Layout seeds that moved a constraint children consume: a resized box,
    /// a viewport resize on a root.
    pub constraint_change_seeds: usize,
    /// Children those seeds asked whether they consume the constraint that
    /// moved.
    pub constraint_dependents_considered: usize,
    /// Of those, the ones that do, and measure again.
    pub constraint_dependents_remeasured: usize,
    /// Of those, the ones that do not -- fixed, or reading only the other
    /// axis -- and keep their measurement.
    pub constraint_dependents_skipped: usize,
    /// Text laid out again at the box a layout pass gave it, from the runs it
    /// had shaped.
    pub resize_text_relayouts: usize,
    /// Text a layout pass's new box made shape again. Zero when only a wrap
    /// width moved.
    pub resize_text_reshapes: usize,
    /// Formatting contexts a constraint change solved again from scratch:
    /// containers whose retained plan could not answer, or that keep none.
    pub resize_context_solves: usize,
    /// Layout seeds style writes queued: a box whose resolved layout moved.
    pub style_to_layout_seeds: usize,
    /// Layout seeds a theme install queued: boxes whose design intent
    /// resolves to a different layout against the new metrics.
    pub theme_to_layout_seeds: usize,
    /// Layout seeds a component state change queued: hover, press, focus,
    /// interaction and accessibility state.
    pub component_state_layout_seeds: usize,
    /// Boxes a metrics install resolved to a different layout.
    pub theme_metric_dependents_invalidated: usize,
    /// Layout seeds a theme install that moved no metric queued. Zero: a
    /// palette is paint.
    pub theme_palette_layout_invalidations: usize,
    /// Style writes that moved no resolved layout field and no presentation:
    /// equal in effect, however they were spelled.
    pub equivalent_style_layout_skips: usize,
    /// Replaced content that changed what it shows and not which resource:
    /// a texture generation, a video frame, a fit or a sampling. Paint.
    pub replaced_content_updates: usize,
    /// Resources that reported a different natural size.
    pub replaced_intrinsic_metadata_updates: usize,
    /// Layout seeds replaced content queued: a natural size its box reads
    /// moved, or it became or stopped being replaced.
    pub replaced_layout_seeds: usize,
    /// Layout seeds a content-only update queued. Zero: a frame is paint.
    pub replaced_content_only_layout_invalidations: usize,
    /// Nodes a natural size change reached through the resource index.
    pub resource_intrinsic_dependents_notified: usize,
    /// Virtual list rows whose extent moved in the list's row index: a
    /// measured row that laid out at a new height (Issue #262).
    pub virtual_row_metric_updates: usize,
    /// Row index entries written: O(log C) for a row over C chunks, and a
    /// chunk's rows once when it stops being one extent. Never the rows.
    pub virtual_prefix_index_updates: usize,
    /// Mounted rows a list took a new height from: a measured row that laid
    /// out at a new height, or a row given a new imposed height.
    pub virtual_rows_remeasured: usize,
    /// Mounted rows a list moved: their placement was patched.
    pub virtual_rows_repositioned: usize,
    /// Logical rows a list looked up, a key by its index or an index by its
    /// key. Bounded by its window, never by its collection.
    pub virtual_logical_rows_scanned: usize,
    /// Times a list's scroll extent, its total height, moved.
    pub virtual_scroll_extent_updates: usize,
    /// Rows a list created for its window.
    pub virtual_rows_materialized_from_layout: usize,
    /// Container axes whose content extent moved while responsive rules read
    /// them (Issue #265).
    pub container_query_size_changes: usize,
    /// Responsive rules evaluated: only those on a container axis that moved.
    pub container_query_rules_evaluated: usize,
    /// Rules that changed bucket.
    pub container_query_results_changed: usize,
    /// Rules that stayed in their bucket, or held it to settle.
    pub container_query_results_unchanged: usize,
    /// Layout seeds the changed rules' variants queued.
    pub container_query_downstream_invalidations: usize,
    /// Evaluations that changed a bucket: the rounds a frame converged in.
    pub container_query_convergence_rounds: usize,
    /// Changes held to settle a frame: a bucket a rule already left this
    /// frame, or a frame out of rounds.
    pub container_query_cycle_fallbacks: usize,
    /// Localization (Issues #268, #269, #270).
    pub i18n: I18nCounters,
    /// Dynamic Layout (Issues #207, #213, #214).
    pub dynamic: DynamicLayoutCounters,
    /// Shared intrinsic measurement authority counters (Issue #198).
    pub intrinsic_measure_requests: usize,
    pub intrinsic_measure_cache_hits: usize,
    pub intrinsic_measure_cache_misses: usize,
    pub intrinsic_measure_full_subtrees: usize,
    pub intrinsic_generation_bumps: usize,
    pub baseline_queries: usize,
    pub cross_context_measure_hits: usize,
    pub cross_context_measure_misses: usize,
    /// `GlyphCache::lookup` hits. `None` until a glyph backend consults the
    /// cache this pass — omitted, never a fake 0.
    pub glyph_cache_hits: Option<usize>,
    /// `GlyphCache::insert` after a lookup miss. `None` until consulted.
    pub glyph_cache_misses: Option<usize>,
    /// `TextLayoutCache` FIFO evictions this shaping pass. `None` until the
    /// cache is consulted. Glyph FIFO trim is not folded into this field.
    pub cache_eviction: Option<usize>,
    /// Coalesced GPU batches rebuilt this frame. `None` until a host encodes.
    pub batch_rebuilds: Option<usize>,
    /// GPU batches issued this frame. `None` until a host encodes.
    pub draw_batches: Option<usize>,
    /// `draw` / `draw_indexed` invocations this frame. `None` until a host encodes.
    pub draw_calls: Option<usize>,
    /// Observed `queue.write_buffer` bytes this frame. `None` until a host
    /// encodes/submits. Not an estimate; missing stays omitted, never a fake 0.
    pub gpu_upload_bytes: Option<usize>,
    /// GPU buffer and texture allocations observed this frame, counting the
    /// per-frame resource creation a cache was supposed to avoid. `None` until a
    /// host encodes.
    pub gpu_buffer_reallocations: Option<usize>,
    /// Retained nodes a mutation-batch validation pass visited. The cost of
    /// validating a batch must track the batch, not the retained world, so this
    /// is the sentinel for a validator that regressed to a full scan. `None`
    /// until a commit path reports it.
    pub validation_nodes_scanned: Option<usize>,
    /// Hit-test entries rebuilt this frame. Incremental invalidation must patch
    /// the changed subtrees, so this is the sentinel for a rebuild that regressed
    /// to the whole document. `None` until a frame driver reports it.
    pub hit_test_nodes_rebuilt: Option<usize>,
    /// Output work observed by a host that renders into an output surface.
    /// These remain `None` until an output host actually runs; a Runtime-only
    /// drain must not fabricate zeros for GPU/output stages.
    pub output_extra_passes: Option<usize>,
    pub output_gpu_copies: Option<usize>,
    pub output_target_recreates: Option<usize>,
    pub output_content_revisions: Option<usize>,
    pub output_idle_reuse_frames: Option<usize>,
    pub output_resolve_count: Option<usize>,
    pub output_gpu_copy_bytes: Option<usize>,
    pub output_gpu_convert_passes: Option<usize>,
}

/// What localization cost (Issues #268, #269, #270). Work counters add up;
/// the index sizes are the largest seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct I18nCounters {
    /// Localized nodes registered: the index size.
    pub localized_nodes: usize,
    /// Literal text a locale change resolved. Always zero: literal text
    /// reads no locale; a sentinel, as `validation_nodes_scanned` is.
    pub literal_nodes: usize,
    /// Scope-to-node message dependencies held: the index size.
    pub message_dependencies: usize,
    /// Localized nodes whose shaping language follows their locale.
    pub language_dependencies: usize,
    /// Scope roots whose direction a locale sets.
    pub direction_dependencies: usize,
    /// Patterns asked of the lookup cache, and how it answered.
    pub catalog_lookups: usize,
    pub catalog_cache_hits: usize,
    pub catalog_cache_misses: usize,
    /// Localized nodes a locale change reached through the scope index.
    pub scope_dependents_notified: usize,
    /// Locale changes applied, each as one transaction.
    pub switch_transactions: usize,
    /// Localized nodes resolved.
    pub nodes_resolved: usize,
    /// Resolutions that showed a different string, and ones that did not.
    pub resolved_content_changed: usize,
    pub resolved_content_unchanged: usize,
    /// Localized nodes whose shaping language moved.
    pub language_changed_nodes: usize,
    /// Scopes whose direction moved.
    pub direction_changed_scopes: usize,
    /// Layout seeds a switch queued itself: a scope root's direction. Text
    /// that measures differently seeds from the text pass.
    pub layout_seeds: usize,
    /// Localized nodes resolved in rows a virtual list mounted: as they mount,
    /// and in a switch.
    pub virtual_rows_resolved: usize,
    /// Transactions that landed whole.
    pub switch_commits: usize,
    /// Messages formatted, and the formatter cache's answers.
    pub format_requests: usize,
    pub format_cache_hits: usize,
    pub format_cache_misses: usize,
    /// Patterns parsed into a compiled message.
    pub message_patterns_compiled: usize,
    /// Argument sets that changed on a localized node.
    pub args_revisions: usize,
    /// Formats whose output differed from the last, and ones whose did not.
    pub formatted_output_changed: usize,
    pub formatted_output_unchanged: usize,
    /// Localized nodes a catalog change reached.
    pub catalog_messages_invalidated: usize,
    /// Formatters built.
    pub formatter_allocations: usize,
}

accumulate_counters! {
    /// Fold another snapshot in: work adds up, index sizes keep the largest.
    I18nCounters {
        max: localized_nodes, message_dependencies, language_dependencies, direction_dependencies;
        sum: literal_nodes, catalog_lookups, catalog_cache_hits, catalog_cache_misses,
            scope_dependents_notified, switch_transactions, nodes_resolved,
            resolved_content_changed, resolved_content_unchanged, language_changed_nodes,
            direction_changed_scopes, layout_seeds, virtual_rows_resolved, switch_commits,
            format_requests, format_cache_hits, format_cache_misses, message_patterns_compiled,
            args_revisions, formatted_output_changed, formatted_output_unchanged,
            catalog_messages_invalidated, formatter_allocations;
    }
}

/// What Dynamic Layout cost (Issues #207, #213, #214): the cost-aware
/// solver that compresses a line's declared elasticity when it overflows,
/// and the envelopes it reads. All work adds up; the beam's widest state
/// set and the scratch high-water mark are the largest seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DynamicLayoutCounters {
    /// Lines whose container solves overflow, looked at.
    pub contexts_considered: usize,
    /// Solves run: cold and incremental.
    pub solver_runs: usize,
    /// Lines that fit without adjusting: no envelope read.
    pub fast_fit: usize,
    /// Solves that touched one cost level of one class.
    pub fast_adjust: usize,
    /// Keep-versus-break comparisons a line decision made (Issue #211).
    pub break_comparisons: usize,
    /// Break or adjustment opportunities a line decision looked at.
    pub opportunities_considered: usize,
    /// Envelopes asked for, and how the cache answered.
    pub envelope_queries: usize,
    pub envelope_hits: usize,
    pub envelope_misses: usize,
    /// Envelopes built again over a held one whose inputs moved.
    pub envelope_rebuilds: usize,
    /// Segments a solve read.
    pub cost_segments_visited: usize,
    /// Discrete candidates created, and the ones a lower bound pruned.
    pub candidates_created: usize,
    pub candidates_pruned: usize,
    /// Coarse segments refined by asking their participant.
    pub deep_expansions: usize,
    /// The widest beam a discrete search held.
    pub beam_states: usize,
    /// Children that resolved an extent their parent assigned.
    pub child_resolves: usize,
    /// Resolutions that had to lay a child's content out again.
    pub child_reflows: usize,
    /// Discrete structural adaptations chosen (explicit opt-in only).
    pub structural_adaptations: usize,
    /// Solves that hit a budget and fell back deterministically.
    pub budget_fallbacks: usize,
    /// Scratch buffers that had to grow.
    pub allocations: usize,
    /// The largest scratch, in bytes.
    pub temp_bytes: usize,
    /// Solves answered from the pass memo or a held line solve.
    pub solver_reuses: usize,
    /// Held line solves whose participants still matched.
    pub previous_result_hits: usize,
    /// Solves that only re-shared the level a held solve stopped in.
    pub incremental_assignments: usize,
    /// Solves run from nothing.
    pub cold_solves: usize,
    /// Participants a class gate or a zero capacity let a solve skip.
    pub pruned_candidates: usize,
}

accumulate_counters! {
    /// Fold another snapshot in: work adds up, the beam and scratch
    /// high-water marks keep the largest.
    DynamicLayoutCounters {
        max: beam_states, temp_bytes;
        sum: contexts_considered, solver_runs, fast_fit, fast_adjust, break_comparisons,
            opportunities_considered, envelope_queries, envelope_hits, envelope_misses,
            envelope_rebuilds, cost_segments_visited, candidates_created, candidates_pruned,
            deep_expansions, child_resolves, child_reflows, structural_adaptations,
            budget_fallbacks, allocations, solver_reuses, previous_result_hits,
            incremental_assignments, cold_solves, pruned_candidates;
    }
}

/// GPU work a renderer observed while encoding or submitting a real frame.
///
/// Recording this (including zeros) means the host ran encode/submit/upload.
/// Do not construct it to invent a quiet CPU-only drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuWorkObservation {
    pub batch_rebuilds: usize,
    pub draw_batches: usize,
    pub draw_calls: usize,
    pub gpu_upload_bytes: usize,
    pub gpu_buffer_reallocations: usize,
}

impl GpuWorkObservation {
    pub fn record_upload(&mut self, bytes: usize) {
        self.gpu_upload_bytes = self.gpu_upload_bytes.saturating_add(bytes);
    }

    pub fn record_realloc(&mut self) {
        self.gpu_buffer_reallocations = self.gpu_buffer_reallocations.saturating_add(1);
    }

    pub fn record_batch_rebuild(&mut self) {
        self.batch_rebuilds = self.batch_rebuilds.saturating_add(1);
    }

    pub fn record_draw_batch(&mut self) {
        self.draw_batches = self.draw_batches.saturating_add(1);
    }

    pub fn record_draw_call(&mut self) {
        self.draw_calls = self.draw_calls.saturating_add(1);
    }
}

/// Work counters for the Window-independent presentation boundary (Issue
/// #242). An observation is emitted by an output producer, not by a consumer
/// merely sampling an already completed texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputWorkObservation {
    pub extra_passes: usize,
    pub gpu_copies: usize,
    pub target_recreates: usize,
    pub content_revisions: usize,
    pub idle_reuse_frames: usize,
    pub resolve_count: usize,
    pub gpu_copy_bytes: usize,
    pub gpu_convert_passes: usize,
}

impl OutputWorkObservation {
    pub fn record_target_recreate(&mut self) {
        self.target_recreates = self.target_recreates.saturating_add(1);
    }

    pub fn record_content_revision(&mut self) {
        self.content_revisions = self.content_revisions.saturating_add(1);
    }

    pub fn record_idle_reuse(&mut self) {
        self.idle_reuse_frames = self.idle_reuse_frames.saturating_add(1);
    }

    pub fn record_gpu_copy(&mut self, bytes: usize) {
        self.gpu_copies = self.gpu_copies.saturating_add(1);
        self.gpu_copy_bytes = self.gpu_copy_bytes.saturating_add(bytes);
    }
}

accumulate_counters! {
    /// Fold another drain into this frame snapshot. `entities_total` is the
    /// latest live count; extract fields are added only when the other snapshot
    /// recorded them.
    WorkCounters {
        latest: entities_total;
        nested: i18n, dynamic;
        sum: entities_changed, entities_spawned, entities_despawned, style_processed, text_shaped,
            layout_nodes, layout_frontier_seeds, layout_frontier_seed_merges,
            layout_frontier_nodes_measure, layout_frontier_nodes_placement,
            layout_frontier_contexts, layout_frontier_nodes_writing,
            layout_dependency_edges_visited, layout_propagations_stopped,
            layout_local_subtree_fallbacks, layout_full_document_fallbacks, layout_measure_nodes,
            layout_measure_cache_hits, layout_measure_cache_misses, layout_placement_nodes,
            layout_result_reused, layout_result_changed, layout_origin_only_updates,
            layout_delta_commits, layout_result_children_visited, layout_context_transitions,
            layout_invalidations_created,
            layout_invalidations_zero_delta, layout_equivalent_mutations_skipped,
            layout_placement_plans_reused, layout_measure_plans_reused, layout_suffixes_replayed,
            layout_children_measured, layout_containers_uncacheable, layout_plan_misses,
            layout_plan_rebuilds, layout_retain_sweeps, layout_scratch_entries,
            layout_scratch_bytes, hit_test_candidates, input_targets, accessibility_nodes_updated,
            render_nodes_changed, render_nodes_extracted, extracted_text_spans, allocations,
            allocated_bytes, text_shaped_runs, text_layout_cache_hits, text_layout_cache_misses,
            text_wrap_layouts, text_reflow_seeds, text_external_metric_changes,
            text_external_metric_unchanged, text_parent_reflows, text_constraint_relayouts,
            text_language_scope_invalidations, text_literal_nodes_invalidated_by_language,
            typography_scale_dependents_notified, typography_scale_text_relayouts,
            typography_scale_parent_reflows, typography_scale_scope_nodes_scanned,
            typography_scale_equivalent_skips, constraint_change_seeds,
            constraint_dependents_considered, constraint_dependents_remeasured,
            constraint_dependents_skipped, resize_text_relayouts, resize_text_reshapes,
            resize_context_solves, style_to_layout_seeds, theme_to_layout_seeds,
            component_state_layout_seeds, theme_metric_dependents_invalidated,
            theme_palette_layout_invalidations, equivalent_style_layout_skips,
            replaced_content_updates, replaced_intrinsic_metadata_updates, replaced_layout_seeds,
            replaced_content_only_layout_invalidations, resource_intrinsic_dependents_notified,
            virtual_row_metric_updates, virtual_prefix_index_updates, virtual_rows_remeasured,
            virtual_rows_repositioned, virtual_logical_rows_scanned, virtual_scroll_extent_updates,
            virtual_rows_materialized_from_layout, container_query_size_changes,
            container_query_rules_evaluated, container_query_results_changed,
            container_query_results_unchanged, container_query_downstream_invalidations,
            container_query_convergence_rounds, container_query_cycle_fallbacks,
            intrinsic_measure_requests, intrinsic_measure_cache_hits,
            intrinsic_measure_cache_misses, intrinsic_measure_full_subtrees,
            intrinsic_generation_bumps, baseline_queries, cross_context_measure_hits,
            cross_context_measure_misses;
        optional: glyph_cache_hits, glyph_cache_misses, cache_eviction, batch_rebuilds,
            draw_batches, draw_calls, gpu_upload_bytes, gpu_buffer_reallocations,
            validation_nodes_scanned, hit_test_nodes_rebuilt, output_extra_passes,
            output_gpu_copies, output_target_recreates, output_content_revisions,
            output_idle_reuse_frames, output_resolve_count, output_gpu_copy_bytes,
            output_gpu_convert_passes;
    }
}

impl WorkCounters {
    /// Record hit-test entries built by a rebuild or patch that actually ran.
    /// Not recorded when the frame left the index alone.
    pub fn record_hit_test_rebuild(&mut self, nodes: usize) {
        self.hit_test_nodes_rebuilt = Some(
            self.hit_test_nodes_rebuilt
                .unwrap_or(0)
                .saturating_add(nodes),
        );
    }

    /// Record observed CPU hot-path heap events. Zero-count/zero-byte calls
    /// are ignored so empty `Vec::new()` is not a fake allocation.
    pub fn record_hot_path_allocation(&mut self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.allocations = self.allocations.saturating_add(count);
        self.allocated_bytes = self.allocated_bytes.saturating_add(bytes);
    }

    /// Record shaping-path stats after Runtime calls the host `TextShaper`.
    pub fn record_text_shape(
        &mut self,
        shaped_runs: usize,
        cache_hits: usize,
        cache_misses: usize,
        wrap_layouts: usize,
        constraint_relayouts: usize,
    ) {
        self.text_shaped_runs = self.text_shaped_runs.saturating_add(shaped_runs);
        self.text_layout_cache_hits = self.text_layout_cache_hits.saturating_add(cache_hits);
        self.text_layout_cache_misses = self.text_layout_cache_misses.saturating_add(cache_misses);
        self.text_wrap_layouts = self.text_wrap_layouts.saturating_add(wrap_layouts);
        self.text_constraint_relayouts = self
            .text_constraint_relayouts
            .saturating_add(constraint_relayouts);
    }

    /// Fold what text that shaped again did to layout: whether its exported
    /// metrics moved or held, and whether a moved one sent its parent to
    /// measure again.
    pub fn record_text_metrics(&mut self, changed: usize, unchanged: usize, parent_reflows: usize) {
        self.text_external_metric_changes =
            self.text_external_metric_changes.saturating_add(changed);
        self.text_external_metric_unchanged = self
            .text_external_metric_unchanged
            .saturating_add(unchanged);
        self.text_parent_reflows = self.text_parent_reflows.saturating_add(parent_reflows);
    }

    /// Fold what language changes reached: text whose language moved, and
    /// text shaped again although its language held.
    pub fn record_text_language(&mut self, scope_invalidations: usize, literal: usize) {
        self.text_language_scope_invalidations = self
            .text_language_scope_invalidations
            .saturating_add(scope_invalidations);
        self.text_literal_nodes_invalidated_by_language = self
            .text_literal_nodes_invalidated_by_language
            .saturating_add(literal);
    }

    /// Fold what a pass's constraint changes asked of children: the seeds
    /// that moved a constraint, the children asked whether they consume it,
    /// and of those the ones measured again and the ones left alone.
    pub fn record_constraint_dependents(
        &mut self,
        seeds: usize,
        considered: usize,
        remeasured: usize,
        skipped: usize,
    ) {
        self.constraint_change_seeds = self.constraint_change_seeds.saturating_add(seeds);
        self.constraint_dependents_considered = self
            .constraint_dependents_considered
            .saturating_add(considered);
        self.constraint_dependents_remeasured = self
            .constraint_dependents_remeasured
            .saturating_add(remeasured);
        self.constraint_dependents_skipped =
            self.constraint_dependents_skipped.saturating_add(skipped);
    }

    /// Fold the text a layout pass's new boxes laid out again, and of it the
    /// text that had to shape again.
    pub fn record_resize_text(&mut self, relayouts: usize, reshapes: usize) {
        self.resize_text_relayouts = self.resize_text_relayouts.saturating_add(relayouts);
        self.resize_text_reshapes = self.resize_text_reshapes.saturating_add(reshapes);
    }

    /// Fold what evaluating responsive rules cost: container axes that moved,
    /// rules read on them, and of those the ones that changed bucket and the
    /// ones that did not.
    pub fn record_container_query_evaluation(
        &mut self,
        size_changes: usize,
        evaluated: usize,
        changed: usize,
        unchanged: usize,
    ) {
        self.container_query_size_changes = self
            .container_query_size_changes
            .saturating_add(size_changes);
        self.container_query_rules_evaluated = self
            .container_query_rules_evaluated
            .saturating_add(evaluated);
        self.container_query_results_changed =
            self.container_query_results_changed.saturating_add(changed);
        self.container_query_results_unchanged = self
            .container_query_results_unchanged
            .saturating_add(unchanged);
    }

    /// Fold what the changed rules cost: the layout seeds their variants
    /// queued, the rounds of changes, and the changes held to settle.
    pub fn record_container_query_results(
        &mut self,
        downstream: usize,
        rounds: usize,
        fallbacks: usize,
    ) {
        self.container_query_downstream_invalidations = self
            .container_query_downstream_invalidations
            .saturating_add(downstream);
        self.container_query_convergence_rounds = self
            .container_query_convergence_rounds
            .saturating_add(rounds);
        self.container_query_cycle_fallbacks = self
            .container_query_cycle_fallbacks
            .saturating_add(fallbacks);
    }

    /// Fold what a typography scale change cost text: text it reached, text
    /// laid out again at the new size, and text whose metrics moved its parent.
    pub fn record_typography_scale_text(
        &mut self,
        notified: usize,
        relayouts: usize,
        parent_reflows: usize,
    ) {
        self.typography_scale_dependents_notified = self
            .typography_scale_dependents_notified
            .saturating_add(notified);
        self.typography_scale_text_relayouts = self
            .typography_scale_text_relayouts
            .saturating_add(relayouts);
        self.typography_scale_parent_reflows = self
            .typography_scale_parent_reflows
            .saturating_add(parent_reflows);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_intrinsic_measure(
        &mut self,
        requests: usize,
        hits: usize,
        misses: usize,
        full_subtrees: usize,
        generation_bumps: usize,
        baseline_queries: usize,
        cross_context_hits: usize,
        cross_context_misses: usize,
    ) {
        self.intrinsic_measure_requests = self.intrinsic_measure_requests.saturating_add(requests);
        self.intrinsic_measure_cache_hits = self.intrinsic_measure_cache_hits.saturating_add(hits);
        self.intrinsic_measure_cache_misses =
            self.intrinsic_measure_cache_misses.saturating_add(misses);
        self.intrinsic_measure_full_subtrees = self
            .intrinsic_measure_full_subtrees
            .saturating_add(full_subtrees);
        self.intrinsic_generation_bumps = self
            .intrinsic_generation_bumps
            .saturating_add(generation_bumps);
        self.baseline_queries = self.baseline_queries.saturating_add(baseline_queries);
        self.cross_context_measure_hits = self
            .cross_context_measure_hits
            .saturating_add(cross_context_hits);
        self.cross_context_measure_misses = self
            .cross_context_measure_misses
            .saturating_add(cross_context_misses);
    }

    /// Record one dependency-aware frontier build.  The caller supplies the
    /// structural observations from the retained scheduler; timing remains in
    /// Runtime's frame profiler.  Keeping this as one operation prevents a
    /// partially filled snapshot when a pass exits early.
    #[allow(clippy::too_many_arguments)]
    pub fn record_layout_frontier(
        &mut self,
        seeds: usize,
        seed_merges: usize,
        nodes_measure: usize,
        nodes_placement: usize,
        contexts: usize,
        nodes_writing: usize,
        edges_visited: usize,
        propagations_stopped: usize,
        local_subtree_fallbacks: usize,
        full_document_fallbacks: usize,
    ) {
        self.layout_frontier_seeds = self.layout_frontier_seeds.saturating_add(seeds);
        self.layout_frontier_seed_merges =
            self.layout_frontier_seed_merges.saturating_add(seed_merges);
        self.layout_frontier_nodes_measure = self
            .layout_frontier_nodes_measure
            .saturating_add(nodes_measure);
        self.layout_frontier_nodes_placement = self
            .layout_frontier_nodes_placement
            .saturating_add(nodes_placement);
        self.layout_frontier_contexts = self.layout_frontier_contexts.saturating_add(contexts);
        self.layout_frontier_nodes_writing = self
            .layout_frontier_nodes_writing
            .saturating_add(nodes_writing);
        self.layout_dependency_edges_visited = self
            .layout_dependency_edges_visited
            .saturating_add(edges_visited);
        self.layout_propagations_stopped = self
            .layout_propagations_stopped
            .saturating_add(propagations_stopped);
        self.layout_local_subtree_fallbacks = self
            .layout_local_subtree_fallbacks
            .saturating_add(local_subtree_fallbacks);
        self.layout_full_document_fallbacks = self
            .layout_full_document_fallbacks
            .saturating_add(full_document_fallbacks);
    }

    pub fn record_layout_execution(
        &mut self,
        measure_nodes: usize,
        measure_cache_hits: usize,
        measure_cache_misses: usize,
        placement_nodes: usize,
        origin_only_updates: usize,
    ) {
        self.layout_measure_nodes = self.layout_measure_nodes.saturating_add(measure_nodes);
        self.layout_measure_cache_hits = self
            .layout_measure_cache_hits
            .saturating_add(measure_cache_hits);
        self.layout_measure_cache_misses = self
            .layout_measure_cache_misses
            .saturating_add(measure_cache_misses);
        self.layout_placement_nodes = self.layout_placement_nodes.saturating_add(placement_nodes);
        self.layout_origin_only_updates = self
            .layout_origin_only_updates
            .saturating_add(origin_only_updates);
    }

    /// `delta_commits` is 1 only when at least one result object was replaced.
    pub fn record_layout_result_publish(
        &mut self,
        reused: usize,
        changed: usize,
        delta_commits: usize,
        children_visited: usize,
    ) {
        self.layout_result_reused = self.layout_result_reused.saturating_add(reused);
        self.layout_result_changed = self.layout_result_changed.saturating_add(changed);
        self.layout_delta_commits = self.layout_delta_commits.saturating_add(delta_commits);
        self.layout_result_children_visited = self
            .layout_result_children_visited
            .saturating_add(children_visited);
    }

    /// Containers whose recorded formatting context changed.
    pub fn record_layout_context_transitions(&mut self, transitions: usize) {
        self.layout_context_transitions =
            self.layout_context_transitions.saturating_add(transitions);
    }

    /// Fold one pass's retained-plan work. A positioned context laid out whole
    /// because its plan was stale is a bounded local fallback, counted with
    /// the frontier's.
    #[allow(clippy::too_many_arguments)]
    pub fn record_layout_plans(
        &mut self,
        placement_plans_reused: usize,
        measure_plans_reused: usize,
        suffixes_replayed: usize,
        children_measured: usize,
        containers_uncacheable: usize,
        plan_misses: usize,
        plan_rebuilds: usize,
        local_context_fallbacks: usize,
        retain_sweeps: usize,
    ) {
        self.layout_placement_plans_reused = self
            .layout_placement_plans_reused
            .saturating_add(placement_plans_reused);
        self.layout_measure_plans_reused = self
            .layout_measure_plans_reused
            .saturating_add(measure_plans_reused);
        self.layout_suffixes_replayed = self
            .layout_suffixes_replayed
            .saturating_add(suffixes_replayed);
        self.layout_children_measured = self
            .layout_children_measured
            .saturating_add(children_measured);
        self.layout_containers_uncacheable = self
            .layout_containers_uncacheable
            .saturating_add(containers_uncacheable);
        self.layout_plan_misses = self.layout_plan_misses.saturating_add(plan_misses);
        self.layout_plan_rebuilds = self.layout_plan_rebuilds.saturating_add(plan_rebuilds);
        self.layout_local_subtree_fallbacks = self
            .layout_local_subtree_fallbacks
            .saturating_add(local_context_fallbacks);
        self.layout_retain_sweeps = self.layout_retain_sweeps.saturating_add(retain_sweeps);
    }

    /// Fold the scratch one pass's dependency graph held.
    pub fn record_layout_scratch(&mut self, entries: usize, bytes: usize) {
        self.layout_scratch_entries = self.layout_scratch_entries.saturating_add(entries);
        self.layout_scratch_bytes = self.layout_scratch_bytes.saturating_add(bytes);
    }

    /// Record `TextLayoutCache` FIFO evictions. Does not invent glyph evictions.
    pub fn record_cache_eviction(&mut self, evictions: usize) {
        self.cache_eviction = Some(self.cache_eviction.unwrap_or(0).saturating_add(evictions));
    }

    /// Record `GlyphCache` lookup/insert from a shaping pass that consulted it.
    /// Zeros are stored as `Some(0)` only because that pass ran, not because a
    /// non-glyph host guessed quiet glyph work.
    pub fn record_glyph_cache(&mut self, hits: usize, misses: usize) {
        self.glyph_cache_hits = Some(self.glyph_cache_hits.unwrap_or(0).saturating_add(hits));
        self.glyph_cache_misses = Some(self.glyph_cache_misses.unwrap_or(0).saturating_add(misses));
    }

    /// Fold GPU work observed on a real encode/submit path. Zeros are stored as
    /// `Some(0)` only because the host ran that path, not because a CPU drain
    /// guessed quiet GPU work.
    pub fn record_gpu_work(&mut self, observed: GpuWorkObservation) {
        self.batch_rebuilds = Some(
            self.batch_rebuilds
                .unwrap_or(0)
                .saturating_add(observed.batch_rebuilds),
        );
        self.draw_batches = Some(
            self.draw_batches
                .unwrap_or(0)
                .saturating_add(observed.draw_batches),
        );
        self.draw_calls = Some(
            self.draw_calls
                .unwrap_or(0)
                .saturating_add(observed.draw_calls),
        );
        self.gpu_upload_bytes = Some(
            self.gpu_upload_bytes
                .unwrap_or(0)
                .saturating_add(observed.gpu_upload_bytes),
        );
        self.gpu_buffer_reallocations = Some(
            self.gpu_buffer_reallocations
                .unwrap_or(0)
                .saturating_add(observed.gpu_buffer_reallocations),
        );
    }

    /// Fold output work observed by a real presenter/producer. Calling this
    /// with the default observation records explicit zeroes, which is useful
    /// for proving a direct window path did no extra work.
    pub fn record_output_work(&mut self, observed: OutputWorkObservation) {
        let record = |slot: &mut Option<usize>, value: usize| {
            *slot = Some(slot.unwrap_or(0).saturating_add(value));
        };
        record(&mut self.output_extra_passes, observed.extra_passes);
        record(&mut self.output_gpu_copies, observed.gpu_copies);
        record(&mut self.output_target_recreates, observed.target_recreates);
        record(
            &mut self.output_content_revisions,
            observed.content_revisions,
        );
        record(
            &mut self.output_idle_reuse_frames,
            observed.idle_reuse_frames,
        );
        record(&mut self.output_resolve_count, observed.resolve_count);
        record(&mut self.output_gpu_copy_bytes, observed.gpu_copy_bytes);
        record(
            &mut self.output_gpu_convert_passes,
            observed.gpu_convert_passes,
        );
    }
}

fn fold_optional_count(slot: &mut Option<usize>, other: Option<usize>) {
    *slot = match (*slot, other) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    };
}

/// Named CPU stages from Issue #8 §4. Runtime times the stages it owns;
/// Batch / GPU Upload / Encode / Submit stay `runtime_unsupported` until a
/// GPU host that actually encodes/submits times them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameStage {
    Input,
    Reconcile,
    Style,
    TextShape,
    Layout,
    HitTest,
    Accessibility,
    Animation,
    Extract,
    Batch,
    GpuUpload,
    Encode,
    Submit,
}

impl FrameStage {
    pub const ALL: [Self; 13] = [
        Self::Input,
        Self::Reconcile,
        Self::Style,
        Self::TextShape,
        Self::Layout,
        Self::HitTest,
        Self::Accessibility,
        Self::Animation,
        Self::Extract,
        Self::Batch,
        Self::GpuUpload,
        Self::Encode,
        Self::Submit,
    ];

    /// Stages the retained Runtime does not own. A profiler should report these
    /// as unsupported with zero duration rather than pretending they ran.
    pub const fn runtime_unsupported(self) -> bool {
        self.gpu_host_owned()
    }

    /// Stages a Scene/WGPU host owns: Batch, GPU Upload, Encode, Submit.
    /// Measurable only when that host actually encodes/submits.
    pub const fn gpu_host_owned(self) -> bool {
        matches!(
            self,
            Self::Batch | Self::GpuUpload | Self::Encode | Self::Submit
        )
    }
}

/// Work counts for one theme / style resolution pass (Issue #101 §4).
///
/// Not [`WorkCounters`]: those are the per-frame dirty-system totals, and
/// `style_processed` there is only "how many nodes the drain scheduled".
/// The Theme baseline needs the inside of that pass — how many of the
/// scheduled nodes really produced a new `ComputedStyle`, how many were
/// answered by the value/epoch fast path, how often the pass read the token
/// authority, and what the pass cost the stages downstream of it.
///
/// Field convention follows [`WorkCounters`]: a plain `usize` is a number the
/// owning pass always knows. There is no optional field here — the style pass
/// either ran (and knows all of these) or did not run at all.
///
/// Invariant: `style_nodes_considered == style_nodes_resolved +
/// style_nodes_skipped`. A node reached through a parent chain counts once for
/// the pass, not once per descendant that pulled it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThemeWorkCounters {
    /// Nodes the pass evaluated, including the ones it then skipped.
    pub style_nodes_considered: usize,
    /// Of those, the ones that published a new resolved style.
    pub style_nodes_resolved: usize,
    /// Of those, the ones whose resolved value and theme generation both
    /// matched what the node already held, so nothing was published.
    pub style_nodes_skipped: usize,
    /// Reads the pass made against the token authority
    /// ([`crate::StyleModelRef`]). A palette mix counts once — it is one
    /// question asked of the theme, not one per role it blends.
    pub theme_reads: usize,
    /// Heap events the style path itself caused: one per published resolved
    /// style, one per theme-install invalidation list. Its own observation,
    /// not a slice of `WorkCounters::allocations` — that field watches drain
    /// lists, layout inputs and text temporaries, and folding per-node style
    /// publications into it would move every existing #8 allocation number.
    pub style_allocations: usize,
    /// Payload bytes of those events.
    pub style_allocated_bytes: usize,
    /// `LayoutStyle` copies made to hold a node's resolved design intent
    /// beside the layout it authored. Counted apart from `style_allocations`
    /// because it is a different event with a different cause, an order of
    /// magnitude larger, and invisible to every allocator counter — it
    /// happens under `Arc::make_mut`, which reuses a unique allocation and
    /// clones a shared one without telling anyone. A theme change that only
    /// moves colour must leave this at 0.
    pub layout_copies: usize,
    /// Payload bytes of those copies.
    pub layout_copied_bytes: usize,
    /// Nodes the theme/style authority invalidated for layout. A palette-only
    /// theme change must leave this at 0.
    pub layout_nodes_from_style: usize,
    /// Nodes whose text shaping or line constraints a resolved-style change
    /// invalidated. Text whose *color* changed is paint, not text, and is not
    /// counted here — otherwise a palette switch would read as reshaping.
    pub text_nodes_from_style: usize,
    /// Nodes the theme/style authority invalidated for paint/extract.
    pub paint_nodes_from_style: usize,
}

impl ThemeWorkCounters {
    /// Record one evaluated node that published a new resolved style.
    pub fn record_resolved(&mut self) {
        self.style_nodes_considered = self.style_nodes_considered.saturating_add(1);
        self.style_nodes_resolved = self.style_nodes_resolved.saturating_add(1);
    }

    /// Record one evaluated node the value/generation fast path answered.
    pub fn record_skipped(&mut self) {
        self.style_nodes_considered = self.style_nodes_considered.saturating_add(1);
        self.style_nodes_skipped = self.style_nodes_skipped.saturating_add(1);
    }

    /// Record reads against the token authority.
    pub fn record_theme_reads(&mut self, reads: usize) {
        self.theme_reads = self.theme_reads.saturating_add(reads);
    }

    /// Record a heap event the style path caused. A zero event is not an
    /// allocation, matching [`WorkCounters::record_hot_path_allocation`].
    pub fn record_allocation(&mut self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.style_allocations = self.style_allocations.saturating_add(count);
        self.style_allocated_bytes = self.style_allocated_bytes.saturating_add(bytes);
    }

    /// Record a `LayoutStyle` copied to resolve design intent.
    pub fn record_layout_copy(&mut self, count: usize, bytes: usize) {
        if count == 0 && bytes == 0 {
            return;
        }
        self.layout_copies = self.layout_copies.saturating_add(count);
        self.layout_copied_bytes = self.layout_copied_bytes.saturating_add(bytes);
    }

    pub fn record_layout_invalidation(&mut self, nodes: usize) {
        self.layout_nodes_from_style = self.layout_nodes_from_style.saturating_add(nodes);
    }

    pub fn record_text_invalidation(&mut self, nodes: usize) {
        self.text_nodes_from_style = self.text_nodes_from_style.saturating_add(nodes);
    }

    pub fn record_paint_invalidation(&mut self, nodes: usize) {
        self.paint_nodes_from_style = self.paint_nodes_from_style.saturating_add(nodes);
    }

    /// An idle steady frame resolved no style and asked the theme nothing.
    /// Issue #100 §15: a retained frame must not re-walk or re-resolve.
    pub fn is_idle(self) -> bool {
        self.style_nodes_considered == 0 && self.theme_reads == 0
    }

    /// A palette-only theme change is paint work: no layout, no reshape, and
    /// no box touched. `layout_copies` is part of the question because moving
    /// colour must not send the style path through `LayoutStyle` at all — a
    /// resolver that rebuilt every box on a palette switch would still report
    /// zero layout *invalidations* while doing the work.
    pub fn is_paint_only(self) -> bool {
        self.layout_nodes_from_style == 0
            && self.text_nodes_from_style == 0
            && self.layout_copies == 0
    }
}

accumulate_counters! {
    /// Fold another pass into this snapshot.
    ThemeWorkCounters {
        sum: style_nodes_considered, style_nodes_resolved, style_nodes_skipped, theme_reads,
            style_allocations, style_allocated_bytes, layout_copies, layout_copied_bytes,
            layout_nodes_from_style, text_nodes_from_style, paint_nodes_from_style;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_counters_are_zero_and_gpu_stages_are_explicitly_unsupported() {
        assert_eq!(WorkCounters::default(), WorkCounters::default());
        assert_eq!(WorkCounters::default().allocations, 0);
        assert_eq!(WorkCounters::default().allocated_bytes, 0);
        assert_eq!(WorkCounters::default().layout_frontier_seeds, 0);
        assert_eq!(WorkCounters::default().layout_frontier_seed_merges, 0);
        assert_eq!(WorkCounters::default().layout_frontier_nodes_measure, 0);
        assert_eq!(WorkCounters::default().layout_frontier_nodes_placement, 0);
        assert_eq!(WorkCounters::default().layout_frontier_contexts, 0);
        assert_eq!(WorkCounters::default().layout_dependency_edges_visited, 0);
        assert_eq!(WorkCounters::default().layout_propagations_stopped, 0);
        assert_eq!(WorkCounters::default().layout_local_subtree_fallbacks, 0);
        assert_eq!(WorkCounters::default().layout_full_document_fallbacks, 0);
        assert_eq!(WorkCounters::default().layout_measure_nodes, 0);
        assert_eq!(WorkCounters::default().layout_measure_cache_hits, 0);
        assert_eq!(WorkCounters::default().layout_measure_cache_misses, 0);
        assert_eq!(WorkCounters::default().layout_placement_nodes, 0);
        assert_eq!(WorkCounters::default().layout_result_reused, 0);
        assert_eq!(WorkCounters::default().layout_result_changed, 0);
        assert_eq!(WorkCounters::default().layout_origin_only_updates, 0);
        assert_eq!(WorkCounters::default().layout_delta_commits, 0);
        assert_eq!(WorkCounters::default().text_shaped_runs, 0);
        assert_eq!(WorkCounters::default().text_layout_cache_hits, 0);
        assert_eq!(WorkCounters::default().text_layout_cache_misses, 0);
        assert_eq!(WorkCounters::default().text_wrap_layouts, 0);
        assert_eq!(WorkCounters::default().glyph_cache_hits, None);
        assert_eq!(WorkCounters::default().glyph_cache_misses, None);
        assert_eq!(WorkCounters::default().cache_eviction, None);
        assert_eq!(WorkCounters::default().batch_rebuilds, None);
        assert_eq!(WorkCounters::default().draw_batches, None);
        assert_eq!(WorkCounters::default().draw_calls, None);
        assert_eq!(WorkCounters::default().gpu_upload_bytes, None);
        assert_eq!(WorkCounters::default().gpu_buffer_reallocations, None);
        assert!(!FrameStage::Style.runtime_unsupported());
        assert!(FrameStage::GpuUpload.runtime_unsupported());
        assert!(FrameStage::GpuUpload.gpu_host_owned());
        assert!(!FrameStage::Extract.gpu_host_owned());
        assert_eq!(FrameStage::ALL.len(), 13);
    }

    #[test]
    fn accumulate_folds_hot_path_allocation_and_text_shape_fields() {
        let mut total = WorkCounters {
            allocations: 1,
            allocated_bytes: 32,
            text_shaped_runs: 2,
            text_layout_cache_hits: 1,
            text_layout_cache_misses: 1,
            text_wrap_layouts: 1,
            cache_eviction: Some(1),
            input_targets: 2,
            render_nodes_changed: 3,
            ..WorkCounters::default()
        };
        total.accumulate(WorkCounters {
            entities_total: 9,
            allocations: 3,
            allocated_bytes: 16,
            text_shaped_runs: 1,
            text_layout_cache_hits: 2,
            text_layout_cache_misses: 0,
            text_wrap_layouts: 1,
            cache_eviction: Some(2),
            input_targets: 1,
            render_nodes_changed: 4,
            ..WorkCounters::default()
        });
        assert_eq!(total.entities_total, 9);
        assert_eq!(total.allocations, 4);
        assert_eq!(total.allocated_bytes, 48);
        assert_eq!(total.text_shaped_runs, 3);
        assert_eq!(total.text_layout_cache_hits, 3);
        assert_eq!(total.text_layout_cache_misses, 1);
        assert_eq!(total.text_wrap_layouts, 2);
        assert_eq!(total.cache_eviction, Some(3));
        assert_eq!(total.glyph_cache_hits, None);
        assert_eq!(total.gpu_upload_bytes, None);
        assert_eq!(total.input_targets, 3);
        assert_eq!(total.render_nodes_changed, 7);
    }

    #[test]
    fn frontier_counters_record_and_accumulate_structural_work() {
        let mut first = WorkCounters::default();
        first.record_layout_frontier(3, 1, 5, 7, 2, 4, 11, 1, 0, 0);
        assert_eq!(first.layout_frontier_seeds, 3);
        assert_eq!(first.layout_frontier_seed_merges, 1);
        assert_eq!(first.layout_frontier_nodes_measure, 5);
        assert_eq!(first.layout_frontier_nodes_placement, 7);
        assert_eq!(first.layout_frontier_contexts, 2);
        assert_eq!(first.layout_frontier_nodes_writing, 4);
        assert_eq!(first.layout_dependency_edges_visited, 11);
        assert_eq!(first.layout_propagations_stopped, 1);
        assert_eq!(first.layout_local_subtree_fallbacks, 0);
        assert_eq!(first.layout_full_document_fallbacks, 0);

        let mut second = WorkCounters::default();
        second.record_layout_frontier(2, 4, 1, 0, 1, 1, 3, 2, 1, 1);
        first.accumulate(second);
        assert_eq!(first.layout_frontier_seeds, 5);
        assert_eq!(first.layout_frontier_seed_merges, 5);
        assert_eq!(first.layout_frontier_nodes_measure, 6);
        assert_eq!(first.layout_frontier_nodes_placement, 7);
        assert_eq!(first.layout_frontier_contexts, 3);
        assert_eq!(first.layout_frontier_nodes_writing, 5);
        assert_eq!(first.layout_dependency_edges_visited, 14);
        assert_eq!(first.layout_propagations_stopped, 3);
        assert_eq!(first.layout_local_subtree_fallbacks, 1);
        assert_eq!(first.layout_full_document_fallbacks, 1);
    }

    #[test]
    fn execution_counters_record_and_accumulate_layout_work() {
        let mut first = WorkCounters::default();
        first.record_layout_execution(4, 3, 2, 6, 5);
        first.record_layout_result_publish(8, 1, 1, 12);
        assert_eq!(first.layout_measure_nodes, 4);
        assert_eq!(first.layout_measure_cache_hits, 3);
        assert_eq!(first.layout_measure_cache_misses, 2);
        assert_eq!(first.layout_placement_nodes, 6);
        assert_eq!(first.layout_origin_only_updates, 5);
        assert_eq!(first.layout_result_reused, 8);
        assert_eq!(first.layout_result_changed, 1);
        assert_eq!(first.layout_delta_commits, 1);
        assert_eq!(first.layout_result_children_visited, 12);

        let mut second = WorkCounters::default();
        second.record_layout_execution(1, 0, 1, 2, 2);
        second.record_layout_result_publish(1, 0, 0, 3);
        first.accumulate(second);
        assert_eq!(first.layout_measure_nodes, 5);
        assert_eq!(first.layout_measure_cache_hits, 3);
        assert_eq!(first.layout_measure_cache_misses, 3);
        assert_eq!(first.layout_placement_nodes, 8);
        assert_eq!(first.layout_origin_only_updates, 7);
        assert_eq!(first.layout_result_reused, 9);
        assert_eq!(first.layout_result_changed, 1);
        assert_eq!(first.layout_delta_commits, 1);
        assert_eq!(first.layout_result_children_visited, 15);
    }

    #[test]
    fn layout_plan_and_scratch_counters_record_and_accumulate() {
        let mut first = WorkCounters::default();
        first.record_layout_plans(1, 2, 3, 4, 5, 8, 9, 6, 7);
        first.record_layout_scratch(8, 64);
        assert_eq!(first.layout_placement_plans_reused, 1);
        assert_eq!(first.layout_measure_plans_reused, 2);
        assert_eq!(first.layout_suffixes_replayed, 3);
        assert_eq!(first.layout_children_measured, 4);
        assert_eq!(first.layout_containers_uncacheable, 5);
        assert_eq!(first.layout_plan_misses, 8);
        assert_eq!(first.layout_plan_rebuilds, 9);
        assert_eq!(first.layout_local_subtree_fallbacks, 6);
        assert_eq!(first.layout_retain_sweeps, 7);
        assert_eq!(first.layout_scratch_entries, 8);
        assert_eq!(first.layout_scratch_bytes, 64);
        let mut second = WorkCounters::default();
        second.record_layout_plans(1, 1, 1, 1, 1, 1, 1, 1, 1);
        second.record_layout_scratch(2, 16);
        first.accumulate(second);
        assert_eq!(first.layout_placement_plans_reused, 2);
        assert_eq!(first.layout_children_measured, 5);
        assert_eq!(first.layout_local_subtree_fallbacks, 7);
        assert_eq!(first.layout_scratch_entries, 10);
        assert_eq!(first.layout_scratch_bytes, 80);
    }

    #[test]
    fn text_reflow_counters_record_and_accumulate() {
        let mut first = WorkCounters::default();
        first.record_text_shape(1, 2, 3, 4, 5);
        first.record_text_metrics(2, 3, 1);
        first.record_text_language(4, 0);
        assert_eq!(first.text_constraint_relayouts, 5);
        assert_eq!(first.text_external_metric_changes, 2);
        assert_eq!(first.text_external_metric_unchanged, 3);
        assert_eq!(first.text_parent_reflows, 1);
        assert_eq!(first.text_language_scope_invalidations, 4);
        assert_eq!(first.text_literal_nodes_invalidated_by_language, 0);
        let mut second = WorkCounters::default();
        second.record_text_metrics(1, 1, 1);
        second.record_text_language(1, 2);
        first.accumulate(second);
        assert_eq!(first.text_external_metric_changes, 3);
        assert_eq!(first.text_external_metric_unchanged, 4);
        assert_eq!(first.text_parent_reflows, 2);
        assert_eq!(first.text_language_scope_invalidations, 5);
        assert_eq!(first.text_literal_nodes_invalidated_by_language, 2);
    }

    #[test]
    fn i18n_counters_add_work_and_keep_the_largest_index() {
        let mut first = WorkCounters::default();
        first.i18n.localized_nodes = 5_000;
        first.i18n.nodes_resolved = 5_000;
        let mut second = WorkCounters::default();
        second.i18n.localized_nodes = 4_000;
        second.i18n.nodes_resolved = 10;
        second.i18n.switch_commits = 1;
        first.accumulate(second);
        assert_eq!(first.i18n.localized_nodes, 5_000);
        assert_eq!(first.i18n.nodes_resolved, 5_010);
        assert_eq!(first.i18n.switch_commits, 1);
    }

    #[test]
    fn container_query_counters_record_and_accumulate() {
        let mut first = WorkCounters::default();
        first.record_container_query_evaluation(1, 100, 10, 90);
        first.record_container_query_results(10, 1, 0);
        let mut second = WorkCounters::default();
        second.record_container_query_evaluation(1, 100, 0, 100);
        second.record_container_query_results(0, 0, 2);
        first.accumulate(second);
        assert_eq!(first.container_query_size_changes, 2);
        assert_eq!(first.container_query_rules_evaluated, 200);
        assert_eq!(first.container_query_results_changed, 10);
        assert_eq!(first.container_query_results_unchanged, 190);
        assert_eq!(first.container_query_downstream_invalidations, 10);
        assert_eq!(first.container_query_convergence_rounds, 1);
        assert_eq!(first.container_query_cycle_fallbacks, 2);
    }

    #[test]
    fn constraint_and_resize_counters_record_and_accumulate() {
        let mut first = WorkCounters::default();
        first.record_constraint_dependents(1, 10, 4, 6);
        first.record_resize_text(3, 0);
        first.resize_context_solves = 2;
        let mut second = WorkCounters::default();
        second.record_constraint_dependents(2, 5, 5, 0);
        second.record_resize_text(1, 1);
        second.resize_context_solves = 1;
        first.accumulate(second);
        assert_eq!(first.constraint_change_seeds, 3);
        assert_eq!(first.constraint_dependents_considered, 15);
        assert_eq!(first.constraint_dependents_remeasured, 9);
        assert_eq!(first.constraint_dependents_skipped, 6);
        assert_eq!(first.resize_text_relayouts, 4);
        assert_eq!(first.resize_text_reshapes, 1);
        assert_eq!(first.resize_context_solves, 3);
    }

    #[test]
    fn typography_scale_counters_record_and_accumulate() {
        let mut first = WorkCounters::default();
        first.record_typography_scale_text(3, 2, 1);
        assert_eq!(first.typography_scale_dependents_notified, 3);
        assert_eq!(first.typography_scale_text_relayouts, 2);
        assert_eq!(first.typography_scale_parent_reflows, 1);
        let mut second = WorkCounters::default();
        second.record_typography_scale_text(1, 1, 0);
        first.accumulate(second);
        assert_eq!(first.typography_scale_dependents_notified, 4);
        assert_eq!(first.typography_scale_text_relayouts, 3);
        assert_eq!(first.typography_scale_parent_reflows, 1);
    }

    #[test]
    fn glyph_cache_fields_stay_none_until_a_backend_records_them() {
        let counters = WorkCounters::default();
        assert!(counters.glyph_cache_hits.is_none());
        assert!(counters.glyph_cache_misses.is_none());
        assert!(counters.cache_eviction.is_none());
        let mut recorded = WorkCounters::default();
        recorded.record_cache_eviction(0);
        assert_eq!(recorded.cache_eviction, Some(0));
        assert!(recorded.glyph_cache_hits.is_none());
        recorded.record_glyph_cache(2, 1);
        assert_eq!(recorded.glyph_cache_hits, Some(2));
        assert_eq!(recorded.glyph_cache_misses, Some(1));
        recorded.record_glyph_cache(0, 3);
        assert_eq!(recorded.glyph_cache_hits, Some(2));
        assert_eq!(recorded.glyph_cache_misses, Some(4));
        let mut total = recorded;
        total.accumulate(WorkCounters {
            glyph_cache_hits: Some(1),
            glyph_cache_misses: Some(0),
            ..WorkCounters::default()
        });
        assert_eq!(total.glyph_cache_hits, Some(3));
        assert_eq!(total.glyph_cache_misses, Some(4));
    }

    #[test]
    fn intrinsic_measure_counters_are_published_and_accumulate() {
        let mut counters = WorkCounters::default();
        counters.record_intrinsic_measure(5, 2, 3, 1, 4, 6, 7, 8);
        assert_eq!(counters.intrinsic_measure_requests, 5);
        assert_eq!(counters.intrinsic_measure_cache_hits, 2);
        assert_eq!(counters.intrinsic_measure_cache_misses, 3);
        assert_eq!(counters.intrinsic_measure_full_subtrees, 1);
        assert_eq!(counters.intrinsic_generation_bumps, 4);
        assert_eq!(counters.baseline_queries, 6);
        assert_eq!(counters.cross_context_measure_hits, 7);
        assert_eq!(counters.cross_context_measure_misses, 8);
        counters.accumulate(WorkCounters {
            intrinsic_measure_requests: 1,
            intrinsic_measure_cache_hits: 1,
            ..WorkCounters::default()
        });
        assert_eq!(counters.intrinsic_measure_requests, 6);
        assert_eq!(counters.intrinsic_measure_cache_hits, 3);
    }

    #[test]
    fn incremental_work_sentinels_stay_none_until_a_pass_reports_them() {
        let counters = WorkCounters::default();
        assert_eq!(counters.validation_nodes_scanned, None);
        assert_eq!(counters.hit_test_nodes_rebuilt, None);

        let mut recorded = WorkCounters::default();
        // A rebuild that ran and built nothing is an observation, not a gap.
        recorded.record_hit_test_rebuild(0);
        assert_eq!(recorded.hit_test_nodes_rebuilt, Some(0));
        recorded.record_hit_test_rebuild(3);
        recorded.record_hit_test_rebuild(4);
        assert_eq!(recorded.hit_test_nodes_rebuilt, Some(7));
        assert_eq!(recorded.validation_nodes_scanned, None);

        // Folding drains sums each sentinel; a drain that reports neither must
        // not turn a missing measurement into a zero.
        let mut total = WorkCounters {
            validation_nodes_scanned: Some(2),
            ..WorkCounters::default()
        };
        total.accumulate(recorded);
        assert_eq!(total.validation_nodes_scanned, Some(2));
        assert_eq!(total.hit_test_nodes_rebuilt, Some(7));
        total.accumulate(WorkCounters {
            validation_nodes_scanned: Some(5),
            ..WorkCounters::default()
        });
        assert_eq!(total.validation_nodes_scanned, Some(7));
        assert_eq!(total.hit_test_nodes_rebuilt, Some(7));
    }

    #[test]
    fn gpu_work_fields_are_queryable_as_unsupported_until_a_host_encodes() {
        let counters = WorkCounters::default();
        assert!(counters.batch_rebuilds.is_none());
        assert!(counters.draw_batches.is_none());
        assert!(counters.draw_calls.is_none());
        assert!(counters.gpu_upload_bytes.is_none());
        assert!(counters.gpu_buffer_reallocations.is_none());
        let mut recorded = WorkCounters::default();
        recorded.record_gpu_work(GpuWorkObservation::default());
        assert_eq!(recorded.gpu_upload_bytes, Some(0));
        assert_eq!(recorded.draw_calls, Some(0));
        assert!(recorded.glyph_cache_hits.is_none());
        recorded.record_gpu_work(GpuWorkObservation {
            gpu_upload_bytes: 64,
            draw_calls: 2,
            draw_batches: 1,
            batch_rebuilds: 1,
            gpu_buffer_reallocations: 0,
        });
        assert_eq!(recorded.gpu_upload_bytes, Some(64));
        assert_eq!(recorded.draw_calls, Some(2));
        assert_eq!(recorded.draw_batches, Some(1));
        assert_eq!(recorded.batch_rebuilds, Some(1));
        assert_eq!(recorded.gpu_buffer_reallocations, Some(0));
    }

    #[test]
    fn output_work_is_absent_until_a_presenter_records_an_observation() {
        let mut counters = WorkCounters::default();
        assert_eq!(counters.output_gpu_copies, None);
        counters.record_output_work(OutputWorkObservation {
            target_recreates: 2,
            content_revisions: 1,
            idle_reuse_frames: 3,
            ..Default::default()
        });
        assert_eq!(counters.output_target_recreates, Some(2));
        assert_eq!(counters.output_content_revisions, Some(1));
        assert_eq!(counters.output_idle_reuse_frames, Some(3));
        assert_eq!(counters.output_gpu_copies, Some(0));

        let mut total = counters;
        total.accumulate(WorkCounters {
            output_gpu_copies: Some(2),
            output_gpu_copy_bytes: Some(2048),
            ..Default::default()
        });
        assert_eq!(total.output_gpu_copies, Some(2));
        assert_eq!(total.output_gpu_copy_bytes, Some(2048));
    }

    #[test]
    fn theme_pass_splits_considered_into_resolved_and_skipped() {
        let mut counters = ThemeWorkCounters::default();
        assert!(counters.is_idle());
        counters.record_resolved();
        counters.record_resolved();
        counters.record_skipped();
        counters.record_theme_reads(6);
        assert_eq!(counters.style_nodes_considered, 3);
        assert_eq!(counters.style_nodes_resolved, 2);
        assert_eq!(counters.style_nodes_skipped, 1);
        assert_eq!(
            counters.style_nodes_considered,
            counters.style_nodes_resolved + counters.style_nodes_skipped
        );
        assert_eq!(counters.theme_reads, 6);
        assert!(!counters.is_idle());
    }

    #[test]
    fn theme_pass_allocation_ignores_a_zero_event_and_folds_downstream_classes() {
        let mut counters = ThemeWorkCounters::default();
        counters.record_allocation(0, 0);
        assert_eq!(counters.style_allocations, 0);
        counters.record_allocation(1, 48);
        counters.record_allocation(2, 16);
        assert_eq!(counters.style_allocations, 3);
        assert_eq!(counters.style_allocated_bytes, 64);

        // A palette-only change is paint work; layout and reshape stay at 0.
        counters.record_paint_invalidation(120);
        assert!(counters.is_paint_only());
        counters.record_layout_invalidation(4);
        counters.record_text_invalidation(2);
        assert!(!counters.is_paint_only());

        let mut total = ThemeWorkCounters::default();
        total.accumulate(counters);
        total.accumulate(counters);
        assert_eq!(total.style_allocations, 6);
        assert_eq!(total.style_allocated_bytes, 128);
        assert_eq!(total.paint_nodes_from_style, 240);
        assert_eq!(total.layout_nodes_from_style, 8);
        assert_eq!(total.text_nodes_from_style, 4);
    }
}
