# Skip Blitz CSS as default — NanaUI Iced shell (Week 1)

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

> **已升级（2026-08-06）**：`nana-ui-blitz` 整 crate 已删除；本文件保留为决策过程存档。
> 现行说明见 [`2026-08-06-blitz-removed-nana-frontend.md`](./2026-08-06-blitz-removed-nana-frontend.md)
> 与 [`2026-08-06-missing-nana-foundations.md`](./2026-08-06-missing-nana-foundations.md)。

Date: 2026-08-06  
Decision source: [强化跳过Blitz CSS结论](9f6d76cd-442c-4ed8-80f2-022dd1ee8b9b)

## 能否跳过 Blitz 排版？

**能，而且应当作为默认可靠路径。**（后续已进一步：编译依赖亦移除。）

| 层 | 决策 |
|----|------|
| 窗口壳（titlebar / 侧栏三层 / 主区几何） | **NanaUI Iced**：`app_shell` / `DesktopShell` / `workspace_view` / `SidebarFrame` |
| 侧栏 footer 图标 | **Rust** `SidebarFooter` / `SidebarFooterButton`（不依赖 Blitz Lucide） |
| Settings / tab / switch / appearance | **Iced** `settings_sidebar` + `settings_page` + `AppearanceSection`（或 nanavue 只填 Primary） |
| 业务复杂页（短期） | 可挂 Primary Region；用 Nana Region 几何或简化自研布局 |
| Blitz CSS / stylo / flex 定高链 | **降级为过渡/实验**，不再全量吃 `@lilia/ui` stylesheet |

动机：Blitz 对 CSS 级联、伪类、oklch、overflow、flex/grid 定高链均不可靠——不只是「补一条 height UA」能修好。继续以修更多 Blitz CSS 为主路径会反复失败。

## 跳过什么 / 保留什么

### 跳过（非默认）

- 用 Blitz 解析整页 `.lilia-app-shell` → workspace → `secondary-panel` 高度链
- 依赖 Blitz 画侧栏 footer Lucide（「footer 消失」实为未画出小图标，非几何丢了）
- 以 `ensure_shell_height_ua_stylesheet` / 更多 `!important` 补丁作为主修复手段
- windowed 全屏 `GpuTextureView` = 整棵 Lilia DOM（含壳）

### 保留（默认 / 合同内）

- NanaUI Iced Gallery / `component-gallery` 不回退
- 单 wgpu Device/Queue；无 WebView
- Vue/JS 引擎、nanavue 组件、权限桥（Issue #5 其他项）
- Blitz **paint-stub / paint-vello** 可作为 Primary Region 内容纹理的**可选**后端（viewport = Region 尺寸，**不含** LiliaAppShell CSS 链）
- Headless evidence PNG 路径可暂留 Blitz 实验管线，但文档标注「非默认」

## Week 1 最小切片（已落地）

`nana-tauri-demo --features windowed -- home --window`：

1. `DesktopShell` + `AppTitleBar` 定壳
2. `SidebarFrame`：顶「概览」+ 中部 mock 仓库列表（可滚）+ 底 **原生** 设置/任务/账号
3. Primary：`GpuTextureView` 仅占 Nana Primary rect（solid slot；无 Lilia shell CSS）
4. Settings：纯 Iced `settings_sidebar` + `settings_page` + `AppearanceSection`

```bash
# 先在 ~/work/LiliaGithub 内构建 IIFE；--bundle 相对 --project（非 NanaUI）
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --window
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunSettings --page settings --window
```

验收焦点：侧栏三层可见且 footer 图标由 Iced 绘制；主区随窗口 resize；不依赖 Blitz 定高链。

## 与 Issue #5 的关系

| Issue #5 项 | 关系 |
|-------------|------|
| #1 Gallery 不回退 | **不变** — 本切片只改 `nana-tauri-demo` windowed |
| #6 Blitz/Vello 布局与绘制 | **重新定界**：paint 能力保留为可选；**CSS/layout 保真不再阻塞默认产品路径**。#6「部分」仍成立，但默认 UI 几何改由 Nana Iced 保证 |
| #8 主题/基础组件/Workspace | **加强** — windowed 直接走 Nana Workspace + AppearanceSection |
| #9 `<nana-gpu>` / HostTexture | Primary `GpuTextureView` 仍走宿主单 Device 合同 |

结论：Issue #5 MVP 签字不撤回；windowed 产品路径从「Blitz 吃全量 Lilia CSS」转向「Nana 排版 + 可选 Blitz 内容纹理」。

## 后续（非本切片）

- Primary 内挂简化 Home 业务（Iced 或 nanavue，viewport=Region）
- 逐步减少 headless 对全量 Lilia PAGE CSS 的依赖
- Blitz UA 定高链测试可保留为实验回归，不再作为 windowed P0
