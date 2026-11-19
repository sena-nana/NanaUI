---
title: GPU and Frame Lifecycle
---

# GPU and frame lifecycle: the `nana-gpu` contracts and one host frame

This chapter answers three core questions: **why the `nana-gpu` type contracts (`GpuContext` / `FrameContext` / `GpuTexture`, device generations and loss, submission order) are designed the way they are**; **how `SceneWgpuPainter` in `nana-ui` organizes render passes and retained state on the host's GPU context**; and **which stages one frame's complete 35-step lifecycle passes through from scheduling to `present`**. This chapter follows the `UiScene` of [Scene extraction](/learn/scene-extraction) and hands over downward to [Window and platform](/learn/window-platform).

## Responsibilities and boundaries

- **`nana-gpu` is the backend contract, and WGPU is its only implementation**: `crates/nana-gpu/src/lib.rs:1` states that `GpuContext` represents the process's only GPU device (carrying the device generation, hardware capabilities, the sticky loss state, texture creation/upload); `FrameContext` exclusively owns one command encoder from `begin_frame` to `submit` or drop (a drop automatically `discard`s and rolls back the retained-state writes the painter registered); `GpuTexture` and `GpuRenderTarget` carry a `DeviceGeneration`, and a resource from another generation is rejected at the entry point. The `wgpu-interop` feature is the explicit low-level escape hatch.
- **The host owns the device and the frames (`AGENTS.md` core contract)**: the host owns `Window`, `Surface`, `GpuContext`, each frame's `FrameContext` and frame scheduling; `SceneWgpuPainter` is built on top of the host context and only records draw commands into the host's frames. The dependency graph holds one wgpu, NanaUI's fork in `third_party/wgpu` (upstream package names and version 30.0.1 kept); creating a second `Device` / `Queue` is strictly forbidden, and CPU readback on the product presentation path is strictly forbidden.
- **`nana-frame-exchange` only carries producer frames**: it provides a bounded latest-frame exchange pool on the same `GpuContext` the host already has (internally copying through a GPU-side `copy_texture_to_texture`, without CPU readback), and does not own the UI device or the submission right itself (`crates/nana-frame-exchange/src/lib.rs:1`).
- **GPU content is a first-class node in the tree**: it is attached to the tree through `CustomRenderNode` or `HostTexture` (`GpuTextureView`), and takes part in layout, clipping, hit testing and document order like an ordinary button.

## Dependencies and neighbors

In the dependency hierarchy: `core(L1) → gpu(L2) → frame-exchange(L3) → runtime(L5) → scene(L6) → nana-ui(L7)`. `nana-ui` is the only crate that holds `SceneWgpuPainter`; `nana-frame-exchange` depends on `nana-gpu` and re-exports `GpuContext`, `GpuTexture`, `DeviceGeneration` and `GpuTextureFormat`, so an independent rendering producer does not need to depend on `nana-ui`:

| Neighbor crate | Collaboration |
| --- | --- |
| `nana-ui-scene` | Provides the painter's input: `UiScene` primitives, `frame_plan()` and the `instance` generation number |
| `nana-window` / `nana-ui-platform` | Manage the native window Surface and the transparent/composition presentation path (see [Window and platform](/learn/window-platform)) |
| `wgpu` | The only graphics backend; `wgpu::*` in public signatures is allowed in the five whitelisted crates only when `wgpu-interop` is on (enforced by `scripts/check-engine-boundary.py`) |
| `nana-diagnostics` | Records zero-I/O frame-path metrics (`FRAMES_DISCARDED`, `FRAME_SLOTS_EXHAUSTED`, etc.) |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `GpuContext` | `crates/nana-gpu/src/context.rs:358` | Process-level device handle (`Arc<GpuInner>`; cheap to clone and shares the same generation) |
| `DeviceGeneration` | `crates/nana-gpu/src/context.rs:19` | `NonZeroU64` device generation number, allocated by a process-level monotonic counter |
| `GpuCapabilities` | `crates/nana-gpu/src/context.rs:204` | Hardware capability and feature table read from the adapter / device |
| `FrameContext` | `crates/nana-gpu/src/frame.rs:38` | Single-frame context: exclusive encoder + retained-write ledger + frame slot lease + in-frame upload batch |
| `GpuSubmission` | `crates/nana-gpu/src/frame.rs:301` | Submission receipt: frame number, device generation, CPU recording time and backend submission sequence number |
| `RetainedWrites` | `crates/nana-gpu/src/frame.rs:360` | Cross-frame retained-write ledger: a discarded frame is automatically marked for rollback, and concurrent recording of the same target across frames is intercepted (`in_flight`) |
| `GpuTexture` / `GpuRenderTarget` | `crates/nana-gpu/src/texture.rs:183` / `:276` | 2D textures and renderable surface targets carrying a `DeviceGeneration` |
| `GpuDeviceState` | `crates/nana-gpu/src/policy.rs:232` | Per-device policy authority: upload ring buffer, bounded frame slots, pipeline registry, retired resource reclamation |
| `SceneWgpuPainter` | `crates/nana-ui/src/scene_paint/mod.rs:381` | Built-in WGPU scene painter: device-level shared pipeline families + independent state per render target |
| `RenderTargetId` | `crates/nana-ui/src/scene_paint/mod.rs:463` | Host-owned window or offscreen render target identifier (`pub u64`) |
| `DrawCommand` | `crates/nana-ui/src/scene_paint/mod.rs:513` | Recorded GPU draw command enum (`Quads` / `Mesh` / `Text` / `Icon` / `HostTexture` / `Custom`, etc.) |
| `HostTexture` / `HostTextureRegistry` | `crates/nana-ui/src/gpu_texture.rs:370` / `:555` | Host texture handles (high 32 bits generation + low 32 bits content version) and the shared slot registry |
| `SceneGpuRenderer` / `SceneResourceProducer` | `crates/nana-ui/src/scene_gpu.rs:247` / `:325` | The in-pass custom renderer contract and the per-graph offscreen resource producer contract |
| `FrameExchange` / `FrameBinding` | `crates/nana-frame-exchange/src/lib.rs` / `crates/nana-ui/src/frame_binding.rs:25` | Cross-thread latest-frame exchange pool and the window consumer-side slot binding |

## Internal structure

### 1. `nana-gpu` core contracts: device generations, sticky loss and submission order

- **Device generation (`DeviceGeneration`) and sticky loss**: every `GpuContext` allocates a unique `DeviceGeneration` (`context.rs:19`); `GpuContext::check_device` (`:715`) returns `GpuError::DeviceMismatch` directly when it meets a texture or render target from another generation. The device loss reason is recorded in a sticky `LossState` (only the **first** loss report is kept, `:312-325`, to prevent a driver callback storm).
- **Submission order**: queue submissions no longer need serializing against `Surface::configure` — NanaUI's wgpu fork makes a reconfigure wait only for the work that still uses that surface's swapchain images rather than for the whole device, so a concurrent submit from another thread no longer triggers `GpuWaitTimeout` (see `third_party/wgpu/README.md`). `GpuInner.submission_order: Mutex<()>` only keeps frame submits, upload batches and native-export signals in real queue order; the former `lock_submission` guard on `wgpu-interop` has been removed.
- **Up-front parameter validation**: `create_texture` (`context.rs:459`) completes size, usage, block-alignment and format-purpose validation before calling down into WGPU, turning every invalid input into a recoverable `GpuError` and never triggering a WGPU validation panic.

### 2. `FrameContext`: exclusive encoder, merged upload ring and discard rollback

Every `FrameContext` (`crates/nana-gpu/src/frame.rs:38`) has only two termination paths:

1. **`submit(mut self) -> GpuSubmission` (`:148`)**: first finishes `encoder.finish()` outside the lock; then takes the `submission_order` mutex, packs the upload batch, places the upload command buffer produced by the `UploadRing` (`crates/nana-gpu/src/upload.rs:1`, a `32 MiB` mapped block pool that removes the standalone staging cost of every small write on Metal/DX12) **before** the frame's render commands, and submits both in the same `queue.submit`; finally it settles the retained-write ledger and returns the frame slot and transient buffers in the `on_submitted_work_done` callback.
2. **`discard(self)` or `Drop` (`:250 / :259`)**: discards the encoder and increments `FRAMES_DISCARDED`; buffer writes already queued within the frame are folded into the device's pending landing batch (landed by the next submission, keeping the CPU-side mirror consistent); at the same time `RetainedWrites::roll_back` marks the render targets painted this frame as rolled back (raising `RETAINED_FRAME_DISCARDED`), forcing the painter to rebuild those targets' retained state before the next paint.

### 3. `SceneWgpuPainter`: device-level shared pipelines and per-target state

`SceneWgpuPainter` (`crates/nana-ui/src/scene_paint/mod.rs:381`) splits into two layers:

- **Device-level shared pipelines (built once)**: the seven pipeline families — quad, motion, mesh, icon, text, host_texture, backdrop — are **compiled in parallel** on scoped threads at construction through the `beside!` macro (the slowest quad pipeline stays on the current thread, `:566`); the `charts` pipeline is compiled lazily the first time a chart is met. On macOS/Metal, built-in shaders load from a precompiled Metal 3.1 MSL archive (`msl_precompiled/mod.rs`), and other WGPU targets consume compacted `b"NAGA1"` Naga IR bytecode (`naga_ir/codec.rs:17`), eliminating runtime WGSL parsing unless the `runtime-wgsl` feature is enabled.
- **Per-target state (`TargetState`, `:469`), HDR/OKLCH presentation, and ROV**: isolates and stores the prepared batch (`PreparedBatch`), the target texture and the per-pipeline instance data by `RenderTargetId`, swapped in and out through `swap_target_state` (`:935`) and tracked by the `RetainedWrites` ledger — if the previous frame was discarded before submission, `begin_frame` (`:955`) automatically clears and rebuilds that target's retained state; if another frame is already in flight for the same target, it returns `ScenePaintError::TargetInFlight` directly to prevent out-of-order submission from corrupting state. Author colors preserve OKLCH/sRGB precision, internal compositing runs in **premultiplied linear scRGB**, and `ScenePresentationProfile` outputs to SDR sRGB, Display P3, or HDR PQ+BT.2020; when the adapter exposes `wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER` (`crates/nana-ui/src/hosted_context.rs:1475`), backdrop and blending passes take the rasterizer-order-view (ROV) path. Frame-level CPU/GPU work attribution is aggregated through `WorkCounters` (`crates/nana-ui-core/src/work.rs:83`).

### 4. The host's complete 35-step frame sequence (`scene_host/present.rs`)

The host's single-window redraw function `redraw` (`crates/nana-ui/src/scene_host/present.rs:114`) executes in the following exact order:

| Step numbers | Core stage | Key design reason and source detail |
| --- | --- | --- |
| **1–2** | Reclaim the previous frame's metrics; check for device loss | If `take_device_lost` is true, run `recover_device` to create a `GpuContext` with a new generation and call the program's `rebuild_gpu` |
| **3–4** | Drain and apply program messages; handle window resize | Feeds `dispatch_program` messages into `RuntimeProgram::update` |
| **5** | Call `program.prepare_window_frame(id, ctx)` | Applies the `HostTexture` or `FrameBinding` this frame is going to sample |
| **6–11** | Output sync, advance the compositing clock, `document.flush`, update the cursor, stage accessibility, obtain the shared `UiScene` | `document.flush(viewport, &mut text)` produces a `RuntimeFrameUpdate`; if it did not converge (`FrameDidNotSettle`), restore the dirty work and skip this frame |
| **12–13** | Resolve the Surface color profile and HDR parameters; check startup takeover | HDR headroom is passed as a uniform and does not trigger pipeline recompilation |
| **14** | **`reserve_scene_host_frame(gpu)` (`present.rs:12`)** | **The frame slot must be requested before acquiring the swapchain drawable**: it calls `try_begin_frame`, and if that fails it calls `poll(Poll)` once and retries; if there is still no free frame slot it gives up on this frame (counting `FRAMES_SKIPPED`) and leaves the previous frame on screen, so that no transparent swapchain buffer is taken without anything to composite |
| **15–16** | `acquire_frame` obtains the Surface texture and wraps it as a `GpuRenderTarget` | An expired or lost Surface moves to the surface recovery flow |
| **17** | `producers.encode_scene(scene, &mut recording)` | Records the `SceneResourceProducer` offscreen preparation passes into the same `FrameContext` (a failure `discard`s the whole frame) |
| **18–19** | Sync the `nana.host-texture` slot redraw subscriptions; inject the Windows native content renderer | Only the host texture slots actually referenced by the current scene are subscribed |
| **20–21** | Set draw parameters; run **`painter.paint_target(...)` (`present.rs:385`)** | Records every visible primitive in document order; on failure it `discard`s the frame and marks a reorganization |
| **22–24** | Record the secondary output outside the window (`window_output`); **`recording.submit()`**; register the completion callback | The window and the offscreen output share one `FrameContext::submit` |
| **25–27** | `present_frame` puts the frame on screen; record presentation diagnostics; apply pending Surface reconfiguration | Surface reconfiguration is strictly ordered after `present` and waits only for that surface's work |
| **28–31** | Update `FrameDemand`; write back the decoded image natural sizes (`commit_image_sizes`); submit the Windows composition tree; deliver out-of-window output frames | Image natural sizes are written back before the program callback; if an `<img>` with no size yet obtains one, the next frame is asked to relayout |
| **32–35** | Publish accessibility deltas; refresh the IME cursor box; call **`window_frame_presented`** | Only at step 35 may the application safely release the old textures replaced by the previous frame |

### 5. The `wgpu-interop` escape hatch and the internal `__framework` channel

Why do `wgpu-interop` and `nana_gpu::__framework` (`crates/nana-gpu/src/__framework.rs`) both exist? Because Cargo features unify across the whole dependency graph — if `nana-ui` or `nana-frame-exchange` reached low-level WGPU by enabling `wgpu-interop`, every ordinary application that depends on `nana-ui` would be forced to enable the escape hatch as well. The framework's own sources therefore reach the raw objects through the hidden `__framework` module, while `wgpu::*` in public signatures is strictly gated by `wgpu-interop` and guarded in CI by `scripts/check-engine-boundary.py`.

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-gpu/src/context.rs` | `GpuContext`, `DeviceGeneration`, the capability table, the sticky loss state, submission order and `begin_frame` |
| `crates/nana-gpu/src/frame.rs` | `FrameContext`'s `submit` / `discard`, the `RetainedWrites` rollback ledger and `GpuSubmission` |
| `crates/nana-gpu/src/texture.rs` | `GpuTexture`, `GpuRenderTarget` and the stable usage flags |
| `crates/nana-gpu/src/policy.rs` | Bounded frame slots, the transient buffer pool, the `StampedCache` pipeline cache and retired resource reclamation |
| `crates/nana-gpu/src/upload.rs` | The `UploadRing` batched merged upload ring |
| `crates/nana-gpu/src/{wgpu_interop,__framework,abi}.rs` | The explicit escape hatch, the framework-internal channel and the logical resource binding ABI |
| `crates/nana-ui/src/scene_paint/mod.rs` | The `SceneWgpuPainter` body, per-target state management and `paint_target` |
| `crates/nana-ui/src/scene_paint/{msl_precompiled,naga_ir}/` | Precompiled Metal 3.1 MSL archive and compacted `b"NAGA1"` Naga IR codec |
| `crates/nana-ui/src/scene_host/present.rs` | The host `redraw` 35-step full lifecycle and `tick_hidden_gpu` |
| `crates/nana-ui/src/{gpu_texture,scene_gpu,frame_binding}.rs` | `HostTextureRegistry`, `SceneGpuRenderer` / `SceneResourceProducer` and `FrameBinding` |
| `crates/nana-frame-exchange/src/lib.rs` | The `FrameExchange` cross-thread bounded latest-frame exchange pool and `FrameColorMetadata` |

## Further reading

- [Scene extraction](/learn/scene-extraction): how `UiScene` and `FramePlan` are produced in the `flush` phase
- [Window and platform](/learn/window-platform): native window Surface, the transparent composition path and material fallbacks
- [Live GPU content and GPU integration contracts (Reference)](/reference/gpu) · [Output outside the window (Reference)](/reference/output)
