# Vue / JS engine dependency policy

## Versions

| Component | Decision |
|-----------|----------|
| WGPU | **30.0.0** — keep aligned with NanaUI/Iced |
| QuickJS | `rquickjs = "0.12.2"` (QuickJS-NG 0.15.1) |
| V8 | crates.io `v8 = "150.4.0"` |
| UI frontend | **NanaUI (Iced)** only — Blitz / Vello / CustomContent paint **removed** |
| MSRV | ≥ 1.92 |

## Dependency direction

```text
nana-ui-core
     ↑
     ├─ nana-ui (Iced / run_hosted / HostTexture)  ← product UI
     └─ nana-ui-vue
          ├─ NanaTreeDocument   (JS custom-renderer ops; simplified layout)
          ├─ MessageBridge      (all visible nodes → Nana iced-view)
          ├─ nana-ui-web-api    (window/document/storage/rAF/history shim)
          └─ nana-js-engine     (traits only)
               ├─ nana-js-quickjs
               └─ nana-js-v8
```

Constraints:

- `nana-ui-core` depends only on serde/serde_json
- `nana-ui-vue` must not depend on concrete QuickJS/V8 types
- App chooses exactly one JS engine (QuickJS XOR V8); UI is always NanaUI when windowed
- No `blitz-dom` / `blitz-shell` / `vello` / `anyrender_vello`
- No CustomContent / CPU raster paint path
- Paint / chrome use **host-injected** Device/Queue only — no second `request_device`

## Runtime pipeline

```text
Windowed (default product path):
  HostedProgram + DesktopShell + SidebarFrame + settings_page
  → NanaUI Iced draw on host wgpu

Optional JS bridge:
  esbuild/Vite IIFE
  → VueHost::initialize_with_web_api
  → hostOps → NanaTreeDocument + MessageBridge
  → semantic snapshot → Nana iced-view
```

## Docs

- 三层兼容合同：[vue-nana-renderer-system.md](vue-nana-renderer-system.md) §0（L1 Tauri Vue / L2 nanavue 组件 / L3 Rust；L1+L2 同树混合）
- Blitz 移除：[performance/2026-08-06-blitz-removed-nana-frontend.md](performance/2026-08-06-blitz-removed-nana-frontend.md)
- 缺失基础能力：[performance/2026-08-06-missing-nana-foundations.md](performance/2026-08-06-missing-nana-foundations.md)
- nanavue 映射：[performance/2026-08-06-nanavue-lilia-mapping.md](performance/2026-08-06-nanavue-lilia-mapping.md)
- Issue #5 验收与三层范围：[performance/2026-08-06-issue5-final-acceptance.md](performance/2026-08-06-issue5-final-acceptance.md)
