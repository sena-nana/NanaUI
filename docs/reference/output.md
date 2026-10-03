# Window-independent presentation

`nana_ui::output` is the output boundary for applications that produce a
presentation without making an OS Window the owner of rendering. It is
available with the `gpu` feature and does not require `hosted` or `winit`.

`RenderTargetPlanner` negotiates a topology only at configuration boundaries:
consumer attach/detach, resize, format/color-space changes, and device
replacement. A single compatible Window consumer selects `DirectSurface`, so
the existing surface path keeps its direct-to-surface work. Retained and
multi-consumer plans select, in order, same-device sampling, explicitly
advertised native sharing, GPU transfer, or an explicitly opted-in CPU
fallback. The planner never assumes native interop from a consumer kind.

With the `hosted` feature, `WindowPresenter` exposes the existing
`HostedGpuSurface` capabilities and lifecycle without creating an offscreen
target. Refresh its requirements after a structural surface change, then keep
the planner outside the per-frame encode loop. It is an adapter seam; the
existing hosted scene loop still needs explicit host wiring before it becomes
the active window path.

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

The external target is currently an in-process same-device resource. The first
Vulkan/Linux interop review found that wgpu 30 can import some DMA-BUF images
through its HAL, but does not expose external semaphore import/export or a
memory export contract. Queue ownership barriers would also be outside the
safe Nana GPU contract. Native handle exchange therefore stays explicitly
unsupported until an interop boundary can carry both resource ownership and
fence tokens. Set `copy_source` when a planner needs to choose an explicit
GPU-transfer or CPU-readback route.

The producer must include revisions for GPU producers or host textures that
are not represented by `UiScene::projection_revision`. A resize or device
replacement first creates the new resources; a failed creation leaves the old
completed surface available. All target resources carry `DeviceGeneration`,
and the surface rejects a different device rather than sending stale handles
to the painter.

Output work is observable through `OutputWorkObservation`, the planner's
`last_work()` snapshot, `WorkCounters`'s `output_*` fields, and
`nana_diagnostics::framework::gpu::OUTPUT_*` metrics. Frame drivers fold a
planner or surface snapshot with `record_last_work`; CPU fallback is therefore
explicit in both planning and diagnostics.
