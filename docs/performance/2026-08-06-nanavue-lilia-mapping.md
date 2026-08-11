# NanaUI ↔ LiliaGithub nanavue 映射落地（C 混合）

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

Date: 2026-08-06  
来源结论：[盘点 NanaUI↔Lilia 组件映射](6cca7928-5c56-45a4-bc92-69801ed5d056) · [查仍用Lilia的控件清单](030d5b28-4ea7-4c22-b5ea-dfd40150efe4)

> **更新（同日）**：Blitz 已从 workspace 移除；windowed **仅** NanaUI Iced。
> 见 [`2026-08-06-blitz-removed-nana-frontend.md`](./2026-08-06-blitz-removed-nana-frontend.md)
> 与缺失清单 [`2026-08-06-missing-nana-foundations.md`](./2026-08-06-missing-nana-foundations.md)。

## 能否直接映射？

| 层 | 结论 |
|----|------|
| Rust / Iced | **能** — `docs/lilia-component-parity.md` 已全量对应 |
| Vue / Nana 宿主 | **不能整棵替换** — nanavue MVP 有限；推荐 **C 混合**（= 三层合同的 **L1+L2**）：壳/侧栏/Settings Appearance 用 Nana **可选**基础控件（L2）；Primary/内容区仍可用 HTML/class/style 降维（L1 子集）。权威分层见 [`vue-nana-renderer-system.md`](../vue-nana-renderer-system.md) §0 |
| **windowed 默认** | **Nana Iced 唯一前端**；无 Blitz DOM/CSS/paint |

## 本轮已映射（含 P0 Appearance 控件）

| 区域 | Lilia / 旧实现 | Nana 实现 | 备注 |
|------|----------------|-----------|------|
| Token / controls CSS | 混用 `#3867ff` fallback | `lilia-tokens.css` + `nana-controls.css`（accent `#4991d7`） | 禁止第二套 accent |
| SecondaryPanel **壳** | `LiliaSidebarFrame` | `NanaSidebarFrame` | 顶/中/底合同；body 业务未动 |
| SecondaryPanel **顶导航** | `LiliaSidebarNavRow` | `NanaSidebarRow` | 仅 overview 一行 |
| Settings **侧栏** | `LiliaSettingsSidebar` | `NanaSettingsSidebar` → Frame + Nav + Row | adapter，路由合同不变 |
| Settings **页容器** | `LiliaSettingsPage` | `NanaSettingsPage` | Nana 路由替换；resolve tab + section |
| Settings **Appearance** | `AppearanceSection` + `UiSwitch` / `UiRangeField` / `UiSegmentedControl` / `SettingsRow` | `NanaAppearancePanel` + `NanaSettingsCard` / `NanaSettingsRow` / `NanaSegmented` / `NanaSwitch` / `NanaRangeField` | 中文标签；语义对齐 Lilia + Rust AppearanceSection |
| Theme 切换 | Chip / 英文 Light·Dark | `NanaThemeToggle` → `NanaSegmented`（暗色/浅色） | |
| Home 语言占比模式 | 手写 `language-tabs` button | `NanaSegmented`（P0-lite） | 业务卡片仍 Lilia |
| Repo 工具栏按钮 | `NanaButton` 混用 | `UiButton` | 停止混用 |
| Repo tabs | 手写 button | `NanaTabs` | 有余力补全 |
| P0 shell fixture | 手写 sidebar / settings nav | Frame + Nav/Row + `NanaSettingsPage` | 与 real 路径对齐 |

## Appearance 行对照（P0）

| 行 | Lilia AppearanceSection | NanaAppearancePanel | Rust AppearanceSection |
|----|-------------------------|---------------------|------------------------|
| 主题 | UiSegmentedControl | NanaSegmented（经 NanaThemeToggle） | SegmentedControl |
| 语言 | 静态文案 | 静态文案 | — |
| 窗口材质 | UiSegmentedControl | NanaSegmented（实色/透明） | SegmentedControl + `WindowMaterialMode`（宿主 `nana-window`） |
| 透明区域 | UiSegmentedControl | NanaSegmented | SegmentedControl + `BackdropTarget` |
| 标题栏跟随 | UiSwitch | NanaSwitch | Switch |
| 材质不透明度 | UiRangeField | NanaRangeField | RangeField |
| 组件圆角 | UiSegmentedControl | NanaSegmented（平滑/普通） | Switch「主区域圆角」+ 半径 |
| 组件圆角半径 | UiRangeField | NanaRangeField（8–28） | RangeField |
| 恢复默认 | — | NanaButton | Subtle「恢复默认」 |

## 明确未映射（本阶段不做）

- ~~SecondaryPanel **仓库行**（`RepoSidebarRow` / remote / favorites 业务）~~ → windowed 已用 `SidebarSection` + 同步徽章补 favorites / recent / 仓库列表（演示数据）
- ~~Home 热力图 / 主区业务卡片（语言模式控件除外）~~ → windowed 已用 `CalendarHeatmap` / `Card` / `SegmentedControl` / `ListItem` 补全（演示数据）
- ~~`ContextMenuHost` / Dropdown 等浮层~~ → windowed 产品页已补 ContextMenu + Popover（Gallery 仍保留对照）；Home SearchDropdown 见缺失清单 #8
- 完整 `LiliaWorkspace` / `LiliaResourcePanel` resize
- Account / Workspace / About 真实业务 section（当前仍指向 `NanaAppearancePanel` 占位）
- Repo README 富文本 / Markdown（tabs + 纯文本骨架已有；见缺失清单 P2）
- ~~nanavue → Iced 消息桥~~ → 已落地 `MessageBridge`（**可选**语义控件路径，非唯一渲染方式；见 `SEMANTICS.md`）
- ~~Vue 自定义样式 / 组件宿主（P1 #15）~~ → CustomContent **已移除**；改走语义降维 / 变体组合（见缺失清单 #15）

## nanavue 映射范围（澄清）

`nanavue-components` 映射的是 **可选基础控件**（壳、Settings、Appearance 等），**不是** Vue 的唯一渲染方式。
业务页可在 Primary Region 保留 Lilia 自造组件与自定义视觉；接入 Nana 不要求 1:1 替换为 `NanaButton` 等。

## 圆角尺度（二选一并文档化）

| 管线 | 尺度 | 来源 |
|------|------|------|
| **Vue / Blitz 宿主（本落地）** | Lilia CSS：`--radius-sm` ≈ **12px** | `@lilia/theme` / `lilia-tokens.css` |
| Iced Gallery | `UI_METRICS`：`radius_sm = 6`，`radius_md = 10` | `nana-ui-core` |

颜色语义已同源；几何在两条渲染管线之间**不追求像素一致**。

## 验证

```bash
# 先在 LiliaGithub 仓库内构建 IIFE（NanaUI 不再内置 fixtures/lilia-github）
cd ~/work/LiliaGithub && # … 按该仓库文档 build → dist/lilia-github.iife.js

# windowed Home / Settings（--bundle 相对 --project）
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup --window
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunSettings --page settings --window
```

验收焦点：Settings Appearance 为 NanaSettingsRow/Card；主题/材质为中文 Segmented；Switch/Range 用 `--accent`；无 `#3867ff`；Home 语言模式为 NanaSegmented。
