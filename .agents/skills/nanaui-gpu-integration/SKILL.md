---
name: nanaui-gpu-integration
description: Maintain NanaUI's host-owned GPU and frame integration. Use when changing the nana-gpu contract (GpuContext, FrameContext, GpuTexture, wgpu-interop), SceneWgpuPainter, GpuView, HostTexture, CustomRenderNode, frame exchange, render passes, redraw scheduling, texture lifetime, or WGPU dependencies.
---

# NanaUI Host GPU Integration

Read [`docs/gpu.md`](../../../docs/gpu.md) and the crate boundaries in [`docs/architecture.md`](../../../docs/architecture.md) before editing the rendering boundary.

## Contract

- The host owns Window, Surface, the `GpuContext`, each `FrameContext`, and frame scheduling. Build `SceneWgpuPainter` on that context and paint into the host's frame; never create a second device or queue.
- `nana-gpu` is the contract, WGPU its only backend. Public signatures of nana-gpu, nana-frame-exchange and nana-ui name `wgpu` only behind `wgpu-interop`; framework sources reach WGPU through `nana_gpu::__framework`. Keep `python3 scripts/check-engine-boundary.py` green instead of widening the public `wgpu` surface.
- `FrameContext` is the single authority for submit and discard: a painter records the targets it wrote into the frame, and a dropped frame rolls them back. Every host failure path drops the frame before abandoning the surface texture. Off-thread raw queue work holds `lock_submission()`, never across `poll(Wait)`.
- Device loss lives in `GpuContext::is_lost()`; a replaced device is a new `GpuContext` with a new `DeviceGeneration`. Key device-built caches on the generation; resources from another generation are refused, not sent to the backend.
- `GpuTextureView` plus a string `HostTextureRegistry` slot is the default product path. Use `GpuView` only for an in-pass view with no intermediate texture. Preserve HostTexture identity/generation invalidation.
- Paint HostTexture and Custom nodes in document order inside the current destination pass. Do not resolve around interleaved GPU nodes or move them to a frame-end overlay.
- Frame exchange is GPU-side. Keep producer handoff, prepare, present, and sampled-lease lifetime ordered; CPU readback belongs only to explicit snapshot tooling.
- Keep `CustomRenderNode` a layout and Scene node. Live2D or other producers supply ordinary HostTexture slots; framework crates do not gain product-specific renderer types.
- Inspect `Cargo.toml`, `Cargo.lock`, and the dependency graph for dependency changes and keep one WGPU major version.

Test geometry, invalidation, replacement, lifetime, and redraw behavior. Use `$nanaui-validation` for evidence and report untested backends or platforms.
