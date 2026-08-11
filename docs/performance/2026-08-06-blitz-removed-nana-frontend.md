# Blitz 移除与 NanaUI 默认前端（2026-08-06）

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

## 决策

**彻底移除** `nana-ui-blitz`、`blitz-dom`、`paint-stub` / `paint-vello`（经 Blitz 的路径）
以及 workspace `vello` patch。**产品 UI 默认且唯一由 NanaUI（Iced）绘制。**

| 层 | 现状 |
|----|------|
| 窗口壳 / 侧栏三层 / Settings | `nana-ui`：`DesktopShell`、`SidebarFrame`、`AppearanceSection` |
| Primary 业务页 | Nana Iced 占位 / 逐步映射；不再走 Blitz 全页 DOM paint |
| Vue / JS | `nana-ui-vue`：轻量 `NanaTreeDocument` + hostOps / web-api / 权限桥；`MessageBridge` → Nana iced-view（CustomContent 已移除） |
| Headless 证据 PNG（Blitz） | **已删除**；改用 Nana snapshot（`ui-snapshots`，见下） |

## Headless 视觉证据（替代 Blitz PNG）

```bash
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
# → target/ui-snapshots/*.png（Iced headless WGPU readback，Cache 保活）
```

详见 [`2026-08-06-missing-nana-foundations.md`](./2026-08-06-missing-nana-foundations.md) #5。

## 移除内容

- workspace member `crates/nana-ui-blitz`（整 crate 删除）
- 依赖：`blitz-dom`、`blitz-traits`、`vello`（及 `[patch.crates-io]` wgpu30 分支）
- feature：`paint-stub`、`paint-vello`（应用层）；`nana-ui-vue` 的空兼容 feature（`layout`/`paint-stub`/`paint-vello`）已删除，改用 `iced-view`
- `nana-tauri-demo` / `vue-counter` / Android 上的 Blitz paint 管线

## 当前默认命令

```bash
# 通用宿主：业务 IIFE 来自外部 Tauri 工程（相对 --project）
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --window
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunSettings --page settings --window

# 验收：无 blitz
cargo check --workspace
cargo tree -i blitz   # 期望：无匹配 / 失败（无该包）
```

## Nana 能力 vs Vue 自由度

**NanaUI 接入 ≠ 限制 Vue。** 二者是「能力与宿主 / 消费与扩展」关系，不是「全部语义化成 Nana widget」。

| 层 | NanaUI 提供 | Vue 保留 |
|----|-------------|----------|
| 壳 / 布局 | `DesktopShell`、`Workspace`、`SidebarFrame`、Region 合同 | 业务路由、页面状态、数据模型 |
| 通用控件 | `Button` / `Switch` / Settings 行等 **可选** 基础 widget | 自造组件、自定义视觉、业务专属 UI |
| 主题 | `lilia-tokens.css` / Iced `ThemeMode` token | 在内容区按需叠加样式（经宿主路径，见缺失清单） |
| GPU | `GpuView` / `nana-gpu` 插槽 | Region 内自定义 GPU 或宿主 paint |

**正确关系**：Nana 提供壳、Region 与通用控件能力；Vue 在 **Primary / 内容 Region** 消费这些能力，并保留扩展自由度——不必 1:1 只用已有 Nana widget。

**错误方向**：强制所有界面语义化成 `NanaButton` 等、禁止 Vue 自定义样式或自造组件。

Blitz 删除后，原先 DOM+CSS 的「任意 Vue 节点即画」路径已移除；**替代承载**是
MessageBridge 语义降维 → Nana iced-view（布局原语 + 基础控件变体/组合），**不是**
CustomContent CPU paint。CustomContent **已移除**。系统化架构见
[`docs/vue-nana-renderer-system.md`](../vue-nana-renderer-system.md)；缺口见
[`2026-08-06-missing-nana-foundations.md`](./2026-08-06-missing-nana-foundations.md)。

### CustomContent（P1 #15，已移除）

- 曾提供 `VueCustomContent` / `CustomPaintScene` CPU 色块 + 位图文字 → RGBA / HostTexture。
- **已删除**：产品与 demo 仅走 `MessageBridge` → Nana iced-view。
- 验收：`cargo run -p vue-counter -- counter --semantic --clicks=2`；
  `cargo run -p vue-counter --features windowed -- --window`；
  `cargo test -p nana-ui-vue --lib style`

## 映射进度（nanavue → NanaUI）

已映射（Iced / windowed）：

- 壳：`AppTitleBar` + `DesktopShell` + Workspace Regions
- 侧栏：`SidebarFrame` / `SidebarRow` / `SidebarFooter`（设置·任务·账号）
- Settings：`settings_page` + `AppearanceSection`（主题 / 圆角）

仍缺基础实现：见 [`2026-08-06-missing-nana-foundations.md`](./2026-08-06-missing-nana-foundations.md)。

### 消息桥（P0 #4，已落地）

- Rust：`nana_ui_vue::MessageBridge` / `BridgeEvent` / `VueHost::semantic_snapshot` /
  `dispatch_bridge_event` / `inject_theme`
- JS：`createWidget`、`nana-button` / `nana-switch` / `nana-sidebar-*` 语义节点；
  `__nanaFireEvent`（Iced→JS）、`__nanaApplyTheme`（Rust→Vue）
- 验收：`cargo run -p vue-counter -- counter --semantic --clicks=2`；
  `cargo check -p nana-ui-vue`；`cargo check -p vue-counter --features windowed`

## 与历史文档

- 旧「Skip Blitz CSS」切片：[2026-08-06-skip-blitz-css-nana-shell.md](./2026-08-06-skip-blitz-css-nana-shell.md) — 现已升级为 **整 crate 移除**
- 旧 paint-vello / paint-stub 证据文档保留为历史存档，**不再描述可运行路径**
