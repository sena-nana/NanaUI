---
name: nanaui-gpu-integration
description: Maintain NanaUI's host-owned GPU and frame integration. Use when changing the nana-gpu contract (GpuContext, FrameContext, GpuTexture, wgpu-interop), SceneWgpuPainter, GpuView, HostTexture, CustomRenderNode, frame exchange, render passes, redraw scheduling, texture lifetime, or WGPU dependencies.
---

# NanaUI Host GPU Integration

Read [`docs/reference/gpu.md`](../../../docs/reference/gpu.md) and the crate boundaries in [`docs/reference/architecture.md`](../../../docs/reference/architecture.md) before editing the rendering boundary.

## Contract

- The host owns Window, Surface, the `GpuContext`, each `FrameContext`, and frame scheduling. Build `SceneWgpuPainter` via `try_new` / `try_new_with_presentation` on that context and paint into the host's frame; never create a second device or queue.
- Precompiled shader artifacts target macOS 14.0+ / iOS 17.0+ (`MSL 3.1`, matching `nana-packager`'s `minimum_system_version = "14.0"`), binary `NAGA1` `naga_ir` (`naga_ir::codec`, zero `bincode`), and DX12 DXIL. On Metal, lazy `msl_precompiled::archive_scope()` + `lease_archive()` does zero decompression on warm cache hits and shares a single `stages.msl.zz` decompression across cold painter build threads via `ArchiveLease`, returning uncompressed MSL resident bytes to `0 B` on drop, while guarding devices below `(3, 1)`.
- Never panic on precompiled shader misses/failures: wrap pipeline creation in GPU error scopes, fall back to runtime WGSL with `gpu::SHADER_PRECOMPILED_FALLBACK` only when `runtime-wgsl` (or test) is enabled, and otherwise emit `gpu::SHADER_BUILD_FAILED` returning `Err(ScenePaintError::ShaderUnavailable)` (custom WGSL materials without `runtime-wgsl` emit `gpu.material.rejected`).
- After changing scene shaders or `third_party/wgpu` Metal/MSL translation, regenerate and verify freshness (`FORK_MSL_HASH`, `MSL_LANG_VERSION`, `builtin_programs_cover_every_production_shader`, `precompiled_msl_stages_pass_metal_syntax_check`):
  - Metal MSL (macOS): `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1`
  - Naga IR (any host): `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1`
  - DXIL (Windows + DXC): `NANA_DXC_PATH=<dxcompiler.dll> cargo test -p nana-ui --lib --features gpu,hosted,full -- --ignored regenerate_precompiled_dxil --test-threads=1`
- `nana-gpu` is the contract, WGPU its only backend. Public signatures of nana-gpu, nana-frame-exchange and nana-ui name `wgpu` only behind `wgpu-interop`; framework sources reach WGPU through `nana_gpu::__framework`. Keep `python3 scripts/check-engine-boundary.py` green instead of widening the public `wgpu` surface.
- `FrameContext` is the single authority for submit and discard: a painter records the targets it wrote into the frame, and a dropped frame rolls them back. Every host failure path drops the frame before abandoning the surface texture. Raw queue work needs no guard: the wgpu fork's `Surface::configure` waits only for that surface's work.
- Device loss lives in `GpuContext::is_lost()`; a replaced device is a new `GpuContext` with a new `DeviceGeneration`. Key device-built caches on the generation; resources from another generation are refused, not sent to the backend.
- `GpuTextureView` plus a `TextureSlotName` in the `HostTextureRegistry` is the default product path. Use `GpuView` only for an in-pass view with no intermediate texture. The registry binding (handle identity, generation, version) is a slot's only identity: host-texture views carry revision 0, the painter resolves the slot every frame, and the host redraws on registry change. Do not reintroduce per-view revisions or caller-assigned `HostTexture` ids.
- Paint HostTexture and Custom nodes in document order inside the current destination pass. Do not resolve around interleaved GPU nodes or move them to a frame-end overlay.
- Frame exchange is GPU-side. Keep producer handoff, prepare, present, and sampled-lease lifetime ordered; CPU readback belongs only to explicit snapshot tooling. Application frame data uses the exchange's typed payload, retained with its lease independently of epoch; a binding reads data from its currently sampled frame, never a separate latest-state snapshot.
- Keep `CustomRenderNode` a layout and Scene node. Live2D or other producers supply ordinary HostTexture slots; framework crates do not gain product-specific renderer types.
- Inspect `Cargo.toml`, `Cargo.lock`, and the dependency graph for dependency changes and keep the one wgpu: NanaUI's fork in `third_party/wgpu`.

Test geometry, invalidation, replacement, lifetime, and redraw behavior. Use `$nanaui-validation` for evidence and report untested backends or platforms.
