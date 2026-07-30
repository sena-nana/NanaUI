---
name: nanaui-gpu-integration
description: Maintain NanaUI's host-owned Iced and WGPU integration. Use when changing GpuView, GpuTextureView, HostTexture, RenderSlot, render passes, texture lifecycle, hosted-gpu-demo, redraw scheduling, Iced or WGPU dependencies, or NanaShader and Live2D integration boundaries.
---

# NanaUI GPU Integration

## Rules

- Read [`rendering-integration.md`](../../../docs/rendering-integration.md) before changing the
  rendering boundary.
- Inspect manifests, lockfile, and dependency graph before dependency work; keep one WGPU major
  version across shared types.
- Keep Window, Surface, Device, Queue, and frame scheduling host-owned. Inject the existing GPU
  context into Iced.
- Preserve `GpuView` Inline/Standalone pass semantics and `HostTexture` identity/generation-based
  invalidation. Remove unused resources and redraw only on real state or content changes.
- Keep CPU readback and PNG encoding inside snapshot tooling. Never use a second Device/Queue or
  copies to hide incompatible dependencies.
- Treat GPU demos as contract evidence, not proof of real NanaShader/Live2D integration.
- Route system materials to `$nanaui-window-materials` and verification to
  `$nanaui-validation`.

## Validation

Test lifecycle, invalidation, texture replacement and geometry. Run the hosted demo for real
Surface changes, inspect the dependency graph, and report untested backends or platforms.
