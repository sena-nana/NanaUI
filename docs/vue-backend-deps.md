# Vue / JS engine dependency policy

## Versions

| Component | Decision |
|-----------|----------|
| WGPU | **30.0.0** — keep aligned with NanaUI Scene host |
| V8 | crates.io `v8 = "150.4.0"` (single product JS engine) |
| UI frontend | Runtime / UiScene retained; `SceneWgpuPainter` is the current desktop painter |
| MSRV | ≥ 1.92 |

QuickJS (`rquickjs`) was removed. Release source-hiding via `QuickJsBytecode` is gone;
V8 snapshots remain for host-free prelude embeds (`nana-v8-snapshot`). Full Vue IIFEs
that call `__nanaHost` at top-level stay `SourceUtf8`.

Desktop hosts use official denoland rusty_v8 150.4.0 prebuilts. Android ARM64 is
packaged by workflow `Package V8`; see [`android-arm64.md`](android-arm64.md).

## wgpu-types 29 / 30

Workspace `wgpu` is 30. `bevy_reflect` 0.19.1 (via `bevy_ecs` 0.19.1) still
depends on `wgpu-types` 29.0.4. NanaUI does not enable a second WGPU runtime;
Scene paint uses wgpu 30 only. Do not "fix" this by adding a second Device/Queue.
Bevy 0.19 is the current pin; revisit when Bevy publishes a wgpu-30-aligned
reflect/ecs line.

## Dependency direction

```text
nana-ui-core
     ↑
     ├─ nana-ui-runtime / nana-ui-scene   ← product retained / render
     ├─ nana-ui (Scene host / run_runtime / HostTexture / SceneWgpuPainter)
     └─ nana-ui-vue                       ← first-class L1/L2 Vue + JS
          ├─ NanaTreeDocument   (JS custom-renderer ops; simplified layout)
          ├─ MessageBridge      (all visible nodes → Runtime/Scene)
          ├─ scene-view         (Scene/Runtime adapter)
          ├─ nana-ui-web-api    (window/document/timer/buffered fetch subset)
          └─ nana-js-engine     (traits only; test injection seam)
               └─ nana-js-v8
```

Constraints:

- `nana-ui-core` depends only on serde/serde_json
- `nana-ui-vue` must not depend on concrete V8 types
- App links `nana-js-v8` (`JsEngine` remains the test seam)
- Windowed UI is Runtime/UiScene, painted by `SceneWgpuPainter` via `run_runtime`
- `nana-ui` / `nana-ui-vue` do not depend on Iced or GPUI; `engine/` observation trees were removed
- WebView is not the product UI path
- Paint / chrome use **host-injected** Device/Queue only — no second `request_device`

## Runtime pipeline

```text
Windowed (default product path):
  Vue/JS L1/L2 → MessageBridge → Runtime/UiScene
  → RuntimeProgram → run_runtime → SceneWgpuPainter

Optional JS bridge:
  Nana Vite entry (Vue SFC/TS/CSS) → reproducible IIFE
  → VueHost::initialize_with_web_api
  → hostOps → NanaTreeDocument + MessageBridge
  → Runtime/UiScene → Scene host (`scene-view`)
```

## Docs

- 三层兼容合同：[vue-nana-renderer-system.md](vue-nana-renderer-system.md)
- Vue 源码兼容范围：[compatibility-roadmap.md](compatibility-roadmap.md)
- 应用 API 与 Fetch 边界：[capabilities.md](capabilities.md)
- 构建与发布产物：[release-artifacts.md](release-artifacts.md)
