# Live GPU Content and GPU Integration Contract

This page specifies the full technical contract for NanaUI's host GPU context (`GpuContext` / `FrameContext` / `GpuTexture`), live texture mounting (`HostTexture` / `GpuTextureView`), same-pass direct drawing (`GpuView`), cross-thread latest-frame exchange (`FrameExchange`), and per-scene offscreen production (`SceneResourceProducer`).

For windowless retained output, target planning, and multi-consumer paths (such as Spout or screen recorders), see [Off-Window Output (ExternalSurface)](output.md). The `GpuContext`, `FrameContext`, and `GpuTexture` described here are also the sole device, submission, and resource owners for that output boundary.

## Backend selection, GPU policy, and HDR presentation

### 1. Native RHI decision gate (Issue #186) and choosing WGPU backends

Issue #186 concluded **NO-GO**: WGPU remains NanaUI's only official graphics backend, and the official path stays `Logical GPU ABI -> WgpuBackend -> wgpu` (see the [Issue #186 delivery record](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/consumer-upgrade-2026-09-25-issue186.md)).

Framework crates do not lock the graphics backend on the application's behalf: the workspace's `wgpu` enables only `std`, `parking_lot`, and `wgsl` by default. When `gpu` is enabled, `nana-ui`'s default feature `wgpu-backends` turns on every standard backend for the current platform (DX12 / Vulkan / GLES on Windows, Metal on Apple, Vulkan / GLES on Linux). To reduce size and ship only specific graphics backends, set `default-features = false` on the dependency and list the backends you need per platform explicitly on the application's own `wgpu` dependency; `hosted_context` only picks adapters among the backends actually compiled in. For how the DX12 shader compiler (`dxcompiler.dll` vs FXC) is chosen, see [Two-phase startup · DX12 shader compiler](startup.md#dx12-shader-compiler).

### Adapter selection

The host chooses only among adapters that can present the main window. Selection happens on the startup thread, and the replacement device after device loss is chosen again by the same rules:

- The application specifies it through `StartupOptions::with_adapter(GpuAdapterChoice)`. `Matching(GpuAdapterId)` matches by vendor/device id; among multiple cards of the same model it then compares the PCI bus address, and when either side has no bus address it compares names. If that card is absent, cannot present the main window, or cannot create a device, selection falls back to automatic; this is not a startup failure.
- `Automatic` (the default) follows the rules of wgpu's `request_adapter`: when `WGPU_ADAPTER_NAME` is set, it takes the first adapter whose name contains it; otherwise it stably sorts by `WGPU_POWER_PREF` and takes the first. When neither is set, it is the first in enumeration order. The application's `Matching` takes precedence over both environment variables.
- A program reads the `GpuAdapterSelection` from this device creation through `RuntimeProgramContext::gpu_adapters()`: the adapters that can present the main window (in enumeration order), the one actually used, and whether the requested card was skipped (`requested_missing`). Contexts in which the host does not choose a device (headless, staged, embedded injection) return an empty list or only the injected device. Settings UIs should read this value instead of enumerating adapters themselves.
- Adapters are not switched while running: the application saves the new choice, and it takes effect on the next launch.

### 2. Unified GPU policy (Issue #184)

`GpuContext::policy()` maintains a shared pipeline registry and work counters per device generation (`DeviceGeneration`). `SceneWgpuPainter`'s static pipelines reuse the underlying WGPU objects across windows through it. `GpuWorkSink` records buffer uploads and reallocations uniformly into the device statistics (`GpuPolicyStats`, including `upload_bytes`, `upload_writes`, `upload_copies`, `upload_flushes`, `upload_ring_allocations`, `upload_ring_waits`, `frame_slot_waits`, `frame_slot_stalls`, and more). The public boundary always stays pure `nana-gpu` types.

### 3. HDR display info and color space contract (Issue #283)

A Surface's HDR capability is determined by `SurfaceCapabilities::format_capabilities`. `wgpu::Surface::display_hdr_info` returns the current display's runtime recommendations, which are used only to adjust the final tone map: Apple EDR provides relative headroom, and Windows provides peak luminance and the SDR reference white in nits. A missing value means only "unknown", not SDR, and it alone must not decide whether to enable an HDR Surface:

- **Event-driven refresh**: the host caches and updates `DisplayHdrInfo` when creating the Surface, on device recovery, and on display events such as window move, scale, and resize (steady-state frames do not poll the operating system, and Metal queries always run on the event thread). When values do not change, no redraw is triggered; when they change, only the presentation uniform is updated, without rebuilding Runtime or Scene state. The safe headroom is `DisplayHdrInfo::tone_map_headroom().unwrap_or(1.0)`, and a missing SDR reference white is treated by the `80 nit` scRGB convention. When an embedded host receives a system display notification, it can call `EmbeddedRuntime::refresh_display_hdr_info()` (or the method of the same name on `HostedGpuContext`) on the window thread.
- **Linear scRGB working space**: `SceneWgpuPainter`'s internal compositing working space is uniformly **premultiplied linear scRGB**. Chart marks (`ScenePrimitiveKind::Chart`) are drawn by three analytic anti-aliased pipelines for polylines, areas, and shapes (edges are a 1-device-pixel transition and do not rely on MSAA). Vertex and style arrays stay resident in the target GPU buffers by revision, and the pipelines are compiled lazily the first time a chart is encountered.
- **Color metadata for cross-thread and offscreen export**: `FrameExchange` copies texels byte for byte and does no color space conversion. When a producer needs to pass a color space across threads, it should call `copy_from_with_metadata` with a `FrameColorMetadata` (declaring primaries, transfer function, and alpha semantics), and the consumer reads it from `FrameLease::color_metadata()` or `FrameBinding::color_metadata()` (the old `copy_from` marks frames `Unknown`, and consumers must never silently assume sRGB). By default, `FrameBinding` directly accepts only full-range sRGB or 80-nit linear scRGB. Frames declared as P3, PQ, HLG, limited range, or non-premultiplied alpha stay in the inbox and are not sampled by mistake; the producer must convert them first or a custom consumer must handle them.
- **`nana-ui-devtools::offscreen` snapshot boundary**: always uses `BGRA8_UNORM_SRGB`. To export a PNG, use the `SnapshotImage` returned by `readback_image` / `paint_image` and call `into_png_srgb` or `write_png_with_metadata` (they convert premultiplied sRGB to straight-alpha sRGB and write the PNG sRGB chunk automatically).

### 4. Frame upload ring (UploadRing), frame slots, and caches

- **Batched upload (`UploadRing`)**: the renderer does not scatter `queue.write_buffer` / `write_texture` calls. Instead, it appends writes through `GpuWorkSink` to the current `FrameContext`'s upload batch. When `FrameContext::submit` is called, the device's pending batch (including off-frame writes and writes from previously discarded frames) and this frame's upload batch are copied with one `memcpy` into a mapped block of the `UploadRing` (capped at `32 MiB`), merged into a single upload command buffer, and land ahead of the render commands in the same `queue.submit`. This removes WGPU's internal staging allocation for each small write. (Text glyph blocks still use an in-encoder `copy_buffer_to_buffer` at the recorded position, so that multiple draws to the same target before a single submit each see the correct glyphs; see #224. Glyph atlas regions written by the second and later paints within the same frame are likewise copied at the recorded position; see "Atlas ordering for multiple targets in one frame" below. These count toward `upload_bytes` but not `upload_writes` / `upload_copies`.)
- **Bounded frame slots**: each host frame (`begin_frame` / `try_begin_frame`) holds a frame slot while recording (discard returns it immediately; submit returns it in the GPU completion callback; producer frames take no slot, see §9). When slots are full, `begin_frame()` does one bounded wait on the oldest submitted frame and frees that slot once the wait completes; `try_begin_frame()` returns `None` immediately when slots are full. The window presentation path uses `try_begin_frame()`, and **must call `get_current_texture` only after obtaining a frame slot**. (If no slot is available, it first does one non-blocking `poll`; if still full, it skips this frame and leaves the buffer already on screen in place. This prevents a transparent swapchain from taking a buffer and then waiting, which would blank the whole window.)
- **Generational caches and shared URL textures**: the pipeline and resource layout registries use a `StampedCache` isolated per `DeviceGeneration` (when full, it linearly evicts the least recently used 1/8 and compiles pipelines in parallel outside the lock). `SceneWgpuPainter` holds a single `UrlTextureCache` shared by Quad backgrounds, `border-image`, and HostTexture masks (at most 4 concurrent HTTP fetches, an idle cap of 256 entries / `64 MiB`, automatic release after 120 consecutive painted frames without a reference; raster image decode is capped at `4096 px` / `64 MiB`, SVG at `2048 px`).
- **Painter core shared across profiles**: incompatible output groups of the headless `PresentationDriver` share one `SceneWgpuPainter`. Profile-independent parts are stored once: the text engine, shape/raster caches and glyph atlas, `UrlTextureCache`, and motion resources. The icon atlas is target state, one per painted `RenderTargetId` (plus one for the painter's own default target). Each working format (and its URL source encoding) keeps one pipeline set (quad, mesh, icon and text programs, HostTexture, backdrop, chart), swapped in when the profile switches and built on first use. URL textures are bucketed by egress and source encoding (sRGB or legacy linear direct write), so the two encodings never overwrite each other. Each target uses only one profile for its lifetime. Whenever the shared atlas evicts or moves glyphs for any profile, the placement epoch advances, and other targets rebuild their retained `PreparedBatch`. Window hosts still hold one painter per profile.
- **Atlas ordering for multiple targets in one frame**: a `FrameContext`'s upload batch lands as a whole ahead of all of that frame's passes at submit time, and the glyph atlas is shared by every target that painter draws. So the second and later paints within one frame write their glyph regions with `copy_buffer_to_texture` at the recorded position in that frame's encoder, ordered after the passes already recorded. When a later group evicts, reuses, or compacts an earlier group's glyph rectangles, the earlier group still samples its own glyphs. The first paint still goes through the upload batch, so the cost of a single-target frame is unchanged. The same applies when a window and its same-profile outputs share a painter within one frame. These copies live with the frame's encoder: when a frame is discarded they never land, and before the next paint the painter makes the atlas forget all glyph positions, so each target re-places glyphs and rebuilds its batches. A painter records into only one frame at a time: the host submits or discards the current frame before painting into the next; two frames submitted out of order would put one frame's atlas writes under the other frame's passes.
- **Motion ordering for multiple scenes in one frame**: motion descriptors, keyframes, and clock buffers are also shared by the painter. The second and later paints in a frame upload only the changed data, at the encoder's recorded position, so a later scene does not overwrite an earlier scene's animation. The first paint keeps using the frame upload batch, and identical motion data is not uploaded twice. Ordered copies count toward upload bytes and add no submits or CPU waits. A dropped frame invalidates motion's CPU upload cache through the same shared resource ledger, and the next frame re-uploads. This rule covers both `paint` and `paint_target`.
- **Painter resource counters**: `SceneWgpuPainter::text_glyph_counters()` reports the glyph atlas, raster cache, and glyph uploads. `resource_counters()` (`ScenePainterResourceCounters`) reports icon atlas count and bytes, URL texture count, uploaded GPU bytes (including all mips, excluding driver padding), pending CPU upload bytes, and in-flight loads. Headless hosts read the same numbers for the shared painter through `PresentationDriver::text_glyph_counters()` / `painter_resource_counters()`. All of these are texel bytes allocated by the painter, not driver video memory.
- **Target attachment and pipeline reuse**: when switching between UI-only and mixed GPU-node content, only the 4x MSAA attachment is added or released; the linear color target, bindings, and group caches of the same size, format, and presentation profile are kept. Scaling must rebuild size-dependent textures, but on a device pipeline cache hit no shader module or pipeline layout is created again, so multi-target first frames do not parse WGSL repeatedly. Changing size, output format, or presentation profile still rebuilds the target under the original contract.


### 5. Shader and pipeline cache across launches

Every `GpuContext` whose device has `PIPELINE_CACHE` (the hosted device requests it whenever the adapter offers it) owns one pipeline cache and makes it the device default through the fork's `wgpu::Device::set_default_pipeline_cache`. Every pipeline created without naming a cache goes through it — all scene families, surface and text materials, chart programs, and renderers that build pipelines on the raw device. On DX12 it holds compiled shader bytecode, so a stage found there skips naga's HLSL compile in DXC/FXC; on Vulkan it is the driver's `VkPipelineCache`. Metal and GL have none (`pipeline_cache_key()` is `None`).

The framework hands out and takes back bytes; the host decides where they live:

- `GpuContext::pipeline_cache_key()` names what the bytes belong to; `file_stem()` is `dx12` (bytecode is adapter-independent) or `vulkan-<vendor>-<device>`.
- `load_pipeline_cache(&bytes)` replaces the cache before painters are built; `pipeline_cache_data()` returns the bytes to save. `restore_pipeline_cache` / `persist_pipeline_cache` do the same through a `PipelineCacheStore` (`load` / `save`).
- Hosted applications pass a store with `NanaApplicationBuilder::pipeline_cache(store)` (or `StartupOptions::pipeline_cache`): the `nana-startup-gpu` thread restores it before the painter compiles and saves it once the painter is built.
- Pipelines compiled later (materials, charts, families built on demand) are saved by `PipelineCacheSaver`: on a thread of its own, every `PipelineCacheSaver::DEFAULT_INTERVAL` (10 s), it reads how many pipelines went through the device cache (the fork's `wgpu::PipelineCache::pipelines_built`), saves only once a burst of compiles has settled (two checks in a row see the same count), and only when the cache actually grew (`GpuContext::persist_pipeline_cache_if_grown`): pipelines that hit the cache and idle time write nothing, and an idle check reads one counter. A clean host shutdown (`WindowManager`'s drop, beside the store's final flush) saves once more what grew. A lost device is never saved.
- After a device loss the host restores the cache from the same store onto the new device before the painters and program renderers rebuild on it (`switch_gpu`), so the rebuild after a TDR skips the compiler like a warm start. That read runs on the event thread in the same stretch where recovery already blocks on the new device, not on a frame path. Hosts that do not use `pipeline_cache(store)` call the methods above themselves, and can use `PipelineCacheSaver::spawn(&gpu, store, interval)` directly.
- The saved bytes carry an envelope (magic, key, length, checksum) over wgpu's own header and validation key, and the DX12 payload checksums each entry. Bytes that are not a cache, belong to another key, are truncated or corrupted, or that the backend rejects are refused (`PipelineCacheLoad::Refused`) and the device compiles as if nothing had been loaded; no backend validation error is raised. A driver update invalidates Vulkan data inside wgpu/the driver, and a DXC update invalidates DX12 data through the validation key; both silently recompile.

#### Design note: a seed cache instead of precompiled DXIL

Not implemented; this weighs the option. Could a shipped seed cache make the first launch warm too, and so retire the passthrough DXIL in `crates/nana-ui/src/scene_paint/precompiled/`? A DX12 cache entry is keyed by a hash of the HLSL naga writes, the entry point, the target profile (stage and shader model) and the compile options (`InstanceFlags::DEBUG`, the strip-debug-info workaround, whether the device enabled `SHADER_F16`/`SHADER_I16`), and the validation key binds the whole bytes to the compiler (kind, version, DXC commit). For a seed to hit on a user's machine, all of these must be settled at build time:

- **Pinning the shader model.** naga itself supports SM 6.0 (`naga::back::hlsl::ShaderModel::V6_0`; the precompiled DXIL is `*_6_0`), and no fork change is needed: wgpu already has `Dx12BackendOptions::force_shader_model` (`unsafe ForceShaderModelToken::with_shader_model`), wgpu-hal takes `min(adapter maximum, forced)`, and forcing `V6_0` makes every DXIL-capable adapter write SM 6.0. The cost is device-wide: features gated on a higher model go with it (`SHADER_F16` needs 6.2, mesh shaders and ray queries 6.5, 64-bit atomics 6.6), including ones an application requests through `shared_device_features`; the framework's own pipelines need none of them. It may only be set with DXC: FXC has no maximum, so the forced value would send it SM 6.0, and FXC stops at 5.1. Debug and release options also differ, so a seed is generated with release flags.
- **Shipping `dxcompiler.dll`.** wgpu loads only that file (`Dx12Compiler::DynamicDxc { dxc_path }`). The x64 `dxcompiler.dll` of DXC v1.8.2502 is 17 986 592 bytes (about 17.2 MiB), against about 195 KB for the 11 precompiled DXIL stages and their generated code. A seed hits only for the DLL build that made it; an application shipping another DXC version has the whole seed dropped by the validation key.
- **The FXC fallback.** FXC's identity is the fixed `fxc d3dcompiler_47` and its shader model is always 5.1, so an FXC seed is the one that would hit on any Windows with nothing shipped. But the key's file stem, `dx12`, is the same for both compilers, so one store holds one kind of bytes, and the other compiler's validation key refuses them and recompiles everything; covering both means shipping two seeds and picking by the compiler the device got. Precompiled DXIL loads as passthrough under either.
- **Recommendation: keep precompiled DXIL, ship no seed cache, and do not pin the shader model in the framework.** The precompiled path does not depend on the compiler, needs no 17 MB DLL and costs no device features; the runtime cache already makes every launch after the first warm for the remaining pipelines (materials, charts, families built on demand). If a first-launch need appears, a seed belongs to the application: one that already ships DXC generates it in its own build with the same DLL, release flags and `force_shader_model = V6_0`; or, for applications without DXC, a single FXC seed covering the framework families outside the precompiled set. Measure either before adopting it.

### 6. Measured: wgpu-core validation share and input-to-present latency (2026-11-11)

Two fork changes to `third_party/wgpu` were held until measured: (a) letting trusted framework passes skip wgpu-core's per-draw validation and resource tracking, and (b) frame pacing on the frame-latency waitable. Machine: Windows 11, RTX 5060 Ti, 60 Hz display, release build with `CARGO_PROFILE_RELEASE_DEBUG=limited`. Probes are measurement tools only (examples and a script); nothing was added to product frame paths.

**Painter CPU.** `stack-sampler` suspends the bench's busiest thread every 2 ms for 8 s (one register read and stack copy, 25–37 µs per sample), walks the copy with dbghelp including inline frames, and `perf/stack_attribution.py` charges each sample to the leaf-most owned frame (`alloc`/`core`/heap/`memcpy` frames go to their caller). "Blocked" samples were inside a kernel wait. CPU per frame = thread CPU time / frames over the same window. "core in replay" = wgpu-core's own frames under `encode_render_pass`, where each recorded draw and `set_*` is validated, tracked and replayed: the most that skipping validation could save, since it also holds bookkeeping that must stay.

| Scene | Backend | Draws/frame | CPU µs/frame | Blocked | NanaUI | wgpu-core | hal + driver | core in replay |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `ui-dense-2k` | DX12 | 4 | 275 | 91% | 10.0% | 13.3% | 70.5% | 11.9 µs |
| `host-textures-64` | DX12 | 68 | 1405 | 32% | 6.4% | 12.2% | 76.5% | 110 µs |
| `shader-nodes-256` | DX12 | 14 | 247 | 86% | 33.4% | 11.6% | 50.5% | 13.4 µs |
| `text-paint-color` | DX12 | 3 | 3584 | 0% | 93.4% | 0.8% | 5.3% | 10.9 µs |
| `text-constraint-resize` | DX12 | 2 | 19584 | 0% | 96.1% | 0.5% | 2.1% | 35 µs |
| `ui-dense-2k` | Vulkan | 4 | 104 | 90% | 14.4% | 19.0% | 62.5% | 8.7 µs |
| `host-textures-64` | Vulkan | 68 | 1322 | 32% | 8.3% | 14.2% | 74.8% | 117 µs |
| `shader-nodes-256` | Vulkan | 14 | 182 | 84% | 36.1% | 11.1% | 50.0% | 9.5 µs |
| `text-paint-color` | Vulkan | 3 | 3918 | 0% | 93.5% | 1.2% | 4.9% | 19.9 µs |
| `text-constraint-resize` | Vulkan | 2 | 21690 | 0% | 97.2% | 0.5% | 1.4% | 22.1 µs |

Shares are of non-blocked samples. The remainder is the `wgpu` front end (≤1.3%) and OS/other frames.

- The painter batches 2,000 nodes into 4 draws, so per-draw validation has almost nothing to scale with. The UI scenes are GPU-bound: the main thread waits in `GpuContext::begin_frame` for a frame slot (`Device::maintain`).
- `host-textures-64` is the many-small-passes case (the bench's 64 producer passes plus the UI pass). Its cost is driver work: 25% is the bench's own `queue.write_buffer` calls, and inside `command_encoder_finish`, 42% is wgpu-core opening a new hal command list per pass (`InnerCommandEncoder::open_pass` → `begin_encoding`, 290 µs/frame on DX12 and 528 µs on Vulkan). Only 19% of `finish` is wgpu-core's own code.
- NanaUI's own work, by inclusive share of CPU: text is 3.5% in `ui-dense-2k`, 57% in `text-paint-color` (mostly `RuntimeDocument::flush`; the painter is 9.6%) and 84% in `text-constraint-resize`. Icons are 0.5% in `ui-dense-2k`. None of the `gpu-scene-*` scenarios draws vector paths, so this measurement does not cover path cost; paths are measured in §10.

**(a) Verdict: no-go.** Skipping validation would save at most 9–35 µs/frame in ordinary scenes and about 110–117 µs in the 65-pass synthetic. That is under 1% of CPU in text-heavy frames, where NanaUI's own work is 93–97%. Revisit only if a real scene shows hundreds of draws or passes per frame. Even then, the per-pass command-list opening is the larger target.

**Input-to-present latency.** `input-latency-probe` opens an ordinary hosted window (Mailbox, `desired_maximum_frame_latency` 1, wgpu's default `Dx12UseFrameLatencyWaitableObject::Wait`). A driver thread posts `WM_LBUTTONDOWN` at random 120–250 ms intervals, and each press flips the window's color. Timestamps are all QPC: `post` before `PostMessageW`, `input` in `RuntimeProgram::input_event` (after lowering and routing), `prepare` and `presented` in the frame hooks (after `present` returned, with the host's stage gauges), and `screen`, the Desktop Duplication `LastPresentTime` of the first composed desktop frame showing the new color. wgpu exposes no `GetFrameStatistics` for the swapchain, so DWM's present time stands in for DXGI frame statistics. Samples where DWM folded more than one update into a frame were dropped. All values are in ms; n = 161–297 per row.

| Window | Backend | post→input | input→prepare | prepare→presented | presented→screen | **post→screen p50 / p90 / p99** | `acquire` p50 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| idle (on demand) | DX12 | 0.37 | 0.19 | 0.88 | 7.01 | **8.5 / 12.8 / 15.9** | 0.024 |
| idle (on demand) | Vulkan | 0.37 | 0.20 | 0.75 | 5.55 | **7.0 / 11.0 / 15.2** | 0.039 |
| continuous 60 fps | DX12 | 0.46 | 0.12 | 15.44 | 16.32 | **32.4 / 38.7 / 40.3** | 14.64 |
| continuous 60 fps | Vulkan | 0.34 | 0.14 | 0.56 | 5.48 | **6.6 / 10.1 / 13.4** | 0.029 |
| continuous 60, waitable `DontWait` (A/B) | DX12 | 0.33 | 0.12 | 0.69 | 55.66 | **59.3 / 68.1 / 74.1** | 0.019 |
| continuous 60, waitable `None` (A/B) | DX12 | 0.34 | 0.12 | 0.64 | 53.55 | **56.5 / 66.7 / 72.2** | 0.019 |

Each cell other than post→screen is a p50. The A/B rows came from a temporary, uncommitted patch that set `backend_options.dx12.latency_waitable_object` from `WGPU_DX12_USE_FRAME_LATENCY_WAITABLE_OBJECT` in `hosted_context::instance_descriptor`.

- Idle UI, which is most NanaUI frames: the input reaches the screen in about 8.5 ms (DX12) or 7 ms (Vulkan) at p50. Of that, about 1.4 ms is NanaUI from post to present, and about 82% is DWM waiting for the next 60 Hz vblank to compose. Frame slots (`reserve` 0.004 ms), the swapchain and the waitable add nothing.
- A continuously redrawing DX12 window is the case that queues. `acquire` blocks 14.6 ms on the frame-latency waitable after the frame has already applied the input, and the presented frame then waits another refresh for DWM. The waitable itself is required: with it disabled, DXGI queues about three frames and p50 rises to 56–59 ms. Vulkan on the same machine does not block (6.6 ms).

**(b) Verdict: go for DX12, scoped to continuously redrawing windows.** Do not remove the wait; move it. Wait on the waitable at the start of the frame, before input is drained and the frame prepared, instead of inside `get_current_texture`. wgpu already creates the handle (`Dx12UseFrameLatencyWaitableObject::DontWait`, hal `dx12::Surface::waitable_handle()`), so the fork needs only an additive way for the host to wait on it. This is an estimate, not a measurement: input that arrives during the wait still waits for the rest of it, so the expected p50 gain is about half of the 14.6 ms block, roughly 32 → 25 ms. Reaching Vulkan's ~7 ms would also need phase alignment (presenting just before DWM composes). Idle on-demand UI gains nothing. Metal drawable timing was not measured (no macOS host), so it stays deferred.

Commands (from the repository root; S is a scratch directory):

```sh
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 12 --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu
cargo build -j 12 --release -p nana-ui --example stack-sampler --example input-latency-probe --features hosted,bundled-fonts
# per backend (dx12 | vulkan) and scene
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 3000 --duration-ms 8000 --interval-us 2000 \
  --out $S/dx12-ui-dense-2k.folded -- target/release/nana-gpu-scene-benchmark.exe \
  --scenario perf/scenarios/gpu-scene-ui-dense-2k.json --sample-seconds 12 --output $S/dx12-ui-dense-2k.bench.json
python perf/stack_attribution.py $S/dx12-ui-dense-2k.folded            # tables; --json for a summary
WGPU_BACKEND=dx12 target/release/examples/input-latency-probe.exe --clicks 300 --output $S/lat-dx12.json
WGPU_BACKEND=dx12 target/release/examples/input-latency-probe.exe --clicks 300 --continuous 60 --output $S/lat-dx12-c60.json
```

### 7. Measured: a persistently mapped upload path (2026-11-11)

The proposed fork change: a buffer created mapped and never unmapped, which NanaUI writes directly once its frame-slot fences show the GPU is not reading that range. It would drop `UploadRing`'s mapped chunk and copy commands, and the text staging ring's `write_buffer_with` and `copy_buffer_to_buffer`. On unified-memory devices the GPU would read that memory directly; on discrete GPUs it would sit in an upload heap or ReBAR. **Decision: not done**, for the reasons the two measurements below give. Same machine as §6 (RTX 5060 Ti discrete, plus the Intel integrated GPU of the Core Ultra 7 265K), release builds. Metal was not measured (no macOS host). The text-engine page's figure of about fifty thousand instructions per `write_buffer` comes from Metal, which has unified memory and is where this change would pay most; re-measure there when a Mac is available.

**The upload paths alone (`crates/nana-gpu/examples/upload-cost.rs`).** Each frame clears a 256×256 target in a render pass, writes `writes` runs of `bytes` bytes to their own destination buffers, calls `FrameContext::submit`, and waits for that frame (one submission in flight at a time). Timing is `QueryThreadCycleTime`, so blocked waits do not count. The table gives the sum of two medians over 2000 frames, thread cycles from the writes to the return of the submit and in the wait for the frame, minus the same frame without uploads, in thousands of cycles (about 3.8 thousand cycles = 1 µs). `ring` is `UploadRing`; `text` is the text path (one `write_buffer_with` plus one copy per run); `copy` records only the copy commands, from a source already on the GPU; `memcpy` copies the same bytes into upload memory that stays mapped, the floor of a persistently mapped path. The example also has `queue` (one `queue.write_buffer` per run), left out of the table.

| Adapter | Backend | Empty frame | 4×256 B: ring / text / copy / memcpy | 32×1 KiB: ring / text / copy / memcpy | 8×64 KiB: ring / text / copy / memcpy |
| --- | --- | ---: | --- | --- | --- |
| RTX 5060 Ti | DX12 | 370–416 | +230 / +282 / +101 / +12 | +199 / +277 / +140 / −5 | +759 / +567 / −122 / +157 |
| RTX 5060 Ti | Vulkan | 310–394 | −52 / −71 / −71 / −85 | +131 / +138 / +114 / +15 | +741 / +225 / −16 / +246 |
| Intel integrated | DX12 | 99–134 | +59 / +102 / +37 / +4 | +143 / +277 / +155 / −10 | +165 / +271 / +100 / +31 |
| Intel integrated | Vulkan | 67–71 | +22 / +30 / +11 / −5 | +108 / +114 / +130 / +32 | +154 / +109 / +75 / +65 |

On NVIDIA the empty frame itself varies by ±50 thousand cycles on both backends; the negative numbers are that noise.

- Small uploads add 6–75 µs a frame (`ring` minus `memcpy`): the most a persistently mapped path could save. Most of it is the driver's cost for the copy commands and the extra command buffer (the `copy` column), not the staging itself.
- Every `write_buffer_with` / `write_buffer` call makes wgpu-core create a staging buffer: about 160–180 thousand cycles (about 45 µs) on NVIDIA DX12. The text path pays it once in each frame that writes instances.
- `ring` writing 512 KiB on NVIDIA adds about 750 thousand cycles, far more than copying the same bytes into mapped memory; Intel does not show it. The cause was not found (that cell was not sampled). It is an `UploadRing` issue, not a fork one.

**Share in real scenes.** Sampled with §6's `stack-sampler` on `gpu-scene-ui-dense-2k`, `text-paint-color` and `host-textures-64`, and on `nana-text-paint-benchmark` with ten thousand labels, `static` and `mutate-1pct`, counted inclusively: `nana_gpu::upload::record` (take a mapped chunk, copy, unmap, record the copies, `map_buffer_on_submit`) is at most 0.5% of non-blocked CPU in every scene, and `ui-dense-2k`'s steady frames upload nothing. Text staging (`write_buffer_with` plus `copy_writes`) is at most 0.5% in DX12 `static` and `mutate-1pct`. What the upload command buffer adds inside `queue_submit` cannot be separated in the samples; from the `copy` column it is at most about 40 µs a frame, about 1% of `text-paint-color`'s 3.6 ms frame. NVIDIA's Vulkan driver spins inside waits, which dilutes the Vulkan percentages, so the decision uses the DX12 numbers. The prominent `queue.write_buffer` in `host-textures-64` and `ui-dense-2k` (18% and 4–7%) is the bench's own GpuView previews writing their uniforms every frame, not framework uploads.

**Why not.** The ceiling is about 1% of CPU in frames that upload, and nothing in still frames. The price is a new class of bug: the CPU writing memory the GPU may be reading, with correctness resting entirely on the frame-slot fences and out of reach of wgpu-core's validation. The fork would have to change wgpu-core's map state, its rejection of mapped buffers at submit, initialization tracking, and the rule that `MAP_WRITE` cannot share a buffer with `STORAGE`/`VERTEX` (today only the native `MAPPABLE_PRIMARY_BUFFERS` feature lifts it). On a discrete GPU without ReBAR every draw would read instance data across PCIe, turning saved CPU into GPU time. If uploads ever become hot, start on NanaUI's side: make the text staging copy from the frame's `UploadRing` chunk (it is unmapped before the submit, so copies in the frame encoder may read it), removing one staging allocation per frame; and find the cause of the 512 KiB cell.

Commands (repository root; S is a temporary directory):

```sh
cargo build -j 12 --release -p nana-gpu --example upload-cost
WGPU_BACKEND=dx12 WGPU_ADAPTER_NAME=nvidia target/release/examples/upload-cost.exe   # also vulkan, intel
UPLOAD_COST_ONLY=ring,8,65536 target/release/examples/upload-cost.exe                # one cell, ten times as long
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 12 --release -p nana-ui --bin nana-text-paint-benchmark --features gpu
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 4000 --duration-ms 8000 --interval-us 2000 \
  --out $S/dx12-text-mutate-1pct.folded -- target/release/nana-text-paint-benchmark.exe \
  --labels 10000 --workload mutate-1pct --frames 2000
python perf/stack_attribution.py --scope "upload::record" $S/dx12-text-mutate-1pct.folded
```

The `gpu-scene-*` sampling commands are as in §6. `nana-text-paint-benchmark` now picks its backend from `WGPU_BACKEND`.

### 8. Measured: partial present (2026-11-11)

What a caret-blink or hover frame costs on DX12, and the design and staged plan for presenting with dirty rectangles and tracking damage, are in [Partial present and damage tracking](./partial-present.md). Verdict: not now.

### 9. Submission budget: an app cannot stall the desktop

The device has one queue, and the desktop compositor (DWM on Windows) waits behind the same GPU. A process that queues seconds of work freezes the whole desktop; an early unthrottled probe did exactly that. `GpuContext` therefore bounds unfinished submissions per **lane** (`FrameLane`):

| Lane | Entry points | Limit | Who waits |
| --- | --- | --- | --- |
| `Presentation` | `begin_frame` / `try_begin_frame` | the frame slot count (default 3) | only on its own lane's oldest submission |
| `Producer` | `begin_producer_frame` / `try_begin_producer_frame` | `GpuDeviceState::set_producer_submission_budget`, default **2** | only on the producer lane's oldest submission |

- **Producers** (video, models, offscreen work behind a `CustomRenderNode`, `FrameExchange` copies) use `begin_producer_frame`. It takes no frame slot, so however busy producers are, window frames still get theirs. With the budget used up, `begin_producer_frame` blocks until the lane's oldest submission finishes, and `try_begin_producer_frame` polls once without blocking and returns `None`. `FrameContext::submit` checks the budget again **before** taking the `submission_order` lock (so producers recording concurrently cannot overshoot it), and a wait never holds up another lane's submit. `FrameExchange::copy_from` uses the producer lane and returns `CopyOutcome::PoolFull` when the budget is used up.
- **Host frames** are paced by frame slots. When every slot is held by an unsubmitted recording, a frame runs without a slot (`gpu.frame_slots_exhausted`); such frames are still held to the same limit and wait on their lane's oldest submission before submitting instead of piling up.
- The count is conservative: a submission leaves only once a completion callback or a wait proved it finished, so the real queue depth never exceeds it. `GpuPolicyStats` reports `producer_submission_waits`, `producer_submissions_peak` and `presentation_submission_waits`; the `gpu.submission_budget_waits` metric counts waits.
- Default 2: one executing and one ready behind it keeps a producer busy without letting it queue several frames of work ahead of the compositor. Producers with very light frames that need more overlap may raise it; producers with heavy frames should stay at 1–2.

**What the budget does not bound** (stated plainly):

- Raw `queue.submit` through `wgpu-interop`, and `GpuContext::flush_uploads`, are not counted. The escape hatch hands out the queue as is; its caller does its own throttling (at most 1–2 in flight; see the GPU load rule in `AGENTS.md`).
- Queue depth cannot bound a **single** draw or dispatch that runs for seconds. Windows resets the device through TDR after about 2 seconds; a submit whose wait outlives 5 seconds goes ahead and reports `gpu.submission_budget_overrun` rather than hanging its thread. Once the device is lost waits end at once, and the host replaces the `GpuContext` per `is_lost()`.
- A lost device is reported by `poll`, not a panic: the fork's `wgpu::Device::poll` returns `PollError::DeviceLost` when the backend reports the device lost (Vulkan's `VK_ERROR_DEVICE_LOST`; on DX12 a fence reading `u64::MAX` while `GetDeviceRemovedReason` reports the device removed). It also marks the device invalid, runs the device-lost callback once, and counts every submission as finished, so pending completion callbacks run; every later `poll` returns the error again (see `third_party/wgpu/README.md`). The framework's polls and waits (frame slot waits, the submission budget, `GpuContext::poll`, the scene host's and `FrameExchange`'s polls) go through the `GpuContext`, which marks itself lost on that error, even a context adopted with `from_wgpu` that installs no loss callback; the upload ring's own polls ignore it and leave it to the context's next poll. Tested with the fork's Vulkan test hook and D3D12's `RemoveDevice`; not verified against a real TDR.

Measured (2026-11-11, RTX 5060 Ti, DX12, Mailbox, `surface-resize-probe --frame-context --resizes 200`; the producer submits back to back with no wait of its own, bounded only by budget 2; 20 s cap per run, load stepped up one level at a time, desktop responsive throughout):

| Load | Window frame p50 / p95 / max | Producer peak in flight | Producer waits | Window lane waits |
| --- | --- | --- | --- | --- |
| no producer | 4.0 / 6.5 ms (one 522 ms outlier, unrelated to producers) | 0 | 0 | 0 |
| `--load 8` | 5.3 / 8.2 / 10.1 ms | 2 | 1043 of 1047 submits | 0 |
| `--load 16` | 6.3–6.4 / 9.2–9.6 / 15.9–33.7 ms | 2 | 618–626 of 620–628 submits | 0 |
| `--load 16`, raw wgpu producer (waits after each submit) | 8.9–9.3 / 50.6–55.2 / 108–109 ms | 1 | — | — |

```bash
cargo run -p nana-ui --example surface-resize-probe --features gpu,wgpu-interop --release -- --frame-context --load 4
```

`--load` is capped at 16 and `--resizes` at 200; never run two GPU probes at once.

### 10. Measured: CPU meshing of vector paths (2026-11-11)

The question: could vector paths be drawn directly on the GPU, without converting them on the CPU first? Today's route (`custom_paint.rs` in `nana-ui-scene`) flattens curves, lets `i_overlay` resolve the fill rule, stroke outline and clip intersection, then tessellates with `lyon` and adds an anti-aliasing fringe. The painter's `MeshPipeline::push_path` then transforms the vertices into scene space, and `PathBuffers::upload` uploads them. None of the §6 scenes draws a path, so this fills that gap. This is measurement only: nothing on the product frame path changed.

**Scenes.** Two new scenarios for `nana-gpu-scene-benchmark` (`--features gpu,rich-text`):

- `gpu-scene-paths-canvas`: one 1200×720 custom-paint canvas of 96 cells. Each cell has a closed cubic fill, a round-capped curve stroke (dashed in every fourth cell) and a miter-joined triangle outline. Every fifth cell adds an even-odd ring, every eighth a star with a shadow, and every third has its contents inside a circular path clip. That makes 332 paths, one mesh each, about 135k vertices.
- `gpu-scene-paths-mermaid`: one Markdown document with a mid-size flowchart (23 nodes, 29 edges, two subgraphs) and a sequence diagram (5 participants, 16 messages, a loop, notes). 156 meshes, about 120k vertices.

Each scene runs four cases, chosen with `--path-animation`:

- still (`none`);
- scroll (`scroll`): the list moves up 1.7 px a frame;
- compositor scale (`scale`): the list scales between 0.9× and 1.1×;
- re-record (`rerecord`): the canvas redraws at a new zoom every frame, so its `paint_key` changes; the Mermaid source changes one comment line every frame, a diagram edited live.

Each frame the bench reports in `path_counters` how many meshes the scene holds, and how many of them are not the same `Arc` as the frame before, that is, were meshed again.

**Method.** The machine is the one in §6 (RTX 5060 Ti, Core Ultra 7 265K), on DX12, with a release build and `CARGO_PROFILE_RELEASE_DEBUG=limited`. `--serialize` waits for each frame's GPU work after submitting it, so only one submission is ever in flight. Each case ran 3 times. Each run first went 5 s unsampled, for `framework_cpu_prepare_ms` (Runtime flush plus painter recording), then 6 s under §6's `stack-sampler` at 2 ms. CPU per frame is the main thread's CPU time in the sampling window divided by the frames in that window. `perf/stack_attribution.py --path-stages` charges each non-blocked sample to exactly one stage by its stack; inside the build it checks `i_overlay`, then `lyon`, then flattening, then the fringe. Tables give the median of the 3 runs, with min–max in brackets.

**Machine load.** An ML training job ran on this machine throughout. GPU utilisation logged at the start of each run was 27–98%, mostly above 90%. It used 12.8 GB of VRAM, and about 25 GB of the 47 GB system RAM was in use. Every quantity in the tables is in-process CPU time. Frame rates include the serialized wait for the GPU, which the training job distorts: they are context, not evidence. No GPU timings were taken.

| Scene | Case | Meshes rebuilt per frame | prepare p50 ms | CPU per frame µs | Build µs | Transform + upload µs | Upload per frame |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| canvas | still | 0 / 332 | <0.01 | 182 [160–218] | 0 | 0 | 0 |
| canvas | scroll | 0 / 332 | 3.8 [3.7–4.0] | 6745 [6441–7093] | 0 | 6059 [5715–6394] (90%) | 9.2 MB |
| canvas | compositor scale | 0 / 332 | 4.5 [4.5–4.9] | 7789 [7403–8937] | 0 | 7170 [6511–7981] (89%) | 9.1 MB |
| canvas | re-record | 332 / 332 | 33.2 [31.1–34.4] | 34976 [33555–36438] | 25769 [23759–25793] (71%) | 8454 [7561–8903] (24%) | 9.3 MB |
| mermaid | still | 0 / 156 | <0.01 | 246 [244–263] | 0 | 0 | 0 |
| mermaid | scroll | 0 / 156 | 4.1 [4.0–4.3] | 6100 [6011–6547] | 0 | 4880 [4775–5136] (79%) | 8.1 MB |
| mermaid | compositor scale | 0 / 156 | 3.9 [3.9–4.3] | 7074 [6894–7168] | 0 | 6312 [5943–6357] (88%) | 8.0 MB |
| mermaid | re-record | 156 / 156 | 24.2 [23.9–24.8] | 23392 [22986–26825] | 17419 [16940–19630] (74%) | 2908 [2719–2950] (12%) | 0 |

"Build" is the sum of four stages: flattening, `i_overlay`, `lyon` and the fringe. "Transform + upload" is the sum of three: `push_path`, `PathBuffers::upload` (block-wise compare and write), and the copy into the upload ring at submit. Percentages are shares of CPU per frame. When Mermaid re-records, its geometry stays the same, so the new vertices are byte-identical to the previous frame's and the compare uploads nothing.

The build stages when re-recording (µs per frame; share of CPU per frame in brackets):

| Scene | Flatten | `i_overlay` | `lyon` tessellation | AA fringe / shadow band | Other | Diagram parse + layout | Record |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| canvas | 318 (0.9%) | 16321 (45%) | 5892 (17%) | 2843 (8%) | 327 (0.9%) | — | 59 |
| mermaid | 136 (0.6%) | 8713 (37%) | 4435 (19%) | 3832 (16%) | 348 (1.5%) | 578 (2.3%) | 77 |

The transform and upload stages when scrolling:

| Scene | `push_path` µs | Compare + write µs | Copy into upload ring µs |
| --- | ---: | ---: | ---: |
| canvas | 1769 | 1866 | 2484 |
| mermaid | 1439 | 1717 | 1778 |

- **Still: the second frame reuses everything.** No mesh is rebuilt, and the painter doesn't touch paths at all: prepare takes under 0.01 ms and nothing is uploaded. Recordings are cached by (`paint_key`, layout size, theme epoch) and the scene caches meshes per recording; both caches hit.
- **Scroll and compositor scale: nothing is meshed again, but every vertex moves every frame.** No mesh is rebuilt, under compositor scale either. Even so, 79–90% of these frames' CPU (4.9–7.2 ms) goes into moving vertices. Each frame, `push_path` transforms every vertex into scene space on the CPU. `PathBuffers::upload` then compares and rewrites the whole vertex array, about 8–9 MB, and submit copies it again into the upload ring. The indices don't change and aren't re-uploaded. This cost has nothing to do with meshing, and fixing it doesn't need GPU coverage: keep the vertices on the GPU in node-local space and pass the origin and affine per draw, as retained text entries already do.
- **Re-record: the build dominates.** It costs 17–26 ms a frame: about 78 µs per path on the canvas and 112 µs per Mermaid mesh. `i_overlay` is the largest stage (37–45%), then `lyon` (17–19%) and the fringe (8–16%). Flattening is under 1%. Mermaid's parse and layout are only 2%.
- **Meshes are heavy.** The average canvas mesh has 406 vertices and the average Mermaid mesh 767. The causes are the 0.05 logical-px flattening tolerance, round joins and the fringe. The vertex count directly sets how much the scroll case moves.

**Verdict: no-go on replacing flatten + `i_overlay` + `lyon` with GPU-side analytic coverage.** Either a per-pixel coverage shader or Vello-style compute would have had to clear one of these thresholds:

1. **Steady-state re-meshing over 0.5 ms a frame** (3% of a 60 Hz budget) in still, scroll or compositor-scale frames. Measured: 0. Not met.
2. **A supported workload that re-records paths every frame and still spends over 4 ms a frame building** (a quarter of a 60 Hz budget) after the cheaper CPU fixes 2 and 3 below. The raw build, 17–26 ms, clears the cost bar. But it only happens when content re-records continuously: a canvas that puts zoom or animation into its `paint_key`, or a diagram re-parsed on every keystroke (usually throttled well below 60 Hz). None of the framework's own paths re-records every frame; Markdown diagrams re-record only when their source changes.
3. **A first-frame build over 50 ms** for an ordinary document. Measured: a one-off 17–26 ms. Not met.

GPU coverage is not free either. Curves still have to be uploaded, and strokes, dashes and path clips still have to be handled. Vello-style compute needs compute shaders, which wgpu's GLES backend only has from GLES 3.1. It also moves the work onto a GPU that, on machines like this one, other jobs already keep busy.

Cheaper fixes with a surer payoff, in order:

1. Keep path vertices resident on the GPU in node-local space and pass the transform per draw. This saves 79–90% of scroll and compositor-scale frame CPU without touching meshing.
2. Let paths that `lyon` can handle directly skip `i_overlay`: simple fills with no clip, and strokes through `lyon`'s `StrokeTessellator`. `i_overlay` is 37–45% of the build.
3. Revisit the flattening tolerance and round-join density (406 and 767 vertices per mesh).

Revisit GPU coverage if, after fixes 2 and 3, a product still needs to re-record more than about 50 paths a frame (about 4 ms at 78–112 µs per path).

Commands (repo root; S is a scratch directory; only one GPU process at a time):

```sh
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 4 --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu,rich-text
cargo build -j 4 --release -p nana-ui --example stack-sampler --features hosted,bundled-fonts
# Per scene (canvas | mermaid) and case (none | scroll | scale | rerecord), 3 runs each
WGPU_BACKEND=dx12 target/release/nana-gpu-scene-benchmark.exe --scenario perf/scenarios/gpu-scene-paths-canvas.json \
  --path-animation scroll --serialize --sample-seconds 5 --output $S/plain-canvas-scroll.json
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 3000 --duration-ms 6000 --interval-us 2000 \
  --out $S/canvas-scroll.folded -- target/release/nana-gpu-scene-benchmark.exe \
  --scenario perf/scenarios/gpu-scene-paths-canvas.json --path-animation scroll --serialize --sample-seconds 9 \
  --output $S/sampled-canvas-scroll.json
# CPU µs per frame = stack-sampler's thread_cpu_ms × 1000 ÷ (bench frame rate × wall_ms ÷ 1000)
python perf/stack_attribution.py --path-stages --cpu-us-per-frame 6745 $S/canvas-scroll.folded
```

## Mounting host textures (`HostTexture` + `GpuTextureView`)

Shader output, preview viewports, and offscreen render textures are nodes in the UI tree just like ordinary buttons: they take part in layout, are clipped, and can be hit. For a first integration, use the default path: draw the content into a sampleable texture, mount a `GpuTextureView` in the tree, and register it in `host_textures()` under the same string slot name.

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop
```

`run_runtime` creates the one `GpuContext` for the process (additional windows only create a new Surface and share that context; an external event loop injects an existing device through `EmbeddedRuntime` and `HostedGpuShared::from_device` and must never call `request_device` again).

When the host creates or restores the shared device, it automatically enables the BC/ASTC texture compression features the adapter supports, so content producers can sample compressed textures on the same device. This does not change the UI's own texture storage policy. Unsupported features are not requested; producers choose formats or fall back according to the device's actual capabilities. Externally injected devices and application offscreen hosts must request the compression features they need themselves when creating the device.

:::api

```rust view
// In the tree: takes layout space just like a Button
let preview = cx
    .mount_view_root(document_id, || view! {
        <Texture resource="preview" />
    })?
    .root::<GpuTextureView>();
```

```rust rust
// In the tree: takes layout space just like a Button
let preview = cx
    .mount_view_root(document_id, || widget(GpuTextureView::new("preview")))?
    .root::<GpuTextureView>();
```

:::

Register the texture slot in a `RuntimeProgram` and update it per frame:

```rust
// 1. initialize / rebuild_gpu: create the texture on the host device and register the slot
let gpu = context.gpu();
let texture = gpu.create_texture(&GpuTextureDescriptor {
    label: Some("preview"),
    width,
    height,
    format: GpuTextureFormat::RGBA8_UNORM_SRGB,
    usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
})?;
self.textures.register(
    "preview",
    HostTexture::new(&texture),
    width,
    height,
    HostTextureAlphaMode::Opaque,
);

fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
    Some(self.textures.clone())
}

// 2. Update pixels or replace the texture before flush
fn prepare_window_frame(&mut self, id: WindowId, context: &RuntimeProgramContext<Self::Message>) {
    // Upload CPU pixels: context.gpu().write_texture(&texture, region, &pixels, bytes_per_row)
    // Replace the texture object or size: host.replace_texture(&new_texture); content-only update: host.invalidate()
    // Note: never modify Runtime nodes to swap a texture; this may still be called when the window is occluded or zero-sized, so do not assume a present follows
}

// 3. Only after present may the old texture replaced in the previous frame be released
fn window_frame_presented(&mut self, _id: WindowId, _context: &RuntimeProgramContext<Self::Message>) {}

// 4. Device loss recovery: the program instance is kept; only rebuild GPU resources on the new context.gpu()
fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {}
```

- **The slot is the only identity**: a view records only the slot name (`TextureSlotName`), and each painted frame looks up the slot's current binding in the registry by name. `HostTexture::new` allocates its own id. Whether you register a new handle in the same slot, swap the texture object with `replace_texture` (`generation` increments), or refresh content with `invalidate` (`version` increments), the host redraws the windows that use the slot, with no change to Runtime nodes. On device rebuild, the host clears the registry first and then calls `rebuild_gpu`; the program just registers again on the new device. If `HostTexture::device_generation()` does not match the current painter's device generation, the whole frame is rejected with `ScenePaintError::StaleHostTexture`.
- **Linear working space requirement**: `HostTexture` samples entering the Scene must be in premultiplied linear scRGB. Ordinary opaque or straight-alpha sRGB images can use `*_SRGB` texture formats and let the hardware sampler decode them. For translucent premultiplied content, use linear `UNORM` or floating-point formats; do not mislabel gamma-space premultiplied bytes as `SRGB`.

### GPU contract and the wgpu escape hatch

#### Logical GPU ABI (Issue #185)

Ordinary render extensions can declare uniform/storage buffers, sampled/storage textures, samplers, dynamic slices, and optional resource arrays through `nana_gpu::ResourceTable`, and bind portable WGSL with vertex/instance layouts through `ShaderInterface`. (`GpuContext::create_resource_layout` and `create_resource_group` validate `DeviceGeneration`, usage, ranges, and optional capabilities before creating opaque handles; the public API never exposes `wgpu::BindGroupLayout`.)

| Core contract type | Responsibility |
| --- | --- |
| `GpuContext` | The process's single GPU device handle: carries `generation()`, `capabilities()`, and the sticky loss state `is_lost()`, and provides `create_texture` / `write_texture` / `begin_frame` |
| `FrameContext` | An exclusive per-frame encoder: `submit()` submits and returns a `GpuSubmission`; drop / `discard()` invalidates it immediately and automatically rolls back the retained target state the painter registered on that frame |
| `GpuTexture` / `GpuRenderTarget` | 2D textures and render targets carrying a `DeviceGeneration` (resources from another generation are rejected at the entry point) |
| `GpuTextureFormat` / `GpuTextureUsages` | Backend-neutral opaque format and usage flags |

When a custom renderer really needs the underlying native WGPU objects, enable the `wgpu-interop` feature explicitly. It provides `GpuContext::from_wgpu` / `wgpu()`, `FrameContext::wgpu_encoder()`, `GpuTexture::from_wgpu` / `wgpu_view()`, `ScenePass::wgpu()`, and more, and hands out **the same** device and encoder behind the contract. Internal framework crates (`nana-ui`, `nana-frame-exchange`, `nana-ui-devtools`) reach the backend through the hidden `nana_gpu::__framework`, enforced in CI by `scripts/check-engine-boundary.py`.

::: tip Queue work needs no submission guard
NanaUI's wgpu fork makes `Surface::configure` (a window resize) wait only for the work that still uses that surface's swapchain images, not for the whole device to go idle, and another thread submitting at the same time no longer makes it fail with `GpuWaitTimeout` (see `third_party/wgpu/README.md`). `third_party/wgpu-fork-tests` checks this on Windows: it reconfigures a window's surface on DX12 and Vulkan while another thread submits. So raw `queue.submit` / `write_texture` / `write_buffer` calls through `wgpu-interop` may run on any thread without a lock; the former `gpu.wgpu().lock_submission()` is deprecated and its guard no longer excludes anything. Inside `GpuContext` only the `submission_order` mutex remains, keeping frame submits, upload batches and native-export signals in queue order.
:::

#### Native texture export (`native-export`)

An external producer that creates its own WGPU device and has no existing device-lost callback uses `GpuContext::from_wgpu_with_loss_tracking` to hand the callback explicitly to the same `GpuContext`; all clones share `is_lost()` / `lost_report()` state. The original `from_wgpu` keeps the host's callback and does not observe loss on the host's behalf; existing hosts still forward it to the embedded runtime themselves. The OpenVR / Electron reference producers use the former, so they never claim to check `is_lost()` without having installed the notification. The `controlled_device_loss` integration test actually calls `device.destroy()` to verify callback ownership, sticky loss, and rejection of the replaced generation; this does not mean recovery from a hardware driver reset or from an external compositor has been tested.

On a Windows + DX12 device, enabling the `native-export` feature (capability bit `GpuCapability::NativeTextureExport`) lets `NativeExportPool` create 3 shared `BGRA8` textures and one shared fence on the host's own device and export them as NT handles, so external D3D11 / Spout consumers can sample across APIs with zero CPU readback. For the full fence timing (`2k + 1` ready, `2k + 2` release) and the consumer contract, see [Off-Window Output · DX12 shared textures](output.md#dx12-shared-textures-native-export).

## Cross-thread latest-frame exchange (`FrameExchange` and `FrameBinding`)

When live content is produced on a separate thread (such as model rendering, broadcast compositing, or video decoding), use `FrameExchange` from `nana-frame-exchange` to hand completed frames to the window, and bind them to a slot on the window side with `FrameBinding` (the producer only needs to depend on `nana-frame-exchange`):

```rust
// Producer thread: call poll on every tick; when there is a new frame, record it and copy_from
let mut exchange = FrameExchange::new(&gpu, frame_exchange::DEFAULT_CAPACITY, epoch, notify);
let inbox = exchange.inbox(); // send to the UI thread

let mut frame = gpu.begin_producer_frame("producer");
scene.render(frame.wgpu_encoder()); // wgpu-interop
frame.submit();
match exchange.copy_from(&rendered, epoch) {
    CopyOutcome::Submitted => {}
    CopyOutcome::PoolFull => {} // pool full: drop this frame without blocking
    CopyOutcome::EmptySource | CopyOutcome::IncompatibleSource | CopyOutcome::DeviceMismatch => {}
}
exchange.poll();

// UI window thread
let mut binding = FrameBinding::new(
    context.gpu(),
    textures.slot("program"),
    HostTextureAlphaMode::Premultiplied,
);
// Call in prepare_window_frame: binding.prepare(Some(&inbox), |token| accept(token));
// Call in window_frame_presented: binding.presented(Some(&inbox), |token| accept(token));
```

- **GPU-side copy and lock-free exchange**: `copy_from` performs one `copy_texture_to_texture` inside the texture pool (the source texture must have `COPY_SRC`, be single-sampled 2D, and be a non-depth format). For a single consuming window, allocate `3` slots (copy in flight, currently displayed, replaced and awaiting present); add `2` slots for each additional window sharing the consumer.
- **Lease order and epoch filtering**: after `prepare` swaps in a new frame, the old frame is kept until `presented` and only then returned; the producer reuses that slot only after the UI submission that sampled it completes. An application can advance the scene generation with `set_epoch` (immediately hiding frames of the old generation in the inbox; while no frame of the new generation has been published and `accept` still allows old frames, `FrameBinding` keeps the displayed content instead of flashing back to transparent blank).

## GpuView: same-pass direct drawing without an intermediate texture

When content has no intermediate texture and must be encoded directly into the current UI render pass, use `GpuView::new(slot_id)` (it projects a `CustomRenderNode` with renderer key `"gpu-view"`; see the `gpu-view-demo` example):

- **`Inline` vs `Standalone`**: `GpuViewMode::Inline` reuses the target pass the painter currently has open (setting viewport and scissor through `ScenePass`); `Standalone` opens a separate `Load / Store` pass on the same frame and target through `SceneGpuRenderContext::with_pass`. A custom renderer's output color must be **premultiplied linear scRGB**, and its pipeline cache key should be `(context.gpu.generation(), context.target_format, context.presentation)`.
- **Instanced batching that preserves document order**: if you implement `batch_capacity() > 1` and `draw_batch_in_pass`, the painter hands it a slice of **directly adjacent** nodes of the same renderer in the display list to draw at once. As soon as any ordinary Quad, text, icon, HostTexture, or composition group boundary appears in between, the slice ends, so occlusion order is bit-for-bit identical to drawing node by node. When several nodes share the same shader, use the same `(slot_id, revision)` and put per-node differences in `params`.
- **Implement `preparation_version` to allow frame caching**: custom renderers re-prepare every frame by default. **As long as one custom renderer in the tree does not implement `preparation_version`, the painter marks the whole frame as uncacheable and rebuilds all draw commands.** Implement it by `(revision, params)` as the built-in `DefaultGpuViewRenderer` does.

## Media and GPU slot contract

| Node / interface | Underlying slot contract | Behavior |
| --- | --- | --- |
| `GpuTextureView` | `"nana.host-texture"` + the slot name registered by the host | Standard host texture node |
| `GpuView` | `"gpu-view"` + decimal `slot_id` | The host must register a matching `SceneGpuRenderer` |
| `Video` | `"nana.host-texture"` + `video:{id}` | The host pushes frames (with a slot, `poster` is not drawn; without a slot only the static `poster` image is shown, and no audio/video is decoded) |
| `BrowserView` | Runtime layout anchor + host native child view | macOS `WKWebView` only (Windows/Linux return `Unsupported`); see the next section |

### Preparing content at the actual painted pixel size

`HostTextureRegistry::painted_extent(slot)` returns the largest device-pixel size the slot needed in the most recent complete paint of each render target (accounting for `ContentFit`, the node's own transform, and the current DPI scale factor; when the same slot is referenced by both a list thumbnail and a large image overlay, the largest consumer size is taken automatically). Producers can watch for size requirement changes through `painted_revision()`, `painted_changes_since(rev, &mut out)`, or `subscribe_painted(callback)` and prepare textures at the real physical resolution. Never compute it yourself as "last frame's `LayoutBox` × scale factor".

## In-app browser

`BrowserView` is a native child-view exception of the window host, not a full-window WebView shell: the Runtime node owns layout, accessibility, and the visibility anchor, while the host manages the native engine child view (currently only macOS `WKWebView` is supported, and it does not appear in offscreen Scene screenshots).

### Headless web content (WebSurface)

`WebSurface` runs a platform web engine instance entirely outside the window view tree, captures page frames at `max_fps` (`WebFrame`: tightly packed RGBA8, premultiplied alpha in sRGB space), and delivers them on a background thread to an application-provided `WebFrameSink`, with zero pixel conversion and zero queuing overhead on the UI thread. The application decides where frames go (for example, writing them into a `HostTexture` slot or a custom compositor):

- **Request reconciliation and in-place updates**: a program declares `WebSurfaceRequest { id, policy, desc, restore_url, revision, command, frames }` in `RuntimeProgram::web_surface_requests()` (withdrawing the request releases the engine), and status events flow back through `web_surface_event`. Changing `desc` (size, scale, transparency, frame rate) takes effect in place; changing the sink or the security policy rebuilds the instance.
- **Temporary interactive window (`ShowWindow`)**: the same headless page can be placed temporarily into an interactive native window (for completing a web login or clicking in-page settings). Cookies and page state are preserved and frames keep flowing meanwhile. After the user closes the window or `HideWindow` is sent, the page returns offscreen and reports `WindowClosed`.
- **Platform backends**: macOS is based on an offscreen borderless `WKWebView` (it detects 10 consecutive static frames and throttles to 4 fps, resuming immediately when content moves; measure it with `cargo run -p nana-window --example web-surface-probe`). Windows is based on a WebView2 Composition Controller on a dedicated STA thread + `Windows.Graphics.Capture` (requires the WebView2 Runtime installed on the system and `WebView2Loader.dll` shipped with the package). On Linux, `web_surface_support()` returns `false`.

## Per-scene offscreen production

When an offscreen texture must be encoded dynamically from the current Scene graph **before** it is sampled, still mount a `GpuTextureView` in the tree, and implement `SceneResourceProducer` on the host side:

- **Batched submission for visible frames**: `encode_scene(scene, &mut frame)` records directly into the same `FrameContext` the host opens after acquiring the Surface, and is submitted with the UI drawing in the same `queue.submit` (if either encoding or painting fails, the whole frame is discarded; producers must never submit the encoder themselves or report completion early).
- **Hidden tick**: when the window is occluded or minimized, a due `FrameDemand` still calls `prepare_window_frame`, runs `scene_resource_producers` on a separate Surface-less `FrameContext`, and submits immediately (at zero size it only prepares and does not run producer encoding).
- **One target per frame rule**: `FrameContext` exclusively manages a frame's submission and its discard rollback. If a previous frame painted some `RenderTargetId` and has been neither submitted nor discarded, painting that target again returns `ScenePaintError::TargetInFlight`. Device loss is recorded in the sticky `GpuContext::is_lost()`. When a device is injected externally through `EmbeddedRuntime`, the host should call `EmbeddedRuntime::notify_device_lost()` on receiving a device loss notification, and call `replace_gpu()` once the new device is ready.

### Preparation caching and content revisions for GPU nodes

`SceneGpuRenderer::preparation_version` only controls reuse of UI command preparation. A node that draws according to time or external state may change pixels every frame even when its resources and geometry do not; such renderers should return `None` from `content_version`. Window retained output, the off-window PresentationDriver, and Spout shared output redraw on host request accordingly, while still reusing the stable prepared commands.

Static or explicitly revised renderers can return `Some(revision)` and increment it when pixels change. To preserve existing consumer behavior, `content_version` defaults to `preparation_version`; dynamic renderers need to distinguish the two explicitly. A dynamic renderer that is installed but not referenced by the current scene does not keep static output painting continuously.

For general text and shape fragment materials, see [General Materials](materials.md).

### Per-frame application payloads

The `M` in `FrameExchange<E, M = ()>` is an immutable application payload published together with the GPU copy, separate from the epoch and `FrameToken<E>` identity. Construct it with `new_with_payload`; `copy_from_with_payload` (or `copy_from_wgpu_with_payload` under `wgpu-interop`) submits the texture, color metadata, and payload together. `FrameLease::payload()` and `FrameBinding<E, M>::payload()` return only the data of the corresponding lease / currently bound frame. The existing payload-free constructors and copy interfaces remain compatible.

The payload is kept together with the texture through pending copy, completed publication, binding, and retirement after presentation; dropped frames, generation invalidation, and slot recycling follow the existing rules. Updating a payload does not switch the epoch, does not clear the latest frame, and adds no history index or texture pool growth. Consumers must not pair an old texture with separately read latest producer state. The payload type is defined by the application; the framework does not depend on the application's coordinate or model types.
