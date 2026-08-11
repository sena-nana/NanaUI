# Nana 兼容性路线图（Compatibility Roadmap）

> **权威文档**（2026-08-11 审计落地）。简体中文为主，关键术语保留中英对照。  
> 本文件是 **Style Model + iced 子集** 相对 Web/CSS/DOM/Vue Custom Renderer 的阶段性兼容合同；  
> **不是**「成为浏览器」的承诺表。

交叉引用：

| 文档 | 关系 |
|------|------|
| [`css-layout-parity.md`](css-layout-parity.md) | 布局 CSS 子集硬闸与 fixture 矩阵 |
| [`vue-nana-renderer-system.md`](vue-nana-renderer-system.md) | 三层兼容（L1/L2/L3）与 Vue host 合同 |
| [`capabilities.md`](capabilities.md) | Host 权限 / transport（非 CSS 矩阵） |
| [`performance/2026-08-10-lilia-fidelity-gap.md`](performance/2026-08-10-lilia-fidelity-gap.md) | Lilia 证据硬闸与宣称面扩展 **X1–X7** |
| [`architecture.md`](architecture.md) | 仓库边界：无 WebView / 无完整 CSSOM 进 `nana-ui` |
| [`performance/2026-08-11-compatibility-phases.md`](performance/2026-08-11-compatibility-phases.md) | 本审计的日期索引（指向本文） |
| [`performance/2026-08-11-button-css-chrome.md`](performance/2026-08-11-button-css-chrome.md) | Button/IconButton CSS chrome 消费（A-06） |

---

## 1. 范围声明（Scope）

### 1.1 Nana 是什么

```text
L1 CSS 子集 / L2 Vue props / L3 Rust API
        │
        ▼
  Nana Style Model（Tokens + Semantics + Layout）
        │
        ▼
  唯一绘制：nana-ui widgets → iced / WGPU
```

- **是**：把业务 CSS / DOM 调用 **映射** 到 `LayoutStyle`、语义控件与宿主 bridge 子集。
- **不是**：浏览器、完整 CSSOM、完整 HTML5、Blitz/stylo 引擎，或第二套 paint。
- 产品运行时 **禁止** WebView；`nana-css-parity` 的 `webview-ref` 仅作 golden 参照，不得链进 `nana-ui`。

### 1.2 Overlay ↔ `position:fixed` 分工（合同）

| 路径 | 用途 | 依据 |
|------|------|------|
| **Nana Overlay** | L2 Dialog / Popover / Drawer / ContextMenu；click-outside / `Node.contains` | 产品浮层；剥离 companion CSS `fixed`/`sticky` |
| **CSS `position:fixed` 视口子集** | 匿名 Vue/CSS 节点：脱流 + CB=viewport + inset + z-index | css-parity **T-P15–P17**；iced 根 stack 绘制 |
| **CSS `position:absolute`** | measure 脱流子集（T-P01–P14）；**iced 流内跳过** | 产品浮层仍走 Overlay |
| **`position:sticky`** | **长期 defer**（见 §5 / Phase B） | 勿假实现抬 SSIM |

禁止：用 class 白名单发明定位；把 L2 浮层改回依赖 CSS fixed；把空 stub 写成「已支持」。

### 1.3 宣称规则

1. 仅当对应 **验收命令绿** 且边界写进本文 / 关联文档后，方可标 **done**。
2. **部分（partial）**：有实现与测例，但语义窄于官方规范或有明确缺口。
3. **defer / 非目标**：永不宣称或长期不做；见 §6。
4. Android 壳 / IME / clipboard 真后端：**本路线图不抬宣称**（见 `android-arm64.md`）。

---

## 2. 审计依据（Normative references）

对照以下官方文档确认「应支持」与「诚实子集」边界（链接为 MDN 入口；实现以 WHATWG / CSSWG 语义为准）：

### 2.1 CSS / 布局

| 主题 | 参照 |
|------|------|
| CSS Flexible Box | [MDN: Basic concepts of flexbox](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_flexible_box_layout/Basic_concepts_of_flexbox)；CSS Flexible Box Layout Module Level 1 |
| CSS Grid（1D 轨子集） | [MDN: Grid layout](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_grid_layout)；CSS Grid Layout Module Level 1（`repeat()` / `minmax()` / `fit-content()`） |
| Box model / `box-sizing` | [MDN: box-sizing](https://developer.mozilla.org/en-US/docs/Web/CSS/box-sizing)；CSS Box Model |
| Positioning | [MDN: position](https://developer.mozilla.org/en-US/docs/Web/CSS/position)；CSS Positioned Layout Module Level 3（`static`/`relative`/`absolute`/`fixed`/`sticky`） |
| Cascade / specificity | [MDN: Cascade and inheritance](https://developer.mozilla.org/en-US/docs/Web/CSS/Cascade_and_inheritance)；CSS Cascade Level 4/5 |
| Selectors | [MDN: CSS selectors](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_selectors)；Selectors Level 3/4（`+`/`~`、`:nth-child`、`:is`/`:where`/简单 `:not`） |
| Values & Units | [MDN: CSS values and units](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_Values_and_Units)；`calc()` / `min()`/`max()`/`clamp()` / `vh`/`vw` / `em`/`rem` / `var()` |
| Fonts / Typography | [MDN: font-size](https://developer.mozilla.org/en-US/docs/Web/CSS/font-size) / [font-weight](https://developer.mozilla.org/en-US/docs/Web/CSS/font-weight) / [font-family](https://developer.mozilla.org/en-US/docs/Web/CSS/font-family) / [line-height](https://developer.mozilla.org/en-US/docs/Web/CSS/line-height) / [letter-spacing](https://developer.mozilla.org/en-US/docs/Web/CSS/letter-spacing) / [color](https://developer.mozilla.org/en-US/docs/Web/CSS/color)（子集 → iced Text；见 A-05） |
| Logical properties | [MDN: CSS logical properties](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_logical_properties_and_values)（本子集仅默认 LTR↔physical） |

### 2.2 DOM / JS Web API

| 主题 | 参照 |
|------|------|
| DOM tree / Node | [WHATWG DOM](https://dom.spec.whatwg.org/)；[MDN: Node](https://developer.mozilla.org/en-US/docs/Web/API/Node)（`contains`、父子关系） |
| Element metrics | [MDN: Element.getBoundingClientRect](https://developer.mozilla.org/en-US/docs/Web/API/Element/getBoundingClientRect)；CSSOM View |
| `scrollIntoView` | [MDN: Element.scrollIntoView](https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollIntoView)；CSSOM View |
| Selectors API | [MDN: Document.querySelector](https://developer.mozilla.org/en-US/docs/Web/API/Document/querySelector) |
| Events | [MDN: EventTarget](https://developer.mozilla.org/en-US/docs/Web/API/EventTarget)；DOM events（capture / bubble / `once`） |
| Clipboard | [MDN: Clipboard API](https://developer.mozilla.org/en-US/docs/Web/API/Clipboard_API)（`readText`/`writeText`） |
| Window / Viewport | [MDN: Window](https://developer.mozilla.org/en-US/docs/Web/API/Window)（`innerWidth`/`innerHeight`、`focus`/`blur`、`resize`）；[Page Visibility](https://developer.mozilla.org/en-US/docs/Web/API/Page_Visibility_API) |
| Observers | [MDN: ResizeObserver](https://developer.mozilla.org/en-US/docs/Web/API/ResizeObserver)；[MutationObserver](https://developer.mozilla.org/en-US/docs/Web/API/MutationObserver)；[IntersectionObserver](https://developer.mozilla.org/en-US/docs/Web/API/IntersectionObserver) |

### 2.3 Vue Custom Renderer

| 主题 | 参照 |
|------|------|
| Custom Renderer API | [Vue: Custom Renderer](https://vuejs.org/api/custom-renderer.html)；`@vue/runtime-core` `createRenderer` nodeOps |
| Teleport | [Vue: Teleport](https://vuejs.org/guide/built-ins/Teleport.html)（Nana：**Overlay / 稳定 mount-root**，非伪 `document.body` portal） |

---

## 3. 现状矩阵（Current status）

状态：`done` = 已验收；`partial` = 子集可用；`open` = 合同已立未闭合；`defer` = 长期不做/延后；`skip` = 产品路径不走。

### 3.1 布局 CSS 子集（Phase A）

| 能力族 | 状态 | 证据 |
|--------|------|------|
| Flex 主/交叉轴（gap/%、justify、align(+self)、grow/shrink(+min)、basis、reverse、order、wrap） | **done** | T-F01–F22、T-W01–W09、T-L04 |
| 尺寸 / 轻量 calc / viewport / em·rem / min·max·clamp | **done** | T-S01–S16 |
| 盒模型（pad/margin 2–4 值与 %、content-box、border-width∈border-box、逻辑盒 LTR） | **done** | T-B01–B12 |
| 轻量 1D grid（template、fr/minmax/%/fit-content、`repeat(N)`、inline-grid） | **done** | T-G01–G24 |
| `repeat(auto-fit\|fill)` / 2D auto-flow / areas / 跨轨 | **defer** | Unsupported 非静默；勿宣称 |
| `display:none` / `visibility:hidden`（均跳过，非 CSS 占位） | **done** | T-V01–V02 |
| Cascade 子集（specificity + source order；author `!important` 两趟；`:nth-child`/`:nth-of-type`；`+`/`~`；简单 `:not`/`:is`/`:where`） | **partial** | `css_cascade.rs`；`:hover`/`@media`/`@layer`/`:has` **defer** |
| Typography（`font-size`/`weight`/`family`、`line-height`、`letter-spacing`、`color`） | **done（子集）** | A-05；`LayoutStyle` → iced Text；`em`/`rem` 随 `FontSizeContext`；见 `css-layout-parity.md` |
| Button/IconButton CSS chrome（padding / border / radius / bg / color / gap） | **done** | A-06；`ButtonPaintOverride`；外层跳过已消费 pad/paint；见 `2026-08-11-button-css-chrome.md` |
| `font-weight: bolder`/`lighter` 相对继承 | **defer** | 解析侧有绝对近似；**勿**宣称相对父权重量级 |
| 动态字体文件 / `@font-face` 加载 | **defer** | 仅已捆绑面（如 `Noto Sans SC`） |
| iced 原生 `letter-spacing` | **defer** | 现用 per-glyph `row.spacing` 近似；ellipsis 路径不走 tracking |
| Shell class hints（Fill 链、settings-row） | **done** | T-L01–L03 |

硬闸：`cargo run -p nana-css-parity -- compare`（≈106 pass fixtures，2026-08-11）。

### 3.2 定位（Phase B）

| 能力 | 状态 | 证据 |
|------|------|------|
| `relative` + inset | **done** | T-P01 |
| `absolute` measure 脱流 + inset（含 %、双侧拉伸、嵌套） | **done** | T-P02–P14 |
| iced 流内 absolute 绘制 | **skip** | → Overlay |
| `fixed` 视口子集（脱流 + viewport CB + inset + z-index） | **done** | T-P15–P17 |
| fixed：`transform`/`filter`/`perspective` 含块、iframe | **defer** | CB 恒为视口 |
| `sticky` | **defer** | `PositionSpec::Sticky` → unsupported；优先 fixed |
| Overlay ↔ fixed 分工 | **done（合同）** | fidelity-gap X3；Overlay 测 + fixed 单测 |

### 3.3 DOM / JS API（Phase C）— 对照 X4–X6

| 能力 | 状态 | 证据 |
|------|------|------|
| `querySelector` / `querySelectorAll`（子集） | **partial** | `renderer` hostOps + shim |
| `getBoundingClientRect` / `layoutBox`（优先 iced writeback） | **partial** | `tree` / shim layout metrics |
| `Node.contains` | **done** | hostOp + `nanavue-runtime` dom-contract |
| `scrollIntoView`（block/inline align 子集，滚 overflow 祖先） | **done** | `scroll.rs` + `renderer` 测；shim→host（**非**空 stub） |
| `scrollTop` / `scrollLeft` 投影 | **partial** | shim layout metrics |
| `navigator.clipboard` read/write（桌面） | **done** | X5；`OsClipboard`/`arboard`；Android **defer** |
| `window` focus/blur/resize + visibilitychange 泵送 | **done** | `__nanaPumpLifecycle`；`VueHost::pump_lifecycle`；QJS 测；windowed 宿主接线 |
| `ResizeObserver`（读 layoutBox，`__nanaNotifyLayout`） | **partial** | shim；非完整规范矩阵 |
| `MutationObserver` / `IntersectionObserver` | **defer（stub）** | 空 observe；勿宣称真观察 |
| EventTarget capture/bubble/`once`（window/document/target 扇出） | **done（子集）** | shim `__nanaInvokePhase` + `events.test.mjs`；**非**祖先链冒泡 |
| 完整 DOM / HTML5 / CSSOM 写回 | **非目标** | — |

### 3.4 Vue host 合同（Phase D）— 对照 X3/X7

| 能力 | 状态 | 证据 |
|------|------|------|
| `createRenderer` nodeOps（insert/remove/patchProp/…） | **done** | `nanavue-runtime` host-ops 测 |
| `nodeCache`（nid↔proxy 同一性） | **done** | `createNanaRenderer.js` + dom-contract |
| 事件桥矩阵（多 listener / capture / fan-out / pointer / Escape） | **done（子集）** | D-04；`__nanaFireEvent` + `events.test.mjs`；详见 [`vue-nana-renderer-system.md`](vue-nana-renderer-system.md) §5.1 |
| Teleport：`to="body"` → 稳定 mount-root wrapNode；Overlay 并存不泄漏 | **done** | transition/teleport-contract；tree/renderer/iced-view 插入移除测；**非** DOM body portal；Transition 仍 0s |
| L2 浮层 → Nana Overlay | **done（业务证据）** | X3 / E-03；`_overlay-evidence/`；剥离 companion fixed |
| 完整 DOM Teleport / ARIA portal | **非目标** | — |

#### 3.4.1 事件桥合同摘要（D-04）

对照 Vue `onXxx` / DOM EventTarget；权威细表见 `vue-nana-renderer-system.md` §5.1。

| 族 | 状态 | 说明 |
|----|------|------|
| 宿主泵送：`pointerdown` / `click`·`press` / `focus` / `keydown` / `input` / `wheel` / `change` / `select` | **done** | iced → `NanaVueApp` → `__nanaFireEvent`；`press`↔`click` 别名 |
| 浮层 outside：`pointerdown` 必扇出；Escape→`keydown`（无焦点时 `mount_root`） | **done** | Lilia dismiss / ContextMenu |
| 多 listener + `capture`/`once` + Vue `onClickCapture`/`Once` | **done** | window→document→target→document→window |
| `stopPropagation` / `stopImmediatePropagation` | **done** | 扇出路径内有效 |
| `passive` 选项 | **partial** | 可解析存储；**不**强制禁止 `preventDefault` |
| 祖先节点冒泡 / `composedPath` / 完整 Pointer·Mouse 构造 | **defer** | 扇出仅 window/document/target |
| CSS `:focus` / `:hover` 伪类 | **defer** | 选择器诚实跳过（§6） |
| window `focus`/`blur`/`resize`/`visibilitychange` | **done** | C-04 lifecycle；非控件桥 |

### 3.5 Lilia 证据页（Phase E）— 对照 X1/X2

| 项 | 状态 | 说明 |
|----|------|------|
| home/settings QJS↔V8 + l1-fidelity | **done** | SSIM 1.0；见 fidelity-gap |
| **X1** Repo 证据页升闸 | **done** | SSIM 1.0；`baselines/l1/repo-light.png`；reachability + 主区非空壳 |
| **X2** Repo 诚实 1D grid（T-G24） | **done** | `repeat(2,minmax(240px,1fr))`；壳用 flex column；`auto-fit` 仍 defer |
| **X3** Overlays 业务证据 | **done** | `--interact=overlays` PNG+JSON；见 `_overlay-evidence/` |
| Diff/Actions 全 workbench、像素级 Markdown | **defer** | 不进本阶段宣称 |

---

## 4. 阶段 Todo（核心）

约定：每条 Todo 含 **ID / 目标 / 官方依据 / 验收 / 依赖 / 状态**。  
勾选语义：`- [x]` = done；`- [ ]` = open；`defer` 单独标注，不假装可勾。

### Phase A — 布局 CSS 子集硬闸

> 目标：保持 css-parity 绿；新布局需求先判 Overlay vs CSS，再开 fixture。

- [x] **A-01** · Flex / wrap / 盒 / 尺寸 L1 硬闸  
  - **目标**：未 defer 的 flex/wrap/box/sizing 用例全部 pass  
  - **依据**：CSS Flexbox / Box Model（§2.1）  
  - **验收**：`cargo run -p nana-css-parity -- compare`（含 T-F* / T-W* / T-B* / T-S*）  
  - **依赖**：无  
  - **状态**：done（见 `css-layout-parity.md`）

- [x] **A-02** · 轻量 1D grid + 诚实 `repeat(N)`  
  - **目标**：T-G01–G24；`repeat(auto-fit|fill)` → Unsupported（非静默）  
  - **依据**：CSS Grid `repeat()` / `minmax()` / `fit-content()`  
  - **验收**：同上 compare；业务含 auto-fit 不得假绿  
  - **依赖**：A-01  
  - **状态**：done（对齐 X2 子集）

- [x] **A-03** · Cascade / 选择器子集（非交互伪类）  
  - **目标**：type/class/id/attr、组合器、`:first-child`/`:last-child`/`:nth-*`、简单 `:not`/`:is`/`:where`、author `!important`  
  - **依据**：Selectors L4；CSS Cascade  
  - **验收**：`cargo test -p nana-ui-vue --lib css_cascade`（及 cascade 相关单测）  
  - **依赖**：A-01  
  - **状态**：done（子集）；`:hover`/`@media`/`@layer`/`:has` → §6

- [ ] **A-04** · 新布局缺口纪律  
  - **目标**：任何新缺口先 `fixture status: ignore` + `gap: Px-y`，禁止静默近似抬宣称  
  - **依据**：本仓库合同（非规范条款）  
  - **验收**：PR / 审计检查 fixture 与 `css-layout-parity.md` 同步  
  - **依赖**：A-01  
  - **状态**：open（流程；持续）

- [x] **A-05** · Typography CSS 子集 → iced Text  
  - **目标**：`font-size` / `font-weight`（数值与 `normal`/`bold`）/ `font-family`（已捆绑面优先）/ `line-height` / `letter-spacing`（px·em）/ `color`（含 `var(--text*)`）映射进 `LayoutStyle` 并驱动 Text 绘制；`em`/`rem` 随祖先/`font-size` 继承  
  - **依据**：§2.1 Fonts / Typography（MDN）  
  - **验收**：`cargo test -p nana-ui-vue --features iced-view --lib typography_layout_drives_text_view_without_panic --locked`；`css_map` typography 单测绿  
  - **依赖**：A-01（`FontSizeContext` / cascade 继承）  
  - **状态**：done（2026-08-11）；**仍 defer**：相对 `bolder`/`lighter`、动态字体文件、`@font-face`、iced 原生 letter-spacing（现 per-glyph row 近似）

- [x] **A-06** · Button/IconButton CSS chrome 消费  
  - **目标**：`padding` / `border`(+width/color) / `border-radius` / `background`(+color) / `color` / `gap`（及显式 `width`/`height`）由 `Button`/`IconButton` 内层消费，避免双层 padding 与圆角/背景画在错层；无 CSS 时仍走 `ControlSize`/`ButtonKind`；**不**发明业务 class 特判  
  - **依据**：§2.1 Box Model / border（MDN）；Lilia `.overview-actions__btn` 取证  
  - **验收**：`cargo test -p nana-ui-vue --features iced-view --lib button_layout_chrome_ --locked`；`button_without_css_padding_*`；`cargo test -p nana-ui --lib control_sizes --locked`  
  - **依赖**：A-01（`LayoutStyle` pad/surface）；A-05（有 CSS 时自建 text weight/size）  
  - **状态**：done（2026-08-11）；证据 [`performance/2026-08-11-button-css-chrome.md`](performance/2026-08-11-button-css-chrome.md)；Hover/Pressed 仍走 kind；Chip 同 consume 路径

### Phase B — 定位（fixed 子集；sticky defer）

- [x] **B-01** · `relative` / `absolute` measure 子集  
  - **目标**：T-P01–P14  
  - **依据**：CSS Positioned Layout（absolute CB = nearest positioned padding box）  
  - **验收**：`cargo run -p nana-css-parity -- compare`（T-P01–P14）  
  - **依赖**：A-01  
  - **状态**：done

- [x] **B-02** · `fixed` 视口子集  
  - **目标**：脱流 + CB=viewport + inset + z-index；iced 根层绘制  
  - **依据**：MDN `position:fixed`（视口含块的常见情形）；**不含** transform 含块例外  
  - **验收**：T-P15–P17；`nana-ui-vue` iced-view fixed 相关单测  
  - **依赖**：B-01  
  - **状态**：done

- [x] **B-03** · Overlay ↔ CSS fixed 分工落地  
  - **目标**：L2 浮层走 Overlay；匿名 fixed 走 B-02；companion fixed/sticky 剥离  
  - **依据**：产品合同 + Vue Teleport/浮层实践（非完整 CSS fixed 引擎）  
  - **验收**：`cargo test -p nana-ui-vue --features iced-view --lib`；业务 Dialog 不依赖 `position:fixed`  
  - **依赖**：B-02；X3  
  - **状态**：done（合同 + 测）；业务页回归随 Phase E

- [ ] **B-04** · `position:sticky` — **defer**  
  - **目标**：不实现真 sticky 引擎；解析标记 unsupported；作者改用 fixed 子集或 Overlay  
  - **依据**：MDN sticky（依赖 scrollport 约束）— **明确不做完整语义**  
  - **验收**：`PositionSpec::Sticky` 保持 `is_unsupported_positioning()`；无假绿 fixture  
  - **依赖**：—  
  - **状态**：defer

### Phase C — DOM / JS API

- [x] **C-01** · 布局度量：`getBoundingClientRect` / `layoutBox`  
  - **目标**：优先 iced writeback；与 measure 诊断盒一致路径可测  
  - **依据**：CSSOM View `getBoundingClientRect`  
  - **验收**：`cargo test -p nana-ui-vue --lib`（layoutBox / tree）；`nana-ui-web-api` shim 投影测  
  - **依赖**：Phase A measure  
  - **状态**：done（子集 / partial 精度以 iced 为准）

- [x] **C-02** · `scrollIntoView` 真定位（X4）  
  - **目标**：溢出祖先滚至目标可见；block/inline align 子集  
  - **依据**：MDN `Element.scrollIntoView`  
  - **验收**：`cargo test -p nana-ui-vue --lib scroll` / `scroll_into_view_host_op_*`；shim 含 `hostCall("scrollIntoView")` 且无空 stub  
  - **依赖**：C-01；scroll store  
  - **状态**：done

- [x] **C-03** · 桌面 clipboard（X5）  
  - **目标**：`navigator.clipboard.readText/writeText` → Rust `ClipboardHost`  
  - **依据**：Clipboard API  
  - **验收**：`cargo test -p nana-ui-platform --lib`；`cargo test -p nana-ui-web-api --lib`；见 `capabilities.md`  
  - **依赖**：platform clipboard  
  - **状态**：done（桌面）；Android clipboard **defer**

- [x] **C-04** · window lifecycle 泵送（X6）  
  - **目标**：focus/blur/resize/visibilitychange → shim EventTarget；resize 更新 `innerWidth`/`innerHeight`  
  - **依据**：MDN Window / Page Visibility  
  - **验收**：`cargo test -p nana-js-quickjs --lib vue_host_pumps_window_lifecycle_events`；windowed `HostedWindowEvent` → `pump_lifecycle`  
  - **依赖**：web-api shim  
  - **状态**：done

- [ ] **C-05** · Selectors API 加深（按 Lilia 取证）  
  - **目标**：补齐业务真实用到的 selector 形态；失败须诚实（勿静默空结果伪装全匹配）  
  - **依据**：Selectors API + Selectors L4  
  - **验收**：针对取证 selector 的 hostOp / 集成测；文档更新本矩阵  
  - **依赖**：C-01；css_cascade  
  - **状态**：open

- [ ] **C-06** · `ResizeObserver` 行为硬化  
  - **目标**：布局变更经 `__nanaNotifyLayout` 投递；尺寸来自真实 layoutBox（禁止硬编码假侧栏）  
  - **依据**：MDN ResizeObserver  
  - **验收**：`cargo test -p nana-ui-web-api --lib resize_observer_*`；桥接测：改盒后 callback 触发  
  - **依赖**：C-01；C-04（可选同帧泵送）  
  - **状态**：open（shim partial 已有；须行为测闭合）

- [ ] **C-07** · MutationObserver / IntersectionObserver — **defer**  
  - **目标**：保持空 stub 或显式 unsupported；业务改走 Vue 响应式 / Overlay  
  - **依据**：MDN（完整观察语义成本过高）  
  - **验收**：文档与 shim 注释标明 stub；禁止宣称「已支持 Observer」  
  - **依赖**：—  
  - **状态**：defer

### Phase D — Vue host 合同

- [x] **D-01** · Custom Renderer nodeOps + `nodeCache`（X7 缓存切片）  
  - **目标**：稳定 nid↔proxy；insert/remove 后 `===` 同一性  
  - **依据**：Vue Custom Renderer；WHATWG Node 同一性期望（子集）  
  - **验收**：`packages/nanavue-runtime`：`npm test`（host-ops / dom-contract）  
  - **依赖**：MessageBridge  
  - **状态**：done

- [x] **D-02** · `Node.contains` + 浮层 outside-click（X3/X7）  
  - **目标**：overlay/anchor `containsTarget` 模式可用  
  - **依据**：DOM `Node.contains`；Lilia `useAnchoredOverlay`  
  - **验收**：dom-contract `containsTarget`；Rust `tree::contains` 单测  
  - **依赖**：D-01  
  - **状态**：done

- [x] **D-03** · Teleport → Overlay / 稳定 mount-root（X7 闭合）  
  - **目标**：`Teleport to=body` 不挂假 DOM portal；L2 浮层插入/移除不泄漏；与 CSS fixed 并存  
  - **依据**：Vue Teleport（语义对齐 Overlay，非浏览器 body）  
  - **验收**：`packages/nanavue-runtime` teleport/transition-contract；`cargo test -p nana-ui-vue --lib teleport_`；iced-view `teleport_mount_root_overlay_coexists_with_css_fixed`；Transition 仍诚实 0s  
  - **依赖**：D-02；B-03  
  - **状态**：done（2026-08-11）

- [x] **D-04** · 事件桥矩阵文档化  
  - **目标**：列出已泵送事件（click/input/keydown/…）与明确不做的（完整 DOM 冒泡全模型、CSS `:focus`）  
  - **依据**：DOM Events；Vue `onXxx`  
  - **验收**：本节 §3.4.1 + `vue-nana-renderer-system.md` §5.1；`node --test tests/events.test.mjs`；`cargo test -p nana-ui-web-api --lib shim_event_target`  
  - **依赖**：D-01；C-04  
  - **状态**：done（2026-08-11；子集；祖先链冒泡 / `:focus` 伪类 **defer**）

### Phase E — Lilia 证据页扩展

- [x] **E-01** · Repo 证据页升引擎硬闸（X1）  
  - **目标**：`--page repo` QJS↔V8 iced evidence SSIM ≥0.98；reachability + 主区非空壳 hard-fail  
  - **依据**：产品保真合同（非 Web 规范）；布局仍受 Phase A/B 约束  
  - **验收**：见 fidelity-gap「Repo」命令块；升 `baselines/l1/repo-light.png` 后 `pixel_ssim_compare` → **1.0**（2026-08-11）  
  - **依赖**：A-02、B-02；外部 LiliaGithub IIFE  
  - **状态**：done

- [x] **E-02** · Repo 作者面 CSS 诚实化（随 X1）  
  - **目标**：业务 CSS 使用 `repeat(N,…)` 等已支持轨；含 auto-fit 则改写或走 Unsupported；页壳用 flex column（勿依赖无轨 grid 假横排）  
  - **依据**：CSS Grid / Flexbox（诚实子集）  
  - **验收**：Repo 页 css-parity / 引擎证据绿；`nana-repo__grid|files` → `repeat(2,minmax(240px,1fr))`；无 class 特判假展开  
  - **依赖**：E-01；A-02  
  - **状态**：done（2026-08-11）

- [x] **E-03** · Overlays 证据交互（随 X3）  
  - **目标**：Dialog/Drawer/ContextMenu 开合 + outside-click 在证据路径可演示；不依赖 CSS fixed  
  - **依据**：B-03 / X3  
  - **验收**：`cargo test -p nana-ui-vue --features iced-view --lib --locked overlay`；  
    `nana-tauri-demo --interact=overlays --png=…` → PNG + `.overlay.json`（见 `docs/performance/_overlay-evidence/`）  
  - **依赖**：D-03；B-03  
  - **状态**：done（2026-08-11：合同测绿 + Lilia Nana 业务 PNG/log；Dropdown→Select；禁止 fixed 假实现）

- [ ] **E-04** · Diff / Actions / 全 workbench — **defer**  
  - **目标**：不进当前宣称面  
  - **依据**：—  
  - **验收**：—  
  - **依赖**：—  
  - **状态**：defer

### Phase F — 后续候选项（按取证增删）

> 仅在 Lilia/业务 **真实取证** 后升格为正式 Todo；默认不排期。

- [ ] **F-01** · `align-content` 多行剩余空间分布（若 wrap 出现可测剩余）  
  - **状态**：open/候选；现解析保留、线间多用 gap 堆叠  
- [ ] **F-02** · `direction:rtl` / `writing-mode` 逻辑映射  
  - **状态**：defer（见 §6）  
- [ ] **F-03** · 完整 2D grid auto-placement  
  - **状态**：defer  
- [ ] **F-04** · Android clipboard / 软 IME  
  - **状态**：defer（本路线图不实现；见 `android-arm64.md`）

---

## 5. 阶段目标一览（摘要表）

| Phase | 主题 | 核心 Todo | 整体状态 |
|-------|------|-----------|----------|
| **A** | 布局 CSS 硬闸 | A-01–A-03 / A-05–A-06 done；A-04 纪律 open | **硬闸闭合**（含 typography + Button chrome）；纪律持续 |
| **B** | 定位 | B-01–B-03 done；B-04 sticky **defer** | **fixed 子集闭合** |
| **C** | DOM/JS API | C-01–C-04 done；C-05/C-06 open；C-07 **defer** | **X4/X5/X6 已兑现**；Observer 真语义 defer |
| **D** | Vue host | D-01–D-04 done | **cache/contains/Teleport/事件矩阵闭合**（祖先冒泡 / `:focus` defer） |
| **E** | Lilia 证据 | E-01–E-03 **done**；E-04 **defer** | **home/settings/repo + overlays 闭合** |
| **F** | 候选项 | 按取证升格 | 默认不宣称 |

与 **X1–X7** 映射：

| 扩展 ID | 映射 Todo | 审计结论（2026-08-11） |
|---------|-----------|------------------------|
| X1 | E-01 | **done**（repo-light SSIM 1.0；2026-08-11） |
| X2 | A-02（+ E-02） | **done**（诚实 `repeat(N)` + flex 页壳） |
| X3 | B-03 / D-02 / E-03 | 合同与 contains **done**；业务证据 **done**（E-03） |
| X4 | C-02 | **done**（代码+测已超前旧 fidelity 文案） |
| X5 | C-03 | **done**（桌面） |
| X6 | C-04 | **done**（代码+测+windowed 接线） |
| X7 | D-01–D-04 | 缓存/contains/Teleport/事件矩阵 **done**（扇出子集；祖先冒泡 / `:focus` defer；Transition 仍 0s） |

---

## 6. 非目标（Non-goals）

下列项 **永不宣称**，或仅在未来另立合同并附独立验收前保持 **defer**：

| 项 | 说明 |
|----|------|
| 完整 CSSOM / 完整 cascade 引擎 | Style Model 映射子集；不进 `nana-ui` 公共核心 |
| 完整 HTML5 / 浏览器用 DOM | 仅 hostOps + shim 兼容面 |
| WebView / Blitz / 第二套 Device·Queue | 见 `AGENTS.md` / architecture |
| 真 `position:sticky` 引擎 | B-04 defer；优先 fixed 视口子集或 Overlay |
| fixed 含块例外（transform/filter/perspective/iframe） | CB 恒视口 |
| 完整 2D grid / `repeat(auto-fit\|fill)` 布局消费 | Unsupported 或改写 `repeat(N)` |
| iced 流内 absolute 绘制 | skip → Overlay |
| `:hover` / `:focus` 伪类 / `@media` / `@keyframes` / cascade `@layer` / `:has` | 选择器诚实跳过 |
| 完整 `calc()` AST | 仅轻量同单位 / viewport / em·rem |
| `direction:rtl` / 竖排 writing-mode 逻辑翻轴 | 勿假翻轴 |
| 真 MutationObserver / IntersectionObserver | stub；改 Vue/Overlay |
| 完整 VisualViewport / 平滑 scrollIntoView 全 options 矩阵 | C-02 仅为子集 |
| 伪 `document.body` Teleport portal | 走 Overlay / 稳定 mount-root |
| 完整 DOM 祖先链冒泡 / `composedPath` / 完整 Pointer·Mouse Event 构造 | D-04 扇出仅为 window↔document↔target |
| CSS `:focus` / `:hover` 驱动的交互样式 | 与选择器 defer 一致；焦点仅 `focus` 事件泵送 |
| `addEventListener({ passive })` 禁止 `preventDefault` | 选项可存；不强制规范语义 |
| Android clipboard 真后端 / 软 IME / DesktopShell / V8-on-Android | 另见 `android-arm64.md`；本路线图不抬 |
| `font-weight: bolder`/`lighter` 相对父权重量级 | A-05 仅数值/`normal`/`bold`；相对关键字勿宣称 |
| 动态字体文件 / `@font-face` 运行时加载 | 仅已捆绑面；未知名回退 `Noto Sans SC` |
| iced 原生 `Text` letter-spacing API | A-05 用 per-glyph `row.spacing` 近似；非 iced 原生 tracking |
| 空 stub 冒充成功（历史反模式） | 禁止空 `scrollIntoView` / 空 `clipboard.writeText` / 未泵送 resize |

---

## 7. 推荐验证命令（按层选用）

```bash
# Phase A / B — 布局硬闸
cargo run -p nana-css-parity -- compare

# Phase A — Button CSS chrome（A-06）
cargo test -p nana-ui-vue --features iced-view --lib button_layout_chrome_ --locked
cargo test -p nana-ui --lib control_sizes --locked

# Phase B/C/D — Vue / iced 桥
cargo test -p nana-ui-vue --features iced-view --lib --locked
cargo test -p nana-ui-vue --lib --locked

# Phase C — Web API / clipboard / lifecycle
cargo test -p nana-ui-web-api --lib --locked
cargo test -p nana-ui-platform --lib --locked
cargo test -p nana-js-quickjs --lib vue_host_pumps_window_lifecycle_events --locked

# Phase D — nanavue-runtime（含 D-04 事件矩阵）
cd packages/nanavue-runtime && node --test tests/events.test.mjs tests/host-ops.test.mjs tests/teleport-contract.test.mjs
cargo test -p nana-ui-web-api --lib shim_event_target --locked

# Phase E — 像素 / 引擎（需本机 Lilia 工程与证据流程）
# 见 fidelity-gap「扩展面验收命令清单」
# Overlays（E-03 / X3）：
#   cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
#     --project ~/work/LiliaGithub --interact=overlays --complete-setup \
#     --png=docs/performance/_overlay-evidence/lilia-home-overlays-quickjs.png
```

验证选型遵循 `$nanaui-validation`：编译通过不能替代证据页 / 目标平台证据。

---

## 8. 维护约定

1. **改宣称先改本文 + 关联矩阵**，再改代码；闭合时把 Todo 勾上并写验收命令结果日期。  
2. fidelity-gap 的 X* 表与本文冲突时：以 **代码 + 本文审计日期** 为准，并回修 fidelity-gap / vue-nana §5.1。  
3. 新增能力必须能指出 §2 的官方依据与「子集边界」一句话。  
4. 不在本文件堆实施细节；细节落在 `css-layout-parity.md` / 模块 rustdoc / 测例名。
