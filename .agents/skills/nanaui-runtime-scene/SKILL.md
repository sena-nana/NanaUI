---
name: nanaui-runtime-scene
description: Maintain NanaUI's retained UI contract from core models through Runtime and UiScene. Use when changing UiWorld, components, layout, input, focus, text, motion, extraction, scene structure, or public runtime boundaries.
---

# NanaUI Runtime and Scene

NanaUI has one retained product tree:

```text
application / Rust or Vue input
        -> nana-ui-runtime (UiWorld)
        -> nana-ui-scene (UiScene)
        -> nana-ui (SceneWgpuPainter)
```

Read [`docs/architecture.md`](../../../docs/architecture.md), [`docs/how-it-works.md`](../../../docs/how-it-works.md), and the focused contract document before changing a boundary.

## Ownership

- `nana-ui-core` owns style models, theme tokens, geometry, workspace data, and Motion IR.
- `nana-ui-runtime` owns the retained tree, components, layout, hit testing, focus, IME, accessibility state, Shell, Workspace, Dock, overlays, and GPU-node declarations.
- `nana-ui-scene` owns extraction and the drawable scene. It stays independent of WGPU.
- `nana-ui` owns the host adapter and painter; it does not become a second tree.
- `nana-text` is the only product text measurement and shaping authority.
- Applications own business state, persistence, routes, and Region content.

Keep Rust controls, Vue input, and L3 component creation on the same `ComponentRegistry`, `Style Model`, `UiWorld`, and `UiScene`. `PendingHostOps` commit only at the host frame boundary; `LayoutBoxStore` is a projection and scrolling must not mutate Runtime layout boxes.

## Invariants

- `CustomRenderNode` and `HostTexture` are ordinary scene nodes: they participate in layout, clipping, hit testing, and document order.
- Presentation values are an overlay over logical `UiWorld` values. Do not write transient animation samples back into base state.
- Dirty masks must match the affected contract: input-index changes invalidate input, layout changes invalidate layout, and paint-only changes do not rebuild unrelated stages.
- Keep application content outside the framework. Expose reusable behavior through public region, message, component, and serialization contracts.
- Do not introduce a second UI tree, a second text engine, a WebView product path, or product-specific Live2D/Cubism types.

Route host GPU work to `$nanaui-gpu-integration`, native window behavior to `$nanaui-window-materials`, and checks to `$nanaui-validation`.
