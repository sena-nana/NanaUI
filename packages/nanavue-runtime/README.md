# @nanaui/nanavue-runtime

Vue `createRenderer` host runtime for NanaUI — **L1**（`createElement` + CSS 子集）与
**L2**（`createWidget` / `nana-*`）共用同一 `MessageBridge` 森林。

系统文档：[`docs/vue-nana-renderer-system.md`](../../docs/vue-nana-renderer-system.md)。

Style Model 路径：

1. L1 HTML·class·role·style 或 L2 语义 props → Rust `MessageBridge`
2. `css_map` → Layout；`widget_map` → Semantics；主题档位 → Tokens（非任意 CSS→token）
3. Runtime / UiScene → `run_runtime` → `SceneWgpuPainter`（桌面产品绘制）

`scene-view` 接入同一 Scene/Runtime 适配，不是 Iced widget 树。CustomContent / CPU 简化 paint 已移除。

应用入口见 [`docs/application-api.md`](../../docs/application-api.md)。

## 扩展控件走哪条路

| 目标 | 路径 |
|------|------|
| 新控件进入布局、命中、Scene | Rust `UiExtension::register_component`（Runtime `ComponentRegistry` / `ComponentTypeId`）+ Vue `nana-*` tag |
| 仅 JS 描述符、props 白名单、命令 | `NativeComponentRegistry`（JS host 组件工厂表）+ `Nana.components.call` |

两张表不是同一条 ABI。不要只扩 `WidgetKind`，也不要只注册其中一张表却期望另一条路径生效。

## Source

- `src/createNanaRenderer.js` — hostOps → `__nanaHost.call`, `scheduleJob` via `queueMicrotask`,
  `createWidget` / `nana-*` elements → Rust `MessageBridge`,
  `cloneNode` / `insertStaticContent` / `setScopeId`,
  `__nanaFireEvent` (Runtime/bridge → JS) and `__nanaApplyTheme` (Rust → Vue theme inject).

Consumed by:

- Application-owned Vite/SFC entries using `createNanaApp()`
- `crates/nana-js-engine/fixtures/vue-sfc-compat`（锁定双引擎验收）
- `examples/vue-counter` (semantic / windowed MessageBridge)

Cargo-side Phase 3 Counter still uses
`fixtures/vue-runtime-probe/dist/vue-phase3.iife.js` for the legacy DOM probe path.
