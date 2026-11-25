# Diagnostics and Application Paths

NanaUI provides a unified diagnostics foundation (collection, lock-free buffering, background persistence, file rotation, crash snapshots, and offline export). The application semantics of diagnostics (which application events and metrics to define, what counts as an application fault, how to redact and report) are up to your application. The core design principle is: **Logging is lossy by design; rendering is not.** Logs may be dropped, but rendering must never stall or fail because of them.

Diagnostics live in the leaf crate `crates/nana-diagnostics`. It depends on no other NanaUI crate, and its cross-platform part uses only the standard library; on Windows it additionally depends on `windows-sys` (for the Win32 APIs needed for high-resolution timing, thread priority, and thread stack sampling during stalls), and on Unix on `libc` (thread stack sampling and symbol lookup during stalls). It is re-exported through `nana_ui::diagnostics`. Cross-platform application directory resolution lives in `ApplicationPaths` in `nana-ui-platform`. For a quick reference of runtime fault codes you commonly hit while writing UI, see [Diagnostic Codes](errors.md).

## Enabling diagnostics in an application

```rust
use nana_ui::{
    ApplicationIdentity, DiagnosticsConfig, NanaApplication, RuntimeApplication, WindowDescriptor,
};

NanaApplication::builder(ApplicationIdentity::new(
    "dev.nana.live",
    "NanaLive",
    env!("CARGO_PKG_VERSION"),
))
.diagnostics(DiagnosticsConfig::default())
.run::<RuntimeApplication<App>>(WindowDescriptor::new("NanaLive"))
```

- **Off by default**: if you do not call `.diagnostics(..)` (or use bare `run_runtime` directly), diagnostics stay off, and every probe in the framework degrades to a single `0.5 ns` `Relaxed` atomic load.
- **Lifecycle management**: when `builder.run(...)` returns, it performs the final drain and `sync_data` automatically. If the application owns its event loop (for example, an embedded host), hold the returned `ApplicationSession` with `let _session = builder.start();` until the process exits. Never write `let _ = builder.start();`: it drops the session immediately and shuts down the diagnostics thread.
- **Tolerant directory resolution**: when a container or unusual environment lacks `HOME` / `LOCALAPPDATA` and platform directory resolution fails, application startup is not blocked. A one-line notice is printed to `stderr`, `session.paths()` returns `None`, and diagnostics data stays only in the in-memory ring buffer.

## Defining application events and metrics

Domain numbers (`Domain`) `0x0000..=0x00FF` are reserved for the NanaUI framework. **Application-defined domains start at `0x0100` (`Domain::APPLICATION_MIN`).** Event and metric IDs are unique within their domain. Once written to `.nlog`, they become a persistent format contract: **you may only append new numbers; never reorder or reuse old ones.**

```rust
use nana_ui::diagnostics::{
    Domain, EventDescriptor, FieldDescriptor as F, HistogramCells, Metric, Severity,
    event, fault, metric, span,
};

const LIVE: Domain = Domain(0x0100);

static MODEL_LOADED: EventDescriptor = EventDescriptor::new(
    LIVE,
    1,
    "live.model_loaded",
    Severity::Info,
    &[F::u64("bytes"), F::f64("ms")],
);
static TRACKER_LOST: EventDescriptor = EventDescriptor::new(
    LIVE,
    2,
    "live.tracker_lost",
    Severity::Error,
    &[F::u64("camera")],
);

static TRACK_CELLS: HistogramCells = HistogramCells::new();
static TRACK_NS: Metric = Metric::histogram(LIVE, 1, "live.track", "ns", &TRACK_CELLS);
static FRAMES_OUT: Metric = Metric::counter(LIVE, 2, "live.frames_out", "count");

event!(MODEL_LOADED, bytes = size, ms = elapsed_ms);
fault!(TRACKER_LOST, camera = id; "camera {id} stopped: {reason}");
metric!(FRAMES_OUT);                 // counter +1
metric!(TRACK_NS, frame_time);       // a Duration is recorded as nanoseconds automatically
let _span = span!(TRACK_NS);         // records elapsed time into the histogram when the scope ends
```

Purpose and measured per-call cost of the four primitives (Apple M4, `--release`, measured with `nana-diagnostics-benchmark`):

| Macro | Use case | Per-call cost | Behavior |
| --- | --- | ---: | --- |
| `metric!` (Counter / Gauge) | High-frequency counts such as frames, bytes, and queue depth | `~1.6 ns` | Writes a static atomic; by default the worker takes and persists a snapshot every `10 s` (`metric_interval`) |
| `metric!` (Histogram) | Distributions of frame time and phase time | `~1.8 ns` | Same as above (note: many threads writing the same histogram at high frequency rises to `~160 ns` from cache-line contention; split the metric per thread) |
| `span!` | Timing a code block's scope | `~39 ns` | Reads the monotonic clock once on scope entry and once on exit, and writes the histogram automatically |
| `event!` | Low-frequency structured fact events (at most 4 typed fields) | `~24 ns` | Writes into a thread-local SPSC ring (field names are checked against the descriptor only in debug builds) |
| `fault!` | Critical errors and faults | One allocation, only with a formatted message | Also writes to a separate emergency ring and wakes the worker immediately to persist as soon as possible |

::: warning Frame path rule
High-frequency data must use `metric!`. **Never call `event!` per frame.** On frame paths, never use an external logging crate, `eprintln!`, string formatting, or any synchronous file I/O.
:::

## Runtime buffering model and persistence policy

```text
Producer thread ──event! / fault!──▶ thread-local SPSC ring buffer (when full, drops the newest event and increments the drop count)
                ──metric! / span!──▶ static atomics
                                   │
                                   │ background worker polls (default poll_interval = 50 ms); producer threads never wait
                                   ▼
                     nana-diagnostics worker (lowered thread priority)
                       ├─ Flight Recorder: always retains the last 60 s (flight_window) / 4 MiB (flight_bytes)
                       └─ batched sequential write buffer ──▶ one sequential disk write when 64 KiB (batch_bytes) accumulates or every 2 s (batch_interval)
```

A producer thread registers itself lazily the first time it records an event (cold path). Latency-sensitive real-time audio or render threads can register ahead of time by calling `nana_ui::diagnostics::register_thread()` at startup. If the disk is slow or a write fails, only the background worker is affected (it drops the current batch, records it in `bytes_discarded`, and retries with a new file using exponential backoff); the in-memory Flight Recorder keeps the recent events as usual.

The persistence level of the session log is controlled by `PersistMode` (the Flight Recorder always keeps every event at or above `min_severity` in memory, so a complete snapshot can be produced on a crash or an explicit export):

| `PersistMode` | Written to the session log (`.nlog`) | Default build profile |
| --- | --- | --- |
| `Off` | No regular session log; crash and explicit snapshots are still written | — |
| `Essential` | Events at `Warn` and above, all `fault!`s, periodic metric snapshots, session info, and clock sync markers | `release` / `dist` |
| `All` | Events and metrics at every level | `debug` |

## Log paths, rotation, and crash export

Log and crash snapshot directories are resolved by `ApplicationPaths` according to platform conventions (if `<exe dir>/runtime/manifest/portable` exists, the install is treated as portable, except inside a macOS `.app` bundle; when run directly from Cargo's `target/`, the layout is `Development`):

| Directory purpose | Windows | macOS | Linux | Portable |
| --- | --- | --- | --- | --- |
| Session logs (logs) | `%LOCALAPPDATA%\{id}\Logs` | `~/Library/Logs/{id}` | `$XDG_STATE_HOME/{id}/logs` | `<root>/data/logs` |
| Crash snapshots (crash) | `%LOCALAPPDATA%\{id}\Crash` | `~/Library/Logs/{id}/Crash` | `$XDG_STATE_HOME/{id}/crash` | `<root>/data/crash` |

- **Naming and rotation**: session logs are named `{app}-{YYYYMMDDTHHMMSSZ}-{pid}.nlog`. When a single file exceeds `max_file_bytes` (default `16 MiB`), it rotates to `.1.nlog`, `.2.nlog`, and so on, never overwriting an existing file.
- **Quota cleanup**: files are removed from oldest to newest by modification time until `max_files` (32), `max_total_bytes` (128 MiB), and `max_age` (14 days) are satisfied. Crash snapshots are limited separately by `max_crash_files` (16) and `max_crash_bytes` (64 MiB). Files modified by another process within the last `live_grace` (10 minutes) are protected from deletion, so logs of instances running in parallel are not removed by mistake.
- **Crash capture and offline viewing**:
  - **Rust panic**: installed ahead of the original panic hook by default (it runs before abort even in the `dist` profile with `panic = "abort"`). It records a `diagnostics.panic` fault and writes the Flight Recorder synchronously to the `crash` directory (a `-panic` snapshot).
  - **Native crash (Windows x64)**: by default (`DiagnosticsConfig::native_crash_handler`) `install` chains an unhandled-exception filter (`SetUnhandledExceptionFilter`: access violations, stack overflows and every other unhandled SEH exception) and a `SIGABRT` handler for C `abort()` in front of the existing handlers. The crashed thread neither allocates nor locks: it stores its registers and stack bounds in static slots, wakes a writer thread created at install time, and waits up to 30 s for it (after a stack overflow the crashed thread only has the few KiB std reserves with `SetThreadStackGuarantee`). The writer first writes the exception code and address into the run marker without allocating and syncs it, then walks and symbolizes the crashed thread's stack (the same code as stall reports; frames are `module+0xoffset` plus the symbol), records a `diagnostics.native_crash` fault (`code`, `address`, `os_thread`; `code` is `0x40000015` for C `abort()`), and writes a `-native-crash` snapshot synchronously. If the crash happened while holding the heap or loader lock, those last steps can block; the wait then times out, the process ends as usual, and the exception code stays in the run marker. `std::process::abort`, `handle_alloc_error`'s default and other `__fastfail`s go straight to the kernel and cannot be caught in-process; only the run marker covers them.
  - **Unclean exit (run marker)**: each session creates and locks `{app}-{stamp}-{pid}.running` in the `crash` directory (or `logs` when there is none), rewrites its last-alive time every metric interval, and deletes it on a clean `shutdown` (`DiagnosticsConfig::run_marker`). When the process is killed, loses power or crashes, the lock goes with the process and the file stays; the next session takes those markers over in `Diagnostics::start`, records a `diagnostics.unclean_exit` fault for each (`session_id`, `pid`, `start_unix_ns`, `last_write_unix_ns`; the message has the version, times, native exception code and that session's crash snapshot) and, before returning, writes an `-unclean-exit` snapshot synchronously (skipped when that session already left a panic or native-crash snapshot). A running instance holds its lock, so it is never reported; `Diagnostics::unclean_exits()` returns the sessions this start found.
  - **Last crash query**: `nana_diagnostics::last_crash()` (or `Diagnostics::last_crash()`, `last_crash_in(dir, app_id)`) returns the app's newest `-panic`, `-native-crash` or `-unclean-exit` snapshot in the `crash` directory with its kind (`CrashKind`) and write time; `stall`, `device-lost`, `export` and other snapshots are not crashes.
  - **Event loop stall**: when a diagnostics instance is installed, the Scene host calls `nana_diagnostics::stall::watch_current_thread()` at startup to watch the event loop thread, and marks each callback (`resumed` / `proxy_wake_up` / `window_event` / `about_to_wait`) with `stall::busy(Callback)`. Waiting for events between callbacks counts as idle and is not timed. When a single callback has not returned after `STALL_THRESHOLD` (5 s, the limit at which Windows declares "Not Responding"), a background watchdog thread records a `diagnostics.main_thread_stalled` fault and writes a `stall` snapshot. Its fields are `elapsed_ms`, `report` (which report this is for the same stall), and `cpu_permille` (the stalled thread's share of one core during the stall, in permille: near 1000 for a busy wait, near 0 for a blocking wait; `u64::MAX` when the platform cannot provide it). The message names the callback. On Windows (x64, ARM64), macOS / iOS and Linux / Android (x86_64, aarch64) it attaches three call stack samples taken 200 ms apart (identical samples are marked "same as", meaning stuck waiting; differing samples mean it is looping). Each frame is formatted as `module+0xoffset`, with a symbol name added when the machine has one: on Windows a PDB next to the module or the DLL export table, on Apple `dladdr`, on Linux `dladdr` plus the module file's own `.symtab`. Rust symbols are demangled into readable paths (v0 and legacy mangling; function pointers, `dyn` and const generics stay mangled). When a release package ships without symbols, resolve the offsets offline with the same version's PDB, `atos` or `addr2line`. While the stall continues, one more single sample is reported at 30 s and again at 120 s. When the callback finally returns, `diagnostics.main_thread_recovered` (`stalled_ms`) is recorded. How each platform samples: Windows and Apple (Mach `thread_suspend`) copy the registers and stack at the instant the thread is suspended; on Linux the watchdog sends the watched thread one real-time signal, and its handler (async-signal-safe, calling only `gettid`) copies the registers and stack into a buffer the watchdog allocated beforehand. None of them allocates, locks, or looks up unwind tables while copying. Unwinding happens on the copy and stops at the first frame it cannot follow inside the copy: Windows x64 follows the unwind tables; Windows ARM64, Apple and Linux follow the frame-record chain (the Windows ARM64 and Apple ABIs guarantee it; Linux aarch64 keeps it in non-leaf functions, and Linux x86_64 has it only when built with `-C force-frame-pointers=yes`, otherwise a sample keeps the frames before the chain breaks). The Linux signal handler is installed on the first sample and only while that signal has no other handler, with `SA_RESTART`; interrupted calls that never restart, such as `poll` or `nanosleep`, return `EINTR`. Thread CPU time comes from `GetThreadTimes` on Windows, `thread_info` on Apple and the thread CPU clock on Linux.
  - **GPU device loss**: the first time `nana-gpu` hears of a loss (WGPU's device-lost callback, or an embedded host's `EmbeddedRuntime::notify_device_lost`), it records the `gpu.device_lost` fault on that thread and writes the `device-lost` snapshot synchronously (`nana_diagnostics::fault_with_snapshot`) before `GpuContext::is_lost()` turns true; no background thread is involved. WGPU hands device-loss errors only to the lost callback; other uncaptured errors (validation, out of memory, internal) go to WGPU's default handler, which panics, and the panic hook records them synchronously.
  - **Explicit export and command-line reading**: an application can call `Diagnostics::snapshot_blocking(reason, timeout)` or `export_package(dest, &PackageOptions)` to export a diagnostics directory containing `.nlog`, `.txt`, `.jsonl`, and `manifest.json`. On the command line, view it directly with `cargo run -p nana-diagnostics --bin nana-nlog -- text <file.nlog>` (or `json`; add `--redact-home` to redact the user's home directory as `~`).

## The `.nlog` binary format and built-in framework probes

`.nlog` (schema v1) consists of the file header `"NANALOG\0"` + a little-endian version number + a sequence of chunks, each with a `CRC32` checksum. The first chunk, `Header`, records the session ID, application identity, operating system, architecture, and the starting point of the platform monotonic clock that Rust's `Instant` uses (macOS `CLOCK_UPTIME_RAW`, Linux/Android `CLOCK_MONOTONIC`, Windows `QPC`). `EventSchema` and `MetricSchema` chunks describe every name, field, and unit inside the file itself, so reading tools do not depend on the application binary. A `ClockSync` chunk records the monotonic clock against the wall clock once a minute to bridge system sleep. When a file is truncated or a chunk is corrupt, the reader stops safely and returns everything that passed validation before that point (`NlogFile::truncated`).

Built-in framework probes are spread across 9 subdomains in `nana_diagnostics::framework` (errors that may recur every frame are only counted in the Runtime layer, and the Host layer records the fault rate-limited to at most one per second so they cannot flood the log):

| Framework domain | Core metrics and events |
| --- | --- |
| `Runtime` | Total CPU time per `flush` and 9 phase histograms (reusing `FrameProfiler`), `flush` count and rounds, style/hit-test/extract counters (`runtime.style.nodes_resolved`, `runtime.hit_test.nodes_rebuilt`, `runtime.extract.nodes_{extracted,changed}`), one-shot `runtime.snippet.transform_unavailable` when `snippet-transforms` is disabled, `runtime.paint.filter_depth_exceeded` once per node whose non-identity filter is a fifth on its ancestor chain and stays the identity, and counts of non-convergence and style/text layout failures |
| `Input` | Events received, coalesces, payload bytes, routing rejections, clip rejections, routing time, hit queries, and focus/capture/hover changes (`runtime.input.*`, IDs 20–37), plus modal focus trap enter/exit/active and directional navigation query/candidate counts (`runtime.focus.*`, IDs 59–63) |
| `Layout` | Layout time, call count, full-tree layout count, dirty root count, and measure/placement node counts (`layout.measure_nodes`, `layout.placement_nodes`); for Dynamic Layout, row solves, envelope rebuilds, and budget fallbacks |
| `Text` | Shaping and layout cache hits/misses, glyph resolutions, glyph atlas page opens and total bytes, budget rejection events (rate-limited to one per 10 s), and atlas compaction and eviction counts |
| `GPU` | Submit time, an upper bound on GPU completion time (a non-blocking poll at the start of each redraw; samples with more than 50 ms of idle time are discarded automatically), upload bytes, draw calls, buffer reallocations, presents/skipped frames, Surface errors, device loss and recovery, frame discards (`gpu.frames_discarded` / `gpu.retained_frame_discarded`), precompiled MSL decompression metrics (`gpu.shader.msl_decompressions`, `gpu.shader.msl_decompress_ns`, `gpu.shader.msl_decompressed_bytes`), shader pipeline fallback/build errors (`gpu.shader.precompiled_fallback` / `gpu.shader.build_failed`), and custom material rejections (`gpu.material.rejected`); the adapter and driver info are registered in the session info |
| `Window` | Window open (including physical size), close, DPI scale factor changes, occlusion, resize counts, and cross-platform color sampler failures (`window.color_sampler.failed`) |
| `Resource` | Resource pack (`.nrpack`) mount time and TOC bytes, per-read time and bytes for packed resource reads/authentication/decompression, miss and integrity check failure faults, plus `resource.fetch_unavailable`, `resource.fetch_failed`, `resource.image_codec_unavailable`, and `resource.icu_unavailable` when optional fetch, codec, or `icu` features are disabled |
| `Package` | `package.json` manifest read time, loaded/missing events, and invalid manifest and identity mismatch faults |
| `Host` | Wall-clock total time per redraw (including message handling, flush, painting, submit, and present), frame periods missed by `Continuous` windows, message queue drain depth, startup metrics (`host.startup.*`), rate-limited `HostFailure` faults, and event loop exit |
