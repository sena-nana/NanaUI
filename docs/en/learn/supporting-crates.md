---
title: Supporting crates at a glance
---

# Supporting crates at a glance

Beyond the core rendering mainline (`core → runtime → scene → nana-ui`), the workspace holds a set of supporting crates with clear boundaries: the chart model, a zero-I/O diagnostics foundation, SVG rasterization, application icons, a vector icon catalog, resource packaging (runtime read side and build-time write side), development-time hot reload, and headless diagnostic sessions (devtools). None of them holds a retained tree or starts a second render pipeline; each guards one orthogonal responsibility. This chapter takes apart the internal structure and source anchors of these 9 supporting crates one by one, and closes with a map of the repository's surrounding ecosystem (`examples/`, `platform/android`, `tools/css-parity`, `perf/`).

## Responsibilities and boundaries at a glance

| Crate | Role in one line | Key architecture boundary |
| --- | --- | --- |
| `nana-ui-charts` | Chart model: ECharts-shaped `ChartOption`, scales, layout, hit testing and GPU mark arrays | Depends only on `nana-ui-core` + `bytemuck`; the `Chart` component lives in `runtime`, the WGSL pipelines in `nana-ui` |
| `nana-ui-diagram` | Mermaid diagrams: parsing, layered layout and orthogonal routing, theme-neutral vector display lists | Depends on `nana-ui-core` and the scales from `nana-ui-charts`; pure CPU, measurement supplied by the caller; the Markdown drawing entry lives in `runtime` (`rich-text`), tessellation goes through the `scene` custom-paint path |
| `nana-diagnostics` | `event!` / `metric!` / `fault!` / `span!` macros and static descriptors | On production threads: zero file I/O, zero string formatting, zero global locks, zero heap allocation after first registration |
| `nana-svg-raster` | CPU-side SVG rasterization (`resvg → pixmap`) | A pure bitmap rasterization tool: not a painter, and it keeps no SVG DOM |
| `nana-app-icon` | Default application icon, PNG/ICO/ICNS encoding and Windows PE resource embedding | Only icon encoding and embedding; directory packaging layout belongs to `nana-packager` |
| `nana-icons-tabler` | Auto-generated Tabler vector icon constant table (about 323k lines) | A script-generated artifact, not a workspace member; unreferenced icons are dropped automatically by the linker |
| `nana-package` | **Runtime read-only side** of the packaging contract (parses `package.json` and `.nrpack`) | Read-only logic and pure-Rust decoders only; no packaging or key-generation code |
| `nana-packager` | **Build-time write side** of the packaging contract, as a CLI | Turns an already compiled executable, the resource tree and `nana-package.toml` into a deliverable; never shipped with the application |
| `nana-ui-dev` | Shared development-time hot-reload foundation (file watching, debouncing and safe reads) | Carries `#[cfg(not(debug_assertions))] compile_error!` at the top and never enters release/dist builds |
| `nana-ui-devtools` | Development-time headless Agent automation sessions and offscreen snapshots | Contains no end-user product UI; CPU readback is confined to the offscreen snapshots here |

## Internal structure and source anchors

### 1. `nana-ui-charts`: CPU solves the geometry, GPU interpolates the vertices

`crates/nana-ui-charts/src/lib.rs` (27 lines) splits charts into two halves: “CPU-side layout and hit testing” and “GPU-side WGSL mark arrays”:

- **ECharts-shaped pure Rust configuration (`option.rs`, 1,424 lines)**: covers `grid`, `xAxis` / `yAxis`, `series` (line, bar, pie/donut, scatter, radar, gauge), `tooltip`, `legend`, `dataZoom`. Sizes support `Length::{Px, Percent}`, colors support `ChartColor::{Role(SemanticColorRole), Palette, Rgba}` (`:35`), and numeric and time labels are supplied by the application through `ValueFormatter` (`:45`) and `TimeFormatter` (`:71`).
- **GPU mark arrays strictly aligned with WGSL (`marks.rs`, 307 lines)**: the header comment states outright that this memory layout is the binary contract for `scene_paint/shader/chart.wgsl` in `nana-ui`. Each mark carries a target state `to` and a start state `from` (for example `GpuPoint { to, from, dist: [f32; 4] }`, with the compile-time assertion `size_of == 48`, `:27`; shape kind `ShapeKind { Rect = 0, Sector = 1, Symbol = 2, Needle = 3 }`, `:38`), so transition animations interpolate directly in the vertex shader by `MarkTransition` progress, with zero CPU recomputation in steady state.
- **Hover never triggers relayout (`layout/mod.rs`, 276 lines)**: chart layout depends only on `ChartOption`, container size, theme, `LabelMeasure` (`:29`, injected by the host text engine) and `ChartViewState` (`:51`, legend visibility and zoom range); pointer movement queries the already-completed layout through `ChartLayout::hover_at` and never relayouts the chart because the mouse moved.

### 2. `nana-diagnostics`: four macros, static descriptors and a background flight recorder

`crates/nana-diagnostics/src/lib.rs` (135 lines) provides four diagnostic primitives: `event!` (`:83`), `fault!` (`:100`), `metric!` (`:118`) and `span!` (`:131`).

| Macro | Write channel | Core semantics and constraints |
| --- | --- | --- |
| `event!` | Thread-local SPSC ring buffer (drops the newest event when full) | Low-frequency structured fact events (at most 4 typed fields) |
| `fault!` | Same, plus a separate emergency ring that wakes the background worker to persist immediately | Critical faults such as device loss and signature failure; the message string is formatted once, only when it fires |
| `metric!` | Static atomics (Counter / Gauge / Histogram) | **All high-frequency per-frame data must go through `metric!`**, snapshotted by the worker periodically; never emit a per-frame `event!` |
| `span!` | Static atomic histogram + scope guard `SpanGuard` | On drop it computes the monotonic-clock duration and writes it into the histogram |

The framework's own descriptors live in `crates/nana-diagnostics/src/framework.rs` (635 lines, domains `0x0001..=0x0009`); line 2 states the rule **"IDs are part of the `.nlog` contract: never renumber, only append"** (retired IDs keep a comment placeholder and are never reused for anything else). For full usage and the `.nlog` format see [Diagnostics and app paths (reference)](/reference/diagnostics).

### 3. `nana-svg-raster`, `nana-app-icon` and `nana-icons-tabler`

- **`nana-svg-raster` (`crates/nana-svg-raster/src/lib.rs`, 243 lines)**: rasterizes SVG into `RasterizedSvg { width, height, rgba: Arc<[u8]> }` (`:7`) on top of `resvg 0.48.1`, and offers four entry points by consumer: `rasterize_stretch` (`:21`, stretches independently to the target width and height), `rasterize_white_mask` (`:42`, white mask for icon atlases, automatically replacing `currentColor` with pure white), `rasterize_document` (`:58`, outputs at document size with straight alpha) and `rasterize_document_capped` (`:71`, URL SVG decoding with a maximum edge length). It is linked only with `nana-ui`'s `svg` feature: SVG images need it, icons do not. It finds no fonts itself: which font files `<text>` is drawn with, and what the generic families are called, come from the caller in `SvgFonts` (`nana-ui` passes what the font set's `resolve_font_faces` returns, sharing the bytes rather than copying them), and without them text is not drawn. `rasterize_white_mask` has one use left: an icon a host wrapped with an SVG and no shapes.
- **`nana-app-icon` (`crates/nana-app-icon/src/lib.rs`, 140 lines)**: PNG / ICO / ICNS encoding, Windows PE resource embedding (the `embed` feature), plus the runtime default icon and macOS Dock `828 / 1024` grid inset fitting (the `runtime` feature, `with_system_grid:32`).
- **`nana-icons-tabler` (`crates/nana-icons-tabler/src/lib.rs`)**: a static constant table generated from `@tabler/icons 3.46.0` by `scripts/generate_tabler_catalog.py`. Each icon is exported as `pub static *_ICON: IconData { name, shapes, svg: "" }`; the atlas draws the `shapes` (`nana-ui/src/scene_paint/icon_raster.rs`): each one is an outline two grid units wide with round caps and joins, `IconShape::Fill` is a fill, drawn by the rasterizer glyphs are scaled with (`nana_text::raster::path_coverage`), and icons with precompiled `shapes` retain no raw `svg` string; because they are declared as separate statics, the linker drops every icon the application does not reference, so the final binary does not grow.

### 4. `nana-package` and `nana-packager`: the read and write sides of the packaging contract

The packaging system (Issue #226, detailed in [Packaging and distribution contract](/reference/packaging)) is split strictly into a runtime read side and a build-time write side:

- **Runtime read-only side `nana-package` (`crates/nana-package/src/lib.rs`)**: pure-Rust dependencies only (`blake3` pure backend, pure-Rust `ruzstd` decoding and `XChaCha20-Poly1305` / `Ed25519`). It defines the three startup resource classes `ResourceClass { EarlySplash = 0, BootstrapUi = 1, Protected = 2 }` (`pack/mod.rs:30`), the first-screen pack limit `EARLY_SPLASH_MAX_PACK_BYTES = 4 MiB` (`:25`), and the pre-window self-check protocol `SELF_CHECK_ENV = "NANA_PACKAGE_VALIDATE"` (`lib.rs:42`).
- **Build-time write side `nana-packager` (`crates/nana-packager/src/main.rs`)**: six subcommands — `package`, `validate` (including `--run` and the six destructive tamper tests of `--tamper-suite`), `keygen`, `inspect`, `delta`, `macos-app` (`:32-38`) — and SteamPipe delta block reuse from the previous artifact (`--baseline` / `--cache`).

### 5. `nana-ui-dev` and `nana-ui-devtools`: development-time hot reload and headless verification

- **`nana-ui-dev` (`crates/nana-ui-dev/src/lib.rs`, 87 lines)**: wraps the path-level silent debouncer `Debouncer` (`watch.rs:25`) and two tiers of reload request `ReloadRequest { Full, Template }` (`lib.rs:46`). The `#[cfg(not(debug_assertions))] compile_error!(...)` on line `:22` rules out release packages at compile time (see [Development-time hot-reload contract](/reference/hot-reload)).
- **`nana-ui-devtools` (`crates/nana-ui-devtools/src/lib.rs`)**: feature-gated headless automation sessions that build up layer by layer (`offscreen` ⊂ `runtime-agent`, see the header of `lib.rs` and `agent/mod.rs`): the pure-Rust `RuntimeAgentSession` drives pointer clicks, keyboard input, accessibility tree export and offscreen PNG screenshot comparison over a JSON protocol on stdin/stdout. `headless-host` adds `HeadlessHost<P>` on top of `runtime-agent`: it runs an entire `RuntimeProgram` (including any auxiliary windows it opens) through `nana_ui::headless::HeadlessRuntime` in the native host's hook order, then paints and reads back each window. **The repository's only GPU → CPU pixel readback is confined to the `offscreen` module**, and the product `run_runtime` presentation path never calls it.

## The repository ecosystem at a glance

Beyond the 24 core crates under `crates/`, the workspace also contains these directories:

| Directory path | Contents and purpose |
| --- | --- |
| `examples/` (5 workspace members) | `component-gallery` (the full control gallery, shipping four extra binaries `ui-benchmark` / `ui-snapshots` / `ui-migration-ab` / `ui-host-ab`), `reactive-sfc` (`.vue` Rust script examples), `native-rhi-probe`, `runtime-host-fixture`, `package-fixture` (the fixture application for packaging and anti-tamper validation) |
| `platform/android/` | Experimental Android ARM64 `GameActivity` host (shares `UiWorld` → `UiScene` → `SceneWgpuPainter`; see [Android experimental host](/reference/android)) |
| `tools/css-parity/` | 149 CSS box-model conformance fixtures and the comparison tool (plus `tools/css-parity-webview` outside the workspace for local WKWebView comparison) |
| `perf/` | The performance regression benchmark suite (covering `compositor-steady`, `windows-composition-steady`, `nana_text_ids` and `nana_gpu_scale_ids`) |

## Further reading

- [GPU and frame lifecycle](/learn/gpu-frame): how the GPU mark arrays `nana-ui-charts` produces are consumed by `chart.wgsl`
- [Authoring toolchain](/learn/toolchain): how `nana-ui-dev`'s `watch_templates` drives `nana-ui-sfc`'s static text hot replacement
- [The 24-crate coverage matrix](/learn/coverage-matrix) · [Evidence checklist](/learn/evidence-checklist)
