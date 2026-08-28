# 布局（Vue CSS 子集）

Rust 第一路径用控件自己的布局，不写 CSS。这篇只描述 Vue 兼容路径能用的那一部分网页 CSS，用来排界面骨架。

它不是浏览器里的 CSS 引擎：没有完整选择器世界，对不上就改布局，不靠容差假装一致。对话框、抽屉、菜单用 [控件](components.md) 里的浮层。

## 能用的

**Flex。** `flex-direction`、`flex-wrap`、`gap`、`align-items` / `align-self`、`justify-content`、多行换行时的 `align-content`（含 `stretch` / `normal`：剩余交叉空间均分给各行）、`order`、`flex-grow` / `flex-shrink` / `flex-basis`。侧栏加主区用这一套就够。

未写 `flex-shrink` 长手时按 **0** 处理，不是网页 CSS 的 initial **1**。溢出的定宽行（列表、工具条）会保留盒子，不会被悄悄压扁。`flex` **简写**省略 shrink 时仍按 CSS 写成 1（`flex: initial`、`flex: 1`、`flex: 1 100px`）。需要网页那种收缩时，显式写 `flex-shrink` 或用简写。`flex: none` / `auto` / 数字简写仍按 CSS 含义写 shrink。

**尺寸。** 像素、百分比、`em` / `rem`、`vw` / `vh`、`min` / `max` / `clamp()`，轻量 `calc`（`+` `-` `*` `/`、括号、嵌套 `calc`，`var()` 展开后再算；结果折进既有 `%±px` / `vh±px` / `em±px` / 纯 px 规格；嵌套上限 16 层 CSS 括号/函数，unary `+/-` 另计最多 16 个符号；无单位结果 / `0px+number` / 除零 / 非有限 f32 fail closed），同单位可折叠的嵌套 `min` / `max` / `clamp`，以及可并进既有 `Min2` / `Max2` / `Clamp3` 的混单位（如 `min(10px, max(1px, 50%))`、`min(1px, 2%, 3px)`，布局时相对包含块兑现；三路互不可比仍 fail closed），以及 `min-content` / `max-content` / `fit-content`。不折行的 flex 行上，`min-content` / `max-content` 都是子项之和；折行 / 块 / 列轴上 `min-content` 取最宽子项。`fit-content` 以最宽子项为下限、可用宽为上限。

**盒子。** padding、margin（含 `margin: 0 auto` 在块 / 列格式化上下文里水平居中），含 `horizontal-tb` 下的 CSS 逻辑属性（`padding-block` / `margin-inline` / `inset-inline` 等）。`direction: rtl` 把 inline 的 start/end 映到 right/left（`padding-inline-start` → 右边），含跨层级联（样式表写逻辑边、后一层或继承才出 `rtl` 也会 remap）；HTML `dir="rtl"|"ltr"` 作为 presentational hint 写入同一条 used `direction`（作者 CSS `direction` 覆盖它；`dir="auto"` 无 first-strong bidi，fail-closed，不假装 ltr）。block 轴仍是 top/bottom。`direction` **不**翻转 flex/grid 主轴 / 交叉轴起点或 item 序——这不是完整 rtl 映射。默认 border-box；也认 content-box。

**网格。** `grid-template-columns` / `rows`、`fr`、`minmax`、固定次数的 `repeat(N, …)`，以及布局时展开的 `repeat(auto-fit | auto-fill)`（可与前后固定轨混写，例如 `80px repeat(auto-fit, 1fr)`；auto-fit 按容器能放下的轨数展开，空轨会收起）。`grid-column` / `grid-row` / `span`、`grid-auto-flow` 自动放置、`grid-auto-columns` / `rows` 隐式轨、网格项上的 `justify-items` / `justify-self`。轨解析之后，项上的 `width` / `height` 百分比与 `Fill` 相对**最终单元格**兑现（量测阶段仍把 `100%` / `Fill` 当成不定尺寸，以免撑破 auto 轨）。`grid-template-areas` + `grid-area: header` 命名区域，以及轨上的 `[name]` 命名线；同名线可用 `foo 2` / `2 foo` 取第 N 根，`foo / foo` 的终点取起点之后的下一根同名线（不是 CSS 那种对调）。`[start] 80px repeat(auto-fit, …)` 的前缀（以及 repeat 内）线名会保留；`repeat(N, …)` / `auto-fit` / `auto-fill` 里的线名按展开次数复制，接缝处相邻名字合并，因此 `mid 2` 相对展开后的几何取值。定高网格里，空的 auto 行（量测≈0）会吃掉剩余高度；有内容的 auto 行（T-G26）保持内容高，不会被拉伸——这不是 CSS `align-content: stretch` 作用在轨道上。自动放置扫到 4096 格仍放不下时，项落在扫描区外，不是宿主错误。`subgrid` 记为 [`GridTrackListUnsupported::Subgrid`](../../crates/nana-ui-core/src/box_layout.rs)，不假装继承父轨。直接构造 `LayoutStyle { display: Grid }`（不经 css_map）时 `align-items` 默认 `start`（手写测试依赖此项；Default 无法按 display 区分「未写」与显式 `start`）；经 `display:grid` 解析会写成 `stretch`。

**行内子集。** 块容器里的 `display: inline` / `inline-block` 走一行内格式化上下文（IFC）：行内子项按行排列，块级兄弟会关掉当前行并独占一行。可用 `text-align`（`start` / `end` 随 `direction`，`left` / `right` 保持物理边，以及 `center`）。`white-space: pre` 在量测里保留换行和空格。`align-items: baseline` 用字号近似第一行基线（`padding + border + 0.8em`），不是完整 CSS 基线对齐。

**浮动子集。** `float: left | right` 把盒子从块流里拿出来，同侧多个浮动按几何并排，放不下就折到下一行。流内 `clear` 和浮动自身的 `clear` 都用折行之后的占用底边，不是单盒预排高度。IFC 行盒按当前行与**兄弟**浮动外边距盒相交的左右 inset 缩窄（shrink-to-avoid-float）；一行里放不下的原子行内项折到下一行，若缩短后仍放不下则把行盒下移到最近占用浮动的底边之下。流内**块级**边框盒不会缩窄去绕开浮动（与 CSS 非 BFC 块一致）。不是完整排除：祖先浮动不侵入子块 IFC，没有 `shape-outside`。flex / grid 项上的 float 按 CSS 被块化，忽略。

**定位。** `relative`、脱流的 `absolute`、相对窗口的 `fixed`、文档流内的 `sticky`（滚动投影之后才贴住，不写回 Runtime `LayoutBox`）。`fixed` 只适合普通节点贴在视口上；产品浮层仍走控件，不要自己用 `fixed` 搭对话框。

**层叠上下文子集。** Scene 按 `(z_index, document_order)` 排序，并把隔离组当成一层：`opacity` 介于 0 和 1 的组、`isolation: isolate`、以及 `position` 非 static 且写了 `z-index`。高 z 子项不会画到组外的后出现兄弟之上。不是完整 CSS Appendix E（负 z 分层、float/inline 层、transform 单独成层等未全做）。命中仍走树结构，组内 z 只在兄弟间比。

**`display: contents`。** 子节点提升到父级格式化上下文；该节点自己没有盒子。

**文字。** 字号、字重、字体、行高、字距、颜色。`line-clamp` / `-webkit-line-clamp` 限制行数并打省略号。`text-decoration: underline | line-through` 由 Scene 在文本盒上描线。`font-feature-settings` 写入 cosmic-text OpenType features。`font-variation-settings: "wght" N` 并进已有 `font-weight`（cosmic-text 0.19 `FontSystem` / `Attrs` 只公开这条可变轴）。其余轴（含阿里妈妈 `BEVL`、`wdth`）只让**该声明** fail-closed（`unsupported_font_variation`），不把标签改写成 `wght`，也不塞进 `FontFeatures`。字距是近似。

**隐藏。** `display: none` 不占位、不参与点击。`visibility: hidden` **仍占位**（参与 flex/grid 测量），但不绘制、不命中。内部 `layout.hidden`（侧栏、菜单等）与 `display: none` 一样跳过布局。

**`transform`。** 只影响绘制，不算进布局。这是 CSS 的正确行为，不是缺口。2D 子集是仿射：`matrix()` / `rotate()` / `rotateZ()` / `translate()` / `scale()` / `skew()`。`translate3d` / `translateZ` 的 z 在没有透视时不改变 z=0 投影。`perspective()` + `rotateY` / `rotateX` / `rotate3d` / 带透视残差的 `matrix3d` 走同一条 Scene 平面单应（4 顶点透视除法）：**Quad、Text、Icon、HostTexture 共用这份 `(g,h)`**，同一节点不会出现梯形底 + 仿射字。Spinner / stroke mesh 与非 HostTexture 的 Custom 在透视下画 **identity**（不做 `scaleX(cos)` 假 3D）。父级 `perspective` 属性与 `transform-style: preserve-3d` 仍 fail-closed。`transform-origin` 是独立的 2D 字段（默认盒中心），对 2×3 与 4×4 都生效；`transition` / `@keyframes` 会插值 `%`/`px`（与 `transform` 同属绘制快照，不算布局）。`transform-box`：`border-box` / `view-box` 相对边框盒（HTML 下 `view-box` 即边框盒，没有 SVG viewport）；`content-box` / `fill-box` 相对内容盒（边框盒减去 border + padding；padding `%` 无包含块时按 0，与其它绘制期 padding 解析一致）。`stroke-box` 不解析。

**绘制。** `background` / `background-color` / `background-image: linear-gradient(...) | radial-gradient(...)`、`background-size` 初值 CSS `auto`（另有 `cover` / `contain` / `stretch` / 长度）、`background-repeat` 初值 `repeat`（`space` / `round` 近似 repeat）、`mask-image` / `-webkit-mask-image`（线性或径向渐变 alpha；GPU 最多 8 个 mask 色标）、`clip-path: inset(...) | polygon(...)`（inset 的 `round` 写入 rounded-box SDF，非平移 transform 下仍保留半径；polygon 对自身与子项做点内多边形测试，子项经 dest-group 合成，不是包围盒 clip）、`filter: brightness() saturate() contrast() hue-rotate() blur() drop-shadow()`（`blur` / `drop-shadow` 是元素自身滤镜，blur cap 16px，与 `backdrop-filter` 分开；`drop-shadow` 在 dest 合成组采样已绘 alpha 轮廓再 offset + 同核 blur，不是 `box-shadow` 盒几何；有子节点/文本/自定义绘制或 `blur`/`drop-shadow` 时用 dest 合成组，叶子 hue/brightness 走 quad shader；未知函数仍整表 fail closed，多层 `drop-shadow` 与 spread 未实现）。`box-shadow` 支持 `inset` 与逗号多层（GPU cap 4）。`outline` 只画 solid 额外描边，不进布局。`mix-blend-mode` GPU 子集是 `normal` / `multiply` / `screen`（dest-group BlendState；其余关键字 fail closed）。`line-clamp` / `-webkit-line-clamp` 走已有文本 ellipsis（并设 overflow hidden）。`border-image` 最小子集：`url()` 或 `linear-gradient` + `slice`（可选 `fill`）走现有 quad URL 纹理 9-slice（`linear-gradient` 先栅成纹理再切；`border-image-width` 默认 `1`×slice，`repeat` 仅 stretch）；`radial-gradient` / outset / round|repeat|space / 非默认 width 仍 fail-closed。四边 `border-*-width` 与 `border-*-color` 参与布局和 GPU stroke。`border-style: dashed | dotted` 在同一条 rounded-box SDF 环上按边做周期遮罩（不另开管线）；`double` / groove / ridge / inset / outset 仍占 used width 但不描。`background-image: url(...)` 与 `<img src>` / 内联 `<svg>` 共用同一条 decode + URL 纹理缓存：`data:image/png|jpeg|svg+xml`、`http://` / `https://`、`file://` 与相对路径；`.svg` 走已有 `resvg`（最长边 2048，保持比例），不是第二套矢量引擎。相对 / `file:` URL 走与样式表相同的 [`canonicalize_within_jail`](../../crates/nana-ui-core/src/url_jail.rs)，非本机 file host 拒绝，不把 `Url::path()` 回退到任意文件系统；`http(s)` 正文上限 8MB（与 `@font-face` 相同）。相对路径优先相对文档/宿主设置的 base（[`set_background_image_url_base`](../../crates/nana-ui/src/scene_paint/image_url.rs)），否则相对进程 cwd。SVG `<image href>` 不读本地文件。同 URL 纹理按 URL 缓存并按 URL 分批 rebind。GPU 侧每 quad 最多 8 个 gradient 色标。CSS `url()` 帧仍走 4× MSAA（与 HostTexture / backdrop 的 interleaved 路径分开）。`backdrop-filter: blur()` 是逐节点 dest 采样模糊（旋转映射 + 祖先 inset/polygon clip），不是整窗 Mica/Acrylic。

**命中。** `pointer-events: auto | none`，按 CSS **继承**。父级 `none` 时，未写该属性的子孙 used 值也是 `none`（不可点）；只有显式 `auto` 的子节点重新成为目标。点在父盒上但没落在那个 `auto` 子上会穿透。未知关键字 fail closed（不改已有指定值）。`inherit` / `unset` 清掉指定值。平面 3D 命中与 Quad/Text/Icon/HostTexture 绘制共用同一份单应逆变换，不按仿射盒或梯形 AABB 空角命中。

**溢出。** `overflow` / `overflow-x` / `overflow-y` 的 `visible` / `hidden` / `clip` / `auto` / `scroll`。`hidden` / `clip` / `auto` / `scroll` 都会裁剪绘制与命中。L1 `overflow: auto|scroll` 的滚动权威仍是 Runtime `ScrollOffset`（JS `scrollTop` / `scrollLeft` / `scrollIntoView` 与滚轮走这条路径，滚动不写回 `LayoutBox`）。自定义滚动条铬是 L2 [`ScrollView`](components.md)，L1 不另做一套 thumb 绘制。

**选择器。** type / class / id / 属性选择器、组合符 ` ` / `>` / `+` / `~`、`:root` / `:first-child` / `:last-child` / `:nth-child()` / `:nth-of-type()`、简单 `:not()` / `:is()` / `:where()`。廉价主体 `:has(.class|#id|type)`（含逗号 OR）按一次 O(n·k) 后序 bitset 匹配（k 为去重后的简单参数，上限 64），不是每主体扫子树。`::before` / `::after` 走生成盒。`::placeholder` 把 `color` / `opacity` 画到 Runtime TextInput 占位文字上（只对 `input` / `textarea`；不是生成盒，也不在非输入上假装占位）。带组合符 / 写在祖先上的 `:has` 计入 skipped_selectors。

**层叠。** 同属 author。样式表内部先 normal 再 `!important`（再比特异度和源序）。prop / inline **普通**声明覆盖样式表普通声明（inline 覆盖 prop）。样式表 `!important` 覆盖 prop / inline 普通声明。prop / inline 上的 `!important` 会去掉标志并作为 author-important 再写：覆盖样式表 `!important`，且 inline important 覆盖 prop important。`:hover` / `:focus` 等交互伪类仍跳过，不会当布局条件。

**`@import`。** 解析 `url(...)` / 引号路径，相对路径相对导入方或样式表 jail（canonicalize 后必须落在 `stylesheet_base` 下）。`stylesheet_base` 由文档/SFC URL 或最近一次 `injectStylesheet` 的 href 写入；未设置时相对 `@import` 跳过，不扫进程 cwd。`http(s)` / `data:` / 越狱 / 超 1MB / 带 `layer` 或 `supports()` 的 prelude 一律 fail closed（记 skip，不加载）。写在普通规则之后的 `@import` 按 CSS 忽略；写在 `@media` 块内的 `@import` 同样忽略。`file://` 会 percent-decode，非本机 host 跳过。循环与深度上限（16）。导入规则并进同一份 cascade。已解析的导入按 canonical href 缓存，视口变化不重解析。

**`@media`。** 子集：`min/max/width`、`min/max/height`（px）、`orientation`、`prefers-color-scheme`，以及 `screen` / `all` / `print` 类型。条件匹配时规则进入 cascade；视口或主题变化只重新 flatten 已解析规则，不重扫 CSS 文本。JS `matchMedia` 经 host op `evaluateMediaQuery` 与 CSS flatten 共用同一套 Rust 求值（`screen`/`all` 为真、`print` 为假）；无 host 时 web-api shim 回退同一子集。

**`@font-face`。** 解析 `font-family` / `src` / `font-weight`（含 `font-weight: 200 700` 范围：宿主按 100 档在 fontdb 登记别名，不是只取起点），经宿主 [`register_host_font_face`](../../crates/nana-ui/src/nana_text.rs) / [`alias_host_font_face_local`](../../crates/nana-ui/src/nana_text.rs) 写入共享 `FontSystem` / fontdb（与 `bundled-fonts` 同一套库）。不是 CSSOM `FontFace`。相对 `url(...)` 相对**声明该规则的样式表**。`src` 跳过 `format()` / `tech()`，按声明顺序尝试 `local()` 与 `url()`：`local("Family")` 命中已加载的 fontdb 家族名或 PostScript 名则别名 CSS `font-family` 且不读 url；未命中则 fail-closed 试下一项。未匹配的 `@media`（含 `print`）里的 `@font-face` 不注册。读上限 8MB；按 canonical 路径或 `local:name` + family + weight 范围去重。

**`@supports`。** 解析期求值，匹配则把内部规则并进同一份 cascade。谓词子集是 L1 已有的：`display: flex|grid|block`（以及 L1 已解析的其它 `display` 关键字）、`color`（`parse_css_color` 能解析的值）、`width`（`LengthSpec::parse` 能解析的值），可加 `not` / `and` / `or`。未知谓词（`selector()`、`lab()` / `display-p3`、未列入的属性）整块 fail-closed，计入 `skipped_at_rules`。

**`@layer`。** `@layer name { }` 与匿名 `@layer { }` 把内部规则按作者源序并进 cascade，并记下层名（`ParsedStylesheet.layer_names`）。**没有** cascade-layer 优先级（unlayered 并不压过 layered；`!important` 也不按层反转）。`@import … layer()` / `supports()` 仍不加载。

增量 `width` / `height` / `min-width` 等 layout prop（`patchProp(width, …)`，不是改 `style`）与 class / style 走同一份 `rebuild_layout_style`：先写完普通层和 `nana-*` class hints，再写样式表 important，然后是 prop / inline important。因此样式表 `width:80px !important` 不会被后来的普通 `width:200px` prop 盖掉；prop / inline 带 `!important` 的仍覆盖样式表 important。`nana-*` hints 压过普通声明，但压不过其后的 important 尾。

自定义属性会去掉 `!important`（与普通声明同一套 `split_important_flag`），所以 `--gap: 8px !important` 经 `var(--gap)` 得到 `8px`，而不是带着标志无法解析。

**JS 查询。** `getBoundingClientRect` / `layoutBox` 读绘制投影：`offsetWidth` 是边框盒，`clientWidth` 是 padding 盒，`scrollWidth` 是内容尺寸；`offsetLeft` / `offsetTop` 相对 `offsetParent`；`clientLeft` / `clientTop` 是边框。滚动不写回 Runtime 的 `LayoutBox`。`getComputedStyle` 是 Vue Transition 桩加上绘制投影上的 used 值（`width` / `height` / `color` / `opacity` / `transform`），不是 CSSOM，也不是完整 `LayoutStyle`。

## 不要指望的

竖排 `writing-mode`（不假翻轴）、`unicode-bidi` 隔离 / 完整 IFC 双向文字、用 `direction` 翻转 flex/grid 主轴或 item 序、`tan()` / `atan2()` / `sin()` 等其余 CSS math 函数、无法折成长度原子的混单位嵌套 `min`/`max`/`clamp`、长度互乘除（完整 `calc()` AST）、当布局条件的 `:hover` / `:focus`、带组合符的 `:has()`、未知 `@supports` 谓词、`@import … layer()` / `supports()`、完整 cascade-layer 优先级、完整浮动排除（祖先浮动侵入子块、`shape-outside`、流内块级边框盒缩窄）、完整 IFC（精确 CSS 基线、行内拆箱）、subgrid、用 `position: fixed` 当 Dialog / Drawer（走 Nana 浮层）、把 `getComputedStyle` 当成已解析的 `LayoutStyle`、`cursor`（窗口光标只服务客户区缩放，没有 CSS→`set_cursor` 映射）、`user-select`、`-webkit-app-region` / `app-region`（任意盒不能当标题栏；拖窗只走 `AppTitleBar` → `nana-window`，写了也不会拖窗）、L1 `overflow:auto` 的自定义滚动条铬（走 L2 `ScrollView`）、`<iframe>` 加载、`<video>` 解码/播放、把 `<canvas>` 当成 2D 位图或浏览器 2D/WebGPU 上下文（无 HostTexture 槽时 `skipped_replaced = canvas`，不写 `content_image`；`data-nana-canvas` / `data-nana-gpu` 走 [实时画面](gpu.md) 的 `"nana.host-texture"`，不是 Chromium 2D）、父级 CSS `perspective` 属性与 `preserve-3d`（元素自身 `perspective()` + `rotateY` 已画）。`font-variation-settings` 除 `"wght"` 外的轴（含 `BEVL`）仍 fail-closed。Android **不是**产品布局目标：实验 NativeActivity 宿主仍用同一份 `LayoutStyle` / Runtime / UiScene，没有第二套 Android 布局引擎；该宿主上的软键盘、无障碍、系统剪贴板 fail-closed（见 [Android](android.md)）。

廉价主体 `:has`、`::before` / `::after` 与 `::placeholder` 见上文「能用的」。

主题色走控件和外观设置，不要用任意业务色去改框架 token。

未知声明会被忽略；不要假设「写了就能在某处生效」。

## 和 Rust 布局的关系

Vue 侧解析出的是同一份 `LayoutStyle`。真正算盒子的是 Runtime 的 `RuntimeLayoutEngine`，产品帧走 `RuntimeDocument::flush`。JavaScript 查询到的盒子是绘制阶段的投影，滚动不写回 Runtime 的布局权威。
