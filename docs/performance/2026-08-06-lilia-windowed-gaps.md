# LiliaGithub Nana windowed — remaining gaps vs original

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

Date: 2026-08-06（主区 Home 布局 P0 后再更新；**2026-08-06 起 windowed 默认改 Nana Iced 壳**）

> **默认路径已切换**：见 [`2026-08-06-skip-blitz-css-nana-shell.md`](./2026-08-06-skip-blitz-css-nana-shell.md)。
> 本文记录的是 **Blitz full-bleed 实验管线** 的历史差距；不再作为 windowed P0 主路径。

After R1–R7 + Tier A–C + P0 glyph/token + 侧栏三层 + **Home 主区 grid/heatmap** +
**C 混合 nanavue 壳映射**（`docs/performance/2026-08-06-nanavue-lilia-mapping.md`），
NanaUI LiliaGithub 交互可用，侧栏三层与主区两列概览可辨认；**仍非**像素级 WebView 复制。

## Fixed in this pass

| ID | Fix |
|----|-----|
| P0-1 | `text_raster`：取消 `c < 0.28` 硬切；预乘 alpha glyph mask；vello `draw_image` 一次 blit |
| P0-2 | seed / fixture token 对齐编译 `@lilia/theme`：`--accent=#4991d7`、selected=`#e2e2e2` |
| P0-3 | CSS class 提示：`:hover/:focus/:active` 不写入静态 map；`.sb-tree__row.is-active` 复合键 |
| P0-5 | 侧栏宽度 `--sidebar-width: 220px`；行高/圆角/`border:0`；选中灰底非蓝 |
| **P0-6** | **侧栏三层**：Blitz UA 补全 shell→workspace→`secondary-panel` 高度链。回归：`secondary_panel_three_layer_height_chain` |
| **P0-H1** | **热力图溢出/伪色**：`contribution-window` 右对齐裁剪；vello clip 到 crop 祖先；stub 按 path 几何填色（禁 path `layout_box` 大矩形） |
| **P0-H2** | **repo-overview 过矮**：UA `!important` 覆盖 scoped `home-page` 为 `auto auto minmax(150px,0.95fr) minmax(170px,1.2fr)`，R4≈172px（原≈106）。回归：`home_primary_content_height_chain` + dump `primary_assert` |
| **P0-H3** | SVG `width`/`height`→style 镜像加强（双轴 definite） |
| **P0-H4** | page-header / overview-title `min-width:0`；overview-actions Lucide 17×17 |
| **MAP-C** | **nanavue C 混合**：`NanaSidebarFrame/Row/Nav` + Settings adapter + `NanaAppearancePanel`；`nana-controls.css` 禁 `#3867ff` |

## Still different from original (honest)

1. **无 WebView / 无完整 stylo paint** — shadow、图片、复杂渐变仍简化；部分区域对比度仍偏平。
2. **文字**：白边已明显减轻（预乘 mask），小字号 CJK 仍不如 WebView 锐利。
3. **侧栏质感**：footer 控件/连接态仍粗；右边框依赖 paint hairline，不如原版 resize-handle。
4. **Home 工具栏 Lucide** 仍可能偏弱；标题 `· 1 个 root` 在窄宽下仍 ellipsis。
5. **Heatmap/pie** 有形态与 mock；绝对定位裁剪后近期周可见，非整年缩放。
6. **`<nana-gpu home-preview>`** — 主题实心填充，非 Live2D/shader（120px 空白条属预期占位）。
7. **960×640 证据视口**下底部卡仍偏紧；放大窗口后 R4 随 `1.35fr` 增长。

## How to confirm (并排)

```bash
# Nana windowed：先在 LiliaGithub 内构建 IIFE，再 --project 指向该根
cargo run -p nana-tauri-demo --features windowed -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup --window

# 原版
cd ~/work/LiliaGithub && yarn tauri:dev
```

验收焦点：

1. 侧栏可见 **顶导航 / 中部仓库列表（可滚动）/ 底 footer** 三层；footer 贴窗口底。
2. 主区：page-header + overview-actions 同行；贡献/语言两列不横向互叠；底部 pending/repo 可见「已 clone」列表。
3. dump：`primary_assert=ok=true` 且 `repo_h>140`；heatmap 不把语言卡盖住。
4. 文字边缘无明显白晕；侧栏选中灰底；accent ≈ `#4991d7`。

## Evidence PNGs（本机重跑）

- `docs/performance/lilia-real-home-light-quickjs.png`
- `docs/performance/lilia-real-home-dark-quickjs.png`
- `docs/performance/lilia-home-vello-quickjs.png`（与 light home 同步）

```bash
cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-quickjs,paint-vello,evidence-png \
  -- home --complete-setup --png=docs/performance/lilia-real-home-light-quickjs.png
```

Layout dump 示例（home light，主区修复后）：

```
primary:740x568@(220,36)
home-page:692x528
overview-grid:692x150 ; contribution-stack:384 ; language-card:296@(640,250)
repo-overview-grid:692x172@(244,412)
primary_assert=ok=true primary_h=568 repo_h=172
heatmap_right≈612 ≤ stack_right≈628
sidebar_chain=… <- lilia-app-shell__content:960x568 <- lilia-app-shell:960x640
```
