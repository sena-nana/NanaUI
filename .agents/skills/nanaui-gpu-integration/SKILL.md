---
name: nanaui-gpu-integration
description: Maintain NanaUI's host-owned GPU and frame integration. Use when changing SceneWgpuPainter, GpuView, HostTexture, CustomRenderNode, frame exchange, render passes, redraw scheduling, texture lifetime, or WGPU dependencies.
---

# NanaUI Host GPU Integration

Read [`docs/gpu.md`](../../../docs/gpu.md) and the crate boundaries in [`docs/architecture.md`](../../../docs/architecture.md) before editing the rendering boundary.

## Contract

- The host owns Window, Surface, Device, Queue, encoder, and frame scheduling. Inject `SceneWgpuPainter` into that context; never create a second device or queue.
- `GpuTextureView` plus a string `HostTextureRegistry` slot is the default product path. Use `GpuView` only for an in-pass view with no intermediate texture. Preserve HostTexture identity/generation invalidation.
- Paint HostTexture and Custom nodes in document order inside the current destination pass. Do not resolve around interleaved GPU nodes or move them to a frame-end overlay.
- Frame exchange is GPU-side. Keep producer handoff, prepare, present, and sampled-lease lifetime ordered; CPU readback belongs only to explicit snapshot tooling.
- Keep `CustomRenderNode` a layout and Scene node. Live2D or other producers supply ordinary HostTexture slots; framework crates do not gain product-specific renderer types.
- Inspect `Cargo.toml`, `Cargo.lock`, and the dependency graph for dependency changes and keep one WGPU major version.

Test geometry, invalidation, replacement, lifetime, and redraw behavior. Use `$nanaui-validation` for evidence and report untested backends or platforms.
