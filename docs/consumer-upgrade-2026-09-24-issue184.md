# Issue #184：统一 GPU policy

`nana-gpu` now exposes a per-device `GpuDeviceState` through `GpuContext::policy()`.
The policy owns bounded upload accounting, in-flight frame-slot bookkeeping, transient
resource keys, pipeline/realization identities, and submission retirement records. The
public surface remains backend-neutral; WGPU stays behind the existing framework and
`wgpu-interop` boundaries. Scene painter work sinks share the policy for upload and
reallocation counters. Retained quad, icon, mesh, motion, text, host-texture, default
GPU-view, backdrop, and destination static pipeline construction paths use the per-device
registry; device replacement continues to invalidate the old generation. Continuous
HostTexture/video producers remain outside static realization reuse; CPU identity/version entries
reuse real per-device `GpuTexture` objects through the policy cache. FrameExchange copies
use `try_begin_frame` and return `PoolFull` without waiting when shared slots are occupied, so
producer work participates in the same frame-slot and queue-completion lifecycle. Its sampled
copy textures now come from the policy's bounded, generation-keyed real `GpuTexture` pool and
are returned before a free slot is reused.

## Verification boundaries

The policy implementation and renderer migration are complete. The items below record evidence
that requires a native window, a deliberate device-loss injection, or the external #177 fixture.

- UploadArena now owns a generation-local, growable backing buffer and retires replaced buffers.
  The policy also owns a bounded, generation-keyed transient buffer pool with descriptor validation
  and budget eviction, alongside FrameExchange's real transient texture pool. Replaced mesh
  buffers are returned at submission completion. `GpuContext::write_texture` consumes its staged bytes through a
  frame copy path. Renderer dynamic buffer diff writes, text atlas, icon atlas, and URL image
  uploads now use the guarded policy helper when a production work sink is present. Test/offline
  compatibility branches remain explicit direct paths; the text-specific staging ring is also
  resized through the policy lease, while its queue write remains a deliberate batching boundary.
  The helper texture path remains a compatibility queue copy rather than an encoder copy from an
  arena offset.
  One-off readback/staging helpers and explicit interop tooling remain separate by design; they do
  not participate in the product transient pool.
- Mesh path/triangle buffers, quad, icon, backdrop uniform slabs, text instance/index and
  run/presentation tables, motion descriptor/keyframe buffers,
  default GPU-view instances, and destination group uniforms now acquire resized buffers through
  the policy and return old buffers through a frame submission-completion lease.
  Remaining direct allocations are fixed-size initialization, bounded one-off staging fallback,
  or explicit readback/interop paths; all other product capacity-growth paths use this lease rule.
- HostTexture/video producers must keep changing versions out of static realization reuse; the
  bounded real `GpuTexture` cache is keyed by identity, version and generation.
- The compatibility `begin_frame` API intentionally waits when every frame slot belongs to an
  unsubmitted recording; host scheduling uses `try_begin_frame` when it must surface `PoolFull`.
  Slot stall and discard behavior are covered by the policy and FrameExchange tests.
- Pipeline/shader/layout identities are registry keys, with bounded eviction and completion-bound
  retirement covered by policy tests. The remaining limitation is evidence: this checkout has no
  repeatable native-window/device-loss harness for proving recovery on every backend.
- Dense UI, HostTexture and retained-text workloads were run through the headless benchmark and
  emitted policy counters. The hosted Metal device probe also completed replacement from
  generation 1 to 2 and resumed FrameExchange submission; it collected no sustained present
  interval samples, so it is recovery evidence rather than a refresh-rate result. A full #177
  realtime fixture, native-window acceptance across other platforms, and cross-renderer reuse
  remain explicitly unexecuted; compilation and scene tests are not being counted as substitutes.
