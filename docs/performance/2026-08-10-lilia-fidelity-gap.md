# Lilia iced vs accept fidelity (2026-08-10)

## Scores

| Dimension | Metric | Result |
|-----------|--------|--------|
| Engine parity (hard) | home/settings/repo QJS↔V8 (light + home-dark) | **1.0** |
| Fidelity iced regression (hard) | `l1-fidelity` baseline↔candidate | **1.0** |
| Cross-capture (tracking) | pre-iced accept↔iced home | **~0.821** |
| Cross-capture (tracking) | pre-iced accept↔iced settings | **~0.857** |

## Hard gates (do not pretend)

1. **Engine** — `docs/ui-snapshots/baselines/l1/` QJS↔V8 same iced evidence path（home/settings/**repo**；**X1 已闭合 2026-08-11**）。
2. **Iced regression** — `docs/ui-snapshots/baselines/l1-fidelity/` same-path baseline↔candidate.
3. **Evidence reachability** — `nana-tauri-demo` evidence-png **hard-fails** (no PNG) when home lacks reachable `home-page` / `overview-grid` / cards, settings lacks reachable `SettingsRow`，或 repo 缺可达主区 / README 面板 / cards；另对 repo 要求主区非空壳像素（拒写空白 SSIM 假绿）。

## Pre-iced cross-capture is NOT a hard gate

`_accept-nana-*-window.pre-iced.png` is a historical denser/native-window capture:

| Mismatch | pre-iced | current iced evidence |
|----------|----------|------------------------|
| Viewport | ~1028×708 | **960×640** (`EVIDENCE_SIZE`) |
| Chrome | macOS traffic lights / native title | custom **single** host `AppTitleBar` (Lilia Nana path no longer mounts TitleBar; see 2026-08-10 dual-titlebar fix) |
| Settings tabs | 外观 / 工作区 / 关于 | +账户（业务 tab 集） |

**Do not** resize or crop to fake ≥0.98. Raise this pair only by re-capturing pre-iced at the **same** `EVIDENCE_SIZE` + chrome contract, or by retiring the historical pair. Until then: **tracking only**.

Same-viewport re-score (optional): keep both PNGs at 960×640 iced evidence chrome before comparing; anything else is a different product surface.

## Closed recently

- Appearance density: native `SettingsCard` + 9 `SettingsRow`; evidence `settings_rows=9/9`
- Theme bidirectional sync: JS `dataset.theme` → bridge `ThemeMode` on snapshot; Rust `inject_theme` → `__nanaApplyTheme`; windowed host pulls `snapshot.theme` into paint tokens
- Language chip / pie, heatmap composite, home height chain (prior rounds)
- `position:absolute` measure 子集（至 T-P13：`inset` 2/3/4 值可混 `%`+px）；iced 流内跳过
- **`position:fixed` 视口子集（2026-08-10）**：脱流 + 视口 CB + inset（T-P15–P17）；iced 根 stack 绘制；`sticky` 仍 defer
- `text-overflow:ellipsis` + nowrap；margin/padding 简写 T-B03–B05；max-height T-S07
- `gap` 双值 + `row-gap`/`column-gap`（T-F11/T-F12/T-W03）；measure/iced `main_gap`/`cross_gap`
- 轻量 grid + T-F17–F19 grow·shrink(+min 冻结) + T-W07–W09 column-wrap(+margin) + T-S13/S14 + wrap gap% + T-F15/F16 + T-V02
- iced Fixed flex-shrink：定主轴时与 measure 共享 `resolve_flex_children_main_sizes`（T-F18/F19）
- T-L04：`flex:0 0 220px` 无 width（basis 主轴）→ 220+580
- T-B08：`box-sizing:content-box`（声明宽+padding → border **120×60**）
- T-B09：`border-width` 计入 border-box（100+pad10+bw5 → content 70×10）
- **布局 L1 收敛（2026-08-10）**：css-parity **89/89**；未 defer 可验收项已穷尽；见 [`css-layout-parity.md`](../css-layout-parity.md)「收敛结论」
- **浮层加深**：ConfirmDialog；ContextMenu 多级；Drawer L2（`side`+宽+**footer**）
- **Lucide 业务别名**：trash/copy/pencil/download/git-PR/alert/ellipsis…→现有 glyph
- **Android 壳切片**：chrome fill + Primary **Icon+Text+Input+Switch+Button** + Motion/KeyEvent（非 DesktopShell）
- **Android IME**：NativeActivity InputConnection **defer**；`ime=false`；KeyEvent→iced **模拟器绿**
- **APK 工具链**：`package-android-host-apk.sh` 可出 debug APK；模拟器 KeyEvent **已绿**；真机 **可选**（见 [`android-arm64.md`](../android-arm64.md)）
- **Drawer footer 操作**：`drawer-footer-confirm` → 抽屉 `SelectValue`；`drawer-footer-cancel` → `Toggle false`
- **明确 defer（勿假实现）**：`sticky`；fixed 的 transform/filter 含块 / iframe；完整 2D grid / `repeat(auto-fit\|fill)` 布局消费；iced 流内 absolute→Overlay；假 DesktopShell；假 `ime=true`；Android clipboard 真后端
- **宣称面收敛（2026-08-10）**：相对 **home/settings** 的未 defer 可验收切片已闭合；见 [`android-arm64.md`](../android-arm64.md)「整体收敛状态」
- **宣称面扩展合同（2026-08-10）**：在原硬闸之上立 **X1–X7**；**X5 桌面 clipboard 已闭合**；其余闭合前勿标绿——见本节下方扩展表
- **桌面 clipboard（X5）**：`PlatformCapabilities::desktop().clipboard=true`；shim `navigator.clipboard.readText/writeText` → Rust host → `arboard`；Android 仍 `false`/defer
- 同会话重捕 `lilia-real-home-dark` QJS↔V8 → **1.0**（替换 8/6 陈旧证据）
- **双 titlebar 闭合（2026-08-10）**：Nana 宿主 `AppTitleBar` 独占 chrome；Lilia `NanaAppRoot` 设 `data-nana-host-chrome`、不再挂载 TitleBar（home/settings 同源）
- **通用 CSS 子集加深（2026-08-10）**：grid `max-content` / `minmax(…,auto|%)` / `repeat(N)`；`place-items`/`baseline`；`:first-child`/`:last-child`；扁平 `var(--*)` 表。取证见 companion CSS → LayoutStyle dump（`repo-status-row`/`home-pending-row`/`sync-columns` 此前 `grid_cols=None`）。**非** Lilia class 特判。
- **通用 CSS 再加深（2026-08-10）**：custom-prop **继承**；`vh`/`vw`/`vmin`/`vmax`；`min()`/`max()`/`clamp()`；`calc` 同单位+viewport；未解析 `var()` 网格轨→Auto；`flex-row` 不盖 grid 轴；简单 `:not(.class\|[attr])`。仍缺：`repeat(auto-fit)`、`:hover`/`@media`/`!important`、复杂 `:not(:pseudo)`、完整 2D grid / `sticky` / fixed 含块
- **CSS `position:fixed` 视口子集 + Overlay 分工**：普通节点 fixed → 视口 CB；L2 Overlay 剥离 companion fixed/sticky（X3 仍成立，且**不**替代匿名 fixed）
- **X1 Repo 证据升闸（2026-08-11）**：QJS↔V8 SSIM **1.0**；`baselines/l1/repo-light.png`；reachability + 主区非空壳 hard-fail；修复 author CSS `display:flex` 被 Column kind 默认污染（toolbar SpaceBetween 纵撑裁切）+ Repo 页壳改 flex column
- **Typography CSS 子集（2026-08-11）**：`font-size`/`weight`/`family`、`line-height`、`letter-spacing`、`color` → iced Text 已测绿（路线图 **A-05**）；`bolder`/`lighter`、动态字体、iced 原生 letter-spacing **仍 defer**（见 [`compatibility-roadmap.md`](../compatibility-roadmap.md) / [`css-layout-parity.md`](../css-layout-parity.md)）

## Strategy

Keep engine + Gallery + css-parity green. Layout：通用子集按取证补洞；勿 Lilia 特判；勿假实现 defer 项抬 SSIM。

## 宣称面扩展合同（2026-08-10）— 相对 home/settings 硬闸

> **权威扩展表**（并行轨合同）。原 home/settings 硬闸**保持已闭合**；下列为**增量宣称**——仅在对应验收命令绿后方可标「闭合」。  
> **2026-08-11 审计**：阶段 Todo 与现状矩阵以 [`compatibility-roadmap.md`](../compatibility-roadmap.md) 为准——**X1（Repo 证据）/ X3 业务 Overlay / X4（scrollIntoView）/ X5（桌面 clipboard）/ X6（window 泵送）/ X7（Teleport + 事件矩阵 D-04）已兑现**。本表未逐行回写处，以路线图为准。  
> 交叉引用：[`css-layout-parity.md`](../css-layout-parity.md)、[`vue-nana-renderer-system.md`](../vue-nana-renderer-system.md)、[`android-arm64.md`](../android-arm64.md)、[`2026-08-11-compatibility-phases.md`](2026-08-11-compatibility-phases.md)。

### A. 原硬闸（保持；不因扩展回退）

| 闸 | 合同 | 状态 |
|----|------|------|
| Engine QJS↔V8 | `baselines/l1/` home/settings/repo（含 home-dark）SSIM ≥0.98 | **闭合 1.0** |
| Iced fidelity | `baselines/l1-fidelity/` baseline↔candidate ≥0.98 | **闭合 1.0** |
| Evidence reachability | home：`home-page` + `overview-grid` + cards；settings：Appearance `SettingsRow`；repo：主区 + README + 非空壳像素 | **闭合** |
| css-parity L1 子集 | `cargo run -p nana-css-parity -- compare` | **闭合于子集** |
| Overlay 双侧同步 | Vue `active`/`toggled`；ConfirmDialog / Drawer footer 已交 | **闭合** |
| Android slot+KeyEvent | 见 android-arm64；**本扩展不改 Android 实现宣称** | **闭合于子集** |

### B. 新宣称表（增量；闭合前 = 开放）

| ID | 新宣称（诚实边界） | 相对原硬闸 | 仍 defer / 禁止宣称 | 验收命令（绿才闭合） |
|----|-------------------|------------|---------------------|----------------------|
| **X1** | **Repo 证据页**进引擎硬闸：`--page repo` QJS↔V8 iced evidence SSIM ≥0.98；reachability + 主区非空壳 hard-fail | 原闸仅 home/settings | Diff/Actions 全 workbench；像素级 Markdown | **闭合 1.0（2026-08-11）**：`baselines/l1/repo-light.png`；见下方「Repo」命令块 |
| **X2** | Repo 布局用 **轻量 1D grid**：`repeat(N,…)` / `minmax` / `max-content` / `%` / `fit-content()`（代表：T-G24 `repeat(2,minmax(240px,1fr))`）；`auto-fit\|fill` **解析为 Unsupported**（非静默）；业务作者面改写为诚实 `repeat(N)`，**非** class 特判假展开 | home 已用同类轨 | **勿**宣称 `repeat(auto-fit/fill)` 引擎或完整 2D auto-flow | `cargo run -p nana-css-parity -- compare`；业务 CSS 含 `auto-fit` 时须显式 Unsupported 路径，勿假绿 |
| **X3** | **浮层交互走 Nana Overlay**（Dialog/Popover/Drawer/ContextMenu + click-outside / `contains`）；companion CSS `fixed`/`sticky` **剥离**；Dropdown→Select（非 fixed 菜单） | Overlay 子集已闭合 | **勿**把 L2 浮层改成依赖 CSS fixed；流内 absolute 仍 skip。**匿名** `position:fixed` 走视口子集（T-P15–P17），与 Overlay **并存** | **闭合（业务证据）**：`cargo test -p nana-ui-vue --features iced-view --lib --locked overlay`；`nana-tauri-demo --interact=overlays --png=…` → `docs/performance/_overlay-evidence/` |
| **X4** | **`scrollIntoView` 真定位**：宿主滚动祖先至目标节点可见（非空实现） | 原闸不覆盖 | 平滑滚动选项 / 完整 DOM scrollIntoViewOptions 矩阵 | **闭合（子集）**：`scroll.rs` + hostOp 测；shim→host（见 compatibility-roadmap C-02） |
| **X5** | **clipboard 真后端**（若产品宣称「复制」）：桌面 `ClipboardHost::write_text` 写入 OS；`navigator.clipboard.readText/writeText` 经宿主，**禁止**空 `Promise.resolve()` 冒充成功 | 原闸不覆盖 | Android `clipboard=false` **仍 defer** | **闭合（桌面）**：`OsClipboard`/`arboard` + host ops；`desktop().clipboard=true`；Android stub |
| **X6** | **window focus / blur / resize → JS 泵送**：宿主窗口事件经 shim `EventTarget`/`dispatchEvent` 到达 Vue 监听 | 原闸不覆盖 | CSS `:focus` 伪类 / `@media`；完整 VisualViewport 矩阵 | **闭合**：`pump_lifecycle` + QJS 测 + windowed 接线（见 compatibility-roadmap C-04） |
| **X7** | **Vue host 深度（与并行 Vue 轨对齐）**：`Node.contains` **已有**；**节点缓存**（稳定 nid↔proxy）、**事件桥**（多 listener/capture/fan-out/pointer/Escape）、**Teleport 语义** = 挂到 Nana Overlay / 宿主浮层根（**非** DOM `body` Teleport） | contains/cache/Teleport/事件矩阵 **done**（扇出子集；Transition 诚实 0s） | 完整 DOM Teleport/ARIA portal；祖先链冒泡全模型；真 CSS Transition 时长；CSS `:focus` | D-01–D-04 **已绿**；见 vue-nana §5.1.1 |

### C. 仍 defer（扩展后亦不宣称）

| 项 | 说明 |
|----|------|
| `position: sticky` | 仍 defer；优先 fixed 视口子集 |
| `position: fixed` 含块例外 | `transform` / `filter` / `perspective` / iframe 等改变 CB → **仍 defer**；本子集 CB 恒为视口 |
| 完整 2D grid / `repeat(auto-fit\|fill)` 布局消费 | 解析 Unsupported 或 1D `repeat(N)` |
| iced 流内 absolute 绘制 | skip；产品浮层 Overlay |
| Android clipboard / 软 IME / DesktopShell / V8-on-Android | 见 [`android-arm64.md`](../android-arm64.md)；本扩展**不**抬 Android 宣称 |
| 完整 cascade / `:hover` / `@media` / `!important` | Style Model 子集非目标 |

### D. 扩展面验收命令清单

```bash
# ── 原硬闸（回归，须保持绿）──
cargo run -p nana-css-parity -- compare
cargo test -p nana-ui-vue --features iced-view --lib --locked
# home/settings QJS↔V8 + l1-fidelity：沿用既有 baselines/l1 与 baselines/l1-fidelity 流程

# ── X1 Repo 证据（扩展硬闸）──
# 先在外部 LiliaGithub 构建 IIFE；路径按本机调整
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunRepo --page repo --repo-id=repo-1 --theme=light \
  --png=/tmp/lilia-repo-qjs.png
cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunRepo --page repo --repo-id=repo-1 --theme=light \
  --png=/tmp/lilia-repo-v8.png
# 期望：两 PNG 均写出；reachability 失败则 hard-fail 无 PNG；SSIM(QJS,V8)≥0.98 后升 baseline

# ── X2 grid 诚实策略──
cargo run -p nana-css-parity -- compare
# 期望含 T-G24（诚实 repeat(2,minmax(240px,1fr))）；含 auto-fit 的 fixture 必须 Unsupported / ignore，不得静默当 1fr 假过

# ── X3 Overlay（非 fixed/sticky）──
cargo test -p nana-ui-vue --features iced-view --lib --locked overlay
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup \
  --interact=overlays \
  --png=docs/performance/_overlay-evidence/lilia-home-overlays-quickjs.png
# 期望：PNG 可见 Dialog/Drawer；`.overlay.json` 四类齐全且 overlay_fixed_stripped=true
# 业务：浮层开合 / outside-click 不依赖 position:fixed

# ── X4 scrollIntoView（实现落地后）──
cargo test -p nana-ui-vue --lib --locked   # 含 scrollIntoView 行为测
# 或 nana-tauri-demo --interact=… 证据日志：目标进入可视

# ── X5 clipboard（桌面已闭合；Android 仍 defer）──
cargo test -p nana-ui-platform --lib --locked
cargo test -p nana-ui-web-api --lib --locked
# 期望：memory/OS 写读一致；shim 含 clipboardReadText/WriteText；android_mvp().clipboard=false

# ── X6 window 事件泵送（实现落地后）──
cargo test -p nana-ui-vue --lib --locked   # focus/blur/resize → JS
# windowed：拖拽改尺寸后 JS listener 与 innerWidth/Height 一致

# ── X7 Vue host 深度──
cargo test -p nana-ui-vue --lib teleport_ --locked
cargo test -p nana-ui-vue --features iced-view --lib teleport_mount_root_overlay_coexists_with_css_fixed --locked
# packages/nanavue-runtime：nodeCache / contains / Teleport；事件矩阵（D-04）
cd packages/nanavue-runtime && node --test tests/teleport-contract.test.mjs tests/transition-contract.test.mjs tests/events.test.mjs
cargo test -p nana-ui-web-api --lib shim_event_target --locked
```

**闭合规则**：未过验收的 X* 保持「开放 / 合同已立」；**禁止**把 stub（空 `scrollIntoView`、空 `clipboard.writeText`、未泵送的 resize）写成「已支持」。  
**2026-08-11**：X1/X3/X4/X5/X6/X7 已闭合（见上表与 [`compatibility-roadmap.md`](../compatibility-roadmap.md)）；Android clipboard 仍 defer。
