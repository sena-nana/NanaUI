//! NanaUI's own events and metrics (domains `0x0001..=0x0009`). IDs are part
//! of the `.nlog` contract: never renumber, only append.
//!
//! Framework crates record these; applications define their own descriptors
//! in domains from [`Domain::APPLICATION_MIN`].

use crate::metric::{HistogramCells, Metric};
use crate::schema::{Domain, EventDescriptor, FieldDescriptor as F, Severity};

macro_rules! histogram {
    ($(#[$meta:meta])* $vis:vis $name:ident, $domain:expr, $id:expr, $label:expr, $unit:expr) => {
        $(#[$meta])*
        $vis static $name: Metric = Metric::histogram($domain, $id, $label, $unit, {
            static CELLS: HistogramCells = HistogramCells::new();
            &CELLS
        });
    };
}

pub mod runtime {
    use super::*;
    const D: Domain = Domain::RUNTIME;

    histogram!(
        /// CPU time of each profiled flush that ran at least one stage.
        pub FRAME_CPU_NS, D, 1, "runtime.flush.cpu", "ns");
    /// Profiled flushes that ran at least one stage (failed ones included).
    pub static FLUSHES: Metric = Metric::counter(D, 2, "runtime.flushes", "count");
    histogram!(pub FLUSH_PASSES, D, 3, "runtime.flush.passes", "count");
    /// Flushes that hit the pass limit. The host reports the failure itself
    /// (rate-limited); this counts every occurrence.
    pub static FLUSH_DID_NOT_SETTLE: Metric =
        Metric::counter(D, 4, "runtime.flush.did_not_settle", "count");
    /// Flushes that failed in style resolution or text / layout.
    pub static FLUSH_FAILED: Metric = Metric::counter(D, 5, "runtime.flush.failed", "count");

    histogram!(pub STAGE_INPUT_NS, D, 10, "runtime.stage.input", "ns");
    histogram!(pub STAGE_RECONCILE_NS, D, 11, "runtime.stage.reconcile", "ns");
    histogram!(pub STAGE_STYLE_NS, D, 12, "runtime.stage.style", "ns");
    histogram!(pub STAGE_TEXT_SHAPE_NS, D, 13, "runtime.stage.text_shape", "ns");
    histogram!(pub STAGE_LAYOUT_NS, D, 14, "runtime.stage.layout", "ns");
    histogram!(pub STAGE_HIT_TEST_NS, D, 15, "runtime.stage.hit_test", "ns");
    histogram!(pub STAGE_ACCESSIBILITY_NS, D, 16, "runtime.stage.accessibility", "ns");
    histogram!(pub STAGE_ANIMATION_NS, D, 17, "runtime.stage.animation", "ns");
    histogram!(pub STAGE_EXTRACT_NS, D, 18, "runtime.stage.extract", "ns");

    // Canonical input and presentation mapping counters. IDs are append-only;
    // source/device detail remains in the endpoint/router snapshots so the
    // high-frequency path does not allocate diagnostic events.
    pub static INPUT_EVENTS: Metric = Metric::counter(D, 20, "runtime.input.events", "count");
    pub static INPUT_COALESCED: Metric = Metric::counter(D, 21, "runtime.input.coalesced", "count");
    pub static INPUT_PAYLOAD_BYTES: Metric =
        Metric::counter(D, 23, "runtime.input.payload_bytes", "bytes");
    pub static INPUT_ROUTE_REJECTED: Metric =
        Metric::counter(D, 24, "runtime.input.route_rejected", "count");
    pub static INPUT_CLIP_REJECTIONS: Metric =
        Metric::counter(D, 26, "runtime.input.clip_rejections", "count");
    histogram!(pub INPUT_ROUTE_NS, D, 31, "runtime.input.route", "ns");
    /// Hit-test queries input routing made (one per uncaptured pointer
    /// event at most), read from the world's own count.
    pub static INPUT_HIT_TESTS: Metric = Metric::counter(D, 33, "runtime.input.hit_tests", "count");
    pub static INPUT_ROUTED_DISPATCHES: Metric =
        Metric::counter(D, 34, "runtime.input.routed_dispatches", "count");
    pub static INPUT_FOCUS_CHANGES: Metric =
        Metric::counter(D, 35, "runtime.input.focus_changes", "count");
    pub static INPUT_CAPTURE_CHANGES: Metric =
        Metric::counter(D, 36, "runtime.input.capture_changes", "count");
    pub static INPUT_HOVER_CHANGES: Metric =
        Metric::counter(D, 37, "runtime.input.hover_changes", "count");
    // Retired with the host-request queue, the coordinate bridge, a routing
    // cache that never existed and the endpoint's own validation (the router
    // counts rejections in 24); the IDs stay taken:
    // 22 runtime.input.stale_dropped, 25 runtime.input.inverse_recomputes, 27 runtime.host_requests.enqueued,
    // 28 runtime.host_requests.rejected, 29 runtime.host_requests.drained,
    // 30 runtime.host_requests.stale, 32 runtime.input.mapping,
    // 38 runtime.input.routing_cache_hits, 39 runtime.input.routing_cache_misses.

    /// Reactive view flushes that ran at least one effect.
    pub static REACTIVE_FLUSHES: Metric =
        Metric::counter(D, 40, "runtime.reactive.flushes", "count");
    /// Signal writes (`set` / `update`), whether or not anything subscribed.
    pub static REACTIVE_SIGNAL_WRITES: Metric =
        Metric::counter(D, 41, "runtime.reactive.signal_writes", "count");
    /// Effects a flush ran: node bindings, structural blocks and watchers.
    pub static REACTIVE_EFFECTS_RUN: Metric =
        Metric::counter(D, 42, "runtime.reactive.effects_run", "count");
    /// Nodes whose bound fields actually changed and were projected.
    pub static REACTIVE_NODES_PATCHED: Metric =
        Metric::counter(D, 43, "runtime.reactive.nodes_patched", "count");
    /// Mutation commits the binding batches made (one per flush round).
    pub static REACTIVE_COMMITS: Metric =
        Metric::counter(D, 44, "runtime.reactive.commits", "count");
    histogram!(pub REACTIVE_FLUSH_NS, D, 45, "runtime.reactive.flush", "ns");

    /// Fault: effects kept re-queueing each other past the round limit; the
    /// rest of the queue was dropped.
    pub static REACTIVE_DID_NOT_SETTLE: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "runtime.reactive.did_not_settle",
        Severity::Error,
        &[F::u64("rounds")],
    );
    /// Fault: a signal was read or written after its scope was disposed. The
    /// message names where it was created.
    pub static REACTIVE_DISPOSED_ACCESS: EventDescriptor = EventDescriptor::new(
        D,
        2,
        "runtime.reactive.disposed_access",
        Severity::Error,
        &[],
    );
    /// Fault (debug builds): a binding the `.vue` compiler declared static
    /// read a signal outside its declared dependencies. The message names
    /// the template site.
    pub static REACTIVE_STATIC_DEPS_MISMATCH: EventDescriptor = EventDescriptor::new(
        D,
        3,
        "runtime.reactive.static_deps_mismatch",
        Severity::Error,
        &[],
    );
    /// Fault: a view failed (`Err`) with no error boundary above it to show
    /// it. The message carries the error.
    pub static VIEW_ERROR_UNHANDLED: EventDescriptor =
        EventDescriptor::new(D, 4, "runtime.view.error_unhandled", Severity::Error, &[]);
}

pub mod layout {
    use super::*;
    const D: Domain = Domain::LAYOUT;

    histogram!(pub PASS_NS, D, 1, "layout.pass", "ns");
    pub static INVOCATIONS: Metric = Metric::counter(D, 2, "layout.invocations", "count");
    pub static FULL_INVOCATIONS: Metric = Metric::counter(D, 3, "layout.full_invocations", "count");
    histogram!(pub DIRTY_ROOTS, D, 4, "layout.dirty_roots", "count");
    histogram!(
        /// Boxes the layout engine emitted for the pass.
        pub BOXES_LAID_OUT, D, 5, "layout.boxes_laid_out", "count");
}

pub mod text {
    use super::*;
    const D: Domain = Domain::TEXT;

    pub static SHAPE_HITS: Metric = Metric::counter(D, 1, "text.shape.hits", "count");
    pub static SHAPE_MISSES: Metric = Metric::counter(D, 2, "text.shape.misses", "count");
    pub static LAYOUT_HITS: Metric = Metric::counter(D, 3, "text.layout.hits", "count");
    pub static LAYOUT_MISSES: Metric = Metric::counter(D, 4, "text.layout.misses", "count");
    pub static GLYPHS_RESOLVED: Metric = Metric::counter(D, 5, "text.glyphs_resolved", "count");
    pub static ATLAS_EVICTIONS: Metric = Metric::counter(D, 6, "text.atlas.evictions", "count");
    /// Page requests the byte budget refused (the event is throttled).
    pub static ATLAS_BUDGET_REFUSALS: Metric =
        Metric::counter(D, 7, "text.atlas.budget_refusals", "count");
    /// Frames that left paragraphs undrawn because a target's glyphs did not
    /// fit the device's storage binding (the event is throttled).
    pub static INSTANCE_LIMIT_FRAMES: Metric =
        Metric::counter(D, 8, "text.instances.limit_frames", "count");

    pub static ATLAS_PAGE_OPENED: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "text.atlas.page_opened",
        Severity::Debug,
        &[F::u64("edge"), F::u64("bytes"), F::u64("atlas_bytes")],
    );
    pub static ATLAS_BUDGET_EXHAUSTED: EventDescriptor = EventDescriptor::new(
        D,
        2,
        "text.atlas.budget_exhausted",
        Severity::Warn,
        &[F::u64("bytes"), F::u64("budget")],
    );
    pub static ATLAS_COMPACTED: EventDescriptor = EventDescriptor::new(
        D,
        3,
        "text.atlas.compacted",
        Severity::Debug,
        &[F::u64("pages")],
    );
    /// `slots` the frame's paragraphs asked for, `limit` the device allows,
    /// `skipped` the paragraphs left undrawn — the largest first.
    pub static INSTANCE_LIMIT_EXCEEDED: EventDescriptor = EventDescriptor::new(
        D,
        4,
        "text.instances.limit_exceeded",
        Severity::Warn,
        &[F::u64("slots"), F::u64("limit"), F::u64("skipped")],
    );
}

pub mod gpu {
    use super::*;
    const D: Domain = Domain::GPU;

    pub static UPLOAD_BYTES: Metric = Metric::counter(D, 1, "gpu.upload_bytes", "bytes");
    histogram!(pub DRAW_CALLS, D, 2, "gpu.draw_calls", "count");
    pub static FRAMES_PRESENTED: Metric = Metric::counter(D, 3, "gpu.frames_presented", "count");
    pub static FRAMES_SKIPPED: Metric = Metric::counter(D, 4, "gpu.frames_skipped", "count");
    histogram!(
        /// `queue.submit` wall time.
        pub SUBMIT_NS, D, 5, "gpu.submit", "ns");
    pub static BUFFER_REALLOCATIONS: Metric =
        Metric::counter(D, 6, "gpu.buffer_reallocations", "count");
    pub static SURFACE_OUTDATED: Metric = Metric::counter(D, 7, "gpu.surface.outdated", "count");
    pub static SURFACE_LOST: Metric = Metric::counter(D, 8, "gpu.surface.lost", "count");
    pub static SURFACE_TIMEOUT: Metric = Metric::counter(D, 9, "gpu.surface.timeout", "count");
    histogram!(
        /// Submit → the host observing the work complete. An upper bound on
        /// GPU time: completion is noticed at the next redraw's poll, and a
        /// sample is kept only when that poll came within 50 ms of the one
        /// before (after idle time it would measure the idle, not the GPU). Exact GPU time needs
        /// timestamp queries, which Metal cannot write inside an encoder.
        pub COMPLETION_NS, D, 10, "gpu.completion", "ns"
    );
    /// A `FrameContext` dropped without being submitted.
    pub static FRAMES_DISCARDED: Metric = Metric::counter(D, 11, "gpu.frames_discarded", "count");
    pub static TRANSIENT_POOL_HITS: Metric =
        Metric::counter(D, 12, "gpu.transient_pool_hits", "count");
    pub static TRANSIENT_POOL_MISSES: Metric =
        Metric::counter(D, 13, "gpu.transient_pool_misses", "count");
    pub static PIPELINE_REGISTRY_HITS: Metric =
        Metric::counter(D, 14, "gpu.pipeline_registry_hits", "count");
    pub static PIPELINE_REGISTRY_MISSES: Metric =
        Metric::counter(D, 15, "gpu.pipeline_registry_misses", "count");
    pub static FRAME_SLOT_STALLS: Metric = Metric::counter(D, 16, "gpu.frame_slot_stalls", "count");
    pub static RETIRED_RESOURCES: Metric = Metric::counter(D, 17, "gpu.retired_resources", "count");
    /// Retired: the texture realization cache it counted was removed. The
    /// id stays reserved.
    pub static REALIZATION_HITS: Metric = Metric::counter(D, 18, "gpu.realization_hits", "count");
    pub static REALIZATION_MISSES: Metric =
        Metric::counter(D, 19, "gpu.realization_misses", "count");
    /// Writes renderers queued through frame uploads.
    pub static UPLOAD_WRITES: Metric = Metric::counter(D, 20, "gpu.upload_writes", "count");
    /// Copy commands those writes became after merging adjacent ranges.
    pub static UPLOAD_COPIES: Metric = Metric::counter(D, 21, "gpu.upload_copies", "count");
    /// Upload command buffers submitted: at most one per frame submission.
    pub static UPLOAD_FLUSHES: Metric = Metric::counter(D, 22, "gpu.upload_flushes", "count");
    /// Staging chunks the upload ring had to create.
    pub static UPLOAD_RING_ALLOCATIONS: Metric =
        Metric::counter(D, 23, "gpu.upload_ring_allocations", "count");
    /// Flushes that waited for an in-flight chunk because the ring was full.
    pub static UPLOAD_RING_WAITS: Metric = Metric::counter(D, 24, "gpu.upload_ring_waits", "count");
    /// `begin_frame` calls that blocked on the oldest in-flight frame.
    pub static FRAME_SLOT_WAITS: Metric = Metric::counter(D, 25, "gpu.frame_slot_waits", "count");
    /// Frame binding observed a live exchange epoch gap and retained the last
    /// presented frame until its replacement was published.
    pub static FRAME_BINDING_REPLACEMENT_GAPS: Metric =
        Metric::counter(D, 30, "gpu.frame_binding.replacement_gaps", "count");
    /// A frame binding had to show its transparent placeholder.
    pub static FRAME_BINDING_PLACEHOLDER_BINDS: Metric =
        Metric::counter(D, 31, "gpu.frame_binding.placeholder_binds", "count");
    /// A frame binding completed a replacement swap.
    pub static FRAME_BINDING_REPLACEMENTS: Metric =
        Metric::counter(D, 32, "gpu.frame_binding.replacements", "count");
    /// A frame was rejected by the binding's acceptance policy.
    pub static FRAME_BINDING_REJECTIONS: Metric =
        Metric::counter(D, 33, "gpu.frame_binding.rejections", "count");

    // Presentation/output boundary (Issue #242). IDs are append-only; these
    // counters keep fallback and extra work observable without allocating
    // diagnostic strings on the frame path.
    pub static OUTPUT_TARGET_PLAN_REBUILDS: Metric =
        Metric::counter(D, 40, "gpu.output.target_plan_rebuilds", "count");
    pub static OUTPUT_EXTRA_PASSES: Metric =
        Metric::counter(D, 41, "gpu.output.extra_passes", "count");
    pub static OUTPUT_GPU_COPIES: Metric = Metric::counter(D, 42, "gpu.output.gpu_copies", "count");
    pub static OUTPUT_CPU_READBACKS: Metric =
        Metric::counter(D, 43, "gpu.output.cpu_readbacks", "count");
    pub static OUTPUT_TARGET_RECREATES: Metric =
        Metric::counter(D, 44, "gpu.output.target_recreates", "count");
    pub static OUTPUT_CONTENT_REVISIONS: Metric =
        Metric::counter(D, 45, "gpu.output.content_revisions", "count");
    pub static OUTPUT_IDLE_REUSE_FRAMES: Metric =
        Metric::counter(D, 46, "gpu.output.idle_reuse_frames", "count");
    pub static OUTPUT_RESOLVES: Metric = Metric::counter(D, 47, "gpu.output.resolves", "count");
    pub static OUTPUT_GPU_COPY_BYTES: Metric =
        Metric::counter(D, 48, "gpu.output.gpu_copy_bytes", "bytes");
    pub static OUTPUT_GPU_CONVERT_PASSES: Metric =
        Metric::counter(D, 49, "gpu.output.gpu_convert_passes", "count");
    pub static OUTPUT_CPU_FALLBACK_FRAMES: Metric =
        Metric::counter(D, 50, "gpu.output.cpu_fallback_frames", "count");
    pub static OUTPUT_CANONICAL_TARGETS: Metric =
        Metric::counter(D, 51, "gpu.output.canonical_target_count", "count");
    pub static OUTPUT_CONSUMERS: Metric =
        Metric::counter(D, 52, "gpu.output.consumer_count", "count");

    /// A low-volume frame-binding transition. `outcome`: 1 a live frame
    /// replaced the placeholder, 2 the placeholder replaced a frame, 3
    /// explicit rejection. Frame-to-frame replacements are counted by
    /// `gpu.frame_binding.replacements`, not reported here. `exchange` and `sequence` identify
    /// the exchange and frame without exposing application epoch types.
    pub static FRAME_BINDING_TRANSITION: EventDescriptor = EventDescriptor::new(
        D,
        20,
        "gpu.frame_binding.transition",
        Severity::Info,
        &[F::u64("exchange"), F::u64("sequence"), F::u64("outcome")],
    );

    pub static SURFACE_LOST_EVENT: EventDescriptor =
        EventDescriptor::new(D, 1, "gpu.surface_lost", Severity::Warn, &[]);
    /// Fault. `reason`: 0 = unknown, 1 = destroyed.
    pub static DEVICE_LOST: EventDescriptor = EventDescriptor::new(
        D,
        2,
        "gpu.device_lost",
        Severity::Error,
        &[F::u64("reason")],
    );
    pub static DEVICE_RECOVERED: EventDescriptor =
        EventDescriptor::new(D, 3, "gpu.device_recovered", Severity::Info, &[]);
    /// Fault.
    pub static DEVICE_RECOVERY_FAILED: EventDescriptor =
        EventDescriptor::new(D, 4, "gpu.device_recovery_failed", Severity::Error, &[]);
    pub static SURFACE_SUSPENDED: EventDescriptor = EventDescriptor::new(
        D,
        5,
        "gpu.surface_suspended",
        Severity::Warn,
        &[F::u64("window")],
    );
    /// A frame a painter had recorded retained writes into was discarded;
    /// the painter rebuilds that target. Once per painter: `target` is the
    /// first target it happened to.
    pub static RETAINED_FRAME_DISCARDED: EventDescriptor = EventDescriptor::new(
        D,
        6,
        "gpu.retained_frame_discarded",
        Severity::Warn,
        &[F::u64("target")],
    );
    /// Fault: every frame slot is held by a recording that was never
    /// submitted, so waiting could never end. The new frame runs without a
    /// slot.
    pub static FRAME_SLOTS_EXHAUSTED: EventDescriptor =
        EventDescriptor::new(D, 7, "gpu.frame_slots_exhausted", Severity::Error, &[]);
}

pub mod window {
    use super::*;
    const D: Domain = Domain::WINDOW;

    pub static RESIZES: Metric = Metric::counter(D, 1, "window.resizes", "count");
    /// Shadow-body projections whose geometry and visibility were unchanged,
    /// so the native companion was intentionally left alone.
    pub static SHADOW_SYNC_SKIPPED: Metric =
        Metric::counter(D, 10, "window.shadow.sync_skipped", "count");
    /// Shadow-body projections that reached the platform companion update.
    pub static SHADOW_SYNC_APPLIED: Metric =
        Metric::counter(D, 11, "window.shadow.sync_applied", "count");

    pub static OPENED: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "window.opened",
        Severity::Info,
        &[F::u64("window"), F::u64("width"), F::u64("height")],
    );
    pub static CLOSED: EventDescriptor =
        EventDescriptor::new(D, 2, "window.closed", Severity::Info, &[F::u64("window")]);
    pub static SCALE_FACTOR_CHANGED: EventDescriptor = EventDescriptor::new(
        D,
        3,
        "window.scale_factor_changed",
        Severity::Info,
        &[F::u64("window"), F::f64("scale")],
    );
    pub static OCCLUDED: EventDescriptor = EventDescriptor::new(
        D,
        4,
        "window.occluded",
        Severity::Debug,
        &[F::u64("window"), F::bool("occluded")],
    );
}

pub mod host {
    use super::*;
    const D: Domain = Domain::HOST;

    histogram!(
        /// Wall time of one presented redraw: program messages, document
        /// flush, paint, submit, and present (which can wait for vsync).
        /// Not GPU time.
        pub REDRAW_NS, D, 1, "host.redraw", "ns");
    /// Every `HostFailure`, including the ones rate-limited out of the log.
    pub static FAILURES: Metric = Metric::counter(D, 2, "host.failures", "count");
    /// Frame periods a `FrameDemand::Continuous` window missed entirely.
    pub static FRAMES_DROPPED: Metric = Metric::counter(D, 3, "host.frames_dropped", "count");
    histogram!(
        /// Program messages waiting when a window drained its queue (only
        /// non-empty drains are sampled).
        pub MESSAGE_QUEUE_DEPTH, D, 4, "host.message_queue_depth", "count"
    );
    /// Due frames the host served itself because the paint it requested for
    /// them did not arrive within one frame period (Windows only).
    pub static FRAMES_SERVED_WITHOUT_PAINT: Metric =
        Metric::counter(D, 5, "host.frames_served_without_paint", "count");

    /// Fault, at most once per second per `kind` (see [`FAILURES`] for the
    /// full count). `kind` is the `HostFailure` variant's stable code.
    pub static FAILURE: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "host.failure",
        Severity::Error,
        &[F::u64("window"), F::u64("kind")],
    );
    /// Fault: the host could not start or its event loop failed.
    pub static RUN_FAILED: EventDescriptor =
        EventDescriptor::new(D, 2, "host.run_failed", Severity::Fatal, &[]);
    pub static EVENT_LOOP_EXITED: EventDescriptor =
        EventDescriptor::new(D, 3, "host.event_loop_exited", Severity::Info, &[]);

    /// A startup milestone (Issue #225). `phase`: 0 entry, 1 splash
    /// committed, 2 ui ready, 3 takeover requested, 4 takeover frame
    /// presented, 5 handoff completed, 6 splash released. `elapsed_ns` is
    /// from the host's entry, not from process creation; phase 1 is the CPU
    /// side of the compositor request, not a time the logo was on screen.
    pub static STARTUP_PHASE: EventDescriptor = EventDescriptor::new(
        D,
        4,
        "host.startup_phase",
        Severity::Info,
        &[F::u64("phase"), F::u64("elapsed_ns")],
    );
    /// What the platform did with the Early Splash request. `outcome`:
    /// 0 animated, 1 static, 2 skipped, 3 failed (`SplashOutcome::code`).
    pub static SPLASH_OUTCOME: EventDescriptor = EventDescriptor::new(
        D,
        5,
        "host.splash_outcome",
        Severity::Info,
        &[F::u64("outcome")],
    );
    /// Fault: startup ended before `UiReady` (no device, no window, ...).
    pub static STARTUP_FAILED: EventDescriptor =
        EventDescriptor::new(D, 6, "host.startup_failed", Severity::Error, &[]);

    /// Longest single event-loop callback between host entry and handoff.
    pub static STARTUP_LONGEST_BLOCK_NS: Metric =
        Metric::gauge(D, 5, "host.startup.longest_block", "ns");
    /// Reading a packaged Early Splash logo out of the package's
    /// `early-splash` pack, on the event thread before the window is shown.
    pub static STARTUP_SPLASH_LOGO_READ_NS: Metric =
        Metric::gauge(D, 6, "host.startup.splash_logo_read", "ns");
    /// Selecting the adapter and creating the device on the startup thread.
    pub static STARTUP_DEVICE_REQUEST_NS: Metric =
        Metric::gauge(D, 7, "host.startup.device_request", "ns");
    /// Building the primary scene painter, its pipeline compiles included,
    /// on the startup thread once the device exists.
    pub static STARTUP_PAINTER_BUILD_NS: Metric =
        Metric::gauge(D, 8, "host.startup.painter_build", "ns");
    /// Fault: a packaged Early Splash logo could not be read; the application
    /// starts without a splash. `code` is `SplashPackageError::code`; the
    /// message names the URL and the reason.
    pub static SPLASH_LOGO_FAILED: EventDescriptor = EventDescriptor::new(
        D,
        7,
        "host.splash_logo_failed",
        Severity::Warn,
        &[F::u64("code")],
    );
}

pub mod persistence {
    use super::*;
    const D: Domain = Domain::HOST;
    pub static PENDING_GENERATION: Metric =
        Metric::gauge(D, 20, "persistence.pending_generation", "generation");
    pub static WRITES_STARTED: Metric =
        Metric::counter(D, 21, "persistence.writes_started", "count");
    pub static WRITES_COMPLETED: Metric =
        Metric::counter(D, 22, "persistence.writes_completed", "count");
    pub static WRITES_COALESCED: Metric =
        Metric::counter(D, 23, "persistence.writes_coalesced", "count");
    pub static BYTES_WRITTEN: Metric = Metric::counter(D, 24, "persistence.bytes_written", "bytes");
    pub static FLUSH_NS: Metric = Metric::histogram(D, 25, "persistence.flush", "ns", {
        static CELLS: HistogramCells = HistogramCells::new();
        &CELLS
    });
    pub static ENCODED_BYTES: Metric = Metric::counter(D, 31, "persistence.encoded_bytes", "bytes");
    pub static FAILURE: EventDescriptor =
        EventDescriptor::new(D, 20, "persistence.failure", Severity::Warn, &[]);
    /// A bounded flush gave up waiting; the write may still land later, or be
    /// lost if the process exits first.
    pub static FLUSH_TIMED_OUT: EventDescriptor =
        EventDescriptor::new(D, 21, "persistence.flush_timed_out", Severity::Warn, &[]);
    pub static RESTORE_HITS: Metric = Metric::counter(D, 26, "persistence.restore_hits", "count");
    pub static RESTORE_MISSES: Metric =
        Metric::counter(D, 27, "persistence.restore_misses", "count");
    pub static MIGRATIONS: Metric = Metric::counter(D, 28, "persistence.migrations", "count");
    pub static SCHEMA_MISMATCHES: Metric =
        Metric::counter(D, 29, "persistence.schema_mismatches", "count");
    pub static CORRUPTIONS: Metric = Metric::counter(D, 30, "persistence.corruptions", "count");
}

pub mod diagnostics {
    use super::*;
    const D: Domain = Domain::DIAGNOSTICS;

    /// Fault written by the panic hook; the message carries thread,
    /// location, and payload.
    pub static PANIC: EventDescriptor =
        EventDescriptor::new(D, 1, "diagnostics.panic", Severity::Fatal, &[]);
}

pub mod resource {
    use super::*;
    const D: Domain = Domain::RESOURCE;

    histogram!(
        /// Opening one pack: header, signature, TOC hash and authentication.
        pub PACK_MOUNT_NS, D, 1, "resource.pack.mount", "ns");
    histogram!(pub PACK_TOC_BYTES, D, 2, "resource.pack.toc_bytes", "bytes");
    histogram!(
        /// One packaged entry read end to end (I/O, authentication,
        /// decompression, hash). Reads happen on cache misses of the image,
        /// font and stylesheet loaders, never per frame.
        pub ENTRY_READ_NS, D, 3, "resource.entry.read", "ns");
    histogram!(pub ENTRY_AUTH_NS, D, 4, "resource.entry.auth", "ns");
    histogram!(pub ENTRY_DECOMPRESS_NS, D, 5, "resource.entry.decompress", "ns");
    pub static BYTES_READ: Metric = Metric::counter(D, 6, "resource.bytes_read", "bytes");
    pub static READS: Metric = Metric::counter(D, 7, "resource.reads", "count");
    pub static MISSES: Metric = Metric::counter(D, 8, "resource.misses", "count");
    /// Signature, hash or authentication failures: tampering or corruption.
    pub static INTEGRITY_FAILURES: Metric =
        Metric::counter(D, 9, "resource.integrity_failures", "count");

    pub static PACK_MOUNTED: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "resource.pack_mounted",
        Severity::Info,
        &[
            F::u64("entries"),
            F::u64("toc_bytes"),
            F::bool("encrypted"),
            F::bool("signed"),
        ],
    );
    /// Fault: a pack the manifest lists could not be opened. `code` is
    /// `nana_package::PackError::code`; the message names the pack.
    pub static PACK_OPEN_FAILED: EventDescriptor = EventDescriptor::new(
        D,
        2,
        "resource.pack_open_failed",
        Severity::Error,
        &[F::u64("code")],
    );
    /// Fault: an entry failed verification (at most once per pack).
    pub static INTEGRITY_FAILED: EventDescriptor = EventDescriptor::new(
        D,
        3,
        "resource.integrity_failed",
        Severity::Error,
        &[F::u64("code")],
    );
}

pub mod package {
    use super::*;
    const D: Domain = Domain::PACKAGE;

    histogram!(pub MANIFEST_READ_NS, D, 1, "package.manifest.read", "ns");

    pub static MANIFEST_LOADED: EventDescriptor = EventDescriptor::new(
        D,
        1,
        "package.manifest_loaded",
        Severity::Info,
        &[F::u64("bytes"), F::u64("packs"), F::bool("signed")],
    );
    /// An installed or portable application without a package manifest.
    pub static MANIFEST_MISSING: EventDescriptor =
        EventDescriptor::new(D, 2, "package.manifest_missing", Severity::Warn, &[]);
    /// Fault: `code` is `nana_package::ManifestError::code`.
    pub static MANIFEST_INVALID: EventDescriptor = EventDescriptor::new(
        D,
        3,
        "package.manifest_invalid",
        Severity::Error,
        &[F::u64("code")],
    );
    /// Fault: the running binary's identity differs from the manifest's.
    pub static IDENTITY_MISMATCH: EventDescriptor =
        EventDescriptor::new(D, 4, "package.identity_mismatch", Severity::Error, &[]);
}
