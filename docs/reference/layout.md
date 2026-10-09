# 布局（Vue CSS 子集）

Rust 的第一路径用控件自己的布局。你不写 CSS。排行、列和边框见 [布局与样式（Rust）](rust-layout.md)。

这篇只描述 Vue 兼容路径能用的那一部分网页 CSS。你用它排界面骨架。

它不是浏览器里的 CSS 引擎。没有完整的选择器世界。对不上，就改布局。不靠容差假装一致。

对话框、抽屉、菜单用 [控件](components.md) 里的浮层。

## Layout intent authority

组件语义、作者 CSS/Rust 和运行时约束都进入同一份 layout intent。字段按
`nana_ui_core::LayoutOwnership` 标记为 component-required 或
component-default；默认值允许作者覆盖，required 值保留给组件。最终样式仍由
Runtime 写入口提交，`LayoutBox` 和滚动投影是输出，不是可写的前端属性。

Vue 的 containing block 传播以 layout mutation footprint 为 seed。静态帧和
paint-only 更新不再扫描整棵投影树；视口变化与结构变更仍从根节点重算，以保持
百分比、Fill 和 intrinsic sizing 的正确性。

Intrinsic metrics 由 Runtime 的 `IntrinsicCache` 统一保存。它记录内容和样式
事实（min/max inline/block、preferred size、首尾 baseline、aspect ratio）以及
generation；当前 containing block 解析出的 used size 单独保留，不能回写这份
事实缓存。缓存 key 不含 Flex、Grid 或 IFC 的名称，因此相同的约束类可以跨
formatting context 复用；百分比、Fill、aspect ratio、字体、writing context 和
viewport 等依赖会进入约束或输入身份。内容、形状样式、边框、min/max、资源
metadata、scale/font 和子树 metrics 的变化才会 bump generation；颜色、opacity、
transform、hover 和 accessibility 更新不会 bump。

## Incremental reflow frontier

Runtime 的增量布局把一次布局失效表示为 `LayoutInvalidation`：它同时记录
`InvalidationKind`、变更字段和 `LayoutDependencyFootprint`。保留布局可以把同一帧
的多个 seed 合并成 measure、placement、writing-context 和 scroll/overflow 各自的
frontier，并记录 `layout_frontier_*` 与 `layout_dependency_edges_visited` 等工作
计数。结果只改变位置时，父级只进入 placement frontier；`LayoutMetricDelta::NONE`
则停止向上传播。父约束只把消费该轴的子节点送进 measure；只改位置的后代留在 placement。
固定边框且导出尺寸不变的盒子挡住子内容：祖先不再进入 measure，外侧兄弟不再进入
placement。顺序容器只对进入 measure frontier 的子项重新测量，后缀 placement 沿用
已测尺寸。一次重算若 `LayoutResult` 与已发布结果逐位相同，则不推进 layout
generation，已发布的结果对象保持不变。

执行计数和 frontier 计数是同一份 `WorkCounters`。`layout_measure_nodes` 只计真正算出 used size 的节点；retained used-size 与 measure plan 的命中记在 `layout_measure_cache_hits`，未命中后重算记在 `layout_measure_cache_misses`。Intrinsic metrics 的命中仍是 `intrinsic_measure_cache_hits`，不另计一份。`layout_placement_nodes` 是这次 placement 写下的盒子，不是 frontier 成员数；Foundation 的 `layout_nodes_placed` 只统计 adapter 的 `publish_result`。`layout_origin_only_updates` 是尺寸没变、只改了原点的写入。`layout_result_reused` / `layout_result_changed` 来自结果发布：几何相同就留着原来的对象，几何变了才替换。`layout_delta_commits` 是真正写下变更结果的那一次发布。`layout_context_local_solves` 不单列，它就是已排进 frontier 的 `layout_frontier_contexts`。

有依赖索引的格式化上下文可以使用 `LayoutDependencyGraph` 表达父约束、包含块、
写作方向和 flex/grid/inline 的局部耦合。Runtime mutation authority 和产品帧统一发布
按节点合并的 typed seeds；Vue bridge 维护
`SnapshotChanges::layout_invalidations`，而普通 semantic `dirty` 仍负责属性和
投影同步。布局调度直接消费 typed seeds。
不确定的 viewport 或结构变化保留根级重算边界。

## 能用的

**Flex。** 你可以用 `flex-direction`、`flex-wrap`、`gap`、`align-items`、`align-self`、`justify-content`。多行换行时还有 `align-content`。`stretch` 和 `normal` 把剩余的交叉空间均分给各行。还有 `order`、`flex-grow`、`flex-shrink`、`flex-basis`。侧栏加主区，用这一套就够。

换行的行按项目**长大之前**的尺寸分行。这是 CSS 的 hypothetical main size。先取 `flex-basis`。没有，就取主轴尺寸。再没有，就取内容。并受 `min-*` 和 `max-*` 约束。分好行之后，才把行里剩下的空间按 `flex-grow` 分出去。

所以 `flex: 1 1 320px` 的两栏，放得下就并排，放不下就各占一行。带 `flex-grow` 的项目也不会独占一行，把放得下的兄弟挤走。

你没写 `flex-shrink` 长手时，按 **0** 处理。不是网页 CSS 的 initial **1**。溢出的定宽行（列表、工具条）会保留盒子。不会被悄悄压扁。

`flex` **简写**省略 shrink 时，仍按 CSS 写成 1。例如 `flex: initial`、`flex: 1`、`flex: 1 100px`。需要网页那种收缩时，显式写 `flex-shrink`，或用简写。`flex: none`、`auto` 和数字简写仍按 CSS 含义写 shrink。

**尺寸。** 你可以用像素、百分比、`em`、`rem`、`vw`、`vh`、`min`、`max`、`clamp()`。

轻量 `calc` 支持 `+`、`-`、`*`、`/`、括号和嵌套 `calc`。`var()` 展开后再算。结果折进既有的 `%±px`、`vh±px`、`em±px` 或纯 px 规格。嵌套上限是 16 层 CSS 括号或函数。unary `+/-` 另计，最多 16 个符号。无单位结果、`0px+number`、除零、非有限 f32，都 fail closed。

同单位、可以折叠的嵌套 `min`、`max`、`clamp` 可以用。能并进既有 `Min2`、`Max2`、`Clamp3` 的混单位也可以。例如 `min(10px, max(1px, 50%))` 和 `min(1px, 2%, 3px)`。布局时相对包含块兑现。三路互不可比，仍 fail closed。

还有 `min-content`、`max-content`、`fit-content`。

无法折成长度原子的 leftover `calc`，可以走 `LengthSpec::Calc` 这棵 AST。不要假装它已经 flatten。

不折行的 flex 行上，`min-content` 和 `max-content` 都是子项之和。折行、块、列轴上，`min-content` 取最宽的子项。

`fit-content` 以最宽子项为下限，以可用宽为上限。折过行的文字，按它不折行的宽度量内容宽，并且不超过可用宽。所以按内容收窄的盒子，在上限放宽后会变宽。它不会停在上次折行的宽度。

flex 子项的 `min-width` 或 `min-height` 写成 `min-content`、`max-content` 或 `fit-content` 时，下限取它量出的内容尺寸。行里空间不够时，先压别的子项。它不小于自己的内容。主轴尺寸要是 `auto`，`100%` 或 `Fill` 量出来的是可用宽。

**盒子。** 你可以用 padding 和 margin。`margin: 0 auto` 在块或列的格式化上下文里水平居中。

也包括 `direction: ltr | rtl` 和 `writing-mode: horizontal-tb | vertical-rl | vertical-lr` 下的 CSS 逻辑属性。例如 `padding-block`、`margin-inline`、`inset-inline`。

逻辑边落在哪条物理边，由最终的 writing-mode 加上 direction 决定。类型是 `nana_ui_core::WritingContext`。

`horizontal-tb` 下，`direction: rtl` 把 inline 的 start 和 end 映到 right 和 left。`padding-inline-start` 落到右边。

竖排时，inline-start 在顶端。rtl 时在底端。block-start 在 `vertical-rl` 的右边，在 `vertical-lr` 的左边。

逻辑边和同一条物理边的声明，按先后定胜负。这包括跨层级联。样式表写了逻辑边，后一层或继承才出现 `rtl`，也会 remap。

HTML 的 `dir="rtl"` 或 `dir="ltr"` 是 presentational hint。它写入同一条 used `direction`。作者的 CSS `direction` 覆盖它。`dir="auto"` 没有 first-strong bidi。它 fail-closed，不假装 ltr。

竖排时，block 轴是横的，inline 轴是竖的。`vertical-rl` 的 block-start 在右。没有 `sideways-*`。

`text-orientation: mixed | upright | sideways` 会继承。它决定竖排里的字是直立还是侧卧。`upright` 按 CSS 把使用值 `direction` 变成 `ltr`。所以 `vertical-rl; direction: rtl; text-orientation: upright` 的行首回到顶端。逻辑边也跟着落。

文本节点按列排。CJK 直立。那用字体的竖排度量，以及 `vert` 标点字形。拉丁字母和数字侧卧。按盒子高度折列。见 [文本引擎](text-engine.md#writing-mode-与-59)。

TextInput 和 TextArea 也在列里编辑。光标横跨一列。上下键沿列。左右键跨列。点击命中列里的字。

`horizontal-tb` 下，`direction: rtl` 把 inline 轴的起点放到右边。这些都跟着翻：flex **行**主轴的起点和 item 序，`justify-content` 的 start 和 end，grid 的列序。第 1 列是最右边那条轨。轨道总宽小于容器时，整体靠右。还有 `justify-items` 和 `justify-self` 的 start 和 end，以及 **column** flex 的交叉轴起点。row flex 的交叉轴是 block 轴，不受影响。

这些都发生在布局层。它们不改 `flex-direction`、`flex-reverse`、`justify-content` 这几个样式字段的值。

flex 和 IFC 的摆放按流相对坐标算。从主轴起点和交叉轴起点量起。按原顺序折行。对齐关键字照原样读。只在写出子项坐标时，经 `WritingContext` 换到页面。反向轴只是这一次换算。不倒转子项列表。不翻转对齐关键字。所以折行的 RTL 行，第一行装的是前几项。

竖排再加 `direction: rtl` 时，inline 轴从底端开始。这是 CSS Writing Modes §2.1。IFC 和 `flex-direction: row` 的第一项在列底。`text-align: start` 贴底。column flex 的交叉轴起点在底端。文字里的 CJK 仍自上而下读。双向算法的行相对顺序不随 `direction` 动。段落末尾的中性标点会像横排 RTL 一样，落到行的另一端。

grid 的列是 inline 轴上的轨道。行是 block 轴上的轨道。这是 CSS Grid §3。竖排时，列沿竖向排。`vertical-rl` 的行从右往左叠。竖排 rtl 的第 1 列在底端。轨道尺寸按沿行或跨行的长度算。摆放同样是流相对坐标换到页面。

`writing-mode` 和 `direction` 在布局里也继承。没写这两项的容器，沿祖先的方向排。Rust API 或样式树直接构造的节点同样如此。不只是走级联的 Vue 路径。祖先改了方向，后代容器随之重排。没写方向的盒子上的逻辑边（例如 `padding-inline-start`）也按继承来的方向落到物理边。

百分比的 margin 和 padding，四条边都按包含块的 **inline 尺寸**兑现。这是 CSS Box Model §5。这个 inline 取自**包含块**的写作方向，不是盒子自己的。竖排父级里的横排子盒，`margin-top: 10%` 按父级的高度算。IFC 折行和 grid 一维轨道的百分比同理。

仍然没有的，是 `unicode-bidi` 隔离，以及完整的双向文字。

默认是 border-box。也认 content-box。

**网格。** 你可以用 `grid-template-columns` 和 `rows`、`fr`、`minmax`、固定次数的 `repeat(N, …)`。布局时还会展开 `repeat(auto-fit | auto-fill)`。它可以和前后的固定轨混写。例如 `80px repeat(auto-fit, 1fr)`。auto-fit 按容器能放下的轨数展开。空轨会收起。

还有 `grid-column`、`grid-row`、`span`、`grid-auto-flow` 的自动放置、`grid-auto-columns` 和 `rows` 隐式轨，以及网格项上的 `justify-items` 和 `justify-self`。

轨解析之后，项上的 `width` 和 `height` 百分比，以及 `Fill`，相对**最终单元格**兑现。量测阶段仍把 `100%` 和 `Fill` 当成不定尺寸，以免撑破 auto 轨。

先定列，再定行（CSS Grid §11.5）。auto 行的高度取项**按它跨的列宽排开**之后的高度，不取列解析之前那次量测：那次 `100%` / `Fill` 的项按零宽量，能折行的芯片一行一个，`auto` 宽的项按整个容器宽量，芯片挤在一行。所以两列 197px 里的五个 60px 芯片折成三个加两个，行高 46，和浏览器一致（T-G36）。

`grid-template-areas` 加上 `grid-area: header` 是命名区域。轨上还有 `[name]` 命名线。同名线可以用 `foo 2` 或 `2 foo` 取第 N 根。`foo / foo` 的终点取起点之后的下一根同名线。不是 CSS 那种对调。

`[start] 80px repeat(auto-fit, …)` 的前缀线名会保留。repeat 内的线名也会保留。`repeat(N, …)`、`auto-fit`、`auto-fill` 里的线名按展开次数复制。接缝处相邻的名字合并。因此 `mid 2` 相对展开后的几何取值。

定高网格里，空的 auto 行（量测约等于 0）会吃掉剩余高度。有内容的 auto 行（T-G26）保持内容高，不会被拉伸。这不是 CSS `align-content: stretch` 作用在轨道上。

自动放置扫到 4096 格仍放不下时，项落在扫描区外。这不是宿主错误。

`repeat()` 不能嵌套。一个轨列表也至多只有一个 auto-repeat。这是 CSS Grid 的文法。`<track-repeat>` 的体是 `<track-size>`。`<auto-repeat>` 的体是 `<fixed-size>`。它们都不是 `<track-list>`。这不是 NanaUI 的缺口。

`repeat(3, repeat(2, 1fr))` 这类写法，整轴 fail closed，并记 `NestedRepeat`。和浏览器把整条声明作废一致。

整值 `subgrid` 继承父网格在该项跨越范围内已经解析的轨道尺寸和 gap。父级没有轨道时，计算为 `none`。不臆造 auto 轨。

把 `subgrid` 当作轨道列表里的单个 token 来写，例如 `subgrid 80px`，仍记 [`GridTrackListUnsupported::Subgrid`](../../crates/nana-ui-core/src/box_layout.rs)。

直接构造 `LayoutStyle { display: Grid }`、不经 css_map 时，`align-items` 默认 `start`。手写测试依赖此项。Default 无法按 display 区分「未写」和显式 `start`。经 `display:grid` 解析，会写成 `stretch`。

**行内子集。** 块容器里的 `display: inline` 和 `inline-block` 走一行内格式化上下文，也就是 IFC。

行内子项按 **inline 轴** 排列。`horizontal-tb` 向右。`vertical-rl` 和 `vertical-lr` 向下。块级兄弟会关掉当前行，并独占一行。

竖排时，盒子的物理宽高仍是 width 和 height。但 IFC 和 `flex-direction: row` 的主轴跟着 writing-mode 的 inline 轴走。`vertical-rl` 的行从右边起排。

你可以用 `text-align`。`start` 和 `end` 随 `direction`，仅横排。`left` 和 `right` 保持物理边。还有 `center`。

`white-space: pre` 在量测里保留换行和空格。

`align-items: baseline` 读取同一份 intrinsic metrics。文本节点优先使用
`nana-text` 保留的首行和末行 baseline；宿主只提供 ascent 时才使用该
fallback。替换内容和 Custom 节点以 block 轴末端作为明确 fallback。Flex、Grid
和 IFC 不各自估算一套基线。

**浮动子集。** `float: left | right` 把盒子从块流里拿出来。同侧多个浮动按几何并排。放不下就折到下一行。

流内的 `clear`，和浮动自身的 `clear`，都用折行之后的占用底边。不是单盒的预排高度。

IFC 行盒按当前行与**兄弟**浮动外边距盒相交的左右 inset 缩窄。这是 shrink-to-avoid-float。一行里放不下的原子行内项折到下一行。缩短后仍放不下，就把行盒下移到最近那个占用浮动的底边之下。

流内**块级**边框盒不会缩窄去绕开浮动。这和 CSS 里非 BFC 的块一致。

这不是完整排除。祖先浮动不侵入子块的 IFC。没有 `shape-outside`。

flex 项和 grid 项上的 float，按 CSS 被块化，然后忽略。

**定位。** 你可以用 `relative`、脱流的 `absolute`、相对窗口的 `fixed`，以及文档流内的 `sticky`。`sticky` 在滚动投影之后才贴住。它不写回 Runtime 的 `LayoutBox`。

`fixed` 只适合普通节点贴在视口上。产品浮层仍走控件。不要自己用 `fixed` 搭对话框。

脱流盒子的高度没写，也没同时写 `top` 和 `bottom` 时，按内容算。它里面按高度填满的子项（`Fill`、百分比高度）没有确定的高度可依，按内容处理。不再取包含块的高度。包含块的高度由这些盒子撑出来时（按内容量高的虚拟列表行），那样会每一趟布局都长高一截，帧永远稳定不下来。

这条只管它里面的子项。盒子自己的百分比 `min-height` / `max-height` 仍按包含块的高度兑现：`fixed` 是视口，`absolute` 是它的包含块。脱流盒子的包含块总有确定的高度（CSS 2.1 §10.7）。所以 `position: fixed; max-height: 80%` 的卡片按内容长高，最高到视口的 80%。不用改写成 `80vh`。

**层叠上下文子集。** Scene 按 `(z_index, document_order)` 排序。它把隔离组当成一层。这些组是：`opacity` 介于 0 和 1 的组、`isolation: isolate`，以及 `position` 非 static 且写了 `z-index`。

高 z 的子项不会画到组外、后出现的兄弟之上。

**例外：** `position: fixed` 的表面画在根层叠上下文。Popover、ActionMenu、HoverCard 的弹出层属于这一类。它们不被父级隔离组裁进卡片里。

弹出层的内容由 Runtime 定成 viewport-fixed。抽取给 Scene 时也按这份样式走。所以祖先的 overflow 裁剪、滚动和变换都不作用于它。

卡面是触发节点画的那块底。它同样不带触发器的变换和裁剪。它和弹出内容排在根层叠上下文的同一层（`MENU_OVERLAY_Z_INDEX`）。位置在触发器之上，也在页面其余内容之上，包括后出现、`z-index` 更高的兄弟。它在弹出内容之下。

重叠处的命中排序用同一条前缀切断。否则后面的兄弟卡会抢走菜单点击。卡面本身也在这一层接指针。落在条目之间或内边距上的按下，留在菜单里。不穿到底下的页面。也不当成按触发器去开合菜单。

不要靠你的应用给整张卡抬 `z_index`。

这不是完整的 CSS Appendix E。负 z 分层、float 和 inline 层、transform 单独成层等，未全做。

命中仍走树结构。组内的 z 只在兄弟之间比。

**`display: contents`。** 子节点提升到父级的格式化上下文。该节点自己没有盒子。

**文字。** 你可以用字号、字重、字体、行高、字距、颜色。

`line-clamp` 和 `-webkit-line-clamp` 限制行数，并打省略号。

`text-decoration: underline | line-through` 由 Scene 在文本盒上描线。

`font-feature-settings` 和 `font-kerning` 写入 `nana-text` 的 OpenType features。

`font-variation-settings` 把已经声明的轴交给 shaper。`wght` 仍并进 `font-weight`。字体没有该轴，则**该轴** fail-closed。不把标签改写成 `wght`。也不塞进 `FontFeatures`。

`user-select: text | none | auto | all | contain` 写入 `LayoutStyle`。它继承，和 `cursor` 同类。`auto` 下，普通 `TextContent` 不可选。`text` 允许文档级拖选复制。`all` 单击即选中该文本节点的全文，不必拖。`contain` 可以选中，但选区留在该子树内，不延伸到邻居。`none` 不进选区。子节点显式写 `text`，可以再打开。

这不是第二套 TextInput。

作者的 `::selection` 和 `::-moz-selection` 只兑现 `background`、`background-color`、`color`，画到文档选区高亮。其余属性 fail-closed。这不是完整的 CSS Highlight API。未写时，选中底仍用主题的 `accent_soft`。

`word-break: keep-all` 和 `line-break: strict|loose` 会跳过。字距是近似。

**隐藏。** `display: none` 不占位，也不参与点击。

`visibility: hidden` **仍占位**。它参与 flex 和 grid 的测量。但不绘制，也不命中。

`pointer-events: none` 仍绘制，仍占位，只是自己不命中。子级写 `auto`，可以挖洞点中。

这是 Runtime `UiWorld` 的命中。不是窗口 Alpha，也不是异形窗。

内部的 `layout.hidden`（侧栏、菜单等）和 `display: none` 一样跳过布局。

**`transform`。** 它只影响绘制，不算进布局。这是 CSS 的正确行为，不是缺口。

2D 子集是仿射：`matrix()`、`rotate()`、`rotateZ()`、`translate()`、`scale()`、`skew()`。

`translate3d` 和 `translateZ` 的 z，在没有透视时，不改变 z=0 的投影。

`perspective()` 加上 `rotateY`、`rotateX`、`rotate3d`，或带透视残差的 `matrix3d`，走同一条 Scene 平面单应。那是 4 顶点透视除法。**Quad、Text、Icon、HostTexture 共用这份 `(g,h)`。** 同一节点不会出现梯形底加上仿射字。

Spinner、stroke mesh，以及非 HostTexture 的 Custom，在透视下画 **identity**。不做 `scaleX(cos)` 那种假 3D。

父级的 `perspective` 属性，以及 `transform-style: preserve-3d`，仍 fail-closed。

`transform-origin` 是独立的 2D 字段。默认是盒中心。它对 2×3 和 4×4 都生效。`transition` 和 `@keyframes` 会插值它的 `%` 和 `px`。仍不算布局。

`transform` 本身的动画走 compositor overlay。不是每帧改逻辑样式。见 [架构](architecture.md)。

`transform-box` 里，`border-box` 和 `view-box` 相对边框盒。HTML 下 `view-box` 就是边框盒。没有 SVG viewport。`content-box` 和 `fill-box` 相对内容盒。内容盒是边框盒减去 border 和 padding。padding 的 `%` 没有包含块时按 0。这和其它绘制期的 padding 解析一致。`stroke-box` 不解析。

**绘制。** 你可以用 `background`、`background-color`，以及 `background-image: linear-gradient(...) | radial-gradient(...)`。

`background-size` 的初值是 CSS `auto`。另有 `cover`、`contain`、`stretch`、`scale-down` 和长度。

`background-repeat` 的初值是 `repeat`。`round` 按整数格缩放铺贴。`space` 和混写 fail closed，不铺成 repeat。

`mask-image` 和 `-webkit-mask-image` 可以是线性或径向渐变的 alpha，或 `url()` 纹理。GPU 最多 8 个 mask 色标。

`clip-path` 支持 `inset(...)`、`polygon(...)`、`circle(...)`、`ellipse(...)`。inset 的 `round` 写入 rounded-box SDF。非平移 transform 下仍保留半径。polygon 对自身和子项做点内多边形测试。子项经 dest-group 合成。不是包围盒 clip。

`filter` 支持 `brightness()`、`saturate()`、`contrast()`、`hue-rotate()`、`invert()`、`opacity()`、`blur()`、`drop-shadow()`。`blur` 和 `drop-shadow` 是元素自身的滤镜。blur 上限 16px。它们和 `backdrop-filter` 分开。

`drop-shadow` 在 dest 合成组里采样已经画好的 alpha 轮廓，再 offset，再用同一个核 blur。它不是 `box-shadow` 的盒几何。

有子节点、文本、自定义绘制，或有 `blur`、`drop-shadow` 时，用 dest 合成组。叶子上的 hue 和 brightness 走 quad shader。未知函数仍整表 fail closed。多层 `drop-shadow` 和 spread 未实现。

`box-shadow` 支持 `inset`，也支持逗号分隔的多层。GPU 上限是 4。

`outline` 只画 solid 的额外描边。它不进布局。

`mix-blend-mode` 的 GPU 子集是 `normal`、`multiply`、`screen`。它们走 dest-group 的 BlendState。其余关键字 fail closed。

`line-clamp` 和 `-webkit-line-clamp` 走已有的文本 ellipsis，并设 overflow hidden。

`border-image` 是最小子集。`url()` 或 `linear-gradient` 加上 `slice`（可选 `fill`）走现有的 quad URL 纹理 9-slice。`linear-gradient` 先栅成纹理再切。`border-image-width` 默认是 `1` 乘 slice。`repeat` 仅 stretch。`radial-gradient`、outset、round、repeat、space，以及非默认 width，仍 fail-closed。

四边的 `border-*-width` 和 `border-*-color` 参与布局，也参与 GPU stroke。

`border-style: dashed | dotted` 在同一条 rounded-box SDF 环上按边做周期遮罩。不另开管线。`double`、groove、ridge、inset、outset 仍占 used width，但不描。

`background-image: url(...)` 与 `<img src>`、内联 `<svg>` 共用同一条 decode 加 URL 纹理缓存。支持 `data:image/png|jpeg|svg+xml`、`http://`、`https://`、`file://` 和相对路径。`.svg` 走已有的 `resvg`。最长边 2048，保持比例。不是第二套矢量引擎。

相对 URL 和 `file:` URL 走与样式表相同的 [`canonicalize_within_jail`](../../crates/nana-ui-core/src/url_jail.rs)。非本机的 file host 拒绝。不把 `Url::path()` 回退到任意文件系统。

`http(s)` 不在这条 jail 里判。它走宿主注入的 `fetch_host`（`MountOptions.fetch_host`）。源白名单、逐跳复核重定向、跨源去掉授权头、超时，都按 `FetchPolicy`。没注入，就拒绝。

取消在 painter 析构时触发，也在图片连续 120 帧无人引用时触发。终结的是网络等待，也就是 socket shutdown。不是已经开始的解码。

正文上限取 8MB 与 `FetchPolicy.max_response_bytes` 的较小值。

相对路径优先相对文档或宿主设置的 base。见 [`set_background_image_url_base`](../../crates/nana-ui/src/scene_paint/image_url.rs)。未设置时，安装版和便携版相对 `ApplicationPaths::runtime_resources()`。开发布局，以及没有应用路径的嵌入宿主，相对进程 cwd。

绝对路径和 `file:` 的 jail 不变。有宿主 base 就用宿主 base，否则用 cwd。

`nana://res/<逻辑路径>` 从打包资源读取。见 [打包与分发](packaging.md)。打包样式表里的相对 `url()`，在加载时改写成同一包内的绝对 `nana://res/` 地址。

SVG 的 `<image href>` 不读本地文件。

同一个 URL 的纹理按 URL 缓存，并按 URL 分批 rebind。GPU 侧每个 quad 最多 8 个 gradient 色标。

CSS `url()` 的帧仍走 4× MSAA。它和 HostTexture、backdrop 的 interleaved 路径分开。

`backdrop-filter: blur()` 是逐节点的 dest 采样模糊。它做旋转映射，也做祖先的 inset 和 polygon clip。不是整窗的 Mica 或 Acrylic。

`<img>` 的 `object-fit: cover | contain | fill | none | scale-down` 经 cascade 写入 `PaintStyle`，再落到 `CustomRenderNode.fit`。HTML attr 是 presentational hint。样式表压过 attr。inline 再压过样式表。

**命中。** `pointer-events` 是 `auto` 或 `none`，按 CSS **继承**。

父级是 `none` 时，没写该属性的子孙，used 值也是 `none`，不可点。只有显式 `auto` 的子节点重新成为目标。点在父盒上、但没落在那个 `auto` 子上，会穿透。

未知关键字 fail closed。它不改已经指定的值。`inherit` 和 `unset` 清掉指定值。

平面 3D 命中，和 Quad、Text、Icon、HostTexture 的绘制，共用同一份单应逆变换。不按仿射盒命中。也不按梯形 AABB 的空角命中。

**溢出。** `overflow`、`overflow-x`、`overflow-y` 可以是 `visible`、`hidden`、`clip`、`auto`、`scroll`。

`hidden`、`clip`、`auto`、`scroll` 都会裁剪绘制，也会裁剪命中。

两轴都裁剪时，后代的绘制按盒子自己的 `border-radius` 裁圆角。裁的是边框盒。四角不等时取最小的那个，不超过短边的一半。这和 CSS 一致。盒子自己的背景和边框仍只按直角边框盒裁。不被同一个圆角再裁一次。命中仍按直角。

L1 `overflow: auto|scroll` 的滚动权威仍是 Runtime 的 `ScrollOffset`。JS 的 `scrollTop`、`scrollLeft`、`scrollIntoView` 和滚轮走这条路径。滚动不写回 `LayoutBox`。

偏移保留小数。画面按设备像素吸附。见 [Runtime 与 Scene](runtime-scene.md)。

滚动起点在 start 边。这是 CSSOM View 的 scrolling area。`direction: rtl` 的行、`vertical-rl` 的块轴、`row-reverse` 和 `column-reverse` 从右或下开始排。初始视图就是右或下边。向左或向上溢出的内容，用负偏移滚到。和 CSSOM 一样，`scrollLeft` 和 `scrollTop` 从 0 往负走。越过 start 边另一侧的内容，滚不到。

每个滚动容器都有度量。写布局盒、增删子节点，或改样式的提交结束时，Runtime 重新测量受影响的滚动容器，并把偏移夹进去。不论盒子是引擎写的，还是宿主写的。多行文本编辑器的度量随塑形结果走。`vertical-rl` 编辑器同样用负偏移滚向左边的列。

自定义滚动条铬是 L2 的 [`ScrollView`](components.md)。L1 不另做一套 thumb 绘制。

**选择器。** 你可以用 type、class、id、属性选择器，以及组合符空格、`>`、`+`、`~`。

还有 `:root`、`:first-child`、`:last-child`、`:only-child`、`:nth-child()`、`:nth-of-type()`、`:nth-last-child()`、`:first-of-type`、`:last-of-type`。简单的 `:not()`、`:is()`、`:where()` 也可以。其中包括廉价的 `:disabled` 和 `:not(:disabled)`。它们和 `[disabled]`、表单控件的 `disabled` 是同一份存在性匹配。

廉价 `:checked` 只匹配 checkbox、radio、switch。它以 `WidgetProps.toggled` 为准。非 checkable 宿主上的 `checked` 属性不匹配。`toggled=false` 会清掉 `attrs["checked"]`。`input:checked + label` 走已有的兄弟组合符。

`:empty` 和 `:not(:empty)` 的意思是：没有元素子节点，并且宿主的 `label`、`value` 和子文本都没有含非空白 Unicode 标量的 UTF-8。空白用 `char::is_whitespace` 判断。

文本节点只认 `#text`、`createText`、`nana-text`。`p`、`span`、`label`、`h1`–`h6` 即使 kind 是 Text，也是元素。空的 span，或 `p` 加 `createText`，都会让父级不是 `:empty`。

空白文本，以及生成盒 `::before`、`::after`，不算内容。`:first-child`、`:last-child`、`:only-child`、`:nth-*` 的兄弟计数不含生成盒，也不含这些文本节点。

廉价主体 `:has(.class|#id|type)`，含逗号 OR，按一次 O(n·k) 的后序 bitset 匹配。k 是去重后的简单参数，上限 64。不是每个主体都扫一遍子树。

廉价主体 `:focus-within` 在焦点变化时按祖先链失效。不整表、每帧重算。

`:focus-visible` 映射为 `:focus`。没有键盘和指针的区分。这是 fail-closed：未聚焦时不匹配，也不会单独画 focus-visible。

`::before` 和 `::after` 走生成盒。

`::placeholder` 把 `color` 和 `opacity` 画到 Runtime TextInput 的占位文字上。只对 `input` 和 `textarea`。它不是生成盒。也不在非输入上假装占位。

`::selection` 和 `::-moz-selection` 把 `background`、`background-color`、`color` 画到文档级文本选区。它不是生成盒，也不是完整的 Highlight API。

带组合符的 `:has` 和 `:focus-within`，以及写在祖先上的，计入 `skipped_selectors`。

**层叠。** 这些规则同属 author。

样式表内部先 normal，再 `!important`。然后再比特异度和源序。

prop 和 inline 的**普通**声明覆盖样式表的普通声明。inline 覆盖 prop。

样式表的 `!important` 覆盖 prop 和 inline 的普通声明。

prop 和 inline 上的 `!important` 会去掉标志，并作为 author-important 再写。它覆盖样式表的 `!important`。inline important 覆盖 prop important。

`:hover`、`:focus`、`:active` 进交互桶。它们按 Runtime 的指针和焦点状态，叠到绘制样式上。不当布局条件。

`::before` 和 `::after` 在有 `content` 时生成匿名子盒。这是部分落地：只有字符串 content。没有 `attr()`，没有计数器，也没有 `quotes`。

`@keyframes` 和 `transition` 已经解析，并编译进 Motion IR。`opacity`、`transform`，以及分类为 Compositor 的 `clip-path`，走 compositor overlay。`color`、`background`、`filter` 走 Paint-class 的 CPU 插值。`width` 和 `height` 走 Layout-class。

`font-variation-settings` 每个轴一条 Layout-class track。只在两端声明同一组轴时，才逐轴插值。否则离散切换。transition 两端的轴集合不同时，直接切换。`@keyframes` 相邻两帧的轴集合不同时，在该段缓动进度过半处切换。某帧没写的轴，在那一段不存在，取字体默认值。

`@keyframes` 缺 `0%` 或 `100%` 时，用元素自身的值补齐。补的是 opacity、transform 和字体轴。

属性分类、fallback 和性能含义见 [架构](architecture.md)。

**增删节点时重算什么。** 插入或删除一个子节点，只重算样式表能观察到的部分。重算哪些，由你启用的规则决定。

用了 `:first-child`、`:nth-child()` 这类从前数的伪类，或用了 `+`、`~`，就重算变动位置之后的兄弟。

用了 `:last-child`、`:nth-last-child()` 这类从后数的，就重算之前的兄弟。

用了 `:empty`，就重算父节点。

用了 `:has()`，只重算祖先链。`:has()` 里不能写组合符，所以它只读后代。

后代选择器和子选择器看不到兄弟。例如 `.a .b`、`.a > .b`。追加一行，只重算这一行。

`:has()` 的后代位集随每次增删、改 class、改属性、改文字做增量维护。不再每次改动后整棵树重扫。

只有 `+` 规则时，只取紧挨着的前一个兄弟。只有计数伪类，才算兄弟位置。

实测用的是 `nana-controls.css`，86 条规则。每行一个行容器、一个文本、一个按钮。release，2026-09-29，负载约 3。挂载 100 行从 473 ms 降到 8.4 ms。800 行从 175 s 降到 70 ms。4,000 行是 403 ms。随行数线性增长。

此前插入时整批重算父节点和全部子节点，每次都为全部兄弟建快照，并整棵树重建 `:has()` 索引。那是三次方。

样式表解析一次约 0.23 ms。级联里应用声明约占 6%。所以没有做样式表预编译，也没有做级联结果缓存。

**`@import`。** 它解析 `url(...)` 和引号路径。相对路径相对导入方，或相对样式表 jail。canonicalize 之后必须落在 `stylesheet_base` 下。

`stylesheet_base` 由文档或 SFC 的 URL 写入，或由最近一次 `injectStylesheet` 的 href 写入。未设置时，相对 `@import` 跳过。不扫进程 cwd。`nana://res/` 打包样式表照常加载。它内部的相对 `@import` 在包内解析。

`http(s)`、`data:`、越狱、超过 1MB、带 `layer` 或 `supports()` 的 prelude，一律 fail closed。记 skip，不加载。

写在普通规则之后的 `@import`，按 CSS 忽略。写在 `@media` 块内的 `@import` 同样忽略。

`file://` 会 percent-decode。非本机 host 跳过。

循环和深度有上限，是 16。导入规则并进同一份 cascade。已经解析的导入按 canonical href 缓存。视口变化不重解析。

**`@media`。** 子集是 `min/max/width`、`min/max/height`（px）、`orientation`、`prefers-color-scheme`，以及 `screen`、`all`、`print` 类型。

条件匹配时，规则进入 cascade。视口或主题变化时，只重新 flatten 已经解析的规则。不重扫 CSS 文本。

JS 的 `matchMedia` 经 host op `evaluateMediaQuery`，和 CSS flatten 共用同一套 Rust 求值。`screen` 和 `all` 为真。`print` 为假。没有 host 时，web-api shim 回退同一子集。

**`@font-face`。** 它解析 `font-family`、`src`、`font-weight`。`font-weight: 200 700` 这种范围，宿主按 100 一档在 fontdb 里登记别名。不是只取起点。

经宿主的 [`register_host_font_face`](../../crates/nana-ui/src/nana_text.rs) 和 [`alias_host_font_face_local`](../../crates/nana-ui/src/nana_text.rs)，写入共享的 `FontSystem` 和 fontdb。和 `bundled-fonts` 是同一套库。这不是 CSSOM 的 `FontFace`。

相对 `url(...)` 相对**声明该规则的样式表**。

`src` 跳过 `format()` 和 `tech()`。按声明顺序尝试 `local()` 和 `url()`。`local("Family")` 命中已经加载的 fontdb 家族名或 PostScript 名，就给 CSS `font-family` 做别名，并且不读 url。未命中则 fail-closed，试下一项。

未匹配的 `@media`（含 `print`）里的 `@font-face` 不注册。

远程 `src` 一律拒绝。`@font-face` 没有网络传输。这和样式表 `@import` 是同一条规矩。只认 `local()`、`data:`，以及 jail 内的本机文件。

读上限是 8MB。按 canonical 路径去重，或按 `local:name` 加上 family 加上 weight 范围去重。

**`@supports`。** 它在解析期求值。匹配，就把内部规则并进同一份 cascade。

谓词子集是 L1 已有的。`display: flex|grid|block`，以及 L1 已经解析的其它 `display` 关键字。`color` 是 `parse_css_color` 能解析的值。`width` 是 `LengthSpec::parse` 能解析的值。可以加 `not`、`and`、`or`。

未知谓词整块 fail-closed，计入 `skipped_at_rules`。例如 `selector()`、`lab()`、`display-p3`，以及未列入的属性。

**`@layer`。** `@layer name { }` 和匿名的 `@layer { }` 把内部规则按作者源序并进 cascade，并记下层名（`ParsedStylesheet.layer_names`）。

**没有** cascade-layer 优先级。unlayered 并不压过 layered。`!important` 也不按层反转。

`@import … layer()` 和 `supports()` 仍不加载。

增量的 `width`、`height`、`min-width` 等 layout prop 走 `patchProp(width, …)`。那不是改 `style`。它们和 class、style 走同一份 `rebuild_layout_style`。先写完普通层和 `nana-*` class hints，再写样式表 important，然后是 prop 和 inline 的 important。

因此样式表的 `width:80px !important` 不会被后来的普通 `width:200px` prop 盖掉。prop 或 inline 带 `!important` 的，仍覆盖样式表 important。

`nana-*` hints 压过普通声明。但压不过其后的 important 尾。

自定义属性会去掉 `!important`。和普通声明同一套 `split_important_flag`。所以 `--gap: 8px !important` 经 `var(--gap)` 得到 `8px`。不是带着标志，无法解析。

**JS 查询。** `getBoundingClientRect` 和 `layoutBox` 读绘制投影。`offsetWidth` 是边框盒。`clientWidth` 是 padding 盒。`scrollWidth` 是内容尺寸。`offsetLeft` 和 `offsetTop` 相对 `offsetParent`。`clientLeft` 和 `clientTop` 是边框。

滚动不写回 Runtime 的 `LayoutBox`。

`getComputedStyle` 是 Vue Transition 的桩，加上绘制投影上的 used 值。used 值是 `width`、`height`、`color`、`opacity`、`transform`。它不是 CSSOM。也不是完整的 `LayoutStyle`。

## 不要指望的

这些你不要指望：

- `sideways-rl` 和 `sideways-lr`。
- 完整的 Tate-chu-yoko。
- 竖排 ruby。
- 竖排 table。
- 竖排加上 float 绕排。
- 用旋转整盒冒充 glyph 朝向。
- 竖排编辑器里的代码编辑器装饰不画，而不是横着画在列上。那包括行号栏、minimap、缩进和列参考线、行尾诊断、补全、hover、签名浮窗。这是 #59。
- `unicode-bidi` 隔离，以及完整 IFC 双向文字。
- `tan()`、`atan2()`、`sin()` 等其余 CSS math 函数。
- 无法折成长度原子的、混单位嵌套的 `min`、`max`、`clamp`。
- 长度互乘除。完整 `calc()` AST 仍只兑现 leftover flatten 失败的那个子集。
- 把 `:hover` 或 `:focus` 当布局条件。
- 带组合符的 `:has()`。
- 未知的 `@supports` 谓词。
- `@import … layer()` 和 `supports()`。
- 完整的 cascade-layer 优先级。
- 完整的浮动排除。祖先浮动侵入子块、`shape-outside`、流内块级边框盒缩窄，都没有。
- 完整 IFC。没有精确的 CSS 基线，也没有行内拆箱。
- 轨道列表 token 形式的 `subgrid`。
- 用 `position: fixed` 当 Dialog 或 Drawer。走 Nana 的浮层。
- 把 `getComputedStyle` 当成已经解析的 `LayoutStyle`。
- 自定义 `url()` 光标图。
- 未知的 `user-select` 关键字。
- 完整的 CSS Highlight API。`::selection` 只兑现 background 和 color。
- `-webkit-app-region` 和 `app-region`。任意盒不能当标题栏。拖窗只走 `AppTitleBar`，再到 `nana-window`。你写了也不会拖窗。
- L1 `overflow:auto` 的自定义滚动条铬。走 L2 的 `ScrollView`。
- `<iframe>` 加载。见 [应用内浏览器](gpu.md#应用内浏览器)。这一条未实现。
- 浏览器式 `<video>` 的解码和播放。宿主推帧和 poster 规则见 [实时画面](gpu.md)。
- 把 `<canvas>` 当成 2D 位图，或当成浏览器的 2D / WebGPU 上下文。没有 HostTexture 槽时，`skipped_replaced = canvas`，不写 `content_image`。`data-nana-canvas` 和 `data-nana-gpu` 走 [实时画面](gpu.md) 的 `"nana.host-texture"`。那不是 Chromium 2D。
- 父级 CSS `perspective` 属性，以及 `preserve-3d`。元素自身的 `perspective()` 加 `rotateY` 已经会画。

畸形的 `font-variation-settings` 仍 fail-closed。字体没有的轴，在 shaping 时跳过，不改写成 `wght`。

Android **不是**产品布局目标。实验 NativeActivity 宿主仍用同一份 `LayoutStyle`、Runtime 和 UiScene。没有第二套 Android 布局引擎。该宿主上的软键盘和无障碍已接第一期。没有 composition IME。读屏动作未映射回 Runtime。系统剪贴板仍 fail-closed。见 [Android](android.md)。

廉价主体 `:has`、`::before`、`::after`、`::placeholder` 和 `::selection` 见上文「能用的」。

`writing-mode: horizontal-tb | vertical-rl | vertical-lr` 已经接入逻辑边，也接入了 IFC 和 flex 行的 inline 轴。见上文「能用的」。

主题色走控件和外观设置。不要用任意业务色去改框架 token。

未知声明会被忽略。不要假设「写了就能在某处生效」。

## 和 Rust 布局的关系

Vue 侧 CSS 解析出来的，是同一份 `LayoutStyle`。

L3 用 [`Stack`](rust-layout.md) 表达这份合同。用 `from_layout`，或用字段 builder。L2 的 `nana-*` 只解析到同一个 `ComponentTypeId`。不按 `display` 或 `flex-direction` 改布局身份。

真正算盒子的，是 Runtime 的 `RuntimeLayoutEngine`。产品帧走 `RuntimeDocument::flush`。

JavaScript 查询到的盒子，是绘制阶段的投影。滚动不写回 Runtime 的布局权威。

## 页面与卡片的留白

NanaUI 的容器边距归属见 [Rust 布局合同](rust-layout.md#边距归属与覆盖)。Vue 和 Rust 共用同一套 Runtime 布局。

普通 `div` 默认没有 padding。`NanaCard` 默认左右 16、上下 14。CSS 只覆盖你声明的边。`padding: 0` 明确清零。

兄弟的 margin 和父级的 gap 相加。不自动去重，也不自动折叠。

```vue
<!-- 页面中嵌套卡片：页面外沿和卡片内部是不同边界。 -->
<NanaSettingsPage :settings="settings">
  <div style="display:flex;flex-direction:column;gap:16px">
    <NanaCard>
      <!-- 卡片内 Stack：gap 排列字段，不额外加 padding。 -->
      <div style="display:flex;flex-direction:column;gap:8px">
        <NanaInput /><NanaButton>保存</NanaButton>
      </div>
    </NanaCard>
  </div>
</NanaSettingsPage>

<!-- 贴边列表：显式关闭负责该边界的容器留白。 -->
<NanaSettingsPage :settings="settings" :content-padding="0">
  <NanaCard style="padding:0"><NanaList /></NanaCard>
</NanaSettingsPage>

<!-- 滚动到底仍保留底部 24px；不要额外添加 Spacer。 -->
<NanaSettingsPage :settings="settings"
  :content-padding="{ top:20, right:24, bottom:24, left:24 }"
  :content-gap="16">
  <YourLongContent />
</NanaSettingsPage>
```

`contentPadding` 接受数值，或完整的 `{top,right,bottom,left}` 对象。单位是逻辑像素。`contentGap` 是数值。

负数按零处理。非法输入回落默认值。移除属性，或设为 `undefined`，就恢复默认。

它们作用于内部的滚动 body。动态更新不重建内容，也不重建滚动节点。full-page Tab 没有这个 body。布局仍由业务负责。

公共 CSS 不再在 SettingsPage 外壳上重复声明 padding 和 gap。

`NanaWorkspaceShell` 的 primary 也不再默认加页面 padding。直接放业务内容的旧页面，应在自己的内容根设置 `padding:20px 24px`。嵌套 SettingsPage 时，不再补这一层。

语义卡片的 class 和 tag 布局提示，不再覆盖 Runtime 的内边距或圆角默认值。

### 边距回归与截图复现

`crates/nana-ui-devtools/examples/spacing-layout.rs` 使用真实的 Runtime 和 UiScene 做离屏绘制。它检查 640×300 和 280×300 下的嵌套卡片、内容区清零和滚动尾部。

默认的 JS fixture 直接提交 host ops：

```bash
cargo run -p nana-ui-devtools --features agent-bin --example spacing-layout --locked
```

输出 PNG，以及布局和无障碍证据，到 `target/spacing-layout/`。你也可以把同目录的 `spacing-layout-vue.js` 用消费应用的 bundler 打成 IIFE，再传入产物路径，验证实际的 `NanaSettingsPage` 和 `NanaCard` wrappers：

```bash
cargo run -p nana-ui-devtools --features agent-bin --example spacing-layout --locked -- path/to/spacing-layout-vue.iife.js
```

此验收覆盖宿主机器上的离屏 GPU 绘制。它不代替 Windows 或 Linux 的原生窗口验收，也不代替 Surface 验收。
