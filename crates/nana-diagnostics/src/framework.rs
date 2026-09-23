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
}

pub mod window {
    use super::*;
    const D: Domain = Domain::WINDOW;

    pub static RESIZES: Metric = Metric::counter(D, 1, "window.resizes", "count");

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
