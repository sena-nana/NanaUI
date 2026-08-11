# nana-tauri-demo

**L1** 通用 **Tauri 前端 → NanaUI** 演示宿主。NanaUI 只提供 **L3** 壳（标题栏 / hosted 窗口）；业务来自你指定的 Tauri 项目前端产物 + JS 引擎，经 CSS/语义映射进入 Style Model，再 MessageBridge → Nana iced-view。可与 **L2** `createWidget` / `nana-*` 同树混合。

- **无** WebView / DOM / CSSOM 进 `nana-ui` / Blitz
- **无** 硬编码业务页（不绑死某个产品）
- **无** NanaUI 仓库内业务 bundle（不加载、不探测 `fixtures/*`）
- **必选** `--project <tauri项目根>`；缺省时报错

权威合同：[`docs/vue-nana-renderer-system.md`](../../docs/vue-nana-renderer-system.md) §0。

## 用法

```bash
cargo run -p nana-tauri-demo -- --project /path/to/SomeTauriApp --window

cargo run -p nana-tauri-demo -- \
  --project /path/to/SomeTauriApp \
  --bundle dist/app.iife.js \
  --entry __nanaAppRun \
  --page home \
  --theme light \
  --window
```

无 `--project`：

```text
missing required `--project <tauri-project-root>`
```

## Bundle 解析（MVP）

优先级（**全部相对 `--project`，不会回落到 NanaUI / CWD**）：

1. `--bundle <iife.js>`（绝对路径，或相对 `--project` 根）
2. 项目根 `nana-demo.toml` 的 `bundle = "..."`（相对 `--project`）
3. 仅在 `--project` 下探测：`dist/*.iife.js`、`nana-demo/dist/app.iife.js` 等；失败时列出已探测路径

可选 `nana-demo.toml`（放在 Tauri 项目根，不是 NanaUI）：

```toml
title = "MyApp"
bundle = "dist/app.iife.js"
entry = "__nanaBoot"
default_page = "home"
theme = "light"

[pages]
home = "__nanaRunHome"
settings = "__nanaRunSettings"
```

`--entry` / `[pages]` 都可省略：若 IIFE 自挂载则只 boot + pump。

## 用外部 Tauri 项目验收（以 LiliaGithub 为例）

业务产物必须在 **该 Tauri 仓库内** 构建；NanaUI 不再提供 `fixtures/lilia-github`。

```bash
# 1) 在 LiliaGithub 仓库内按该项目文档构建 Nana / IIFE 产物
cd ~/work/LiliaGithub
# … yarn / npm build → 例如 dist/lilia-github.iife.js

# 2) 从 NanaUI 仓库启动通用宿主，--bundle 相对 --project
cd /path/to/NanaUI
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome \
  --page home \
  --complete-setup \
  --window
```

也可在 LiliaGithub 根放 `nana-demo.toml`，把 `bundle` / `[pages]` 写进去，之后：

```bash
cargo run -p nana-tauri-demo -- --project ~/work/LiliaGithub --page settings --window
```

## Tauri invoke

| Surface | Status |
|---------|--------|
| `window.__TAURI_INTERNALS__.invoke` → Rust `tauriInvoke` | 通用 soft stub |
| `plugin:window|*` / `plugin:store|*` / dialog / opener | stub |
| 未知 command | `null` + stderr 日志 |
| 真实 Tauri / 网络后端 | **未接** — 用原 Tauri 桌面版 |

演示宿主默认 grant `workspace.switch`（可用 `--no-grant-workspace-switch` 关闭）。

## 标题栏合同

`nana-tauri-demo` 的 `AppTitleBar` / `DesktopShell` **独占**窗口 chrome。Lilia 的 Nana
入口（`NanaAppRoot` + `data-nana-host-chrome`）不挂载自绘 `TitleBar`，避免 home/settings
叠两层 36px。业务页眉（如「项目总览」+ 工具条）不是第二层 titlebar。

## Headless

```bash
cargo run -p nana-tauri-demo --no-default-features \
  --features engine-quickjs,headless-js -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome \
  --page home --complete-setup --headless
```

## Overlay 业务证据（Phase E / X3）

```bash
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome \
  --page home --complete-setup \
  --interact=overlays \
  --png=docs/performance/_overlay-evidence/lilia-home-overlays-quickjs.png
```

打开 Dialog/Drawer/ContextMenu（Nana Overlay）与 Dropdown→Select；产物含 PNG + `.overlay.json`。**禁止** CSS `fixed` 假实现。详见 `_overlay-evidence/README.md`。

## 验收清单

1. `cargo check -p nana-tauri-demo --features windowed`
2. 无 `--project` → 非零退出 + 用法提示
3. 带 `--project` + `--bundle`（项目内相对路径）+ `--entry` 打开窗口，主区为 Vue 业务树 paint（`paint_ops>0`）
4. `cargo tree -i blitz` — 无 blitz
5. **Lifecycle 焦点刷新**（host → shim EventTarget）：
   - 单测：`cargo test -p nana-ui-web-api shim_pumps_window_lifecycle --locked`
   - 集成：`cargo test -p nana-js-quickjs vue_host_pumps_window_lifecycle --locked`
   - 手动（windowed）：启动 Lilia home 后失焦/聚焦窗口应触发 JS `window` `blur`/`focus`（lifecycle 焦点刷新门槛）；缩放手窗口应触发 `resize`；最小化/遮挡应更新 `document.visibilityState` 并派发 `visibilitychange`

Default features: `engine-quickjs` + `windowed`。
