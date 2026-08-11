# 缺失的 Nana 基础实现清单（Blitz 移除后）

Date: 2026-08-06  
上下文：[`2026-08-06-blitz-removed-nana-frontend.md`](./2026-08-06-blitz-removed-nana-frontend.md)、
[`2026-08-06-nanavue-lilia-mapping.md`](./2026-08-06-nanavue-lilia-mapping.md)

Blitz / CSS / paint 管线删除后，原先由 Vue DOM + Blitz 承担的能力不再可用。
下列项是 **NanaUI / nanavue→Iced 仍缺的基础实现**（按优先级大致排序）。

> **架构边界（变体 / 组合）**：所有可见 UI 最终落到 **NanaUI 基础能力**
> （布局原语 + 基础控件及其变体）。Vue 自由度体现在 **组合、逻辑与变体参数**，
> 不是另起一套 paint。裸 `div`/class/role 应 **降维映射** 到 Nana 布局盒与控件变体。
> CustomContent / `VueCustomContent` 简化 paint **已移除**。

## P0 — 阻塞 LiliaGithub 产品页

| # | 能力 | 说明 | 建议落点 | 状态 |
|---|------|------|----------|------|
| 1 | Home 主区业务 | 真实业务 IIFE → 语义树降维 → Nana widget；经通用宿主 `--project` + `--bundle` 加载 | `nana-tauri-demo` | **通用加载器（Lilia 仅作验收示例）** |
| 2 | Repo 详情页 | 真实 `__nanaLiliaRunRepo`（NanaRepoPage readme/files 子集） | 同上 `--entry` / `[pages]` | **同上** |
| 3 | Profile 页 | 真实 `__nanaLiliaRunProfile` | 同上 | **同上** |
| 4 | nanavue → Iced 消息桥 | `MessageBridge` + HTML/class/role 降维；`iced-view` 用真实 `nana_ui::*` 绘制。验收：`vue-counter --semantic` / `--features windowed` | `nana-ui-vue` + `nanavue-runtime` + `nanavue-components` | **已落地（扩展映射）** |
| 5 | Nana 截图 / 证据路径 | 替代 Blitz evidence PNG；`component-gallery --bin ui-snapshots --features snapshots` → `target/ui-snapshots/*.png`（Iced headless WGPU readback） | `component-gallery` / hosted readback | **已落地（ui-snapshots）** |

## P1 — 壳与控件 parity

| # | 能力 | 说明 | 状态 |
|---|------|------|------|
| 6 | 仓库侧栏行业务态 | 业务侧栏经语义降维 / nanavue `SidebarRow`；手写演示见 `demo-shell` | **默认走真实 IIFE；demo-shell 归档** |
| 7 | 浮层完整集 | Dialog/Popover/Drawer/ContextMenu；ConfirmDialog；Drawer **`side`+宽+footer 插槽**；footer **confirm/cancel 操作约定**（对齐 ConfirmDialog）；Vue `active`/`toggled` 双侧同步 | **宣称闭合**（嵌套确认流=按需加深） |
| 8 | Search / 命令输入 | 真实 Home 搜索在业务包内；demo-shell 仍有 SearchDropdown | **默认真实包；demo-shell 归档** |
| 9 | 窗口材质设置段 | Appearance 中材质/透明度行；loader `settings` 页走真实 `__nanaLiliaRunSettings`；`demo-shell` 仍有 Nana AppearanceSection | **loader settings + demo-shell 双路径** |
| 10 | Android Nana 壳 | 几何 + chrome fill + Primary **Icon+Text+Input+Switch+Button** + Motion/**KeyEvent**（模拟器 logcat 绿）；完整 DesktopShell / 软 IME **defer**（`iced_shell=false` / `ime=false`） | **slot+KeyEvent 宣称闭合** |
| 15 | ~~Vue 自定义样式 / 组件宿主路径~~ | **已移除** CustomContent 简化 paint；自定义 = Nana 变体/组合 + 语义降维。 | **已移除** |

## P2 — 桥与引擎（按需加深；非宣称阻塞）

| # | 能力 | 说明 | 状态 |
|---|------|------|------|
| 11 | 树文档几何 | `NanaTreeDocument` 为合成 stack/嵌套布局；可见绘制以 Iced 语义 view 为准 | 诊断路径；非宣称缺口 |
| 12 | 富文本 / Markdown 主区 | Repo README → Nana 文本/列表变体组合（非位图） | **按需加深** |
| 13 | SVG / Lucide 图标全量 | **常用业务别名已扩**（trash/copy/pencil/download/upload/external-link/ellipsis/git-PR/alert…→现有 glyph）；全量 Lucide 矢量路径仍缺 | **子集闭合**；全量=按需 |
| 14 | 主题 token 双向同步 | JS `dataset.theme` / `setDocumentTheme` ↔ bridge `ThemeMode` + `inject_theme`；windowed 拉 `snapshot.theme` | **已最小闭合** |
| 16 | Dialog / Dropdown / Popover / ContextMenu 语义节点 | Dialog/Popover/ContextMenu iced 映射已有；ContextMenu→MenuStore+ContextMenuHost；Dropdown→Select | **已落地** |
| 17 | Textarea 完整 Content 合同 | `EditorStore` 宿主持有 `text_editor::Content`，`BridgeEvent::Editor` + `prepare_editors`；无 store 时仍回退 Input | **已最小闭合** |

> **2026-08-10 宣称面收敛**：布局 L1 / 浮层加深 / Android slot+KeyEvent/APK 等相对 **home/settings** 的**未 defer 可验收项已闭合**。  
> **同日扩展合同 X1–X7**（Repo / Overlay 非 fixed / scrollIntoView / 桌面 clipboard / window 泵送 / Vue host 深度）：见 [`2026-08-10-lilia-fidelity-gap.md`](2026-08-10-lilia-fidelity-gap.md)；多数项**验收前开放**；**桌面 X5 clipboard** 并行轨已交可标闭合，Android clipboard 仍 defer。余项见 [`android-arm64.md`](../android-arm64.md) Defer；勿假实现 DesktopShell / `ime=true` / `fixed`/`sticky` / Android 空 clipboard。

## 明确不再做

- 恢复 `blitz-dom` / stylo / paint-stub / paint-vello 作为默认或可选产品路径
- WebView / 第二套 wgpu Device
- 用更多 CSS UA 补丁修复定高链
- 以 `VueCustomContent` / CustomContent 简化 paint 作为自定义 UI 的呈现通道（**已移除**）

## 验证锚点

```bash
cargo check --workspace
cargo tree -i blitz          # 无 blitz / blitz-dom / nana-ui-blitz
cargo run -p vue-counter -- counter --semantic --clicks=2
cargo check -p vue-counter --features windowed
cargo test -p nana-ui-vue --features iced-view --lib
cargo check -p nana-tauri-demo --features windowed
# 先在 LiliaGithub 内构建 IIFE；--bundle 相对 --project（NanaUI 无内置业务包）：
cargo run -p nana-tauri-demo -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup --window
```
