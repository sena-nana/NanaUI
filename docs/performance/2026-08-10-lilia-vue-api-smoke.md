# LiliaUI → Nana Vue API 冒烟（2026-08-10）

## 路径关系（邻仓）

| 仓库 | 路径 | 关系 |
|------|------|------|
| **LiliaUI** | `/Users/zt-c604184/Documents/workspace/LiliaUI` | `@lilia/*` 视觉/组件事实源 |
| **LiliaGithub** | `/Users/zt-c604184/Documents/workspace/LiliaGithub` | Tauri 消费端；`yarn liliaui:local` → `portal:../LiliaUI/packages/*` |
| **NanaUI** | `/Users/zt-c604184/Documents/workspace/sena-nana/NanaUI` | 宿主；`build-nana-iife.mjs` alias `../sena-nana/NanaUI` 的 nanavue |

同仓：**否**。三仓并列；Nana 路径 = LiliaGithub `src/nana` + `yarn build:nana` → `dist/lilia-github.iife.js`，由 `nana-tauri-demo --project` 加载。

## 可重复命令

```bash
# 1) 链本地 LiliaUI 并打 IIFE
cd /Users/zt-c604184/Documents/workspace/LiliaGithub
yarn liliaui:local   # 可选：yarn liliaui:status
yarn build:nana

# 2) evidence PNG（home / settings）
cd /Users/zt-c604184/Documents/workspace/sena-nana/NanaUI
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project /Users/zt-c604184/Documents/workspace/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --page home --theme light --complete-setup \
  --png=docs/performance/_vue-api-smoke/home-light.png
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project /Users/zt-c604184/Documents/workspace/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --page settings --theme light \
  --png=docs/performance/_vue-api-smoke/settings-light.png
```

证据目录：`docs/performance/_vue-api-smoke/`。

## 本次结果摘要

| 页 | JS settle | 语义门禁 | 主题桥 | 像素 vs iced light 基线 |
|----|-----------|----------|--------|-------------------------|
| home | `ready=true route=/` | `home_ok` / `overview_ok` / cards 可达 | `snap.theme=Light` `token_bg=(255,255,255)` | **偏暗**（sidebar≈`#202020`） |
| settings | `ready=true route=/settings` | `settings_rows=9/9` Appearance 行可达 | 同上 | **主区接近 `#181818`**，对比基线失败 |

挂载与语义降维：**通过**。Light 主题视觉保真：**失败**（桥接 theme 已是 Light，但表面填充仍落暗色）。

## Vue host / DOM / web-api 缺口清单

### P0（阻塞 light 保真 / LiliaUI token）

| ID | 缺口 | 证据 | 落点 |
|----|------|------|------|
| **A1** | `oklch()` / `color-mix()` 完整解析 | `@lilia/theme` tokens 全量 oklch；无解析时表面易落到暗 hex / 暗 Semantic 表面 | `nana-ui-vue` `parse_css_color` |
| **A2** | light 主题下文档级 `--bg` / `--bg-elev` 与 iced 表面不同步 | `snap.theme=Light` 且 `token_bg=白`，PNG 仍 `#181818`/`#202020` | cascade `collect_document_css_custom_properties` + companion CSS 注入序 |
| **A3** | `console.*` 空实现 | QJS/shim 吞掉 warn/error，API 失败不可见 | `nana-js-quickjs` / `nana-ui-web-api` shim → stderr |

### P1（交互 / 观察者）

| ID | 缺口 | 说明 |
|----|------|------|
| **B1** | `MutationObserver` | shim 空 observe；LiliaWorkspace 几何刷新降级 |
| **B2** | `IntersectionObserver` | 空 stub |
| **B3** | `getComputedStyle` / `visualViewport` | 子集；浮层/resize 几何弱 |
| **B4** | `color-mix(...)` | 跳过；heatmap / state-layer 保真差 |
| **B5** | clipboard 真后端 | 平台边界；非本次阻塞 |

### P2（已知 defer / 非阻塞）

- `position: fixed|sticky`、完整 2D grid、`repeat(auto-fit)`、`:hover` / `@media` / `!important`
- Tauri 真 invoke / 网络后端（soft stub）

## 本回合已修

1. **`LengthSpec::{Em,Rem,CalcEmOffset,CalcRemOffset}`** 接入 `length_from_spec`（编译阻塞）。
2. **`oklch()` 无彩度近似**（`L%`→灰阶）+ 单测 `parses_achromatic_oklch_tokens`；**尚未**恢复 light PNG 与基线一致（A2 仍开）。
3. 顺手修了并发改动引入的编译破损（web-api 测试引号、`OsClipboard` Debug、`scroll` `Id::from`、重复 `first_child` 等），以恢复冒烟可跑。

## 未做

- 不 commit；不碰 Android；未改 CSS 大轨（仅颜色解析子集）。
- windowed 人工点击冒烟未跑（evidence 语义门禁已覆盖挂载）。
