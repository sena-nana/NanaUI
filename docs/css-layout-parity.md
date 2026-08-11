# CSS 布局一致性测试（WebView 参照）

> **产品运行时无 WebView**（见 `AGENTS.md`）。本测试集仅在 `nana-css-parity` 包内可选启用 WebView / headless 浏览器作 **golden 参照**，**不得**链进 `nana-ui` 默认依赖。
>
> 全栈兼容阶段 Todo（布局 = Phase A；定位 = Phase B）：[`compatibility-roadmap.md`](compatibility-roadmap.md)。

## 目标

同一套 HTML/CSS fixture：

1. **Nana**：`inline style` / class hints → [`LayoutStyle`](../crates/nana-ui-core/src/box_layout.rs)（经 [`css_map`](../crates/nana-ui-vue/src/css_map.rs) parse）→ [`measure_layout`](../crates/nana-ui-vue/src/measure.rs) 得到 `(x,y,w,h)`
2. **参照**：fixture 内嵌 `expected`（CSS 正确期望），或 `webview-ref` 下 WKWebView/`wry` 的 `getBoundingClientRect`
3. **断言**：对应节点盒在容差内一致（默认 **±2px**，用例可覆盖）

语义对齐 iced `row`/`column` + `LayoutStyle` 子集，**不是**完整 CSS 引擎；无 Blitz。

## 如何跑

```bash
# 默认：Nana measure vs fixture expected（无 WebView）
cargo test -p nana-css-parity

# 列出用例
cargo run -p nana-css-parity -- list

# 比较全部 pass 用例
cargo run -p nana-css-parity -- compare

# 导出某用例 HTML（可粘到 Safari / Playwright）
cargo run -p nana-css-parity -- html T-F01

# 可选：本机 WebView 直播对照（需显示环境；CI 无显示请跳过）
cargo test -p nana-css-parity --features webview-ref -- --ignored webview
cargo run -p nana-css-parity --features webview-ref -- compare --webview

# 强制跳过 WebView
NANA_CSS_PARITY_SKIP_WEBVIEW=1 cargo run -p nana-css-parity --features webview-ref -- compare --webview

# 像素闸门（Gallery 快照子集，SSIM similarity ≥ 0.98；见 scripts/pixel_ssim_compare.sh）
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
./scripts/pixel_ssim_compare.sh --dir docs/ui-snapshots/baselines target/ui-snapshots
```

## 一致性定义

| 项 | 约定 |
|----|------|
| 坐标系 | 相对 viewport 左上，逻辑像素 |
| 盒模型 | fixture HTML 使用 `box-sizing: border-box`；Nana measure 的 `w/h` 为 border box |
| 对齐节点 | `data-id` / fixture `id` |
| 容差 | 默认 `tolerance_px: 2`；亚像素 / 字体度量差异 |
| 隐藏节点 | `display:none` / `visibility:hidden` → Nana **均不产出盒**（非 CSS 占位）；WebView 侧亦不参与比较 |
| 非目标 | 完整 cascade / 伪类 / `sticky` / absolute·fixed 完整定位引擎（含 transform 含块） / 完整 2D grid / 完整 `calc()` AST |
| 轻量已验收 | grid 至 T-G24；T-F17–F22；T-L04 basis；T-B08/B09 content-box·border；T-B11/B12/P14 逻辑盒 LTR；T-P15–P17 fixed 视口；T-W07–W09；T-S13/S14；wrap gap%；T-V02 |

## 用例矩阵（盘点优先）

| ID | 描述 | 默认 | 备注 |
|----|------|------|------|
| T-F01 | 横排 gap | **pass** | |
| T-F02 | 纵排 gap | **pass** | |
| T-F03 | justify center | **pass** | `JustifySpec` + measure / iced |
| T-F04 | space-between | **pass** | 原 P0-1 |
| T-F05 | align-items center | **pass** | |
| T-F06 | flex:1 行等分 | **pass** | |
| T-F07 | flex:1 列等分 | **pass** | 原 P0-2：`child_main_length` |
| T-F08 | 220px + flex:1 | **pass** | |
| T-F09 | space-around | **pass** | measure + iced Fill 近似（端 1 / 间 2） |
| T-F10 | space-evenly | **pass** | measure + iced n+1 Fill |
| T-F11 | gap 双值 row | **pass** | `gap:8px 20px`；主轴=column-gap |
| T-F12 | row/column-gap 长手 | **pass** | Column 主轴=row-gap |
| T-F13 | row gap % | **pass** | `gap:10%` @200 → 间距 20；b@x60 |
| T-F14 | column gap % | **pass** | `gap:10%` @h300 → 间距 30；b@y70 |
| T-F15 | justify flex-end | **pass** | 主轴末端打包；a@310 b@360 |
| T-F16 | align flex-end | **pass** | 交叉轴末端；tall@y20 short@y80 |
| T-F17 | flex-grow 加权 | **pass** | `flex:1` + `flex:2` @300 → 100+200 |
| T-F18 | flex-shrink 溢出 | **pass** | 150+150@200 shrink:1 → 100+100 |
| T-F19 | shrink + min 冻结 | **pass** | min-width:120 冻结 → 120+80（非 100+100） |
| T-F20 | align-self | **pass** | 覆盖容器 `align-items`；short@y80 |
| T-F21 | row-reverse | **pass** | main-start 对调；a@160 b@110 |
| T-F22 | flex `order` | **pass** | 升序（含负）再源序；b(-1)→c(0)→a(2) |
| T-S01 | width 50% | **pass** | measure 带父盒 |
| T-S02 | height 100% 链 | **pass** | measure Fill 链；iced 产品路径建议再回归 |
| T-S03 | min-height | **pass** | |
| T-S04 | min-width:0 | **pass** | 几何盒；ellipsis 见 iced/`text-overflow` |
| T-S05 | px 精确 | **pass** | |
| T-S06 | max-width 钳制 | **pass** | Fill 子项 clamp |
| T-S07 | max-height 钳制 | **pass** | Fill 列子项 clamp |
| T-S08 | calc(100% - Npx) | **pass** | 轻量 `CalcPercentOffset`；非完整 calc |
| T-S09 | 嵌套 width% + calc | **pass** | mid 50% → leaf `calc(50%-10px)` |
| T-S10 | calc 轻量扩展 | **pass** | `px+%` / `%+%` / `px-px`（非 AST） |
| T-S11 | 嵌套 padding 父链 % | **pass** | px pad → % pad → width 50% → 144 |
| T-S12 | min-width 非零保底 | **pass** | `width:10%;min-width:80px` @300 → 80 |
| T-S13 | 多子项 flex min 重分配 | **pass** | `flex:1;min-width:200` + `flex:1` → 200+100 |
| T-S14 | 多子项 flex max 重分配 | **pass** | `flex:1;max-width:100` + `flex:1` → 100+200 |
| T-S15 | vh / min / clamp / vw-calc | **pass** | `50vh`/`min(520px,92vw)`/`calc(100vw-32px)`/`clamp` |
| T-S16 | em / rem / calc(em) | **pass** | 默认 16px；`2em`→32；`calc(2em+8px)`→40；`min(10rem,50%)` |
| T-L01 | app shell Fill | **pass** | class + height Fill |
| T-L02 | 侧栏+主区 | **pass** | 显式 `width:220` + `flex:1` |
| T-G01 | grid 220px 1fr | **pass** | template-columns → Row 轨宽；非完整 grid |
| T-G02 | grid var + minmax | **pass** | `var(--x,200px) minmax(0,1fr)` |
| T-G03 | grid 80px 1fr 1fr | **pass** | 双 1fr 等分剩余宽 |
| T-G04 | grid 100px 1fr 2fr | **pass** | 加权 fr（1:2）分剩余宽 |
| T-G05 | minmax 非零下限 | **pass** | `minmax(400px,1fr) 1fr` 冻结 400+200 |
| T-G06 | 1fr 1.5fr | **pass** | 小数 fr 权重 2:3 |
| T-G07 | 多轨 min 冻结 | **pass** | 两轨同时触达 min → 250+250+100 |
| T-G08 | minmax 像素上限 | **pass** | `minmax(50px,120px) 1fr` → 120+280 |
| T-G09 | 多轨 max 冻结 | **pass** | 两轨同时触达 max → 100+100+200 |
| T-G10 | template-rows + gap | **pass** | `100px 1fr 1fr` + `gap:20px 40px` → 100/130/130 |
| T-G11 | rows minmax 下限 | **pass** | `minmax(200px,1fr) 1fr` @300 → 200+100 |
| T-G12 | rows minmax 上限 | **pass** | `minmax(50px,120px) 1fr` @400 → 120+280 |
| T-G13 | rows 多 min 冻结 | **pass** | 两轨同时触达 min → 120+120+60 |
| T-G14 | rows 多 max 冻结 | **pass** | 两轨同时触达 max → 80+80+140 |
| T-G15 | columns+rows 同节点 | **pass** | rows 先写仍走 columns；100+300 横排 |
| T-G16 | columns:none→rows | **pass** | 清 columns 后纵向 80+120 |
| T-G17 | rows:none→columns | **pass** | 清 rows 后横向 100+300 |
| T-G18 | 双边 none→Row | **pass** | 清尽轨后横排，不残留 Column |
| T-G19 | grid 轨 gap% | **pass** | `gap:10%` @300 → 间距 30；1fr=170 |
| T-G20 | repeat(3,1fr) | **pass** | 固定次数展开 → 200×3 |
| T-G21 | 轨 % + 1fr | **pass** | `25%`→`GridTrack::Percent`，布局相对内容宽兑现 → 100+300 |
| T-G22 | inline-grid | **pass** | 1D 轨消费同 `grid` |
| T-G23 | fit-content(Npx) | **pass** | ≈ minmax(auto,N) 上限钳制 → 120+280 |
| T-G24 | repeat(2,minmax(240px,1fr)) | **pass** | Repo 诚实轨（非 auto-fit）；600−12 → 294+294 |
| T-L03 | settings row | **pass** | `.nana-settings-row` hints |
| T-L04 | flex-basis 侧栏 | **pass** | `flex:0 0 220px` **无 width** → 220+580 |
| T-B01 | padding 四值 | **pass** | |
| T-B02 | margin 四值 | **pass** | measure 推进下一兄弟；iced 外层 padding |
| T-B03 | margin 两值 | **pass** | 垂直/水平简写 |
| T-B04 | padding 三值 | **pass** | top \| horizontal \| bottom |
| T-B05 | margin 三值 | **pass** | top \| horizontal \| bottom |
| T-B06 | % pad + % margin | **pass** | pad10% + margin 0 10% → a@72 b@184 |
| T-B07 | column % margin | **pass** | `margin:10% 0` 相对宽度 → a@y20 b@y80 |
| T-B08 | content-box 宽+padding | **pass** | `width:100;padding:10` → border **120×60**；b@x120 |
| T-B09 | border-width 计入 border-box | **pass** | `100+pad10+bw5` → content **70×10**；inner@15；b@x100 |
| T-B10 | 负 margin + rem pad | **pass** | `margin-right:-20` → b@x60；`padding:0.5rem` |
| T-B11 | logical padding → LTR | **pass** | `padding-block/inline` ≡ T-B01 几何 |
| T-B12 | logical margin → LTR | **pass** | `margin-*-start/end` ≡ T-B02 几何 |
| T-V01 | display:none | **pass** | 不产出盒、不占位 |
| T-V02 | visibility:hidden | **pass** | Nana：等同跳过（非 CSS 占位）；几何同 T-V01 |
| T-W01 | flex-wrap | **pass** | measure 折行；iced borrowed/owned 多行拆分 |
| T-W02 | wrap-reverse | **pass** | 行序反转；与 T-W01 同几何 |
| T-W03 | wrap 双值 gap | **pass** | 行内 column-gap / 行间 row-gap |
| T-W04 | wrap + 水平 margin | **pass** | 折行判断含 margin 左右 |
| T-W05 | wrap 行间 gap% | **pass** | `gap:10% 12px`；auto 高→row-gap% 回退宽=20 |
| T-W06 | wrap-reverse + gap% | **pass** | 同 T-W05 几何，行序 cd↑ / ab↓ |
| T-W07 | column + flex-wrap | **pass** | 纵排折列；a/b@x0 c/d@x88；iced row-of-columns |
| T-W08 | column wrap-reverse | **pass** | 同 T-W07 几何，列序 cd@x0 / ab@x88 |
| T-W09 | column-wrap + 垂直 margin | **pass** | outer=72 触发折列；b@x88（对称 T-W04） |
| T-P01 | position:relative + inset | **pass** | |
| T-P02 | absolute 脱流 + top/left | **pass** | 相对 relative 祖先；不占流 |
| T-P03 | absolute right/bottom | **pass** | |
| T-P04 | absolute left+right / top+bottom | **pass** | 双侧 inset 定宽高 |
| T-P05 | absolute + 父 padding CB | **pass** | padding box 原点/尺寸 |
| T-P06 | absolute 无 inset 静态落点 | **pass** | CB 原点 |
| T-P07 | absolute 百分比 inset | **pass** | `%` 相对 CB 宽/高 |
| T-P08 | 嵌套 absolute | **pass** | 子相对父 absolute padding box |
| T-P09 | inset 两值简写拉伸 | **pass** | `inset: y x` → 四边 + 定宽高 |
| T-P10 | inset 混用 `%`+px | **pass** | `inset:10% 8px` |
| T-P11 | 单边混用 `%`+px 拉伸 | **pass** | left%/top px/right%/bottom px |
| T-P12 | inset 三值混用 `%`+px | **pass** | top \| horizontal \| bottom |
| T-P13 | inset 四值混用 `%`+px | **pass** | top right bottom left |
| T-P14 | logical inset → LTR | **pass** | `inset-block/inline` ≡ T-P09 几何 |
| T-P15 | fixed 脱流 + 视口 top/right | **pass** | 匿名盒；CB=viewport；不占流 |
| T-P16 | fixed 百分比 inset 相对视口 | **pass** | `%` 相对 viewport，非祖先 |
| T-P17 | fixed left+right / top+bottom | **pass** | 双侧 inset 定宽高相对视口 |
| — | iced 流内 absolute | **skip** | 不绘制、不占流；浮层用 Overlay |
| — | iced 流内 fixed | **pass** | 流内跳过；根 stack 视口层绘制（非 Overlay） |
| — | sticky | **defer** | 声明缺口；优先 fixed 可用 |
| — | `direction:rtl` / `writing-mode` 竖排逻辑映射 | **defer** | 逻辑盒属性仅默认 LTR↔physical；勿假翻轴 |
| — | 完整 2D grid | **defer** | 仅轻量 template 轨（至 T-G24 + repeat(N)/max-content/%/`fit-content()`）；无 auto-flow 布局消费 / 跨轨 / 区域 |
| — | iced Fixed `flex-shrink` | **pass** | 定主轴时 `resolve_flex_children_main_sizes` → Fixed 覆盖（T-F18/F19） |
| — | `repeat(auto-fit/fill)` | **defer** | 解析为 [`GridTrackListUnsupported`](../crates/nana-ui-core/src/box_layout.rs)（`grid_*_unsupported`）；**勿**静默丢弃；仅固定次数 `repeat(N, …)` 展开。**Repo 证据页**作者面改为诚实 `repeat(2, minmax(240px, 1fr))`（T-G24），不以业务 class 特判假展开 |
| — | `grid-auto-columns` / `rows` / `flow` | **defer** | **解析保留**；measure/iced **不**消费（隐式轨 / auto-placement） |
| — | `:hover` / 复杂 `:not` / `:has` / `@media` / `!important` | **skip** | 选择器诚实跳过；`:first-child`/`:last-child`/`:root`/`:is`/`:where`/简单 `:not` 已支持 |
| — | 兄弟组合器 `+` / `~` | **pass** | Selectors L4 §14.3–14.4；`MatchContext.preceding_siblings` |

Fixture 目录：[`crates/nana-css-parity/fixtures/`](../crates/nana-css-parity/fixtures/)。

## 收敛结论（2026-08-10）

**判定**：在「iced row/column + `LayoutStyle` 子集、非完整 CSS 引擎」边界内，**未 defer 且可验收的布局 L1 缺口已基本穷尽**。
`css-parity compare`：**pass≈96**（含 T-F20/F21/F22；Grid 轨 T-G21 若失败属并行轨，非本 Flex 交付）。

### 已支持矩阵（摘要）

| 族 | 覆盖 | 代表 |
|----|------|------|
| Flex 主/交叉轴 | gap / % gap / justify / align(+self) / grow·shrink(+min) / basis / reverse / flow / order | T-F01–F22、T-L04 |
| 尺寸与 calc | px/% / min·max / 轻量 calc / 嵌套 % | T-S01–S14 |
| 盒模型 | pad·margin 2–4 值与 %、content-box、border-width∈border-box、逻辑盒（LTR） | T-B01–B12 |
| Wrap | row/column ± reverse、双值 gap、margin 触发折行 | T-W01–W09 |
| 轻量 grid | template columns/rows、fr/minmax/%/fit-content()、repeat(N)、inline-grid、轨切换、gap% | T-G01–G24（1D 轨；G24=Repo `repeat(2,minmax)`） |
| 定位子集 | relative；absolute 脱流+inset；**fixed 视口子集**（脱流+inset+z-index） | T-P01–P17 |
| 可见性 | `display:none` / `visibility:hidden` → Nana 均跳过 | T-V01–V02 |
| Shell hints | Fill 链、侧栏+主区、settings-row | T-L01–L03 |
| Typography（2026-08-11） | `font-size`/`weight`/`family`、`line-height`、`letter-spacing`（row 近似）、`color` → iced Text | `css_map` + iced-view typography 测；路线图 **A-05** |
| Button chrome（2026-08-11） | padding / border / radius / bg / color / gap → `Button`/`IconButton` 内层；外层 `consume` 跳过双层 | `ButtonPaintOverride`；路线图 **A-06**；[`2026-08-11-button-css-chrome.md`](performance/2026-08-11-button-css-chrome.md) |

### 明确 defer / skip（勿假实现）

| 项 | 状态 | 产品替代 |
|----|------|----------|
| `position: fixed` 视口子集 | **pass**（T-P15–P17） | 普通节点：视口 CB + inset；iced 根层绘制。**非**完整 fixed 引擎（见 defer） |
| `position: sticky` | **defer** | 仍缺口；优先 fixed |
| 完整 2D grid（auto-flow 布局 / 跨轨 / areas / `repeat(auto-fit\|fill)`） | **defer** | 组合 Row/Column 或轻量 1D 轨（含固定 `repeat(N)`）；`auto-fit/fill` 解析为明确 Unsupported；宣称 Repo 面用诚实 `repeat(2,…)`（T-G24） |
| `grid-auto-*` 布局消费 | **defer** | 解析写入 `LayoutStyle`；measure/iced 仅消费 `grid-template-*` 1D 轨 |
| `align-content` 多行剩余空间分布 | **defer** | 解析入 `align_content`；wrap 线间仍用 `cross_gap` 顺序堆叠（自动交叉尺寸常无无剩余） |
| iced 流内 absolute 绘制 | **skip** | 产品浮层 Overlay |
| fixed：`transform` / `filter` / `perspective` 含块、iframe、复杂 `will-change` | **defer** | 本子集 CB **恒为视口**；勿假含块 |
| Nana Overlay ↔ CSS fixed 分工 | **合同** | L2 Dialog/Popover/Drawer/ContextMenu **剥离** companion `fixed`/`sticky`，走 Overlay；匿名 Vue/CSS `position:fixed` 走视口子集 |
| `direction:rtl` / `writing-mode` 竖排下的逻辑↔physical | **defer** | 默认 LTR 映射已验收（T-B11/B12/P14）；勿假翻轴 |
| `:hover` / 复杂 `:not` / `:has` / `:is` / `@media` / `!important` | **skip** | Style Model 子集；`:first-child`/`:last-child`/简单 `:not(.class\|[attr])` 已兑现 |
| 完整 calc AST / `repeat(auto-fit)` | **非目标** | 轻量加减同单位 + viewport；非完整 AST |
| 完整 cascade / 交互伪类 | **非目标** | Style Model 映射子集 |
| `font-weight: bolder`/`lighter` 相对继承 | **defer** | 仅数值/`normal`/`bold` 宣称；见 A-05 |
| 动态字体文件 / `@font-face` | **defer** | 捆绑面（`Noto Sans SC` 等） |
| iced 原生 letter-spacing | **defer** | per-glyph `row.spacing` 近似；ellipsis 不走 |

### 2026-08-10 通用子集加深（Lilia companion 取证）

对照 `lilia-github.css` cascade dump，本轮补齐（**非** Lilia class 特判）：

| 能力 | 落点 | 取证触发 |
|------|------|----------|
| `max-content`/`min-content`/`fit-content` 轨 | `parse_grid_track_list_result` → `GridTrack::Auto` | `.repo-status-row` 等 `… max-content` |
| `fit-content(<length>)` | → `MinMax{max_px}`（1D 近似） | MDN function form |
| `minmax(min, auto\|*-content)` | ≈ `minmax(min,1fr)` | `minmax(0,auto)` |
| `minmax(min, N%)` | `%`→px（需 CB） | `.home-pending-row` `minmax(104px,32%)` |
| `repeat(N, …)` 固定次数 | 展开轨列表 | `.sync-columns` `repeat(3,…)`；Repo `repeat(2,minmax(240px,1fr))`（T-G24） |
| `repeat(auto-fit\|fill)` | `GridTrackListUnsupported`（非静默） | **扩展 X2 诚实策略**：Repo/业务可依赖 `repeat(N)`；**禁止**宣称 auto-fit 引擎；含 auto-fit 的 CSS 须走 Unsupported / 改写为固定 N 或 Row/Column |
| `display:inline-grid` | `DisplaySpec::InlineGrid` | 1D 同 grid |
| `grid-auto-columns/rows/flow` | 解析保留；布局不消费 | 隐式轨 / auto-placement defer |
| `place-items` / `place-content` | align（+ justify 近似） | badge / avatar |
| `align-items: baseline` | ≈ Start | `.page-header` |
| `:first-child` / `:last-child` | MatchContext 兄弟位 | `.contribution-stack > :first-child` |
| `var(--token)` 无 fallback | 扁平 `--*` 表（含轻量 `calc(Npx * k)`） | `border-radius: var(--radius-*)` |
| custom-prop **继承** | document `:root` 基 + 祖先/自身匹配 `--*` | `.menu { --row-h }` → 子 `var(--row-h)` |
| `vh`/`vw`/`vmin`/`vmax` | `LengthSpec::Viewport` + 活跃 viewport | `50vh` / `92vw` |
| `em`/`rem` | `LengthSpec::Em`/`Rem` + `FontSizeContext`（默认 16px；可随根/`font-size` 继承） | `2em` / `1.5rem` / `calc(2em + 4px)` |
| **Typography 子集（2026-08-11 闭合）** | `LayoutStyle` → iced Text；测绿：`typography_layout_drives_text_view_without_panic` | 见下行 |
| `font-size` / `font-weight` / `font-family` | px·em·rem；数值/`normal`/`bold`；已捆绑面优先（`Noto Sans SC`） | Home `13px`/`18px`/`600`；`var(--font-sans)` |
| `line-height` / `letter-spacing` / `color` | `LineHeightSpec`；px/em tracking（**per-glyph row 近似**，非 iced 原生）；`resolve_paint_color`（含 `var(--text*)`） | `1.55` / `0.5px` / `#5a616e` |
| `font-weight: bolder`/`lighter` 相对继承 | **defer** | 解析有绝对近似；勿宣称相对父权重 |
| 动态字体 / `@font-face` | **defer** | 仅捆绑面 |
| iced 原生 letter-spacing | **defer** | per-glyph `row.spacing` 近似；ellipsis 不走 tracking 近似 |
| **Button CSS chrome（2026-08-11 闭合）** | `Button`/`IconButton` 内层消费；外层 `apply_widget_box_model(..., consume)` | 见下行 |
| `padding` / `gap`（icon+text） | 控件自身 padding；`layout.gap_or(6)`；外层跳过 pad 免双层 | Lilia `.overview-actions__btn` |
| `border` / `border-radius` / `background` / `color` | `ButtonPaintOverride`（Active）；Hover/Pressed 仍 kind | 错层 radius/bg、toolbar muted 色 |
| 显式 `width`/`height` + 有 CSS 时 font weight/size | 内层覆盖 ControlSize；自建 text content | sort 22px；primary `font-weight:700` |
| `ButtonKind` 业务 class 特判 | **非目标** | 仍 prop / `nana-btn--*` Semantics |
| `min()`/`max()`/`clamp()` | `Min2`/`Max2`/`Clamp3` | `min(520px,92vw)` / `clamp(176px,38vw,260px)` |
| `calc` 同单位 + viewport | `CalcViewportOffset` / px±px | `calc(100vh - 32px)` / 裸 `100vw - 32px` |
| 未解析 `var()` 网格轨 | 降级 `Auto`（保列数） | `var(--sidebar-width) minmax(0,1fr)` |
| grid 轴不被 `flex-row` 盖掉 | class hint 在已有 grid 轨时不改 direction | `grid-template-rows` + `flex-row` |
| 简单 `:not(.class\|[attr])` | CompoundSelector 否定匹配 | `.tab:not(.is-active)` |

### 硬闸状态

| 闸 | 阈值 | 状态 |
|----|------|------|
| Gallery `pixel_ssim_compare` baselines | ≥0.98 | **1.0** |
| QJS↔V8 evidence PNG | ≥0.98 | **1.0** |
| `nana-ui-vue` iced-view lib | — | 绿 |
| editor_store | — | 本轮未改 |

### 盘点备注

- [`capabilities.md`](capabilities.md) 是 **Host 权限 / transport**，不是 CSS 布局矩阵。
- [`missing-nana-foundations`](performance/2026-08-06-missing-nana-foundations.md) 剩余项转向 **Android 壳、Lucide 全量、产品浮层加深、Markdown 组合** 等，而非新开布局引擎缺口。

### 建议下一阶段（布局线外）

> **2026-08-10**：相对 **home/settings** 的布局 L1 + Android slot/KeyEvent/APK + 浮层加深等**原宣称面已闭合**（见 [`android-arm64.md`](android-arm64.md)）。  
> **同日扩展合同**（Repo / Overlay 非 fixed / scrollIntoView / clipboard / window 泵送 / Vue host 深度）见 [`performance/2026-08-10-lilia-fidelity-gap.md`](performance/2026-08-10-lilia-fidelity-gap.md)「宣称面扩展合同」；**阶段 Todo 权威表**见 [`compatibility-roadmap.md`](compatibility-roadmap.md)（2026-08-11：X4–X6 已兑现；**A-05 typography / A-06 Button chrome 已闭合**；X1 等见路线图）。

1. **Android**：真机可选；完整 DesktopShell / 软 IME **defer**（勿假接）；本扩展**不**抬 Android 布局/IME 宣称
2. **Lucide**：常用别名已扩；全量路径 / 业务图标集按需加深  
3. **产品浮层**：走 Nana Overlay（扩展 **X3**）；L2 组件剥离 companion `fixed`/`sticky`。匿名 CSS `position:fixed` 走视口子集（T-P15–P17）
4. 新布局需求：先判 Overlay vs CSS fixed 分工；仅当 iced 子集可验收时再开 fixture；`sticky` / 完整 2D grid / transform 含块 **defer**

### 宣称闭合表（布局相关 · 含扩展）

| 切片 | 宣称 | 状态 | 验收 |
|------|------|------|------|
| Flex / 盒 / wrap / 定位子集（含 fixed 视口） / 1D grid `repeat(N)` | L1 measure 子集 | **原面闭合** | `cargo run -p nana-css-parity -- compare` |
| Typography（size/weight/family/lh/tracking/color） | iced Text 子集（A-05） | **闭合（2026-08-11）** | iced-view typography 测；`bolder`/`lighter`/动态字体/原生 tracking **仍 defer** |
| Button/IconButton CSS chrome | pad/border/radius/bg/color/gap 内层消费（A-06） | **闭合（2026-08-11）** | `button_layout_chrome_*`；见 [`2026-08-11-button-css-chrome.md`](performance/2026-08-11-button-css-chrome.md) |
| Repo 页证据 + 同子集轨 | 扩展 **X1/X2** | **闭合（X1/X2）** | Repo QJS↔V8 evidence + css-parity；见 fidelity-gap |
| `repeat(auto-fit\|fill)` / 2D auto-flow | — | **仍 defer** | Unsupported 非静默；勿升宣称 |
| `position: fixed` 视口子集 | 脱流 + 视口 CB + inset + z-index | **本轨闭合** | T-P15–P17；非 Overlay 节点绘制 |
| `position: sticky` | — | **仍 defer** | 优先 fixed |
| Overlay ↔ CSS fixed 分工 | L2=Overlay；匿名 fixed=视口子集 | **合同** | Overlay 测绿 + fixed 单测 |

## 与产品边界

```text
nana-ui / 消费应用     —— 禁止依赖 wry / WebView
nana-ui-vue            —— LayoutStyle + measure_layout（公开测量 API）
nana-css-parity        —— 仅测试；feature webview-ref 可选
```

- 产品绘制仍走 MessageBridge → `iced_app` → NanaUI widgets。
- `measure_layout` 供测试/诊断；不替代 iced 布局引擎。
- 新缺口：fixture `status: ignore` + `gap: P0-x|P1-x`，测试加 `#[ignore = "…"]`。

## 目录结构

```text
crates/nana-css-parity/
  fixtures/T-*.json
  src/lib.rs            # 加载 / 比较 / HTML 导出
  src/webview.rs        # feature = webview-ref
  src/bin/css_parity.rs
  tests/parity.rs
docs/css-layout-parity.md
```
