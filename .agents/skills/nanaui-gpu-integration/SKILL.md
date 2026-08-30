---
name: nanaui-gpu-integration
description: Maintain NanaUI's host-owned WGPU integration. Inject SceneWgpuPainter into the host GPU context. Use when changing GpuView, GpuTextureView, HostTexture, RenderSlot, SceneWgpuPainter, render passes, texture lifecycle, hosted-gpu-demo, redraw scheduling, WGPU dependencies, or NanaShader and Live2D HostTexture-slot boundaries.
---

# NanaUI GPU Integration

## Rules

- Read [`gpu.md`](../../../docs/gpu.md) before changing the
  rendering boundary.
- Inspect manifests, lockfile, and dependency graph before dependency work; keep one WGPU major
  version across shared types.
- Keep Window, Surface, Device, Queue, and frame scheduling host-owned. Inject
  `SceneWgpuPainter` into that GPU context.
- Product default is `GpuTextureView` plus a **string** `HostTextureRegistry` slot
  (`"nana.host-texture"`). `GpuView` (`u64` `slot_id`, renderer `"gpu-view"`) is
  in-pass only when there is no intermediate texture. Preserve `GpuView` Inline
  join (`draw_in_pass`) and `HostTexture` identity/generation-based invalidation.
  `HostTexture::id` is a painter cache key, not the registry slot.
- Remove unused resources and redraw only on real state or content changes. Paint
  HostTexture in document order inside the current dest pass; do not open a pass
  per slot. GPU-interleaved (HostTexture/Custom) frames share `sample_count = 1`.
  Frames without GPU nodes may keep 4x MSAA for Quad/Mesh; Text paints after
  resolve with Load. Do not resolve around custom nodes, and do not put
  HostTexture after MSAA resolve.
- `RuntimeProgram::scene_gpu_renderers(None)` installs the demo `"gpu-view"`
  painter. `Some(empty registry)` leaves `"gpu-view"` unpaintable.
- `<video data-nana-video>` samples `video:{id}` HostTexture. Do not also paint
  `poster` as `content_image` on a slotted surface.
- Keep CPU readback and PNG encoding inside snapshot tooling. Never use a second Device/Queue or
  copies to hide incompatible dependencies.
- Treat GPU demos as contract evidence, not proof of real NanaShader/Live2D integration.
  `CustomRenderNode` is a first-class layout/Scene citizen. Live2D stays a set of
  HostTexture slots sampled at those nodes; do not describe that as Cubism drawing
  the Surface, and do not add Cubism types to NanaUI.
- Route system materials to `$nanaui-window-materials` and verification to
  `$nanaui-validation`.

## Validation

Test lifecycle, invalidation, texture replacement and geometry. Run the hosted demo for real
Surface changes, inspect the dependency graph, and report untested backends or platforms.
