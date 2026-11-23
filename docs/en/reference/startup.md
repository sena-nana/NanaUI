# Two-phase startup and the native splash contract

This page specifies NanaUI's two-phase startup (Issue #225): without introducing a second UI engine, it decouples **native first-screen feedback (Early Splash)** from **full UI readiness (UiReady / visual takeover)**, so the user no longer waits on a blank window while the application initializes and shaders compile.

## The two phases and the core boundary

1. **Phase one: Early Splash (native startup window)**. While the GPU device, pipelines, and application business state are not yet ready, the host first shows a separate, non-activating, small native splash window. The logo can be compiled into the binary or read once from an `early-splash` resource pack. Animations such as pulsing or rotation are driven directly by the operating system compositor (macOS Render Server / Windows DWM); this process runs zero timers and zero per-frame callbacks (`SplashWork::animation_submissions` is always `1`).
2. **Phase two: `UiReady` and visual takeover**. Once the window, Surface, GPU device, `SceneWgpuPainter`, and text system are all ready and an ordinary document can be mounted, laid out, and painted, the host enters `UiReady` and calls `RuntimeProgram::initialize`. The application mounts a loading page or its main UI there and declares a takeover policy. **The host removes the native splash only after it has confirmed that the first frame of the takeover content was successfully submitted to the system compositor.**

::: tip Capability handoff vs. visual handoff
`UiReady` only means "UI rendering capability is ready". It does not mean that long-running business loading has finished. If `early_splash` is not configured, the host creates no extra layer or native window, and behavior is exactly the same as traditional single-phase startup.
:::

## Quick setup

```rust
static LOGO: &[u8] = include_bytes!("../assets/logo.png");

NanaApplication::builder(identity)
    .early_splash(
        SplashSpec::new(SplashLogo::png(LOGO))
            .with_logo_size(128.0, 128.0)
            .with_animation(SplashAnimation::Pulse),
    )
    .run::<RuntimeApplication<App>>(WindowDescriptor::new("App"))
```

Hosts that do not go through `NanaApplication::builder` can use `nana_ui::with_startup(StartupOptions::default().with_splash(spec), || run_runtime::<P>(settings))`. For a complete runnable demo and acceptance probe, see `crates/nana-ui/examples/startup-splash.rs` (pass `--probe` to print a structured JSON timing record after the handoff completes and to check the contract).

## Phase one: Early Splash configuration and platform implementation

`SplashSpec` has the following four settings. This phase provides no text layout, progress bar, scripting, or custom shaders; to show the product name, bake it into the logo image.

| Field | Description |
| --- | --- |
| `logo` | `SplashLogo::png(&'static [u8])` (an embedded static PNG) or `SplashLogo::packaged("nana://res/…")` (read from an `early-splash` resource pack). Encoded size limit `1 MiB`, longest-side limit `1024 px`, decoded pixel limit `4 MiB` (the PNG header is validated before startup; anything over a limit is rejected without decoding). |
| `logo_size` | The logical pixel size of the separate splash window and the logo content box (the logo is scaled proportionally and centered; the separate splash window does not appear in the taskbar, does not steal focus, and only follows the main window's center point and DPI). |
| `background` | `System` (the default theme palette's background color for the current system light/dark mode), `Color(..)`, or `Transparent` (draw only the logo). If the application's own theme does not follow the system light/dark mode, specify `Color(..)` explicitly to avoid a background color jump at handoff. |
| `animation` | `None`, `FadeIn` (default; fade in once and hold), `Pulse` (breathing pulse), `Rotate` (continuous rotation). |

### Cross-platform native implementation and handoff

| Platform | Native splash implementation | System compositor animation | Flicker-free first-frame handoff |
| --- | --- | --- | --- |
| **macOS** | A separate `NSWindow` with no title bar, a transparent background, and mouse events ignored, plus a `CALayer` | `CABasicAnimation` (driven by the Render Server) | During the splash the main window is kept at `alphaValue = 0` (it can lay out but is invisible and receives no pointer input); after the takeover target frame is `present`ed, the main window's `alphaValue` is restored to `1` and the splash is removed with `orderOut` |
| **Windows** | A DirectComposition visual tree on a separate non-activating `WS_POPUP` window (a short-lived D3D11 device uploads the bitmap once and is released right after handoff) | `IDCompositionAnimation` (driven by DWM) | During the splash the main window is hidden with `DWMWA_CLOAK`; after the takeover target frame is `present`ed and `on_submitted_work_done` has completed: uncloak → `DwmFlush()` → remove the splash visual and release D3D11/DComp |
| **Linux and others** | No native splash is shown; `Skipped(PlatformUnsupported)` is reported directly | — | `UiReady` and the takeover ticket contract work as usual; application code needs no platform branches |

The applied outcome is recorded in `StartupStatus::splash` (`SplashOutcome`); the requested value is never treated as the result: `Shown { animation: Applied(..) | Static { requested, reason } }` (when the system has "reduce motion" enabled or the compositor rejects the animation, it automatically degrades to a static logo and records `ReducedMotion` / `NativeAnimationFailed`), `Skipped(..)` (not configured, hidden startup with `visible = false`, embedded host, or unsupported platform), or `Failed(Logo(..) | Package(..) | Native(..))` (a corrupt or oversized logo does not block application startup; only the splash is skipped).

### Windows real-machine handoff acceptance (Issue #273)

`startup-splash --probe` checks the startup record's contract (one device request, one painter, ordered milestones, no splash resources after the handoff, the animation submitted once). `scripts/validate-startup-handoff.ps1` also captures the desktop compositor's output while it runs (about 45 frames per second, so DWM cloaking and DirectComposition are what is captured) and, against the desktop before launch, requires that no frame between the splash appearing and the process exiting falls back to "nothing on screen".

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-startup-handoff.ps1 -ExePath target/release/examples/startup-splash.exe -Arguments '--probe'
```

Measured 2026-11-23 (Windows 11 Enterprise 25H2 build 26200.9974, RTX 5060 Ti / DX12, release, 60 Hz):

| Launch | Splash outcome | Capture |
| --- | --- | --- |
| Development run, fade / pulse / rotate, `--defer=400`, `--composition` | `Shown { Applied(..) }`; committed at 243–422 ms, `UiReady` at 915–1265 ms, handoff done at 1005–1513 ms; every contract check passes | the splash switches straight to the main window with content, no empty frame |
| Development run, `--packaged-logo` (with `packaged-resources`, read from `examples/assets`) | `Shown`, logo read in 0.35 ms | no empty frame |
| Portable package built by `nana-packager`, `--packaged-logo` (logo in the `early-splash` `.nrpack`) | `Shown`, logo read in 1.1 ms | no empty frame |
| `--packaged-logo` without `packaged-resources` | `Failed(Package(Unsupported))`; the application starts normally | — |
| `--bad-logo` | `Failed(Logo(NotPng))`; the application starts normally | — |
| `--no-splash` | `Skipped(NotConfigured)` | one frame (about 30 ms) shows an empty window with only its border |

Two observations: without a splash the main window is shown at `UiReady` and gets its first frame about 150 ms later, uncloaked meanwhile, so an empty window flashes; with a splash the longest event-thread block is 260–445 ms (about 50 ms without), the time spent creating the splash. The capture rate cannot resolve a single duplicated or empty 16 ms frame. macOS was not executed (no machine); Linux reports `Skipped(PlatformUnsupported)` as in the table above and was not run on a real Linux machine this time.

### Reading the logo from an `early-splash` resource pack

With the `nana-ui` `packaged-resources` feature enabled and `resource_packs` configured, `SplashLogo::packaged("nana://res/splash/logo.png")` reads the logo from a resource pack in the `early-splash` category (the time is recorded in `StartupWork::splash_logo_read`; measured locally, a 71 KB logo takes about `0.25 ms`):

- The path is routed to exactly one pack by longest-prefix match in the signature-verified `package.json`. That pack must belong to the unencrypted `early-splash` category (otherwise opening is refused with `NotEarlySplash`, and `KeyProvider` is never called).
- After reading the 4 KiB header, the reader checks the total file size (over `4 MiB` reports `EarlySplashTooLarge`), checks the TOC to confirm the entry is at most `1 MiB`, then verifies hashes block by block and checks the PNG header (during development, when running from `target/`, it automatically falls back to the file of the same name under `loose_root`).

## Phase two: `UiReady` and visual takeover

`UiReady` is reached when `RuntimeProgram::initialize` (or `ApplicationState::initialize` + `build`) is called. At that point `initialize` only needs to mount the first-screen view (a loading page or the main UI), dispatch long-running business work to background threads, and return immediately.

`RuntimeProgram::startup_takeover()` decides when the native splash is removed:

- **`Immediate` (default)**: the main window document built by `initialize` is the takeover content. (When a splash is configured, the initial messages returned by `initialize` are processed in batches of `2 ms / 64 messages` before takeover is requested, so the frame that removes the logo already includes the effects of the initial messages.)
- **`Deferred`**: keep showing the splash (or the loading state) until the application explicitly calls `context.startup().take_over(ticket)`:

```rust
let startup = context.startup();
let ticket = startup.status().ticket.expect("a ticket is assigned after UiReady");
startup.take_over(ticket)?;          // callable from any thread
startup.cancel_takeover(ticket)?;    // withdraw the takeover request and invalidate the old ticket
```

- **Generational tickets and target-frame confirmation**: calling `cancel_takeover` invalidates the old `ticket` (if the takeover frame was already presented, it returns `AlreadyHandedOff`; a late background task calling with an old ticket receives `StaleTicket`). `take_over` records the main window's current `flush` sequence number. **Only a main window frame that completes `flush` after that point and is actually `present`ed successfully completes the handoff** (skipped frames, failed frames, and old frames from before a device rebuild do not count).

## Parallel startup threads and faster shader compilation

After the standalone host (`run_runtime`) creates the window and Surface on the window thread, it immediately moves GPU adapter selection, the device request, and `SceneWgpuPainter` pipeline compilation onto the background `nana-startup-gpu` thread, while mounting the native splash on the main thread. **Splash display therefore fully overlaps GPU initialization.**

Inside the `nana-startup-gpu` thread:
1. If a composition backend was requested (`CompositionCapable`), it reuses the adapter list already enumerated by the DX12 composition probe that ran before window creation, skipping a second enumeration (time drops from about `0.74 s` to `0.08 s`); without a probe it enumerates once here. An adapter is picked from this list according to `StartupOptions::adapter`; see [GPU · Adapter selection](gpu.md#adapter-selection) for the rules.
2. The major `SceneWgpuPainter` pipeline families (quad, mesh/path, icon, text, HostTexture, backdrop) compile in parallel on scoped threads, so total build time is set by the slowest single family (quad) rather than the sum of all of them. The motion evaluation pipeline, used only for test readback, is not compiled on the product startup path.
3. On macOS, the only slow step, Dock icon resampling and PNG encoding, runs asynchronously on a separate `nana-startup-icons` thread and does not block `UiReady`.

### DX12 shader compiler

On Windows DX12, almost all of `SceneWgpuPainter`'s startup build time is spent compiling shaders. **DXC is about twice as fast as the system's built-in FXC** (measured on an RTX 5060 Ti: release-build painter compilation drops from FXC's `2.24–2.39 s` to DXC's `1.18–1.79 s`; debug builds drop from `4.73–7.77 s` to `1.61–1.98 s`):

- When an x64 `dxcompiler.dll` is present in the same directory as the executable (`.exe`) (DXC `v1.8.2502` or later is recommended, from [DirectXShaderCompiler releases](https://github.com/microsoft/DirectXShaderCompiler/releases) or Windows SDK `10.0.26100`; `dxil.dll` is not needed), the host loads it first by its full absolute path (unaffected by `PATH` or the current working directory).
- Without `dxcompiler.dll` it automatically falls back to the system's built-in FXC. You can also choose explicitly with the `WGPU_DX12_COMPILER` environment variable (`fxc` / `dxc` / `staticdxc` / `auto`).

### Precompiled shader artifacts (DXIL / Metal MSL 3.1 / Naga IR)

- **DXIL (Windows DX12, `scene_paint::precompiled`)**: the shader stages of the quad, mesh/path and text families (including the dual-source subpixel program) are compiled to DXIL (shader model 6.0) inside the framework ahead of time and ship in the binary. The host requests `PASSTHROUGH_SHADERS` where the DX12 adapter supports it, loading DXIL directly instead of going through naga → HLSL → DXC; the driver lowers DXIL once and caches it across launches. Measured (RTX 5060 Ti, release, DXC): quad `1.26–1.30 s` → `1.4–1.6 ms`, mesh `199–248 ms` → `2 ms`, text `126–160 ms` → `2 ms`; the whole painter drops from `1.40–1.74 s` (all WGSL) and about `250 ms` (only quad precompiled) to `75–84 ms`, with backdrop and dest remaining (~`90–105 ms`).
- **Metal MSL 3.1 (Apple, `scene_paint::msl_precompiled`)**: targets a floor of macOS 14.0+ / iOS 17.0+ (`MSL 3.1`, matching `nana-packager`'s default `minimum_system_version = "14.0"`). `wgpu_hal::metal::Device` respects a leading `// language: metal<maj>.<min>` header (`metal3.1`) capped against the adapter ceiling and rejects adapters below `(3, 1)` instead of feeding incompatible MSL. All built-in programs are bundled in a single compressed archive `stages.msl.zz`; painter construction enters a lazy `ArchiveScope` (`archive_scope()` / `lease_archive()`), doing zero work on creation and zero decompressions on a warm pipeline-cache hit, sharing a single decompression across worker threads on cold misses (`gpu::MSL_ARCHIVE_DECOMPRESSIONS`, `gpu::MSL_ARCHIVE_DECOMPRESS_NS`, `gpu::MSL_ARCHIVE_DECOMPRESSED_BYTES`), and dropping to `0 B` resident uncompressed MSL when the outer scope exits.
- **Binary Naga IR (non-Apple / fallback, `scene_paint::naga_ir`)**: entry-point-compacted IR is serialized into `*.naga` by the deterministic self-contained binary codec `naga_ir::codec` (`NAGA1` magic + LEB128 varints + fixed little-endian primitives, no `bincode`, panic-free). `builtin_programs_cover_every_production_shader` scans `crates/nana-ui/src/` to ensure manifest completeness.
- **Non-panicking fallback & recovery**: only framework-built artifacts are loaded (`try_pipeline` wraps module/pipeline creation in `OutOfMemory` / `Internal` / `Validation` error scopes). When a stage is missing, mismatched, or refused by the device, `Program::try_pipeline` records `gpu::SHADER_PRECOMPILED_FALLBACK` and compiles WGSL if `runtime-wgsl` (or tests) is enabled, otherwise recording `gpu::SHADER_BUILD_FAILED` and returning `Err(ScenePaintError::ShaderUnavailable)` (custom materials rejected without `runtime-wgsl` emit `gpu.material.rejected`); `SceneWgpuPainter::try_new` / `try_new_with_presentation` propagate errors cleanly instead of panicking.
- **Regeneration & freshness verification**:
  - Metal MSL (macOS): `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1` (`precompiled_msl_stages_are_fresh` checks `WGPU`, `FORK_MSL_HASH`, `MSL_LANG_VERSION == (3, 1)`, `UNCOMPRESSED_LEN`, and full `builtin_programs()` coverage; `precompiled_msl_stages_pass_metal_syntax_check` runs `xcrun -sdk macosx metal -std=metal3.1 -fsyntax-only` and available iOS Simulator checks).
  - Naga IR (any host): `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1`.
  - DXIL (Windows + DXC): `NANA_DXC_PATH=<dxcompiler.dll> cargo test -p nana-ui --lib --features gpu,hosted,full -- --ignored regenerate_precompiled_dxil --test-threads=1` (paired with `precompiled_families_paint_what_their_programs_paint` on DX12).

### Shader and pipeline cache across launches

Every pipeline the painter (and anything else on the device) builds goes through the device's default pipeline cache ([GPU · Shader and pipeline cache across launches](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/gpu.md#5-shader-and-pipeline-cache-across-launches)). With `NanaApplicationBuilder::pipeline_cache(store)` the `nana-startup-gpu` thread restores the cache before the painter compiles and saves it after; pipelines compiled later are saved by a background `PipelineCacheSaver` when the cache grows (once compiles settle, at most every 10 s, and once more on exit), and a device replaced after a loss restores from the same store before it rebuilds; on DX12 a warm start skips DXC/FXC for every stage the cache holds. Measured with `startup-splash --probe --no-splash --app-ms=100 [--pipeline-cache=DIR]` (RTX 5060 Ti, release, DXC `10.0.26100` beside the executable, `painter_build_ms`; the first run after a build is slower and is left out):

| Backend, path | Without a cache | Cold (empty cache, then saved) | Warm |
| --- | --- | --- | --- |
| DX12, precompiled quad/mesh/text | `69–70 ms` | `70 ms` | `23–38 ms` (cache file `93 KB`) |
| DX12, all WGSL (precompiled path turned off for the measurement) | `1.56–1.69 s` | `1.80 s` | `31–33 ms` (`264 KB`) |
| Vulkan | `43–46 ms` | `46 ms` | `31–35 ms` (`617 KB`) |

Warm time is the restore and save included: reading and writing the file happen inside the timed build. Bytes saved with DXC and loaded under FXC (`WGPU_DX12_COMPILER=fxc`) are dropped by the validation key: that start compiled everything (`3.1 s` with FXC and no precompiled DXIL) and saved FXC bytecode in their place.

**Could a shipped seed cache replace `scene_paint::precompiled`?** Performance: yes, an all-WGSL painter with a warm cache (`31–33 ms`) is within a few milliseconds of precompiled DXIL plus the cache (`23–38 ms`); what remains is naga writing HLSL for the stages, which the cache key needs. Coverage: no, an entry hits only for the compiler build and shader model that made it. Pinning the shader model, shipping `dxcompiler.dll` (about 17.2 MiB) and the FXC fallback are weighed in [GPU · Design note: a seed cache instead of precompiled DXIL](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/gpu.md#design-note-a-seed-cache-instead-of-precompiled-dxil); the precompiled path stays.

### Shader IR cut per entry point

On every platform the framework's scene programs (quad, mesh, text, icon, chart, backdrop, dest, HostTexture, the motion evaluator, and the surface and text material programs) reach the device through `scene_paint::wgsl::Program`: a program is parsed and validated once, when first needed, and each entry point becomes a naga module of its own holding only the functions, types and constants it reaches (`naga::compact`), handed to wgpu as `ShaderSource::Naga`. Pipelines that share an entry point share its module; the device validates only the cut-down IR; wgpu-hal no longer clones, compacts and revalidates the whole module for every pipeline stage; and each pipeline keeps only its own stages' code (Metal writes MSL per pipeline, so modules stay alive). For example, the blit program has 39 entry points and 658 expressions, and one stage carries 5–424 of them; a mesh stage carries 53–737 (of 1 518), a chart stage 411–1 282 (of 2 593). A cut module no longer declares the overrides it does not use, and a pipeline sets only the constants its stage still declares (`Stage::constants`). A program that does not parse or validate here (the framework's do not; the tests compile each one) goes to the device whole as WGSL, which reports the error as before. The small programs of the window output, the colour transfer, the default GpuView and the native-content placeholder reach the device through `Program` too.

Framework programs are compiled without loop bounding (the counter wgpu adds to every loop): every loop in the framework's shaders ends whatever the data, its counter starting at a constant, stepping by one and compared strictly against its bound, and the three others each have a stated reason; `loop_bounds_hold` checks each one. Bounds and integer-division checks stay, because indices and divisors come from buffers the CPU writes and Metal has no hardware robustness behind them. Application and skin material programs (`Program::material`) keep every check. Without the loop counters the quad, mesh and text DXIL is 1–2.5% smaller.

## Timeline metrics

`StartupStatus::timeline` records six key milestones measured from the host entry point (`run_runtime`): `splash_committed` → `ui_ready` → `takeover_requested` → `first_frame_submitted` → `handoff_completed` → `splash_released`. `StartupStatus::work` and `nana_diagnostics::framework::host` (`STARTUP_PHASE`, `SPLASH_OUTCOME`, `host.startup.device_request`, `host.startup.painter_build`, and others) also record the longest single blocking stretch on the event thread, device request time, painter compile time, and the number of live native resources (after handoff completes, `SplashWork::live_resources` returns strictly to zero).
