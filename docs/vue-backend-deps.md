# Vue / JS engine dependency policy

## Versions

| Component | Decision |
|-----------|----------|
| WGPU | **30.0.0** — keep aligned with NanaUI/Iced |
| QuickJS | `rquickjs = "0.12.2"` (QuickJS-NG 0.15.1) |
| V8 | crates.io `v8 = "150.4.0"` |
| UI frontend | Runtime / UiScene retained; Iced is the current compatibility view |
| MSRV | ≥ 1.92 |

## Dependency direction

```text
nana-ui-core
     ↑
     ├─ nana-ui-runtime / nana-ui-scene   ← product retained / render
     ├─ nana-ui (Iced adapter / run_hosted / HostTexture)  ← compatibility view
     └─ nana-ui-vue                       ← first-class L1/L2 Vue + JS
          ├─ NanaTreeDocument   (JS custom-renderer ops; simplified layout)
          ├─ MessageBridge      (all visible nodes → Runtime/Scene)
          ├─ iced_app           (Iced compatibility view; feature iced-view)
          ├─ nana-ui-web-api    (window/document/timer/buffered fetch subset)
          └─ nana-js-engine     (traits only)
               ├─ nana-js-quickjs
               └─ nana-js-v8
```

Constraints:

- `nana-ui-core` depends only on serde/serde_json
- `nana-ui-vue` must not depend on concrete QuickJS/V8 types
- App chooses exactly one JS engine (QuickJS XOR V8)
- Windowed UI is Runtime/UiScene, painted by the Iced compatibility view
- WebView is not the product UI path
- Paint / chrome use **host-injected** Device/Queue only — no second `request_device`

## Runtime pipeline

```text
Windowed (default product path):
  HostedProgram + DesktopShell + SidebarFrame + settings_page
  → Runtime/UiScene retained
  → Iced compatibility view on host wgpu

Optional JS bridge:
  Nana Vite entry (Vue SFC/TS/CSS) → reproducible IIFE
  → VueHost::initialize_with_web_api
  → hostOps → NanaTreeDocument + MessageBridge
  → Runtime/UiScene → Iced compatibility view (`iced-view`)
```

## Docs

- 三层兼容合同：[vue-nana-renderer-system.md](vue-nana-renderer-system.md)
- Vue 源码兼容范围：[compatibility-roadmap.md](compatibility-roadmap.md)
- 应用 API 与 Fetch 边界：[capabilities.md](capabilities.md)
- 构建与发布产物：[release-artifacts.md](release-artifacts.md)
