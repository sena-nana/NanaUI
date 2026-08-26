# 布局（Vue CSS 子集）

Rust 第一路径用控件自己的布局，不写 CSS。这篇只描述 Vue 兼容路径能用的那一部分网页 CSS，用来排界面骨架。

它不是浏览器里的 CSS 引擎：没有完整选择器世界，对不上就改布局，不靠容差假装一致。对话框、抽屉、菜单用 [控件](components.md) 里的浮层。

## 能用的

**Flex。** `flex-direction`、`flex-wrap`、`gap`、`align-items` / `align-self`、`justify-content`、多行换行时的 `align-content`（含 `stretch` / `normal`：剩余交叉空间均分给各行）、`order`、`flex-grow` / `flex-shrink` / `flex-basis`。侧栏加主区用这一套就够。

未写 `flex-shrink` 时按 **0** 处理，不是网页 CSS 的 initial **1**。溢出的定宽行（列表、工具条）会保留盒子，不会被悄悄压扁。需要网页那种收缩时，显式写 `flex-shrink` 或 `flex: initial`。`flex: none` / `auto` / 数字简写仍按 CSS 含义写 shrink。

**尺寸。** 像素、百分比、`em` / `rem`、`vw` / `vh`、`min` / `max` / `clamp()`，轻量 `calc`（`%±px`、`vh±px`、`em±px`），以及 `min-content` / `max-content` / `fit-content`。不折行的 flex 行上，`min-content` / `max-content` 都是子项之和；折行 / 块 / 列轴上 `min-content` 取最宽子项。`fit-content` 以最宽子项为下限、可用宽为上限。

**盒子。** padding、margin（含 `margin: 0 auto` 在块 / 列格式化上下文里水平居中），含 LTR `horizontal-tb` 下的 CSS 逻辑属性（`padding-block` / `margin-inline` 等）。默认 border-box；也认 content-box。

**网格。** `grid-template-columns` / `rows`、`fr`、`minmax`、固定次数的 `repeat(N, …)`，以及布局时展开的 `repeat(auto-fit | auto-fill)`（可与前后固定轨混写，例如 `80px repeat(auto-fit, 1fr)`；auto-fit 按容器能放下的轨数展开，空轨会收起）。`grid-column` / `grid-row` / `span`、`grid-auto-flow` 自动放置、`grid-auto-columns` / `rows` 隐式轨、网格项上的 `justify-items` / `justify-self`。轨解析之后，项上的 `width` / `height` 百分比与 `Fill` 相对**最终单元格**兑现（量测阶段仍把 `100%` / `Fill` 当成不定尺寸，以免撑破 auto 轨）。`grid-template-areas` + `grid-area: header` 命名区域，以及轨上的 `[name]` 命名线；同名线可用 `foo 2` / `2 foo` 取第 N 根，`foo / foo` 的终点取起点之后的下一根同名线（不是 CSS 那种对调）。`[start] 80px repeat(auto-fit, …)` 的前缀（以及 repeat 内）线名会保留；`repeat(N, …)` / `auto-fit` / `auto-fill` 里的线名按展开次数复制，接缝处相邻名字合并，因此 `mid 2` 相对展开后的几何取值。定高网格里，空的 auto 行（量测≈0）会吃掉剩余高度；有内容的 auto 行（T-G26）保持内容高，不会被拉伸——这不是 CSS `align-content: stretch` 作用在轨道上。自动放置扫到 4096 格仍放不下时，项落在扫描区外，不是宿主错误。没有 subgrid。直接构造 `LayoutStyle { display: Grid }`（不经 css_map）时 `align-items` 默认 `start`（手写测试依赖此项；Default 无法按 display 区分「未写」与显式 `start`）；经 `display:grid` 解析会写成 `stretch`。

**行内子集。** 块容器里的 `display: inline` / `inline-block` 走一行内格式化上下文（IFC）：行内子项按行排列，块级兄弟会关掉当前行并独占一行。可用 `text-align`（start / center / end）。`white-space: pre` 在量测里保留换行和空格。`align-items: baseline` 用字号近似第一行基线（`padding + border + 0.8em`），不是完整 CSS 基线对齐。

**浮动子集。** `float: left | right` 把盒子从块流里拿出来，同侧多个浮动按几何并排，放不下就折到下一行（不是文字绕排）；流内 `clear` 和浮动自身的 `clear` 都用折行之后的占用底边，不是单盒预排高度。流内盒子**不会**缩窄去绕开浮动。flex / grid 项上的 float 按 CSS 被块化，忽略。

**定位。** `relative`、脱流的 `absolute`、相对窗口的 `fixed`、文档流内的 `sticky`（滚动投影之后才贴住，不写回 Runtime `LayoutBox`）。`fixed` 只适合普通节点贴在视口上；产品浮层仍走控件，不要自己用 `fixed` 搭对话框。

**`display: contents`。** 子节点提升到父级格式化上下文；该节点自己没有盒子。

**文字。** 字号、字重、字体、行高、字距、颜色。字距是近似。

**隐藏。** `display: none` 不占位、不参与点击。`visibility: hidden` **仍占位**（参与 flex/grid 测量），但不绘制、不命中。内部 `layout.hidden`（侧栏、菜单等）与 `display: none` 一样跳过布局。

**`transform`。** 只影响绘制，不算进布局。这是 CSS 的正确行为，不是缺口。

**绘制。** `background` / `background-color` / `background-image: linear-gradient(...) | radial-gradient(...)`、`background-size: cover | contain | stretch`、`mask-image` / `-webkit-mask-image`（线性或径向渐变 alpha；GPU 最多 8 个 mask 色标）、`clip-path: inset(...) | polygon(...)`（inset 的 `round` 写入 rounded-box SDF，非平移 transform 下仍保留半径；polygon 对自身与子项做点内多边形测试，子项经 dest-group 合成，不是包围盒 clip）、`filter: brightness() saturate() contrast()`（有子节点或同节点文本时用 dest 合成组，叶子 quad 走 shader）。`background-image: url(...)` 支持 `data:image/png|jpeg;base64,...`、`http://` / `https://`、`file://` 与相对路径；相对 URL 优先相对文档/宿主设置的 base（[`set_background_image_url_base`](../../crates/nana-ui/src/scene_paint/image_url.rs)），否则相对进程 cwd。同 URL 纹理按 URL 缓存并按 URL 分批 rebind。GPU 侧每 quad 最多 8 个 gradient 色标。CSS `url()` 帧仍走 4× MSAA（与 HostTexture / backdrop 的 interleaved 路径分开）。`backdrop-filter: blur()` 是逐节点 dest 采样模糊（旋转映射 + 祖先 inset/polygon clip），不是整窗 Mica/Acrylic。

**层叠。** 同属 author。样式表内部先 normal 再 `!important`（再比特异度和源序）。prop / inline **普通**声明覆盖样式表普通声明（inline 覆盖 prop）。样式表 `!important` 覆盖 prop / inline 普通声明。prop / inline 上的 `!important` 会去掉标志并作为 author-important 再写：覆盖样式表 `!important`，且 inline important 覆盖 prop important。`:hover` / `:focus` 等交互伪类仍跳过，不会当布局条件。

增量 `width` / `height` / `min-width` 等 layout prop（`patchProp(width, …)`，不是改 `style`）与 class / style 走同一份 `rebuild_layout_style`：先写完普通层和 `nana-*` class hints，再写样式表 important，然后是 prop / inline important。因此样式表 `width:80px !important` 不会被后来的普通 `width:200px` prop 盖掉；prop / inline 带 `!important` 的仍覆盖样式表 important。`nana-*` hints 压过普通声明，但压不过其后的 important 尾。

自定义属性会去掉 `!important`（与普通声明同一套 `split_important_flag`），所以 `--gap: 8px !important` 经 `var(--gap)` 得到 `8px`，而不是带着标志无法解析。

**JS 查询。** `getBoundingClientRect` / `layoutBox` 读绘制投影：`offsetWidth` 是边框盒，`clientWidth` 是 padding 盒，`scrollWidth` 是内容尺寸；`offsetLeft` / `offsetTop` 相对 `offsetParent`；`clientLeft` / `clientTop` 是边框。滚动不写回 Runtime 的 `LayoutBox`。`getComputedStyle` 仍是 Vue Transition 桩，不是 CSSOM，也不是 `LayoutStyle`。

## 不要指望的

从右到左的书写方向（`direction: rtl`、`writing-mode`；逻辑属性仍按 LTR 映射）、完整 `calc()` AST、样式表里的 `@media`、当布局条件的 `:hover` / `:focus`、伪元素、`:has`、`@layer`、完整浮动绕排（文字绕开 float）、完整 IFC（双向文字、精确 CSS 基线、行内拆箱）、subgrid、用 `position: fixed` 当 Dialog / Drawer（走 Nana 浮层）、把 `getComputedStyle` 当成已解析的 `LayoutStyle`。

主题色走控件和外观设置，不要用任意业务色去改框架 token。

未知声明会被忽略；不要假设「写了就能在某处生效」。

## 和 Rust 布局的关系

Vue 侧解析出的是同一份 `LayoutStyle`。真正算盒子的是 Runtime 的 `RuntimeLayoutEngine`，产品帧走 `RuntimeDocument::flush`。JavaScript 查询到的盒子是绘制阶段的投影，滚动不写回 Runtime 的布局权威。
