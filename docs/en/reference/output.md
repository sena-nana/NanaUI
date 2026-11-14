# Off-window output (ExternalSurface)

`nana_ui::output` is the presentation boundary after `UiScene`. It is available with the `gpu` feature and does not require `hosted`, winit or an OS window. The device, frames and submission stay under the host's same `GpuContext` / `FrameContext`, and every UI draw still uses `SceneWgpuPainter`.

Ordinary window-only presentation keeps painting directly into the Surface target the host acquired. Adding compatible consumers shares one persistent canonical output; incompatible size, pixel density, text antialiasing, alpha or color contracts form separate draw groups. The framework has no implicit CPU readback/upload fallback, and enabling output adds no target texture, pass, copy or submit to an ordinary window.

## Windowless entry point and target planning

`PresentationDriver` takes the caller's `RuntimeDocument`, runs style, text, layout and incremental Scene extraction through the same flush as a window, and then produces retained output.

Visible window presentation, hidden output and the headless entry all call the internal `PresentationFrameDriver`: `prepare` chooses direct, shared canonical or a separate group and reserves targets, `encode` draws and transfers with the host's existing frame, and `finish` binds the real submission and lease. The window adapter keeps Surface acquire/present, pacing and the native transport; it no longer carries its own sharing or duplicate-draw decisions. Plain direct uses a stack-allocated target description and creates no output pool and no planner run.

```rust
let mut driver = nana_ui::output::PresentationDriver::new(&gpu);
driver.configure(&consumers)?;
driver.render_document(
    &mut document, [640.0, 480.0], host_revision,
    [0.0; 4], Some(&textures), None,
)?;
// A non-blocking poll drives completion callbacks; before completion, sample returns NoCompletedFrame.
driver.poll();
let completed = driver.sample(consumer_id)?;
```

`render_document` returns the Runtime flush result and does not guarantee GPU submission. If `driver.work().output_deferred` is greater than zero, the host should retry presentation later; `render_scene(document.scene(), ...)` retries the same Scene without repeating business mutations. `poll` alone does not submit skipped production frames. `render_scene`'s return value reports each group as `Submitted`, `Reused` or `Deferred`.

All incompatible groups share one `SceneWgpuPainter` inside the driver and record into the same frame. Each group's target state, uniforms and instance buffers are its own, and the shared glyph atlas uploads in recording order (see [Live GPU content](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md)); removing a group reclaims its targets and the pipeline sets no longer used. A `PresentationDriver` is pinned to the `GpuContext` generation it was created on: `configure` rejects consumers from another generation and keeps the old plan, `render_scene` returns an error and submits nothing after device loss, and the host creates a new driver when it moves to a new generation.

A windowless host can inject the same policy-gated HTTP(S) image transport a window uses with `set_resource_fetch_host`, and have asynchronous image completion wake its own scheduler with `set_image_update_waker`; install the waker before the first frame. Switching or clearing the fetch host cancels unfinished requests on the old egress, and old caches are never used across hosts.

`set_resource_producers(Some(Arc<SceneResourceProducerRegistry>))` feeds graph-resource production into the same output frame: every compatible and incompatible UI group shares one producer encode and one overall submit, `submitted` is reported only after the real submission, and any encoding error drops the whole frame. `SceneResourceProducer::preparation_version` defaults to `None`, keeping the existing dynamic production behavior; with a stable version provided, unchanged node input and version skip production, and a producer that is not referenced does not make the output volatile. When every output slot is saturated, `Deferred` is returned before producer encoding. `PresentationError` gains `ResourceProduction`, which exhaustive matchers must handle.

`consumers` is the resolved `OutputConsumer` list, carrying stable consumer / presentation identity, `RenderTargetRequirements` and the concrete adapter's `PresenterCapabilities`. `configure` returns `false` when nothing changed, allocating nothing and rebuilding no plan; a failed configuration keeps the old plan and the resources already completed. Adding or removing only compatible consumers keeps the original target pool and revision. One driver accepts one presentation and one logical viewport; the current general driver uses proportional `Contain` and supports grayscale, premultiplied-alpha retained targets. Direct Surface, natively renderable shared targets, subpixel and straight-alpha presentation are the matching host's responsibility, and a missing implementation returns an explicit error.

`RenderTargetPlanner` runs for real window output and in the driver. It reuses the plan when no structure changed; a content revision does not trigger topology negotiation. Its result says how a consumer reaches its target:

| Path | Current behavior |
| --- | --- |
| `DirectSurface` | Keeps direct when a compatible group holds only valid window consumers; independent windows each keep their own direct path. |
| `SharedTexture` | Consumers on the same device lease a canonical texture together, and the UI is rendered once per group. |
| `GpuTransfer` | The implemented GPU transfers: final window presentation, native export copies, and the like. |
| `NativeShared` | Available only when the adapter explicitly supports rendering straight into an exportable canonical resource; DX12 export-copy does not pose as this path. |
| `Unsupported` | An unimplemented capability or an invalid target; rejected explicitly, never silently rerouted to the CPU. |

Compatibility compares presentation, viewport mapping identity, physical size, profile, alpha, text AA, device generation and canonical ownership. The host derives `viewport_revision` from the full mapping of logical viewport, DPR, fit and origin; a content revision cannot stand in for it. A persistent requirement can raise the whole group; a native-copy transport can consume the same same-device canonical without rendering the UI again. `OutputPlan::incompatibilities` returns the concrete reason for each separate group.

SDR sRGB `BGRA8_UNORM`, `BGRA8_UNORM_SRGB`, `RGBA8_UNORM` and `RGBA8_UNORM_SRGB` can share one draw: the final transfer handles channel order and sRGB encode/decode through texture views and keeps the encoded premultiplied channel values; when a `BGRA8_UNORM` consumer exists, it is preferred as the canonical. The linear direct-write compatibility profile stays a separate group. Subpixel versus grayscale, different alpha, HDR/P3 and different pixel densities are never quietly degraded to gain sharing. Consumer targets are always single-sampled textures, and requirements have no sample-count field. Rasterization MSAA is internal to the Scene painter: the painter resolves before a consumer reads.

The original `ApplicationState` / `RuntimeApplication` / `RuntimeProgramContext` remain the compatibility entry for window applications. A windowless application is driven by `RuntimeDocument` + `PresentationDriver`; `EmbeddedRuntime`, which still depends on a winit window, is not this entry.

## Retained resources, versions and leases

`ExternalSurface` manages a bounded pool of 2–8 persistent GPU texture slots, 3 by default. Steady state advances only `content_revision`; a resize, profile or device change creates a new resource generation. Slots are recycled after a safe retirement and are not created, exported and destroyed per frame. A consumer should compare resource generation and content version and must not treat the pool slot taken for one frame as a single permanent handle.

- With unchanged content, `render` returns `Reused` before encoding: 0 UI renders, texture rebuilds, copies/blits, uploads, submits and CPU pixel moves. A consumer may sample repeatedly at 120/240 Hz and does not thereby drive NanaUI to refresh.
- New content is published only after the real GPU submission completes. `sample` gets the most recent completed version; while a new submission is still running, the previously completed version stays available.
- `ExternalFrame` can be cloned; the producer slot is released only after the last display/submission reference retires. Asynchronous sampling uses `FrameContext::keep_alive_until_complete`, or calls `ExternalFrame::retire_after` after submission. Cloning the raw `GpuTexture` keeps the allocation alive only and reserves no producer slot.
- A full slot or frame slot returns `Deferred` instead of blocking. `poll` drives non-blocking callbacks only; `device.poll(WAIT)` cannot replace the synchronization contract.
- Resources carry a `DeviceGeneration`; a wrong device, a stale plan, a stale resource generation and producer teardown are all rejected explicitly. A failed resize/configure keeps the last valid completed result.

A staged host uses `prepare` → `record` → `bind_submission`. `prepare` reserves a slot exclusively; dropping the plan releases an unrecorded reservation. After recording, the frame holds the slot until discard or GPU completion, even if the plan is dropped. `record` rejects a Scene, viewport, registry revision, alpha or device that disagrees with the prepare step; `bind_submission` accepts only the frame that actually recorded that plan. A discard publishes no content or dependency; binding or completing out of order cannot label an old key as new content.

When migrating existing staged callers, `ExternalFramePlan` is now a non-cloneable one-shot reservation: borrowed while recording, its ownership handed over when binding the real submission. An old plan cannot be cached and replayed for a later frame, and another frame's submission cannot be bound in its place.

`ExternalFrame` carries the real presentation profile, alpha encoding, and a stable `OutputSurfaceId` / completed-revision ancestry. When composing through a retained nested HostTexture, `record` checks only the slots the Scene actually references and freezes their textures, leases and provenance, so a concurrent replacement cannot bypass the check. When the sampled dependency closure already contains the current producer, it returns `CompositionCycle` and encodes no GPU work; an ordinary DAG and multi-parent consumption can share one child. This check covers causal dependencies of completed revisions; it maintains no second UI/business tree and is not a global graph analysis of every logical endpoint in the application. A low-level host that splits out raw textures, strips leases/provenance or uses a custom renderer has to maintain its own dependency and resource contracts.

The host should fold external content that is not part of the Scene revision into `host_revision`. A standard renderer registry uses the preparation version of the renderers the Scene actually references; only renderers without a stable version are treated as volatile. The cache key includes presentation / motion revisions and the real clock of active animations; repeated consumption at the same instant does not drive an animation redraw. Asynchronous image results are collected before the static-reuse decision, and completion advances the image revision, after which static reuse resumes.

The HostTexture static decision reads only the existence, instance, generation, version, size and alpha of the slots the Scene actually references, and no longer relies on a global revision of the whole registry. Calling `HostTexture::invalidate` directly also invalidates the matching output; an update to an unreferenced slot in a shared registry does not make other static presentations redraw.

## Nested output and compatibility migration

`EmbeddedSurfaceNode::from_frame` builds an ordinary HostTexture from a completed `ExternalFrame`; the parent keeps using its own layout, clipping, transform, opacity and document order. `update_frame` keeps node / HostTexture identity, updates the real child source and the resource/content versions, and re-registers the slots that still belong to that node automatically, publishing the size, advancing the registry revision and waking the parent; slots that were removed or replaced are never recreated. The same revision number from different producers is not mistaken for the same frame.

A late old revision from the same producer and resource generation returns `StaleFrame` and does not roll back the newer version already shown; switching producers still allows the same revision number.

The registry holds a display lease, and the normal HostTexture painter keeps a separate GPU completion reference for each parent-frame recording. One child can be sampled repeatedly by several parent frames and several windows; a discard releases only that recording's reference and does not end the long display lease. The old `bind_to_submission` remains as a post-submission compatibility helper; each call keeps the child alive separately, and a child still on display cannot be reclaimed early.

An explicit `detach(self)` consumes the node, atomically withdraws the registered slots that still belong to it by instance identity, and releases the node's own reference; it returns how many were withdrawn. A later slot owner is not removed by mistake, and the parent receives a revision / wake notification. Externally held bindings, HostTexture clones and recorded GPU snapshots stay alive independently until each retires safely. An ordinary Drop still allows the registry to keep showing the content; the compatibility `retire()` only stops the node's explicit frame access and is not the same as detach.

`EmbeddedOutputMetadata` carries the logical/texture size, the real profile/alpha, the source, the device/resource/content generations, a separate endpoint generation and an optional `EmbeddedInputMapping`. The input mapping anchors on the parent's authoritative node and geometry revision and creates no second coordinate tree; full input routing belongs to Issue #243. Only a premultiplied linear scRGB contract that can be sampled directly enters an ordinary HostTexture; gamma-premultiplied content, SDR not encoded as an sRGB format, and P3/HDR need an explicit conversion first and are never silently read as linear.

## Window output

`RuntimeProgram::window_output(id)` or `ApplicationWindow::output` returns a `WindowOutputConfig`; returning `None` stops output and restores window-only direct.

```rust
fn window_output(&self, id: WindowId) -> Option<WindowOutputConfig> {
    (id == TYPEWRITER).then(|| WindowOutputConfig::default()
        .with_extent(WindowOutputExtent::MatchWindow)
        .with_export(WindowOutputExport::SameDevice))
}
```

- **Shared and separate drawing.** A transparent SDR window plus output of the same size/viewport and grayscale draws the gamma-premultiplied canonical once, then does the final Surface transfer; a straight output converts separately and does not require redrawing the UI. The default output is 1920×1080 `Contain`, and an opaque window may use subpixel and a different alpha contract; when incompatible, each group draws separately, still sharing the host's submission for that frame and never laying out twice.
- **Static and throttled.** Unchanged output records no output; `max_fps` controls only the production cadence. A hidden, minimized or occluded window still drains messages and flushes under `while_hidden`, and encodes only for outputs or producers that changed; hidden static content is 0 UI renders / submits. A zero size produces no frame.
- **Fit and alpha.** `Fixed`, `MatchWindow`, `Scale` and proportional `Contain` / `Cover` are supported. `Premultiplied` uses gamma-space premultiplication suited to an external compositor; `Straight` uses a conversion target per canonical slot, and the extra pass is observable.
- **Async lifecycle migration.** `WindowOutputFrame::texture()` keeps the in-callback read entry; asynchronous consumption must take `frame.lease()`. `WindowOutputLease` can be cloned, and the caller's frame uses `keep_alive_until_complete` before sampling. The "reused after two frames" assumption is gone: a slow consumer makes the pool return `Deferred` instead of overwriting a slot still in use.
- **Slot capacity.** The window output pool has one more slot than the host's in-flight frame slots (3 + 1 = 4 by default, at most 8): the host already holds the current frame's slot before planning output, each remaining in-flight frame holds one output slot until it completes, and a published frame holds one more. With fewer, any GPU lag (Windows WARP, for example) defers output periodically under `SourceSlots`, and the window presents while the output drops frames.
- **State.** `Planned { plan }` reports the real plan only on structural changes such as consumer, size, display configuration or device; content changes are not re-announced. `Active`, `NativeUnavailable`, `NativeDeferred`, `NativeRetired`, `Failed` and `Stopped` are announced only when the state changes. Same-device output can be kept while native is unavailable; that is not a CPU fallback.

When migrating callers that exhaustively match the state or error enum, handle the new `Planned`, the cyclic-dependency / stale-plan rejections, and the native stage's `ProducerBusy`. The last means a production reservation already exists, which is different from a consumer that has not yet released resources; do not merge the two states into success or release resources on your own.

## Host adapters and capability refusal

What follows separates each consumer path from the evidence gathered locally; an interface, a compile or a mock is not the same as a measurement on an external platform:

| Consumer | Entry and local evidence |
| --- | --- |
| Window | Real `presentation-output-probe`, covering direct, shared, separate groups, detach, resize and hidden static output. |
| Nested | `EmbeddedSurfaceNode` with real child → parent pixel tests, including 120/240 parent submissions. |
| Headless | `PresentationDriver::render_document`, with no window or event loop. |
| Native host | `PresentationDriver::present_to` performs a same-device target transfer; the Windows `window-output-probe` / `native_export` covers real D3D11 shared-handle import and GPU wait/copy/release. Windows WARP CI has passed native-export and a 240-iteration native-host consumption subset; the full hardware/cross-process matrix is not signed off. |
| OpenVR / SteamVR | `openvr-output-probe` submits to SteamVR through DX12 ready/release fences, a same-adapter D3D11 shared texture and `IVROverlay_028`. By default it updates persistent textures with the official `DXGISharedHandle` atomic `CopyResource`; locally only compilation and protocol checks ran, not Windows/SteamVR. |
| Web bridge | [Electron 44.9.1 reference host](https://github.com/sena-nana/NanaUI/blob/main/examples/web-output-bridge/README.md) + `web-output-producer`: cross-process NT handle → managed `sharedTexture` → `VideoFrame` → WebGPU draw. Locally the protocol module the real main process uses is covered; the Windows/Electron GPU path did not run. An ordinary Web API cannot import an arbitrary native GPU handle, so a concrete host must install and detect a dedicated bridge; a generic GPU transport does not mean an Electron bridge is installed. |

The generic `OverlayHostAdapter` handles same-device transfer and the asynchronous consumption credentials the host provides; it neither offers nor detects a browser's native import. A host using this transport first `record`s on its own `FrameContext`, calls `bind_submission` with the same submission after submitting, and then `poll`s without blocking. The transport is called only after NanaUI's copy completes. Each transport uses its own target pool, an unreleased target returns `Deferred`, and a native single-consumer token must never be copied to several independent consumers.

`OverlayRelease` is an unsafe contract whose synchronization correctness the host must maintain: release may be reported only when the external GPU has finished reading and will not read that version again, and an adapter drop or disconnect must not release resources a foreign process is still using either. The [official OpenVR texture interface](https://github.com/ValveSoftware/openvr/blob/master/headers/openvr.h) defines texture submission, clearing and native handle release separately; this adapter does not infer GPU completion from the return value of `SetOverlayTexture`. When the required guarantee cannot be obtained, the transport returns `Unsupported` or keeps the target, and never guesses a frame count to wait for.

`OverlayAdapterError` gains `DeviceLost`. After device loss, completion callbacks are not taken as proof of valid pixels, `poll` / `record` / `bind_submission` refuse to publish to a foreign transport, and an existing unretired receipt keeps its separate resource responsibility.

Metal / Vulkan native import/export is explicitly unsupported where no complete safety contract exists; a passing same-device GPU transfer does not mean these cross-device or cross-process capabilities pass.

The OpenVR reference pins a [version of the official header](https://github.com/ValveSoftware/openvr/blob/0924064316de3effbcd1acf1e309182a2deb1c05/headers/openvr.h). By default it allocates one consumer shared texture, submits the handle once at initialization, and from then on performs atomic GPU copies under that interface's contract; the local fence proves only that the NanaUI source has been copied. Final resource retirement by the compositor is not proven, so consumer resources are kept until the process exits. The reference sets the premultiplied alpha flag explicitly; the current maintenance workload is fully opaque, which is not a visual sign-off of transparent edges. `--immutable-snapshots` is a conservative mode that defers after three bounded slots and is not the performance contract for continuous updates. The return value of `SetOverlayTexture` is never taken as completion of the lease.

The Electron reference sends the handle after non-blocking observation of ready on the production side. The temporary cross-process handle is closed after a successful import; the consumer must satisfy both the renderer's `queue.onSubmittedWorkDone()` and the main process's managed `allReferencesReleased` before confirming release. A failure, a disconnect or a missing credential quarantines the unfinished resources and stops new output; an IPC return or a timeout must not be used to reuse early. Only a definite missing capability returns `unsupported`; runtime validation, device loss and protocol faults return `error`. Both references are host fixtures; they introduce no WebView product shell and no CPU fallback.


## DX12 shared textures (`native-export`)

Window output supports `WindowOutputConfig::with_suspended(true)`: it submits one frame of transparent content into the same retained targets and native shared pool, then requests no further frames for hidden output. The window itself keeps showing the original scene and cannot share this rasterization with the transparent output. Restoring `false` forces a submit of the current scene, reusing resources whose size, device and pixel contract are compatible; revoking the whole configuration still retires resources immediately. Configuration revocation is synchronous with the program update and does not wait for a hidden window to receive a draw event. The host is responsible for a finite retention period while suspended and for sending transparent frames under the consumer's protocol; this interface offers no indefinite cache and no shortcut that reuses a frame once a completion timeout expires.

`nana_gpu::NativeExportPool` (with the `native-export` feature, Windows + DX12 devices only; capability bit `GpuCapability::NativeTextureExport`) holds three `DXGI_FORMAT_B8G8R8A8_UNORM` submission textures created with `D3D12_HEAP_FLAG_SHARED` and the render-target flag (without that flag D3D11 refuses to open a shared D3D12 texture), exports each as one NT handle, and pairs them with a shared fence. They are wrapped as `COPY_DST`-only textures on the host's own device, and there is no second device or queue:

- `stage`: records one copy from the rendered texture into the next slot in the caller's current frame;
- `FrameContext::submit`: under the submission-order lock, signals the frame's *ready* fence value on the device queue right after `queue.submit`;
- `finish`: wraps the submitted copy as a `NativeFrameToken`. Window output configured with `WindowOutputExport::Native` runs this sequence for every frame it produces.

The `k`-th successful stage reservation (counting from 0) uses ready value `2k + 1` and release value `2k + 2`. A discard, or a reservation discarded after submission without `finish`, also consumes a number, so later values may skip; a consumer must use the value the token gives and must not derive it from `issued()` or the number of delivered frames. Otherwise a discarded submission could let the next ready wait pass early. The consumer contract is:

1. **Open the pool per `token.pool_generation()` (once per generation)**: create a D3D11 device on the adapter named by `token.adapter_luid()` (`IDXGIFactory4::EnumAdapterByLuid`), call `ID3D11Device1::OpenSharedResource1` for every `token.texture_handle(slot)` with `slot < NATIVE_EXPORT_SLOTS`, and call `ID3D11Device5::OpenSharedFence` for `token.fence_handle()`. The handles stay valid as long as any token of that pool is alive; the opened objects belong to the consumer.
2. **Consume each frame's token**: call `token.accept_release()` first, then run, in order, `ID3D11DeviceContext4::Wait(fence, token.ready_value())` → read `textures[token.slot()]` (for example `CopyResource` into a sender texture) → `Signal(fence, token.release_value())` → `Flush` on the consumer's context.
3. **Signal release only on the GPU timeline after the Wait.** If release is signalled early, before the producer's ready signal lands, the producer's signal pulls the fence value back when it arrives and the pool hangs (defers) for good.
4. **A token dropped without calling `accept_release`** is released automatically by the producer on its own queue, after the ready signal; a consumer can drop a token to skip a frame.

There is no CPU wait anywhere. While the consumer has not released the previous frame, `stage` returns `Deferred(ConsumerBusy)` and records no commands; window output then reports `NativeDeferred` and keeps producing same-device texture frames. If the consumer has not released for more than 2 seconds, the old pool retires (reported as `NativeRetired`). The host holds `NativeExportRetirement` until the last token comes back and the consumer's GPU fence actually reaches release; dropping an accepted token is not GPU completion. Each window keeps at most two retired pools that are not yet released; at that budget it keeps reporting `NativeDeferred` and stops allocating new pools, while existing same-device output continues. A size change, an alpha/export mode switch or a device rebuild also retires the old pool, and the next token carries the new `pool_generation`. Except where `Straight` is requested explicitly, pixels are sRGB-encoded with premultiplied alpha. The export path does no CPU readback; the D3D11 readbacks in tests and `window-output-probe` are for acceptance checking only.

```powershell
cargo test -p nana-gpu --features native-export --test gpu_loss_and_export native_export
cargo run -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export
```

The tests cover: exporting a frame and opening it on a D3D11 device of the same adapter, byte-for-byte copy comparison, an unreleased frame deferring the next one, a dropped token releasing itself, dropped frames not polluting the pool, and a retired pool closing its handles after the last token is released. `window-output-probe` runs one transparent composited window through four phases (visible/hidden × animated/static) and consumes every native token on a D3D11 device. Phases advance by progress rather than a fixed duration: the animated phase ends once it has 30 frames of output, and the static phase watches for 2.5 seconds from the moment output goes idle (reusing completed frames), with frames that finish the previous phase's last content before that idle counted separately as `settle_frames`; a phase that does not finish within 60 seconds fails. The reason is that a software adapter (WARP on CI) spends seconds of GPU work on the first few frames and then manages only a few frames per second, so a fixed 2.5-second window would really be testing throughput rather than function. Pixel readback uses `D3D11_MAP_FLAG_DO_NOT_WAIT` on later frames: the readback copy waits for the producer's ready signal, and that signal may be queued behind a present that needs the event loop to keep running, so blocking `Map` on the event loop thread would deadlock the two. The perf mode (`--perf-output`) likewise judges a stall by 30 seconds without a new token and no longer caps the total duration.

Windows CI runs the native-export tests with `NANA_REQUIRE_DX12_NATIVE_TESTS=1`: a missing DX12 adapter fails outright, so a local optional skip is not miscounted as platform sign-off. CI also runs the real Window/D3D11 import probe above and keeps its raw evidence; having CI configured does not mean Windows tests ran on this machine.

The window path likewise keeps its production cache by the producer signatures actually referenced and the real GPU device generation, and only a successful submit advances the version; a failure or discard does not. An unreferenced registry and stable versions produce no empty submissions on a hidden tick, and a producer without `preparation_version` keeps dynamic semantics.

## Performance, diagnostics and acceptance

`OutputWorkObservation`, `PresentationDriver::work`, `ExternalSurface::last_work`, Runtime `WorkCounters::output_*` and the append-only diagnostics record real planning, UI renders, submissions, rebuilds, reuse, copy bytes, resolve/convert passes, readbacks and defers. A stage that did not run stays unobserved, and missing data is not zero; retired diagnostics IDs are never reallocated.

The seven real GPU fixtures are `output-static`, `output-multi`, `output-nested`, `output-conversion`, `output-slow`, `output-recovery` and `output-incompatible` (the SDR group and its float working group share one painter with one submit per frame and report the shared painter's texel bytes as `painter_resources`); `output-direct` is signed off separately by the real native Window/Surface probe. That probe also runs attach/detach, compatible and incompatible output, resize and the hidden/static phases. A headless target must not pose as a direct Surface, and resize recovery must not pose as real device loss.

```bash
cargo test --locked -p nana-ui --features hosted,bundled-fonts --lib output:: -- --test-threads=1
cargo run --release --locked -p nana-ui --features gpu --bin nana-output-benchmark -- --output target/performance/output/live.json
cargo run --release --locked -p nana-ui --example presentation-output-probe --features hosted,bundled-fonts -- --output target/performance/output/window.json
python3 perf/runners/nana/run.py --scenario output-direct --from-report target/performance/output/window.json --output target/performance/output/direct.json
python3 perf/contract.py --evaluate-invariants target/performance/output/direct.json
```

The adapter, invariants and report commands for every scenario are in [Performance contracts](https://github.com/sena-nana/NanaUI/blob/main/perf/README.md). Windows/DX12, SteamVR, Electron foreign import and real device loss need evidence from the matching platform or service; when they did not run, report unavailable explicitly, never a passing compile or a synthetic zero count. There is no general third-party framework/transport protocol implemented, and no CPU fallback.

Six further `transport-{overlay-static,overlay-changing,overlay-multi,overlay-slow,web-static,web-changing}` scenarios verify independent consumer pools and retirement with real local GPU transfers, with foreign retirement explicitly mocked. They run separately with `--transport-only` so the load of the original seven scenarios is unchanged. Three `transport-{native-host,openvr-native,web-native}` scenarios accept only the matching real reference report: 30 real warm-up iterations, then 240 measured ones, strictly checking the fixture, platform path, device, size, production/consumption work and CPU timing. A missing metric, an empty load or a mock report cannot pass as platform sign-off.

The concrete references and the nine transport gates are now in code. Windows WARP native-export, same-adapter D3D11 consumption and a controlled `device.destroy()` notification have real run evidence; that is not the same as a hardware driver reset. SteamVR live, real Electron GPU import/consumption/dual-credential release, the full hardware and cross-process matrix, and the missing GPU execution time/VRAM are still not signed off. The Electron hosted runner capability probe failed for lack of a WebGPU adapter, and no mock or zero frames were substituted for successful consumption. The current delivery scope and raw evidence are in the [final closeout record](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/presentation-output-final-closeout-2026-10-26.md); closing the issue does not change these capability boundaries.


### Tracing by producer / consumer

`PresentationDriver::set_output_trace_enabled(true)` enables `output_traces()` for each real target group. The window entry implements `RuntimeProgram::output_trace_enabled` and collects numeric snapshots of direct Surface, canonical and real transfer consumers from `window_output_trace`. It is off by default; cold-path state is allocated only when enabled. `OutputTraceSnapshot` carries stable source/consumer identity, path, format, size, device/resource/content generations, work, rebuild reasons, defer/failure state and the optional wait observation.

Wait data comes from the real `FrameContext` acquisition / `GpuSubmission` and is never inferred for a single consumer from differences in global counts. An unobserved value stays `None`. A shared submission is counted once, by the owner that actually submits, and a deferred group does not take another group's submission data. `PresentationDriver::work()` keeps its existing aggregate semantics; when producer/consumer attribution is needed, read the trace — a consumer transfer in the aggregate is not a producer redraw.

Normal observe/query updates in-memory values and metrics only and emits no per-frame event. An explicit `snapshot.capture_diagnostics()` emits nine structured records sharing one trace ID when Debug is on, so nlog can reconstruct them; call it on demand off the frame path. `presentation-output-probe --trace-outputs` saves the per-item observations of a real window, separate from the default timing samples.

`presentation-output-probe --profile-stages` turns on stage metrics and the macOS thread CPU clock separately, separating the wall-clock time of CPU calls such as prepare, acquire, encode, submit and present; `--frame-rate 30` compares presentation waiting against the default 120. Stage and thread sampling start at slightly different points, so the parts need not add up exactly to the frame; none of this data is GPU execution time. The default performance comparison leaves profiling and tracing off.

## See also

- [Live GPU content](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md) · [Window](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md) · [Architecture contracts](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/architecture.md) · [Issue #242 delivery record](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/issue242-delivery.md) · [GPU and frame lifecycle](/learn/gpu-frame)
