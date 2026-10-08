# Window-independent presentation

`nana_ui::output` is the output boundary for applications that produce a
presentation without making an OS Window the owner of rendering. It is
available with the `gpu` feature and does not require `hosted` or `winit`.

A window presents to its own surface directly. Anything else that consumes
the same scene, such as a Spout sender, a recorder or a nested host, takes an
output: the scene is painted a second time into an `ExternalSurface` inside
the frame the window records anyway (see *Window outputs* below). There is no
topology planner and no CPU fallback; a consumer that needs the pixels on
another device or in another process gets them through native sharing.

`ExternalSurface` owns a bounded set of persistent GPU textures and uses the
same `SceneWgpuPainter` as a Window. `render` compares the Scene projection,
host revision, texture registry revision, viewport, and target generation. An
unchanged presentation returns `Reused` before starting a frame, so it does
not allocate, encode, copy, submit, or read pixels. A submitted revision is
published only after the queue completion callback runs. `sample` returns a
lease to the last completed texture; dropping the lease releases its slot
without waiting on the CPU. `GpuContext::poll` and `ExternalSurface::poll`
only drive non-blocking completion callbacks.

`EmbeddedSurfaceNode` is the nested-consumer seam. It keeps a child
`ExternalFrame` lease while the parent records its scene and can bind that
lease to the parent's `GpuSubmission`; the completion callback retires the
child slot after the parent has finished sampling it. Logical and texture
extents, device/resource generations, and content revision are carried as
explicit metadata, so parent transforms and clipping stay in
the parent scene. A `HostTextureRegistry` slot retains a clone of the child
handle, so the parent must replace or remove that slot when the node is no
longer used; otherwise a later child revision could recycle the target while
the old slot is still registered.

`render` is `prepare` → `record` → `bind_submission` in a frame of the
surface's own. A host that already records a frame uses the three steps
directly: `prepare` answers `Reused`, `Deferred` or `Record(plan)` without
recording anything; `record` paints the plan's slot into the caller's
`FrameContext`, with the surface's own painter or, for a surface created with
`ExternalSurface::with_host_painter`, with a painter the host passes in (the
`target_namespace` keeps its slot ids apart from the painter's other
targets; remove `target_ids()` from that painter when the surface goes);
`bind_submission` marks the slot in flight on that frame's `GpuSubmission`.
`plan_texture` is the slot's texture, for work appended in the same frame.

The external target is an in-process same-device resource. On Windows/DX12
the `native-export` feature adds the one native exchange NanaUI ships (see
*DX12 shared textures* below). The first Vulkan/Linux interop review found
that wgpu 30 can import some DMA-BUF images through its HAL, but does not
expose external semaphore import/export or a memory export contract, and
queue ownership barriers would be outside the safe Nana GPU contract; native
exchange stays unsupported there. Set `copy_source` when a consumer copies
the output texture on the GPU.

The producer must include revisions for GPU producers or host textures that
are not represented by `UiScene::projection_revision`. A resize or device
replacement first creates the new resources; a failed creation leaves the old
completed surface available. All target resources carry `DeviceGeneration`,
and the surface rejects a different device rather than sending stale handles
to the painter.

Output work is observable through `OutputWorkObservation`, the surface's
`last_work()` snapshot, `WorkCounters`'s `output_*` fields, and
`nana_diagnostics::framework::gpu::OUTPUT_*` metrics. Frame drivers fold a
surface snapshot into the frame's counters with `record_last_work`.

## Window outputs

A window can be painted a second time into an offscreen output for a consumer
outside it (a Spout sender, a recorder). `RuntimeProgram::window_output(id)`
returns a `WindowOutputConfig` (or `ApplicationWindow::output` for an
`ApplicationState`); the host reads it every frame and `None` stops the output.

```rust
fn window_output(&self, id: WindowId) -> Option<WindowOutputConfig> {
    (id == TYPEWRITER).then(|| WindowOutputConfig::default() // 1920x1080, Contain
        .with_extent(WindowOutputExtent::Fixed { width: 1920, height: 1080 })
        .with_export(WindowOutputExport::Native))
}
fn window_output_frame(&mut self, id: WindowId, frame: &WindowOutputFrame, cx: &RuntimeProgramContext<Msg>) {
    // frame.texture(): same device, BGRA8_UNORM, sRGB-encoded, premultiplied
    // frame.take_native(): the DX12 token below (Windows, native-export)
}
fn window_output_status(&mut self, id: WindowId, status: WindowOutputStatus, cx: &RuntimeProgramContext<Msg>) {}
```

- **Same scene, same frame.** The host paints the window's own `UiScene` into
  the output inside the `FrameContext` it records for the window, after the
  window's paint and before the submit, with the host's shared painter for
  the output profile (separate `RenderTargetId`s, grayscale text,
  `AlphaEncoding::Gamma`, SDR parameters). Nothing is laid out twice.
- **Nothing changes, nothing is recorded.** The output is an
  `ExternalSurface`; a frame whose projection, host textures, images and
  viewport are unchanged is `Reused` and produces no output frame and no
  submit. Custom GPU renderers and compositor animation count as changing
  every frame.
- **Hidden windows.** With `while_hidden` (the default) a window that cannot
  present (hidden, minimised, occluded) ticks at `max_fps` (30 by default):
  the host drains its messages, flushes the document, and records only the
  output (and the window's scene producers) in a frame of its own. A hidden
  static window records nothing. A window whose size is 0 produces nothing.
- **Size and fit.** `Fixed { width, height }` (the default, 1920×1080),
  `MatchWindow`, or `Scale(f)` of the window's physical size. The painter
  scales uniformly and the scene is not laid out for the output, so content
  of another aspect ratio is placed with `Contain` (whole window, centred,
  transparent around it) or `Cover` (filled, cropped). A non-uniform stretch
  is not offered.
- **Alpha.** `Premultiplied` (gamma-space, what OBS's "Premultiplied" Spout
  setting expects) or `Straight`, which adds one small pass dividing colour
  by alpha into a texture of its own.
- **Lifetime of a frame.** `WindowOutputFrame::texture` is valid for GPU work
  submitted to the host's device from the callback on (queue order); the
  output recycles the texture two frames later. Nothing waits for the GPU.
- **Status.** `WindowOutputStatus` is reported once per change: `Active`,
  `NativeUnavailable`, `NativeDeferred`, `NativeRetired`, `Failed`, `Stopped`.
  `max_fps` throttles produced frames; a throttled change is produced when it
  comes due.

## DX12 shared textures (`native-export`)

`nana_gpu::NativeExportPool` (feature `native-export`, Windows, a DX12
device; `GpuCapability::NativeTextureExport`) owns three committed
`DXGI_FORMAT_B8G8R8A8_UNORM` textures created with `D3D12_HEAP_FLAG_SHARED`
and render-target use (D3D11 refuses to open a shared D3D12 texture without
it), each exported as an NT handle, and one shared fence. They are wrapped as
`COPY_DST`-only textures on the host's own device; no second device or queue
exists. `stage` records a copy of a rendered texture into the next slot of the
caller's frame; `FrameContext::submit` signals that frame's *ready* value on
the device queue right after `queue.submit`, under the submission guard;
`finish` turns the submitted copy into a `NativeFrameToken`. A window output
with `WindowOutputExport::Native` does this for every produced frame.

Frame `k` (counting from 0) is ready at fence value `2k + 1` and released at
`2k + 2`. The consumer contract:

1. Open the pool once per `token.pool_generation()`: create a D3D11 device on
   the adapter `token.adapter_luid()` names (`IDXGIFactory4::EnumAdapterByLuid`),
   `ID3D11Device1::OpenSharedResource1` every `token.texture_handle(slot)` for
   `slot < NATIVE_EXPORT_SLOTS`, and `ID3D11Device5::OpenSharedFence` the
   `token.fence_handle()`. The handles stay open while any token of that pool
   lives; the opened objects are the consumer's own.
2. For each token: `token.accept_release()`, then on the consumer's context
   `ID3D11DeviceContext4::Wait(fence, token.ready_value())`, read
   `textures[token.slot()]` (for example `CopyResource` into the sender's
   texture), `Signal(fence, token.release_value())`, `Flush`.
3. Signal release only on the GPU timeline after the wait. A release
   signalled before the producer's ready signal ran would move the fence
   backwards when that one lands, and the pool would then defer forever.
4. A token dropped without `accept_release` is released by the producer on its
   own queue, ordered after the ready signal; a consumer may skip frames by
   dropping their tokens.

Nobody waits on the CPU. While the consumer has not released the previous
frame, `stage` answers `Deferred(ConsumerBusy)` and records nothing; a window
output then reports `NativeDeferred` and keeps producing same-device frames.
A consumer that has not released for 2 seconds gets the pool retired and a new
one (`NativeRetired`). A resize, an alpha/export change and a device switch
also retire the pool; the next token names a new `pool_generation`. The
content is sRGB-encoded with premultiplied alpha unless the output asked for
`Straight`. The export path performs no CPU readback; the D3D11 readback in
the tests and in `window-output-probe` is verification only.

```powershell
cargo test -p nana-gpu --features native-export --test native_export
cargo run -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export
```

The tests export frames, open them on a D3D11 device of the same adapter,
copy and compare every byte, check that an unreleased frame defers the next,
that a dropped token releases itself, that a discarded frame leaves the pool
unchanged and that retiring closes the handles once the last token is gone.
`window-output-probe` runs a transparent composition window through four
phases (visible/hidden × animated/static) and consumes every native token on
a D3D11 device. On 2026-10-08 (Windows 11, RTX 5060 Ti, DX12) it produced 62
frames visible and 76 hidden while animated, 0 in both static phases, every
token was consumed, and the read-back box texel matched the window
(BGRA 51,76,229,255). Not covered: the D3D12 debug layer, and a cross-process
consumer.
