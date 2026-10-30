# 布局（CSS 子集）

Rust 的第一路径用控件自己的布局。你不写 CSS。排行、列和边框见 [布局与样式（Rust）](rust-layout.md)。

这篇只描述 `nana-ui-css` 能映射的那一部分网页 CSS。`.vue` 方言的 `<style>`、`view!`、`stylesheet!` 与 `css!` 在构建期用它。你用它排界面骨架。

构建期编译的是 class 选择器（`.card`、`.card.active`）、其中的 `@container` 规则和 `transition`。其他选择器、`:hover` 等交互态、`@media`、`@keyframes` 以及 Style Model 没有字段的声明只给出构建警告，不进视图。下文写到的选择器和 at-rule 能力属于引擎本身，由 `nana-ui-css` 的单元测试覆盖。`tools/css-parity` 只用 inline `style` 和 class hints 对照浏览器校验盒模型，不跑样式表。

它不是浏览器里的 CSS 引擎。没有完整的选择器世界。对不上，就改布局。不靠容差假装一致。

对话框、抽屉、菜单用 [控件](components.md) 里的浮层。

## Layout intent authority

组件语义、作者 CSS/Rust 和运行时约束都进入同一份 layout intent。字段按
`nana_ui_core::LayoutOwnership` 标记为 component-required 或
component-default；默认值允许作者覆盖，required 值保留给组件。最终样式仍由
Runtime 写入口提交，`LayoutBox` 和滚动投影是输出，不是可写的前端属性。

containing block 传播以 layout mutation footprint 为 seed。静态帧和
paint-only 更新不再扫描整棵投影树；视口变化按约束轴传播（见下文「约束变化（#264）」），
结构变更仍从根节点重算，以保持百分比、Fill 和 intrinsic sizing 的正确性。

样式、主题与组件状态（#261）都按变化的 footprint 进入布局：
- 样式写入按**解析后**的布局分类，即作者写的布局再套上设计意图（圆角档位、控件高度、
  padding 档位、面板内边距）。只改意图的写入也会排出对应的 seed；解析结果不变的写入，
  不论写法是否相同，都算等价写入，不排 seed、不标脏。
- 主题只换调色板（Light↔Dark）时只重绘，零布局。主题度量变化时，只重新解析声明了设计意图
  的节点（有一份索引，不扫描整个文档），并且只给解析结果真的变了的节点按变化的字段排 seed：
  改 Button 的水平 padding 档位，只有用这个档位的按钮重新布局，输入框、卡片和列表行不测量；
  放在固定尺寸格子里的按钮，变化停在格子上。
- Button 的配方只改颜色角色，配方变化让按钮重新投影，但零布局。
- 悬停、按下、焦点这些组件状态只改绘制时，零布局。

计数都在 `WorkCounters` 上：
- `style_to_layout_seeds`：样式写入排出的 seed；
- `theme_to_layout_seeds` / `theme_metric_dependents_invalidated`：主题度量变化排出的 seed，
  以及解析结果变了的节点；
- `theme_palette_layout_invalidations`：没有改度量的主题安装排出的 seed，应当为 0；
- `component_state_layout_seeds`：悬停、按下、焦点、交互和无障碍状态变化排出的 seed；
- `equivalent_style_layout_skips`：等价的样式写入。

门禁在 `world/issue261.rs`：悬停零布局；调色板切换不整形、不测量、不放置；改 Button 的
padding 档位只排这些按钮的 seed；单个按钮改 padding 的开销在 1k 与 10k 控件下相同；
1000 次等价写入不排 seed；配方变化零布局；度量变化与只改意图的写入逐趟对照全量布局。
#259 也加了主题负载：1k、10k、100k 页面上切换调色板和度量都零布局。

还没做的：字体相关的继承属性（字号、字重、行高、书写方向）变化仍对子树做整体重排，
没有按依赖 em 单位的后代精确传播。

Intrinsic metrics 由 Runtime 的 `IntrinsicCache` 统一保存。它记录内容和样式
事实（min/max inline/block、preferred size、首尾 baseline、aspect ratio）以及
generation；当前 containing block 解析出的 used size 单独保留，不能回写这份
事实缓存。缓存 key 不含 Flex、Grid 或 IFC 的名称，因此相同的约束类可以跨
formatting context 复用；百分比、Fill、aspect ratio、字体、writing context 和
viewport 等依赖会进入约束或输入身份。内容、形状样式、边框、min/max、资源
metadata、scale/font 和子树 metrics 的变化才会 bump generation；颜色、opacity、
transform、hover 和 accessibility 更新不会 bump。

## LayoutNode、Participation 与 FormattingContext（#197）

布局把一个节点拆成三件事，互不替代：

- **身份**：`StableNodeId` 和它的 retained 记录。布局从不为了换算法创建、包装或销毁节点。
- **参与方式**（`ParticipationKind`）：这个节点在父级的 formatting context 里怎么被排。
  它由父级的 context、节点解析后的布局意图和节点自己的内容推出来，是一个视图，
  不是组件，也不另存一棵树。
- **建立的 context**（`FormattingContextKind`）：这个节点怎么排自己的子项。布局记下它
  实际跑的那一种：`Flow`（`display: block` 或不写）、`Flex`、`Grid`、`Inline`（在流
  子项里有行内级盒子）、`Layers`（`display: layers`，叠层）、`Overlay`（模态框按槽位放
  子项）。有文字的叶子为自己的文字建立
  `Inline`，字形归 nana-text，不变成布局节点；空叶子不建立 context。

measure 和 placement 用同一个函数选 context，两趟不会对同一个节点选出不同的算法。
算法本身没有变。context 只读 `resolved_layout`，也就是 layout intent authority 的输出；
它不再解析一次样式。

| 父级 context | 子项的参与方式 |
|---|---|
| `Flex` | `FlexItem` |
| `Grid` | `GridItem` |
| `Flow` | `FlowItem`；浮动的是 `Float`；被块拆开的行内元素是 `InlineSpan` |
| `Inline` | 行内文字是 `NativeText`，有子项的行内元素是 `InlineSpan`，`inline-block`、控件和替换内容是 `AtomicInline`，块级盒子是 `FlowItem`（断行） |
| `Layers` | `LayerItem` |
| `Overlay` | `OverlayItem` |

`Absolute` / `Fixed`、`display: contents`（`Contents`）和不生成盒子的节点（`NoBox`）
在任何 context 里都一样。文本、图片、按钮、容器和 custom render 都走这一张表：
它们的区别只在内容（`Children`、`Text`、`Replaced`、`Empty`），不在组件类型。同一个
文本节点在 `Inline` 里是 `NativeText`，在 flex 或 grid 里就是一个按内在尺寸排的 item。
节点可以任意嵌套：一个 grid 在 flex 父级里是 `FlexItem`，对自己的子项仍然是 `Grid`。

`UiWorld::layout_node(id)` 每次调用时从记录拼出这个视图：解析后的意图、内容、建立的
context、父级 context、参与方式、context generation 和结果 generation。父级还没布局时
参与方式是 `None`，不猜。`LayoutResult::formatting_context` 记下结果是在哪个 context 里
排出来的，子项 fragment 的种类也由它决定（`Flex` → `FlexChildPlacement`，
`Grid` 和 `Layers` → `GridChildPlacement`，`Inline` → `InlineAtomic`，其余 →
`ChildPlacement`）。不生成盒子的容器不建立 context：已经布局过的容器后来变成 `NoBox`，
它和里面的节点撤掉各自建立的 context，盒子收成原点处的空盒，与冷布局相同；这不算一次
context 切换，不计 `layout_context_transitions`，只推进 context generation。再显示时
整棵子树重新放置，重新记下 context（Issue #392，门禁在 `world/issue392.rs`）。

`Layers` 是叠层，和 SwiftUI `ZStack`、Compose `Box`、Flutter `Stack` 同义：在流子项共用
同一块区域，按 `justify-items` / `align-items`（子项的 `justify-self` / `align-self`
优先）对齐，默认起点；容器随最大的子项，脱流子项照常定位、不撑大容器；命中和绘制按文档
顺序，后写的在上面。它跑的是网格算法：一行一列 `auto` 轨，所有子项都放进这一格，子项自己
的网格位置和容器上的轨道声明都不起作用；measure 用随内容的轨道，所以容器随最大的子项，
placement 再把这一格撑满容器的内容盒，对齐因此以整个容器为参照。Row/Column 预设仍是
`Flex`，`Stack::layers()` / `Stack::overlay_layer()` 才是叠层，flex 与叠层不再共用一种
context。门禁在 `world/issue204.rs`（共用区域、随最大子项、九个锚点、RTL、`stretch`、
子项覆盖、后写的那层接住指针、与冷布局相同）和 `tests/view_macro.rs` 的
`issue204_layers_css_and_rust_share_one_overlay_context`；`world/issue199.rs` 的千组件
切换序列也经过 `Layers`。

context 切换（同一组子项 Flex → Grid → Inline）只换 context 记录、它的 generation、
子项的参与方式和布局结果；按样式指针比对的计划自然失效。子项的 id、组件、状态、焦点、
监听和挂载都不变，不生成也不销毁节点。每个记录到不同 context 的容器计一次
`layout_context_transitions`；静态帧、只改绘制的写入和等价写入都是 0。
DevTools `inspect` 的 `layout_node` 给出节点、意图摘要、内容、建立的 context、父级
context、参与方式，以及 metrics、context 和结果三个 generation。

门禁在 `world/issue197.rs`：同一组子项（带标签的 inline-block、行内文本、替换图片、
嵌套 grid）经过 Flex → Grid → Inline → Flex，每一步只记一次切换、没有节点生灭、
参与方式随父级变化，并与冷布局逐项相同；静态、只改绘制和等价写入不记切换。
`framework/tests.rs` 里的组件版本确认 TextInput 的内容、焦点和 Button 的处理函数都保留。

Flex、Grid、Inline、Overlay 的算法各在 `layout_engine/` 下自己的模块里（`flex.rs`、
`grid.rs`、`inline.rs`、`overlay.rs`），`Layers` 复用 `grid.rs`；`context.rs` 是两趟共用的
分派，模态框也在这里认出：带 `ModalFrame` 视觉的节点不论 `display` 都建立 `Overlay`，
再由 `overlay.rs` 按槽位契约放子项。门禁是 `world/issue199.rs` 的
`issue199_a_modal_frame_places_its_slots_as_overlay_items`（槽位是 `OverlayItem`，重排不记
context 切换，增量与冷布局相同；分派不认模态框时 placement 的断言失败）。同一容器在 Flex → Grid → Layers → Inline 之间来回切换 20 轮，保留缓存在第
一轮后不再变化，回到 Flex 时与只排过 Flex 的冷缓存相同（`issue199_repeated_context_switches_keep_the_retained_cache_flat`）。

锚定的浮层已经由布局放置，见下文「锚点定位」。`absolute` 的包含块按 CSS 解析，`fixed` 挂在自己文档的视口上，裁剪和滚动沿
包含块走的 clip 链作用，Scene、命中和无障碍读同一条链，见下文「定位」里的「clip 链」（#201）。

## 组件内容部件（#205）

控件不自己排内容。图标、指示框、标签、hint、spinner 这类没有身份的内容声明为轻量 Flex
parts（`layout_engine::parts`），用 retained Flex 的同一组求解函数分配主轴、收缩和对齐，
结果随节点的 `LayoutResult.parts` 一起发布。部件没有 `StableNodeId`，不产生 wrapper 节点。
一份部件声明同时供四处读：intrinsic measure 取除文字以外的宽度与间距（`chrome_extent`），
段落预算取可用宽度减去同一份（`text_budget`），placement 求解它们，`ComponentGeometry`
和 Scene 只读发布的盒子、附加绘制数据。文本片段（TextLine/TextRun）落在 `TextContent`
部件里。

部件可以嵌套方向：Switch 的一行是 track 与一个文字列，文字列再按 Flex 列解出标签和 hint，
两次求解用同一个 `flex_parts`。intrinsic measure 把文字能要的宽度先扣掉文字旁的部件
（图标、指示框、圆点），与段落预算相同，文字铺满时盒子不会因为这些部件超出可用宽度。部件读的主题度量没有出现在节点自己的样式里时（Switch 的
track 与间隔，Range 的间隔与 unit 字号，Progress 的标题行与间隔，StatusBadge / ValidationMessage 的圆点格宽与间隔，带图标的 SelectionOption 的图标间隔），主题改了这些度量，节点照样重新测量（`visual_parts_read_theme`）；裸 Toast 的部件读 Toast 配方，配方变了它重新测量。一个组件最多发布四个内容部件（`ContentParts` 的四格，Toast 恰好用满）。

已迁移：Button、Chip、ActionMenuItem、Checkbox、Switch、Range（label、value 槽、unit，#384）、Progress（label、cancel，#384）、ModalFrame（标题、说明、正文文字，#384）、StatusBadge、ValidationMessage、SelectionOption（竖排圆环、分段项、标签页）、EmptyState、Toast（裸提示的记号、标题、说明、关闭目标，装配后的记号）、MenuSurface 触发器（Popover / ActionMenu / HoverCard 的标签与图标）、ListItem/SidebarRow/FileTab 的文字部件、
LabeledValue、FormField、Card 标题、Select 触发器。交互、数据行、文本编辑几何（菜单行、
树行、编辑器）与图表、路径、画布不进部件。逐组件的分类和待迁移项见
[组件内容布局与迁移审计](component-layout.md)。

有子节点的组件把排在子节点前面的部件声明为前置部件（`parts::LeadingParts`，#385）：
`UiWorld::leading_parts` 给出一条沿组件自己的轴（`Column` 在子节点上方，`Row` 在行内起点：
LTR 在左边，RTL 在右边，`LeadingParts::end`，#388）的带子，长 `extent`，交叉方向至少 `breadth`。带子在 content box 的起点、padding 的
里面：测量把 `extent` 加在子节点前面、把 `breadth` 当作 content box 交叉方向的下限，并在交给
子节点的可用空间里扣掉 `extent`；placement 让子节点从带子之后开始；组件在 content box 起点
解出自己的部件。padding 只是盒子声明的那份（作者写的，或组件的默认值），发布的 used padding
不含部件，作者的 padding 与部件空间既不叠加也不互相顶替。前置部件随 `VisualContent` 进测量
计划的输入比较；grid 增量不重解带前置部件的盒子。已用：EmptyState 的图标、标题、说明（操作
按钮之前）、带标题 Card 的标题带（正文之前）、装配后 Toast 的记号与间隔（文案之前）。门禁是
`world/issue385.rs`：三者在作者 padding、主题切换、部件显隐下逐步与冷布局对照，并核对
content box 等于 padding box 减去声明的 padding、部件从 content box 起点开始、首个子节点在
带子之后。

文字被清空的盒子交出原来的度量：一个盒子的文字变成空串时，下一次文字工作把它解析成
「没有文字」，度量归零、布局重新量它，结果与冷布局相同。门禁是 `world/issue205.rs` 的
`issue205_a_cleared_label_lays_out_as_a_cold_unlabelled_control`。

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

样式写入只按这个节点自己的字段分类，不遍历文档，也不遍历它的后代。效果相同的写入
（同一个值，或者 `direction: None` 和 `Some(Column)` 这类同义写法）什么都不排队，
记在 `layout_equivalent_mutations_skipped`；分类结果为空的写入（包括只改颜色、
opacity、transform 的写入）记在 `layout_invalidations_zero_delta`；真正排进队列的
typed cause 记在 `layout_invalidations_created`。一个节点最多挂一个待处理的
`LayoutInvalidation`，同一帧的多次写入合并进这一格，不按写入历史增长。布局字段的
变更只隐藏这个节点和它到固定边框为止的祖先链上的结果；后代的结果留到 frontier
收录它们的那一趟再重新发布。

文本重新整形后，对外的宽、高和基线都没变，变化就停在文本上：不排 seed，不布局父级，
在文本 pass 里记一次 `layout_propagations_stopped`。文字属于一个固定尺寸的盒子本身时
（比如固定尺寸的按钮的标签），只要这个盒子不是行内级、父级也不按基线对齐它，度量变了
也只在盒子里重新布局：它导出的边框盒不变，父级不进入 measure。父级不是固定边框时，它按读到的
子项度量重新测量，但和导出路径上的其他祖先一样，不把自己的兄弟带进 frontier：它的盒子真变了，
后面的兄弟由容器的 placement 移动（顺序容器从它开始重放后缀），没变就都不动。长页面开头一个
标签的 frontier 不随它后面的兄弟增长。产品帧在布局之后对这趟的范围重新
整形；整形改了度量的文本自己排出 typed seed，下一趟只按这些 seed 走 frontier，不再把
整个范围当成全依赖的 seed 重排一遍。

子项列表变了的父级（插入、移动、detach、park、despawn）同样不把自己的兄弟带进 frontier：新来的子树
带着自己的拓扑原因整棵布局，父级的盒子真变了，后面的兄弟由容器的 placement 移动。整棵布局的子树
（新来的、显示或隐藏切换的、原因未知的）也不把块级流里后面的兄弟连进来，只有不向父级导出度量的隔离
盒子还连着它们。父级按新列表重新摆放子项时，内容盒若只是主轴长度变了，留下的子项只要样式没变、不读
主轴长度、内部也没有变化，就沿用上一份计划记下的尺寸，不再整棵重新测量。

顺序容器（每个子项的位置只取决于它前面的子项）连子项也不再逐个遍历：新旧子项列表共同的前缀和后缀
之间就是这次编辑。measure plan 里离开的子项减去自己那一份尺寸和一道间距，新来的子项测一次再加上；
placement plan 从编辑处重放后缀，编辑前的子项不动，编辑后的子项按离开和到来的差移动，plan 围绕编辑
重建。长页面末尾增删一个盒子，一帧只做这次编辑；开头增删时后面每个盒子各移动一次，这是真实输出，
没有子树为测量被整棵遍历。新来的子项会改变格式化上下文（行内级、浮动、定位、`display: contents`、
伸缩、`order`、auto margin、读主轴长度）、交叉轴随内容的容器里最宽的子项全部离开、流变空或从空开始
时，照旧重新测量和摆放子项。plan 修补出一个尺寸之后，容器保留的 intrinsic facts 量的是修补前的内容，
随这一趟退出保留缓存，之后别的约束要重新测量，不读旧的。

同一批节点里的多个 seed 先合成一张依赖图（seed 的并集闭包），每条边至多走两遍；这张
图的边数和邻接条目记在 `LayoutFrontierStats` 的 `graph_edges` 和 `scratch_entries`，
随闭包变化，不随文档变大。一个不再生成盒子的节点即使被容器写成了零盒子，它整棵子树
保留的盒子也一起收成零尺寸，和全量布局一致。

DevTools 的 `inspect` 回答一个节点为什么被布局：还没布局时是排队的 cause，布局之后
是上一趟 frontier 收录它时合并的 cause，并标明它是种子本身，还是被别的节点的依赖边
带进来的。`reason` 和 `changed_inputs` 说为什么被标记，`kind` 说跑哪几个阶段，
`affected_axes` 说 frontier 从它沿哪些依赖往外走。

Issue #255 点名的计数没有另设第二份：`layout_dependency_footprints_read` 就是
`layout_dependency_edges_visited`（每访问一条边读一个 footprint），
`layout_metric_deltas_none` 就是 `layout_propagations_stopped`，
`layout_metric_deltas_measure`、`layout_metric_deltas_placement` 和
`layout_metric_deltas_writing` 分别是 `layout_frontier_nodes_measure`、
`layout_frontier_nodes_placement` 和 `layout_frontier_nodes_writing`。

执行计数和 frontier 计数是同一份 `WorkCounters`。`layout_measure_nodes` 只计真正算出 used size 的节点；retained used-size 与 measure plan 的命中记在 `layout_measure_cache_hits`，未命中后重算记在 `layout_measure_cache_misses`。Intrinsic metrics 的命中仍是 `intrinsic_measure_cache_hits`，不另计一份。`layout_placement_nodes` 是这次 placement 写下的盒子，不是 frontier 成员数。`layout_origin_only_updates` 是尺寸没变、只改了原点的写入。`layout_result_reused` / `layout_result_changed` 来自结果发布：几何相同就留着原来的对象，几何变了才替换。`layout_delta_commits` 是真正写下变更结果的那一次发布。`layout_context_local_solves` 不单列，它就是已排进 frontier 的 `layout_frontier_contexts`。

Issue #259 点名的其余计数也在这份 `WorkCounters` 上，不另设权威。
`layout_measure_requests`、`layout_measure_hits` / `layout_measure_misses` 和
`layout_full_subtree_measures` 是 #198 的 `intrinsic_measure_requests`、
`intrinsic_measure_cache_hits` / `intrinsic_measure_cache_misses` 和
`intrinsic_measure_full_subtrees`；最后一个就是真正走了一遍子项的测量。
计划命中分两种：`layout_placement_plans_reused` 是容器按保留的 placement 计划放下子项
（只移动了的容器带着计划平移，也记在这里），
`layout_measure_plans_reused` 是按保留的 measure 计划得出尺寸，两者都不遍历子项。
`layout_plan_misses` 是手里有对应输入的计划、却仍然遍历了子项的容器。
`layout_plan_queries` 不单列，它等于两种命中加未命中。
`layout_plan_rebuilds` 是遍历后在同一容器、同一约束的旧计划上重新记下的计划；
容器的第一份计划不算重建。`layout_suffixes_replayed` 是只重放变化子项之后那一段的顺序容器，
`layout_children_measured` 是容器遍历时读过尺寸的子项，`layout_containers_uncacheable`
是放下了子项却记不下计划的容器，`layout_retain_sweeps` 是清掉已销毁 id 的保留缓存清扫。
定位上下文因计划过期而整个重排，记进 `layout_local_subtree_fallbacks`。
measure 计划沿用的前提是节点自己的输入都没变：约束、样式、子项列表、文本度量，以及标准视觉画在文字旁边的部分
（按钮的图标、勾选框的方框）。后者只在视觉里，只给按钮加上或去掉图标时样式和文字都没变，计划照样失效、重新测量。
`layout_scratch_allocations` 就是 `layout_scratch_entries`：这一帧各趟依赖图分配的邻接条目，
趟结束即释放；`layout_scratch_bytes` 是这些条目占的字节。
`plan_stats` 只做 benchmark 的阶段计时，不计数。
四个基准（`nana-runtime-benchmark`、`nana-framework-benchmark`、`nana-dirty-frame-benchmark`、
`nana-card-update-benchmark`）输出的计数和 `perf/contract_support/schema.py` 的 `WORK_COUNTER_KEYS`
同名，带上了上面这些计划、scratch、suffix、子项、intrinsic 测量、约束传播和 resize 文字计数，
报告里读到的就是门禁读的那份 `WorkCounters`。

增量结果靠两层冷对照校验。第一层是 `layout_engine::verify`：测试里，以及打开
`layout-verify` feature 时，每一趟保留布局之后都从头做一次全量布局，逐个比对盒子。
第二层是测试用的 `world::reflow_oracle`，它比对发布出去的结果：同一组输入，在一个从没跑过
增量帧的世界里布局一次，再逐节点比较以下各项：
- 盒子；
- `LayoutResult::geometry_eq` 覆盖的全部几何：边框盒、内边距盒、内容盒、基线、fragments、
  overflow、scroll extent、containing block 等；
- 命中几何；
- 无障碍边界。

增量帧之间留下的状态（整形结果、计划、保留尺寸）只存在于增量那一边，冷的世界没有，
所以这些状态出的错藏不住。#256 和 #259 的门禁都以这一层收尾。

`world/issue259.rs` 是 #259 的规模矩阵，覆盖这些维度：
- 文档规模：1k、10k、100k 节点；
- 同一帧的编辑：1、8、100 处；
- 编辑位置：头、中、尾；
- 嵌套深度：浅层，或 8 层包裹；
- 书写方向：LTR、RTL、vertical-rl。

场景有九种：只改绘制、固定边界内的内容、顺序尺寸变化、父约束、flex 重新分配、grid 贡献、
行内重排、语言作用域（#260，见 [文本引擎](./text-engine.md) 的「语言作用域与外部度量」）、
字号缩放作用域（#266，见 [文本引擎](./text-engine.md) 的「字号缩放作用域」）。
有 2000 段文字的作用域放在 100k 节点文档里的规模门禁，在 `world/issue266.rs`。

门禁只读计数，不看时间：
- 只改绘制时，所有布局计数为零。
- 局部编辑的 frontier、走过的边、放下的盒子、遍历的子项、整棵测量的子树数和结果发布
  走过的子项，在 1k、10k、100k 下完全相同。
- 顺序编辑放下的盒子不少于真正移动的盒子，也不超过移动的盒子加上通往编辑处的路径；
  整棵测量的子树数不随文档增长。在增长不推动后面任何东西的那一端（正向块轴的尾、
  vertical-rl 反向块轴的头），一帧的全部工作都是常数。重放后缀的只有被编辑的容器，
  个数不随文档增长；未命中和重新记下的计划（`layout_plan_misses`、`layout_plan_rebuilds`）
  也只来自被编辑的容器，在 1k 和 10k 下相同，不超过真正移动的盒子数：盒子按页面坐标放置，
  移动的组要把子项重新写一遍，但带着计划平移（见下文「只移动了的容器」）；记不下计划的行内
  格式化上下文移动时仍要遍历自己的子项。编辑之前的部分一份也不重建。
- 页头一列 1200 px 宽，里面只包住自己文字的标签换成更长、仍放得下一行的文字：一帧只排一次布局，
  开销和不能换行的标签相同，标签下面的盒子都不动，1k 和 10k 相同（文本侧见 [文本引擎](./text-engine.md)
  的「省略号」一节）。
- 同样的标签，页面的卡片组直接跟在它那一列后面，是这一列的兄弟：放得下一行的更长文字（能不能
  换行都一样）不改列高，一帧建的图、走的边、放下的盒子在 1k 和 10k 下完全相同，只有标签移动；
  在 300 px 宽的列里折行的文字让列变高，后面每一组都只下移一次，这是真实输出，随页面增长，
  找到它们的 frontier、走过的边和整棵测量的子树数仍然相同。下移的组带着自己的卡片平移（见下文
  「只移动了的容器」）：遍历的子项、未命中和重建的计划在 1k 和 10k 下也相同，查过的尺寸不随页面增长。
- 同样的页面，往那一列里、或在它前面的页面开头插入一个固定盒子再删掉：高度随内容的列和页面变高又缩回，
  新盒子后面的每个盒子各移动一次，都不进 frontier，找到它们的 frontier、走过的边、建的图和整棵测量的
  子树数，以及移动它们时遍历的子项、未命中和重建的计划，在 1k 和 10k 下相同；列是固定高度时，一帧的
  全部开销在 1k 和 10k 下相同。
- 同样的页面，页面自己的子项列表变了：页面开头或末尾的固定盒子离开（开头那次就是从零布局之后的第一帧），
  再来一个：没有子树为测量被整棵遍历，页面自己也不；移动的组带着卡片平移，不为放置遍历子项。frontier、
  走过的边、建的图、测量的节点数、遍历的子项和未命中、重建的计划在 1k 和 10k 下相同；在末尾时别的盒子
  都不动，一帧的全部开销在 1k 和 10k 下相同，页面自己的结果除外（它为每一组记一份放置）。
- 100 处编辑走同一张并集闭包，每条边至多两遍，任何一项都不超过 100 次单独编辑之和。
- 100k 节点上连续 1 万次编辑之后，以下各项满足：
  - 保留缓存的盒子、placement、两类计划和计划条目数与开始时相同；
  - 产品帧不保留全量布局副本；
  - 保留的 used size 每个节点至多两个变体，intrinsic facts 至多四个；
  - 没有哪一帧的 scratch 超过一次编辑的闭包。

规模在 10k 及以上的页面跳过每趟守卫（`skip_layout_verify`），最后以冷对照收尾；
1k 页面每一趟都校验。1k 与 10k 的全矩阵跑 1、8 处编辑；10k 与 100k 的局部编辑和安静端的
顺序编辑跑 1、100 处，浅层和 8 层嵌套、三种书写方向各一遍
（`issue259_contained_edits_cost_the_same_at_100k_{ltr,rtl,vertical_rl}`）。局部编辑放在
安静端的另一端：10k 的嵌套页只有 263 组，放在中间的 100 组会和安静端的 100 组重叠，
先热了那些组的保留状态，10k 页就比全新的页少做一些，100k 页不会重叠。

下面几处做法让局部编辑的工作不随文档增长。

反向主轴（vertical-rl 的块轴、RTL 的行内轴）。原点从远端量起：容器的主轴尺寸变了，
远端移动多少，变化处之前的子项就一起平移多少。只移动真正移动了的盒子，什么都不重新测量；
`ContainerPlan::placed_main` 记着这些原点是按哪个主轴长度放的。

只移动了的容器。前面的兄弟变了，后面的容器只是被推开：它的 placement 计划除原点外都成立（放回原来的
位置照样能沿用），它的子项都不在 frontier 上、也不通向 frontier 上的节点，这一趟也还没放过它。这时它不
逐个查子项的尺寸，也不重建计划：每个子项放在新的内容原点加上计划记下的、它在内容盒里的偏移，在原来的
包含块里（grid 的子项是它的单元格）；子项有自己的计划时同样平移下去。绝对定位的子项以这个容器为包含块时
跟着移动；包含块是更上层的祖先或初始包含块时，只随那个包含块动，按它现在的盒子决定要不要重放。固定定位的
子项挂在视口上，不动。整棵子树的盒子都要重新写一遍，这是真实输出，计划只换掉原点。偏移就是
放置时加到内容原点上的那个值，所以平移后的位置和从头布局逐位相同。被反向主轴的远端或换行 flex 前面
一行按差值挪过的子项没有精确偏移，subgrid 的子项要读继承的轨道，这样的容器移动时照旧重新放置子项。

保留的 intrinsic facts。预算随活树变化（每个节点四个约束变体），并且按内容分组存放：
一个节点的事实变了，只替换它自己的几个变体；删除一个节点也只删它的那组，都不扫整张表。

保留的 used size。一个节点在一趟里可能按好几个约束被测量，但只留两个变体：按这一趟测量的
先后写回，留下最后测到的两个，计数是确定的。measure plan 同样按约束最多留两份；某一趟里容器的子项变了，
却只按其中一个约束测量了它（比如视口宽度来回缩放，中间改了一个子项），另一份没见到这次变化，就标记为
过期：之后再查到它时重新遍历子项、原地重建，不再直接沿用。

结果发布。一个已发布的结果，只要还在，就是在它的子项列表最后一次被编辑之后建的：插入、
移动、detach、park、despawn 都会先丢掉父节点的结果，再往上丢到第一个固定边框为止（父节点自己是
固定边框就停在它）。所以检查祖先的结果是否仍然成立时，
只需要核对这次发布覆盖到的子项（通过 `index_in_parent` 直接定位），不用把宽容器的每个子项
都比一遍。结果的来源（布局写回时先按兼容写入发布、随后按布局再发布一次）不算几何变化：
几何不变的结果保留原对象和原来的来源。`layout_result_children_visited` 记下发布核对或重建时
走过的子项；局部编辑下它不随文档增长，只有自己的子项放置真的变了的容器，才为自己的子项数
付出一次重建。

计时门槛：`issue259_local_frame_time_scales_flat_to_100k` 在固定机器上记录局部编辑一帧的
p50、p95、p99，并检查 p95 从 1k 到 100k 的增长不超过 1.75 倍：

```bash
cargo test --release -p nana-ui-runtime --lib issue259_local_frame_time -- --ignored --nocapture
```

PR CI 只以计数为硬门禁，计时不在 CI 里判定。每周的 Runtime performance 工作流以
`continue-on-error` 跑这一项，p50 / p95 / p99 写进 job summary：托管的 runner 不是固定机器，
这一步只做记录，不挡合并。

约束变化（#264）。视口缩放、容器改尺寸、拖动分隔条、Dock 改面板比例，都走同一条约束传播：
- 样式写入按轴分类。宽度及其上下限只改变子项的行内约束，高度及其上下限只改变块约束；
  flex 的 grow、shrink、basis 只改变父级那一行所沿的轴；box-sizing、aspect-ratio、间距、
  对齐和流的形状两轴都算。
- 视口缩放给文档根按变化的轴播种，往下只有消费这条轴约束的子项重新测量。读视口的盒子
  按它读的那一侧播种（`LayoutStyle::viewport_axes`）：`vw` 读宽，`vh` 读高，`vmin` / `vmax`
  和 `position: fixed` 两侧都读；只改高度不碰 `vw` 盒子，只改宽度不碰 `vh` 盒子。引擎记着
  每个文档上次布局用的视口，不论从哪个入口进来，视口一变都由引擎补上这些 seed。
- 测量缓存的键和布局计划只比较 `viewport_basis`：盒子读的那一侧视口，不读的一侧记为 0。
  不读视口的盒子不因视口变化失效，`vw` 盒子不因只改高度失效。
- 某一轴尺寸是自己的确定长度、不读包含块的盒子，测量缓存的键里不放这一轴的可用尺寸。
  父级变了，它直接命中缓存，不再遍历子项。
- 只因位置变化被触及的兄弟，不再把整棵子树拉进 frontier。它在新原点重新放置时子树跟着放，
  原点没变时子树保持原样。
- 主轴尺寸变了，读取容器主轴尺寸的子项（主轴方向的百分比、fill、basis）不再沿用旧计划。
- 一个子项沿父级的线（行的行内轴、列的块轴）改了尺寸，线上可伸缩的兄弟（`flex_grow > 0`
  或这一轴是 fill）分到的尺寸也跟着变：它们也记作约束 seed，往下只有读这一轴的子项重新
  测量。分隔条的第一个面板变宽，旁边 fill 的面板变窄，两边的子项都被问到。容器不论
  `display` 都沿 `direction` 排成一条线（默认列）；grid、行内格式化上下文和竖排不走这一条，
  照旧由按可用尺寸为键的测量缓存兜住。

计数都在 `WorkCounters` 上：
- `constraint_change_seeds`：改变了子项约束的 seed；
- `constraint_dependents_considered` / `_remeasured` / `_skipped`：被问到是否消费这次变化的
  子项，以及其中重新测量的和保持原测量的；
- `resize_text_relayouts` / `resize_text_reshapes`：布局之后那一趟按新盒子重新排版的文字，
  以及其中需要重新整形的（只有换行宽度变时为 0）；
- `resize_context_solves`：约束变化触发的那一趟里，从头求解的格式化上下文，即计划无法复用
  或没有计划的容器。

门禁在 `world/issue264.rs`。"测量"读的是布局真正算过的节点（测试用的
`layout_engine::measure_trace`，越过本趟 memo 和保留 intrinsic 之后才记），不是 frontier 的
MEASURE 位：memo 的键变了也会重测，frontier 看不到这种工作。layout-verify 守卫自己那趟全量
布局不记。
- 10k 节点的工作区里拖动真实的 `SplitPane` 240 次。固定盒子和 Dock 不测量，文字不重新整形，
  节点不增不减，最后与冷布局一致；1k 工作区逐趟对照全量布局。每次拖动有两个约束 seed：
  被拖的面板和旁边 fill 的面板。
- 只改宽度时，只读高度的盒子不测量；只改高度时，只读宽度的行不测量。
- 固定尺寸的子树和它的后代不测量。
- 拖动一次的开销在 1k 与 100k 节点下相同，问到的子项也相同。
- 视口缩放的开销在 1k 与 100k 节点下相同，Dock 不进布局；`vw` 盒子只在宽度变时测量，
  `vh` 盒子只在高度变时测量。
- 像 Dock 那样改 flex 比例时，只读高度的盒子不测量；拖动真实的 `Dock` 分栏比例也一样，
  读宽度的段落每次都重新测量。

高度随内容的盒子，子树里没有东西读给它的块向可用尺寸时，测量缓存的键不放这一轴
（`world/block_reads.rs`）。读这一轴的有：自身或子孙的百分比、fill 高度及其上下限，
跟随包含块的 flex basis，按可用高度换列的列向 wrap，任何 grid，aspect-ratio，逻辑盒边，
声明了竖排。每个节点记着自己的样式算不算读、在父级的测量里算什么角色（照常、自带高度截断、
没有自己的盒子而透传、不参与），以及有几个子级把读向上传；改样式重新分类一个节点，改子项
列表增减一个计数，只有某个祖先的结论变了才继续往上走，与文档规模无关，park 的子树保留结论。
layout-verify 守卫的参照布局不用这条规则，按真实高度测量，谓词漏掉的读法会表现为盒子不一致；
守卫同时从零重算聚合比对。

同一份聚合也修好了一处遗漏：子孙的百分比高度按引擎传下去的可用高度解析，而前沿和容器计划
原先按 CSS 的语义当成 auto，只改高度时这样的行停在旧高度。现在子树会把读向上传的子级，前沿把
块向约束传给它，列容器的计划也把它记作依赖主轴尺寸。

#259 加了缩放风暴：在 1k、10k、100k 的页面上连续缩放视口 240 次，开销相同；vertical-rl
页面逐趟对照全量布局。

Gate E（可用尺寸仍落在缓存的包络段里时不重排）见下文「Dynamic Layout」：第一个面板里一条
求解溢出的工具栏，在同一个代价档里拖动时不建包络、不从头求解，每帧只重分保留的那一档。

替换内容（#263）。图片、视频、HostTexture、CustomRender 的三种修订分开处理：
- 内容修订（像素、视频帧、纹理代际）和 fit、采样只重绘，零布局，也不推进 intrinsic generation。
- 只有固有元数据进入布局：资源上报的自然尺寸。图片解码出的尺寸、视频或纹理的分辨率，用
  `MutationQueue::set_replaced_metadata` 上报。
- 每个资源记着显示它的节点（资源到节点的索引），上报只通知这些节点，不扫描文档。其中盒子
  读取自然尺寸的（宽或高是 auto）才重新布局；宽高都由样式决定的，只重绘。
- 宽高都没写的盒子用自然尺寸；只写了一边的，另一边按自然宽高比换算。换算在每条求得
  used size 的路径上都做，包括跨 context 复用的 intrinsic facts（#202）。
- 自然宽高比和样式里的 `aspect-ratio` 一样：flex 行与 grid 的 `stretch` 不把只写了宽的替换盒
  拉到行高或区域高，高度仍由宽度按比例得出。
- min/max 不扭曲自然宽高比（#202，CSS 2.1 §10.4）。宽高都没写时，自然尺寸先按宽的上下限、再按高的
  上下限等比缩放，两边的限制互相冲突时各自夹住、比例让步；只写了一边时，另一边按比例得出后再受它
  自己的 min/max 约束。120×80 的图片 `max-width: 60px` 得 60×40，`min-width: 240px; max-height: 40px`
  得 240×40。
- 节点变成或不再是替换内容时，它对齐用的基线跟着变，也重新布局。
- Markdown 里的图片解析出尺寸后，块列表变了，文档的新高度会进入布局。
- 宽度由自己决定的容器，不再因为子项在交叉轴上变宽而退回遍历全部子项。

计数都在 `WorkCounters` 上：
- `replaced_content_updates`：只换了内容、没换资源的更新；
- `replaced_intrinsic_metadata_updates`：上报了不同自然尺寸的资源；
- `replaced_layout_seeds`：替换内容排出的布局 seed；
- `replaced_content_only_layout_invalidations`：只换内容的更新排出的 seed，应当为 0；
- `resource_intrinsic_dependents_notified`：经资源索引通知到的节点。

门禁在 `world/issue263.rs`：
- 1200 帧视频零布局，intrinsic generation 不变；
- 图片自然尺寸到达，只重排读取它的那个盒子；固定 300×200 的只重绘；宽 400 的按比例得出高度；
- 一张共享图片经索引通知 100 个节点，在 10k 与 100k 文档里开销相同，只有读尺寸的 50 个重新布局；
- fit 与采样变化零布局；
- 显式尺寸下视频分辨率连变 100 次，零回流；
- 每一趟对照全量布局，结尾对照冷布局。

替换内容是一种布局项（#202）。图片（`content_image`）、`Video`、`GpuTextureView`（HostTexture）、
`GpuView`（GPU 画布 / 渲染目标）和挂在普通元素上的 `CustomRenderNode`（呈现锚点）都按显示的资源
（`ReplacedResource`）进同一个索引，`LayoutNode::content` 都是 `Replaced`，在 Flex、Grid、行内与
absolute 里的 participation 和普通盒子一样，同一个自然尺寸给出同一个盒子。没有按组件分的尺寸路径。
组件的默认样式各有不同：`Video`、`GpuTextureView`、`GpuView` 没写宽高时铺满（`fill`），写 `auto`
才读自然尺寸；图片没写宽高、尺寸未到时没有面积。

fit 只有一份数学：`ContentFit::fitted_size`（`nana-ui-core`）。fit 是绘制，只读布局给的盒子，不回写
布局。
- `CustomRenderNode.fit`（视频、HostTexture、Avatar、Thumbnail）由画家用 `LogicalRect::fitted` 求目标
  矩形，`GpuTextureView::texture_uv_at` 与 `Thumbnail::picture_rect` 用同一函数换算坐标。
- 图片的 `object-fit` 写在 `PaintStyle::object_fit` 和 `content_image` 的 `BackgroundImageFit` 上。画家
  `url()` 路径经 `BackgroundImageFit::object_fit` 换成 `ContentFit` 再求尺寸；`background-size` 的
  关键字同样走这里，只有写长度的 `background-size`（`Length`）是背景自己的规则。图片另外认
  `object-position`，HostTexture 居中。
- `ImageViewer` 的初始画面是 `scale-down`。
- 标记里 `<video>` 与 `<gpu>` 的 fit：层叠出的 `object-fit` 优先，`fit` 属性是 presentational hint，
  两者都认全部五个关键字（原先属性只认 `contain`，其余都当 `fill`）。

`world/issue202.rs` 的门禁：
- A：同一对图片（无尺寸、只写宽）放进 Flex、拉伸的 Flex 行、Grid、拉伸的 Grid、行内与 absolute，
  自然尺寸到达后逐一对照尺寸、participation 和冷布局。
- B：图片和 CustomRender 在 `align-items: baseline` 的 flex 行与原生段落里以底边落在文字基线上。
- C：上面五种替换内容各按应用的写法建出，在六种 context 里 content 都是 `Replaced`、participation
  相同，同一个自然尺寸到达后盒子相同，5 个 replaced seed、0 全文档回退，对照冷布局。
- D：min/max 与自然宽高比的八种组合在 Flex、Grid、行内、absolute 下的尺寸，对照冷布局。
- E：同一尺寸再报一次不产生任何工作；换成同尺寸的另一个资源 0 seed、0 measure；换成另一尺寸的
  资源只重排读它的两个盒子。
- F：`Video` 与 `GpuTextureView` 从标记得到的 fit 与图片的 `object-fit` 是同一个值。
- G：普通块级流里同一行的原子按基线对齐：没有行盒的 inline-block、`overflow` 非 visible 的
  inline-block 和图片以底外边距边落在基线上，带文字的 inline-block 以末行基线落在基线上，块的
  高度就是行盒从最高顶到最低底的跨度。换一个更高的原子、同尺寸盒子里文字折成多行、原本空的原子
  里来了文字，每一步都重新摆放这一行，并与冷布局逐项相同。
- H：行盒都在后代里的 inline-block 以末个后代行盒的基线落在行的基线上：标签在空块下面的固定盒子、
  块里折成多行的标签（取第二行，不取首行）、两个各占一行的 inline-block 标签（取第二行）、固定
  尺寸块里的标签。后代文字折行、上方的块变高、固定尺寸块里的文字折行都重新摆放这一行；行数不变的
  文字改动不移动任何盒子。每一步与冷布局逐项相同。
- I：普通块级流的行盒从块的 strut 起算：`line-height` 等于字号时空 inline-block 下面留出字体的
  descent，默认行高再加半个 leading；很矮的原子顶不高于 strut 的 ascent；块级子项独占的行没有
  strut；原生段落的行同样在原子下面留出空隙。

`nana-ui` 的 `scene_paint/replaced_context_tests.rs` 另有两项：host texture 跨 context 移动不重新绑定
纹理；五种 fit 下 `url()` 路径与 host-texture 路径给出相同的目标矩形。

基线：替换内容按底边对齐（`aligns_by_bottom_edge`），由布局决定，不由画家修正 y。行内格式化上下文
（原生段落 `InlineFlow` 的行盒，和普通块级流里 inline-block 原子同行的行盒）都按 CSS 默认的
`vertical-align: baseline`（CSS 2.1 §10.8.1）对齐，两者读同一个函数
`LayoutInputMap::atom_line_metrics`：
- inline-block 以末个流内行盒的基线落在行的基线上，基线值取 `baseline_metrics` 的末行基线。
  行盒都在后代里的 inline-block（`UiWorld::line_boxes_are_descendants`：没有自己的文字、控件或
  段落，只在流里有）取持有末个行盒的后代所在的位置（CSS 2.1 §10.8.1）：
  `inline::descendant_baselines` 按 placement 的方式从子项在它内容盒里量出的尺寸排它的流
  ——块级流逐个叠放，行内格式化上下文用 `pack_ifc_line_boxes` 排行、按 `ifc_line_metrics` 定
  行的基线——末个行盒要么是这样一行，要么是独占一行的块，后者再取它自己的末行基线（行盒也在后代里
  就递归）。同一趟推导也正着找首个行盒，得出首个基线（#382）。每趟按节点和所用尺寸记在 `LayoutInputMap::derived_baselines`，读原子 ascent 的每一处
  （块的测量与 placement、measure plan 的比对与增量、原生段落）先 `prime_atom_baselines`，
  `baseline_metrics` 再读它，所以只有这一份推导。块级流按 placement 的方式定子项的位置：
  `justify-content` 把空余放在前后或中间（`justify_offsets`），反向的流从底部往上叠，每个子项的
  margin 也随之反过来；换行的块级流按 `pack_wrap_lines` 分成并排的几列，每列各自这样叠放；找首末行盒
  的顺序仍是流的顺序（门禁 `world/issue382.rs` 的 E、G）。行旁有浮动的
  行内格式化上下文和 placement 一样先打包浮动（`pack_floated_children`），行排在浮动留下的空间里，
  放不下的原子落到浮动下面（门禁 F，css-parity `T-FL07`，Chrome 实测）。flex / grid 容器、按等宽分行
  （`equal_wrap`）、有自动 margin 或浮动的块级流不推导，仍是自身字体首行的近似；
- 替换内容、`overflow` 非 visible 的 inline-block、没有流内行盒的 inline-block（`UiWorld::atom_sits_on_line_box`）
  以**底外边距边**落在基线上，`margin-bottom` 把它抬高；
- 行盒高度是对齐之后从最高的顶外边距边到最低的底外边距边的跨度，块级子项独占一行、从行顶排起。
- 普通块级流里有行内级子项的行盒从块的 **strut** 起算（`inline::line_strut`）：一个零宽、用块
  自己的字面、字号和 `line-height` 的行内盒，ascent 和 descent 是块自己字面的真实度量（#382），
  `line-height` 与两者之和的差一半加在上、一半加在下。所以以底边落在基线上的空 inline-block 下面
  留出字面的 descent 加半个 leading，很矮的原子顶不高于 strut 的 ascent。块级子项独占的行没有
  strut。`line-height` 读块自己的 `LayoutStyle`（CSS 路径由 `inherit_typography_from` 继承下来），
  不读 `ComputedStyle`。
  - 度量和原生段落（`nana-text` 的 `LineStrut`）是同一份：文字阶段替块问 shaper
    （`TextShaper::strut_metrics`，`NanaTextEngineShaper` 读 `NativeTextEngine::strut_metrics`），
    按块字号的每 px 存在 `UiWorld::block_strut`，布局乘上字号读它。所以同字面、同行高下，普通块级流
    和原生段落在空原子下面留的空隙相同。产品 UI 字面 Noto Sans SC 的 ascent / descent 是
    1.16em / 0.288em，16px、行高 1.2 时 strut 的 descent 约 2.6px（Chrome 实测见 css-parity
    `T-I04`、`T-I06`、`T-I07`）。
  - 只有要它的块持有 strut（`UiWorld::wants_block_strut`）：不是 flex / grid 容器、没有自己的
    行盒、子项里有流内的行内级盒子。shaper 不报度量时（默认实现返回 `None`），退回共享近似
    `TEXT_APPROX_ASCENT_EM`（0.8em），descent 是 em 的其余 0.2em。
  - 失效：块的字体族、字重、斜体、可变轴变化，子项的 display、position、float 变化，子项增删，都把
    块（和变化子项的父级）记为候选；字体集或语言提示变化（`TextBackendEpoch`）重读每个持有者。
    文字阶段只重读候选，值变了才按文字度量变化的路径排布局；只改字号不重读（存的是每 px 的值），
    语言不参与首选字面的选择，也不重读出新值。IFC 的 measure plan 记下它用的 strut 并比对。
    测试与 `layout-verify` 构建里，每趟文字阶段后把每个保留的 strut 和重读的比一遍
    （`UiWorld::audit_block_struts`），漏掉的候选直接报错。门禁在 `world/issue382.rs` 的 B。

`align-items: baseline` 的 flex 行按项目的首个基线对齐，替换内容取边框盒底边（CSS Align 在 flex / grid
里从边框盒合成基线）。行盒都在后代里的项目（`UiWorld::aligns_by_first_baseline` 且
`line_boxes_are_descendants`）取首个流内行盒所在的位置，和 inline-block 的末行基线是同一份推导
（`inline::descendant_baselines`）：测量发布 intrinsic facts 前、placement 读行的基线前、flex 计划比对
基线前都先 `prime_first_baselines`，`baseline_metrics` 的首个基线再读它；测试与 `layout-verify` 构建里
没预备就读会直接报错。这样的项目也读内容（`atom_baseline_reads_content`），固定尺寸不再是度量边界，
里面后代的字号、文字、上方块的高度变化会让这一行重新对齐；首行没动的变化不移动任何项目。门禁在
`world/issue382.rs` 的 D。grid 不做基线对齐，`baseline` 当 `start`。行的基线发布在 `LayoutResult` 的 TextLine 片段上。`LayoutStyle` 没有
`vertical-align`，默认的 `baseline` 就是全部行为。这不是 replaced 专有的规则。

原子的基线随内容移动，失效也跟着走：`overflow` 为 visible 的固定尺寸 inline-block 在普通块级流里
不再是度量边界（`UiWorld::atom_baseline_reads_content`），它里面的变化会让这一行重新对齐；
这样的原子流里的流内块（再往下也一样）是找末个行盒的地方，固定尺寸也不是边界，frontier 也不在它
那里停（`has_definite_size` 读同一个函数）。`overflow` 非 visible 的和替换内容的位置只看自己的盒子，
仍然是边界，里面的变化不重排这一行。IFC 的 measure plan 记下每个原子在行上的 ascent，盒子尺寸没变
但 ascent 变了时重新测量这个块；ascent 没变（后代文字改了但末行没动）时沿用计划，盒子都不动。

#259 加了替换内容负载：在 1k、10k、100k 页面末尾，视频帧、图片自然尺寸、共享图片的开销相同。

宿主把图片解码出的尺寸报上来。一张 `url(...)` 图片多大，只有画家知道：
- 没写尺寸的 `<img>` 在尺寸到达前没有面积，不会被画。画家对落在裁剪区里、没有面积的替换盒子
  照样加载它的图片，只是不画。
- 画家全新 prepare 一个绘制目标时，记下它画的每个带内容图片的 quad 的自然尺寸（解码完才有）。
  quad 开始显示一张图、或图片解码出另一个尺寸时交出一次；画进没有面积的盒子时每次都交出，这个
  盒子可能正等着这个尺寸。不再画的 quad 就忘掉。画家只写自己的记录，不碰文档。
- 内建宿主在窗口 present 之后取这个目标的记录（`SceneWgpuPainter::take_image_natural_sizes`），
  经 `commit_image_natural_sizes` 以 `set_replaced_metadata(ReplacedResource::Url(..), ..)` 提交给
  这个窗口的文档，再请求下一帧。读取自然尺寸的盒子在下一帧重新布局。
- 只提交文档里有节点显示（`UiWorld::shows_replaced`）、文档还没有这个尺寸的图片。Markdown 和
  Painter 自己画的图片不进文档。
- 远程图片按文档的 fetch host 分桶加载，尺寸只交给画它的那个目标的文档，一个文档的策略放行的
  图片不会把尺寸透给另一个文档。窗口隐藏、只有输出在画时，取输出目标的记录。

宿主侧的门禁在 `nana-ui` 的 `scene_paint/natural_size_tests.rs`：data URL 和本地 HTTP 图片各一例。
没写尺寸的盒子先没有面积；图片就绪、宿主提交后按解码尺寸布局并画出；之后不再上报。

`GpuTextureView` 的 HostTexture 不上报。这种纹理默认按 `painted_extent` 准备，那是布局定下的绘制
尺寸，把它当成自然尺寸，auto 尺寸的节点和纹理会互相放大。宿主自己生产内容的纹理，需要时自己用
`set_replaced_metadata` 上报。

虚拟列表（#262）。行高变化按差值移动列表，不扫描逻辑集合：
- 行高索引 `VirtualListLayout` 按 512 行分块。同一高度的一段行存成一个计数：一百万行估计高度
  只占约两千个块。两棵 Fenwick 树（块的总高、块的行数）回答前缀和偏移查询：O(log C)，再加
  一个块内的一段。
- 量出一行的新高度：只改它所在的块和 O(log C) 个索引项。一个块第一次不再等高时，它的 512 行
  展开一次。插入行只动所在的块，块拆分时才重建块索引；删除行每次都重建块索引。重建是 O(C)。
- 树 `VirtualTreeLayout` 的每行后代数和深度也分块存。展开、折叠时按深度往上找祖先，整块都比
  目标深的直接跳过，不逐行扫描父节点前面的行。
- 按内容量高的列表（`VirtualListItems::measured`）只读挂着的行的高度。数据没变（fingerprint
  和行数都没变）时，挂着的行还在上次放置的下标上，不再按 key 查。窗口还是那些下标时（量高只
  让行移动了），窗口的 key 也不再查。
- 一行变高：下面挂着的行只改 `top`，做原点平移；列表总高变一次。定位容器的测量计划比较样式时
  不看 inset，只改了 `top` 的行复用测量，不再整棵重测。
- `each_virtual(..).measured()` 每次布局之后都重读行高。行内容后来变高（展开、换行）时，下面的
  行跟着移动。

计数都在 `WorkCounters` 上：
- `virtual_row_metric_updates`：行高索引里变了的行；
- `virtual_prefix_index_updates`：写入的索引项，从列表上次放置行算起，包括使用方自己的插入、删除；
- `virtual_rows_remeasured`：列表拿到新高度的行，即量出新高度的行，或被施加了新高度的行；
- `virtual_rows_repositioned`：改了放置的行；
- `virtual_logical_rows_scanned`：按下标取 key、按 key 取下标的次数；
- `virtual_scroll_extent_updates`：列表总高变化的次数；
- `virtual_rows_materialized_from_layout`：为窗口新建的行。

门禁在 `world/issue262.rs`。一百万行，视口 40 行，上下各 8 行 overscan：
- A：可见的一行高 8 px。不查任何逻辑行（上限 64），只量出这一行；索引写入不超过一个块加
  O(log C) 项；下面的行只做原点平移。
- B：单元格从 "99" 改成 "98"，行高不变：列表总高、其他行的放置都不变，列表不重排。
- C：窗口第 21 行变高：前 20 行的放置不变，后面的行只平移；测量量和窗口顶部附近一行变高时相同。
- D：没挂出来的行数据变了：零布局。使用方因此改了 fingerprint，也只重查窗口的 key。滚过去，
  看到的是新数据。
- E：一百万行的树里，展开一个可见节点的 10 个子节点：只新建这 10 行，其余行不重建，只查窗口
  的 key。
- 表格单元格改文字、尺寸不变：零布局，窗口不重新放置。

#259 加了虚拟列表负载：10 万行和 100 万行的列表里一行变高，开销相同。只有索引深度
O(log C) 跟着集合变。

还没做的：
- `each_virtual` 的数据（`Keyed`）一变，就按全部项重新分组，O(N)：它拿到的是整个列表。保留式
  API（`sync_virtual_list_*`）的数据在使用方手里，没有这一步。
- 树的行高固定，按内容量高只用于列表。虚拟表格的列宽固定，没有按内容定的列。
- 文字比所在的盒子还宽、放不下一个词时（例如列表宽度为 0），重新排版前后它的宽度会变，增量
  布局和全量布局对不上。门禁给列表定了宽度，这个问题单独跟进。

响应式规则（#265）。节点可以按某个容器的尺寸换样式：`MutationQueue::set_responsive(节点,
ResponsiveRule)`。
- 规则读容器内容盒的一条轴：inline 或 block（按容器的书写方向），或者 width 或 height（不随书写
  方向）。
- 容器有三种：
  - 节点的父节点（`ResponsiveContainer::Parent`）；
  - 指定的节点（`ResponsiveContainer::Node`）；
  - 往上最近的查询容器（`ResponsiveContainer::Nearest { name }`），也就是 CSS `@container` 的语义：
    `container-type` 不是 `normal`、回答这条轴的祖先。`inline-size` 只回答它的 inline 轴，`size` 两条都回答。
    写了名字时，还要 `container-name` 里有这个名字。找不到这样的祖先时，节点保持作者写的样式。
    祖先改了 `container-type` 或 `container-name`，或者节点挪了位置，下面的规则会重新找容器。
- 查询容器不做尺寸包含（size containment）：它的尺寸仍然随内容变。变体反过来撑大、缩小容器时，靠下面的收敛规则停住，
  这一点和浏览器不同。
- 断点把尺寸分成有限个桶，最多 16 个断点。每个桶可以带一个变体：
  - `ResponsiveRule::below` / `at_least` 用闭包写，按是不是同一个闭包比较；
  - `ResponsiveRule::from_buckets` 直接给出断点和每个桶的 `StyleVariant`。
  `StyleVariant::between(基础, 目标)` 是数据：记录两份布局里不同的字段和目标的值，按值比较，
  所以同一份规则再发一次是空操作。没有变体的桶用作者写的样式。
- 变体写在作者的样式上，就像作者直接这样写：设计意图在变体之后解析，和它对待作者写的样式一样。
  `LayoutStyle` 里的字段都能变，布局之外，颜色、背景、字体、不透明度也会跟着变。
  - 节点在有变体的桶里时有两份样式：作者写的（`node_style()` 返回这一份，投影拿它比较，之后的
    `SetStyle` 也从它出发），和生效的（作者样式加上变体，管线读这一份）。
  - 换桶就是一次样式变化，按 `SetStyle` 同一套规则分类、标脏、排 seed，进同一个增量布局。只改
    绘制的变体只重绘，不排 seed。
- 规则登记时按容器建索引。布局写回让容器内容盒变了的轴，在那次提交结束时只评估读这条轴的
  规则，不扫描文档。还在原来桶里的规则什么也不改。
- 收敛：一帧里换桶的轮数最多 4 轮。某条规则要回到这一帧已经离开过的桶，或者轮数用完，就停在
  当前的桶上（计一次 fallback），这一帧收敛；下一次尺寸变化重新评估。变体会改变容器自己尺寸的
  规则由此确定地停下，不会在 240Hz 缩放下来回振荡。
- 节点挪到别的父节点下，`Parent` 规则改读新父节点；被摘下（Detach、Park）时不读任何容器，回到自己的布局；规则清掉后也回到自己的布局。

计数都在 `WorkCounters` 上：
- `container_query_size_changes`：规则读到的、变了的容器轴；
- `container_query_rules_evaluated`：评估的规则，只有轴变了的才评估；
- `container_query_results_changed` / `container_query_results_unchanged`：换了桶 / 没换桶的规则；
- `container_query_downstream_invalidations`：换桶的变体排出的布局 seed；
- `container_query_convergence_rounds`：有规则换桶的评估轮数；
- `container_query_cycle_fallbacks`：为收敛停下的换桶。

门禁在 `world/issue265.rs`：
- A：一个面板下 100 条规则，面板宽度变化只评估这 100 条，在 1 万与 10 万节点的文档里开销相同；
- B：同一个桶里连续缩放 240 次，没有规则换桶，变体不排 seed，行不测量；
- C：跨过一个断点，只有在这里断开的 10 条规则换桶，每条排一个 seed，其余 90 个固定行不测量；
- D：嵌套 8 层容器，每层 50 个固定叶子：缩放只评估 7 条规则、不碰叶子，最多 4 轮收敛，没有
  fallback，结果和同样输入的冷布局一致；
- E：索引只存登记的规则；一帧换桶的历史到下一帧就清空。

#259 加了响应式负载：在 1k、10k、100k 页面末尾，桶内缩放、跨断点、跨回、嵌套缩放的开销相同。

各层都投影到同一种规则：
- Rust：`MutationQueue::set_responsive`；
- `.vue` 的 `<style>`、`view!`、`stylesheet!`：构建期把 `@container` 编进样式表，元素挂载时装上规则
  （见 [响应式视图](reactive-view.md)；`css!` 只写声明，里面的 `@container` 给出警告）。

CSS 路径只产出数据，不量容器（见下文「`@container`」）。

有依赖索引的格式化上下文可以使用 `LayoutDependencyGraph` 表达父约束、包含块、
写作方向和 flex/grid/inline 的局部耦合。Runtime mutation authority 和产品帧统一发布
按节点合并的 typed seeds。布局调度直接消费 typed seeds。
不确定的结构变化保留根级重算边界。

## 布局递归与栈

测量（`measure_node`）和放置（`place_node_scoped`）都按树的深度递归。每一层都先经过 `layout_engine::recursion::level`：

- 读取当前线程剩下的栈；不足 256 KiB 时，这一层改到新分配的 4 MiB 栈段上继续（`stacker::grow`）。所以不论 debug 还是 release、线程栈多大，任何深度的树都不会让布局把栈用完。平台报告不了剩余栈时（Android、iOS、wasm，`stacker::remaining_stack` 为 `None`），这一层照常在原来的栈上运行。
- 同一次读数也给这一层定价：入口处的剩余栈比父层入口少了多少，就是这一层用掉的栈。每趟取最大值，发布为 `WorkCounters::layout_measure_stack_per_level` 和 `layout_placement_stack_per_level`（字节，一帧内取最大；平台报告不了时为 0）。
- `layout_engine::stack_budget` 的测试给每层定了预算（block、flex 列、grid 三条路线分别量），并在 1 MiB 的线程上布局 400 层的树。兜底防崩溃，预算防止每层的帧再长回去。

放置分两步。`lay_out_children` 走一遍容器的子项：测量、打包浮动、分行、对齐、求解 2D grid，给每个流内子项定好盒子，返回待放置的 `Descend` 列表和计划草稿，自己不放置任何子树。`place_node_level` 等这一帧退出后再依次进入这些子树，然后记录计划、放置浮动和定位子项，顺序和以前一样。复用计划的三条快速路径（`place_moved_container`、`place_edited_child_list`、`place_from_plan`）也各是一个函数，只在走到时才占栈。两步能分开的前提是：一个子项的盒子只取决于容器自己的游标、子项的样式和测量结果，不读兄弟子树放置时写下的东西。

x86_64 Windows（MSVC）上每层的栈：放置 debug 从 25.8 KiB 降到 9.0 KiB，release 从 4.8 KiB 降到 3.2 KiB；测量 debug 12.2 KiB，release 2.5 KiB。

### 路线：子树级调度，子树内递归

目标是混合架构。测量自下而上，继续用递归加缓存；放置由调度器以子树为单位管理，子树内部递归。测量之后每个节点的尺寸都已确定，所以放置阶段任何节点都可以作为边界。按顺序：

1. 已完成：递归兜底和每层栈预算；放置分成“定盒子”和“进入子树”两步。
2. 测量层：把 `measure_node_level` 里不递归的部分移出这一帧。
3. 放置边界：`Descend` 成为调度器的任务单元。在固定深度（每 K 层）、滚动容器和 `layout_isolation` 节点处自动切分，子树内的递归深度不超过 K。边界的结果只取决于它的输入（位置、尺寸、视口），因此可以单独调度和缓存。
4. 调度：按帧预算分片，没做完的子树整体沿用上一帧的结果（结果双缓冲）；按离视口的距离排序；新的变更让没做完的子树失效时取消它。队列长度和分片次数计入 `WorkCounters`。
5. 互相独立的边界子树并行放置：每棵子树写自己的输出，最后合并。

测量阶段的分片粒度受内容依赖限制：尺寸由内容决定的容器会把依赖一直传给祖先，这样的一串只能作为一个单元。超大的扁平列表靠虚拟化，不靠调度。

## Dynamic Layout

一行放不下时，按代价压缩子项声明的弹性（#207）。合同和求解器在 `nana_ui_core::dynamic_layout`，
文本（#211）、组件和容器用同一套类型，没有第二套代价。

合同（#208）：
- 子项向父级导出有界的包络 `EnvelopeShape`：至多 8 段，每段一份容量、每单位的视觉代价
  `LayoutCost`（有限值或显式的 `Forbidden`）、调整种类和执行档位 `ExecutionClass`
  （PlacementOnly < LocalBox < LocalReflow < Structural）。超出 8 段时把同档最贵的两段合并，
  代价只往高取。包络不带首选尺寸，那仍是固有度量的权威。
- 代价说的是结果看起来差多少，不是算得多慢；档位说的是要做多少工作。
- 节点经 `LayoutStyle.adaptation: Option<AdaptationProfile>` 声明：作为子项，`inline` / `block`
  说 padding、子项间 gap、盒内 gap 各能收到多少、每像素多少代价，`aggregate_children` 说把子项的
  弹性一并交给父级；作为容器，`solve_overflow` 说自己这一行溢出时要不要求解。

求解（#210）：`solve_deficit` 先用每档容量和判断能不能盖住缺口（不读段），找到能盖住的最便宜的档，
更便宜的档整档吃掉，更贵的档不看；这一档里按代价从低到高逐级吃满，盖住缺口的那一级按容量分，余数
按参与者顺序逐单位给。全部是 1/64 px 的整数运算，同样的输入得同样的结果，与构建和缩放的顺序无关。
同一档里参与的段超过预算时，按档容量比例分，次次相同（`budget_fallbacks`）。离散的结构适配要显式
允许，候选的固定代价作下界剪枝。

层级（#209）：顺序容器把自身 gap 和子项包络的段合并、规范化；交叉轴上按断点扫描，代价是所有超过
目标的子项各自下一单位代价之和。父级只读汇总，从不试排子项。

接入（#212）：
- 默认不求解。普通的 Row / Column / Stack / Grid 溢出时照旧溢出或交给原来的 shrink，求解器零运行、
  包络零查询；子项的声明只是样式里的数值，没有容器读就不花一点代价。
- `solve_overflow` 为真、不换行的单行容器，在 `distribute_flex_main` 里溢出时：主轴随内容的子项交出
  包络（作者写了 px / fill / 百分比主轴的不压），连同自身 gap 一起求解；剩下的再交给 shrink 因子。
- 子项在放置时用同一个求解器把分到的量拆给自己的 padding（两边各一半）和自己这一行；交给这一行的
  部分表现为这一行的溢出，由这一行照样求解。`aggregate_children` 隐含这一行也求解。
- 只压 chrome 的子项内容盒不变，跳过按已用主轴尺寸的强制重测；容器计划在内容盒原点变化时不复用；
  开启求解的容器在计划里记作依赖主轴尺寸。
- 没做的：grid 轨道下限、按钮图标与文字间的盒内 gap、竖排。

保留（#213）：
- 包络按盒子的样式、padding、gap 和子项包络的 generation 保留；与给它的尺寸无关，缩放找得到它。
  形状没变 generation 不动，父级的汇总也就不重建。
- 溢出的行保留求解停在哪一级（`SolveFrontier`）和参与者的 generation。缺口仍落在这一级里，只重分
  这一级——冷解的最后一步，结果相同；跨级或参与者变了，从头求解。测试与 `layout-verify` 下每次
  重分都同时冷解一遍断言相等。
- 父级复用计划、不重新求解的那一趟，子项沿用上次保留的分配。
- 没有默认的滞回；保留的求解只是提示，不改变答案：从大到小、从小到大缩放到同一宽度，结果相同。
- devtools 的 inspect 多了 `dynamic`：包络各段（长度为 1/64 px）、generation、这次的分配和 padding
  收掉的部分。

计数在 `WorkCounters.dynamic`（`DynamicLayoutCounters`）：考察的行、求解次数、无需调整的行、只触及
一级的求解、包络的查询 / 命中 / 未命中 / 重建、读过的段、候选的创建与剪枝、细化、beam 宽度、子项
落实、子项重排、结构适配、预算回退、scratch 分配与峰值、复用、增量重分、冷解、被档位跳过的参与者。
四个基准和 perf schema 以 `dynamic_<字段>` 输出。诊断在 layout 域 id 6–8。

门禁：
- `world/issue212.rs`：放得下的行与无声明的逐盒相同；溢出时先收 gap 再收 padding；shrink 只吃剩下的；
  不求解的容器不读声明；拖动中每趟对照全量布局；inspect。
- `world/issue213.rs`：同一级里缩放只重分、不建包络；跨级冷解一次；从上从下到同一宽度结果相同。
- `world/issue214.rs`：A 静态帧和只改绘制不求解；B 1k 项的便宜缺口每项读一段、1k→2k 线性、级内
  缩放不分配；C 分支 4 的嵌套树每节点每趟至多落实一次、工作随节点数增长；E 240 次缩放跨三级，只在
  跨级处冷解；F 超出预算的行恰回退一次且确定；G 500 帧缩放、增删项、改声明后保留的包络和求解跟着树走。
- `world/issue264.rs` 的 Gate E；`nana-ui-core` 的 `dynamic_layout` 单测（含同一级里每个缺口的增量
  重分等于冷解的穷举）。

## 能用的

**Flex。** 你可以用 `flex-direction`、`flex-wrap`、`gap`、`align-items`、`align-self`、`justify-content`。多行换行时还有 `align-content`。`stretch` 和 `normal` 把剩余的交叉空间均分给各行。还有 `order`、`flex-grow`、`flex-shrink`、`flex-basis`。侧栏加主区，用这一套就够。

换行的行按项目**长大之前**的尺寸分行。这是 CSS 的 hypothetical main size。先取 `flex-basis`。没有，就取主轴尺寸。再没有，就取内容。并受 `min-*` 和 `max-*` 约束。分好行之后，才把行里剩下的空间按 `flex-grow` 分出去。

所以 `flex: 1 1 320px` 的两栏，放得下就并排，放不下就各占一行。带 `flex-grow` 的项目也不会独占一行，把放得下的兄弟挤走。

你没写 `flex-shrink` 长手时，按 **0** 处理。不是网页 CSS 的 initial **1**。溢出的定宽行（列表、工具条）会保留盒子。不会被悄悄压扁。

`flex` **简写**省略 shrink 时，仍按 CSS 写成 1。例如 `flex: initial`、`flex: 1`、`flex: 1 100px`。需要网页那种收缩时，显式写 `flex-shrink`，或用简写。`flex: none`、`auto` 和数字简写仍按 CSS 含义写 shrink。

**尺寸。** 你可以用像素、百分比、`em`、`rem`、`vw`、`vh`、`min`、`max`、`clamp()`。

轻量 `calc` 支持 `+`、`-`、`*`、`/`、括号和嵌套 `calc`。`var()` 展开后再算。结果折进既有的 `%±px`、`vh±px`、`em±px` 或纯 px 规格。嵌套上限是 16 层 CSS 括号或函数。unary `+/-` 另计，最多 16 个符号。无单位结果、`0px+number`、除零、非有限 f32，都 fail closed。

同单位、可以折叠的嵌套 `min`、`max`、`clamp` 可以用。能并进既有 `Min2`、`Max2`、`Clamp3` 的混单位也可以。例如 `min(10px, max(1px, 50%))` 和 `min(1px, 2%, 3px)`。布局时相对包含块兑现。三路互不可比，仍 fail closed。

还有 `min-content`、`max-content`、`fit-content`，以及两个 NanaUI 扩展关键字：`fill`（`LengthSpec::Fill`，占满可用空间，和 `100%` 同义）和 `shrink`（`LengthSpec::Shrink`，按内容在当前可用空间下量出的大小，Row/Column 预设用的就是它）。`shrink` 不同于 `fit-content`：可用宽比内容窄时，`fit-content` 会夹到可用宽并压缩子项，`shrink` 取子项量出的尺寸之和。

无法折成长度原子的 leftover `calc`，可以走 `LengthSpec::Calc` 这棵 AST。不要假装它已经 flatten。

不折行的 flex 行上，`min-content` 和 `max-content` 都是子项之和。折行、块、列轴上，`min-content` 取最宽的子项。

`fit-content` 以最宽子项为下限，以可用宽为上限。折过行的文字，按它不折行的宽度量内容宽，并且不超过可用宽。所以按内容收窄的盒子，在上限放宽后会变宽。它不会停在上次折行的宽度。

会折行的文字，只有在不宽于这个盒子的宽度上折出的行，才按行自己的宽度算：放不下的长单词让行、也让盒子更宽。还没有盒子时（第一次整形）或在更宽的盒子里折出的行不算，文字按这个盒子重新折行。所以同一棵树的结果不取决于文字上一次是在多宽的盒子里整形的，增量布局与从头布局一致。

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

`writing-mode` 和 `direction` 在布局里也继承。没写这两项的容器，沿祖先的方向排。Rust API 或样式树直接构造的节点同样如此。祖先改了方向，后代容器随之重排。没写方向的盒子上的逻辑边（例如 `padding-inline-start`）也按继承来的方向落到物理边。

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

`display: layers` 是 NanaUI 的扩展值，见上文 `Layers`。它不写入 `stretch`：叠层的子项默认按起点对齐，和 ZStack / Box 一样；`justify-items` 没写时也按起点，不按网格的 `stretch`。

**行内子集。** 块容器里的 `display: inline` 和 `inline-block` 走一行内格式化上下文，也就是 IFC。

行内子项按 **inline 轴** 排列。`horizontal-tb` 向右。`vertical-rl` 和 `vertical-lr` 向下。块级兄弟会关掉当前行，并独占一行。

竖排时，盒子的物理宽高仍是 width 和 height。但 IFC 和 `flex-direction: row` 的主轴跟着 writing-mode 的 inline 轴走。`vertical-rl` 的行从右边起排。

你可以用 `text-align`。`start` 和 `end` 随 `direction`，仅横排。`left` 和 `right` 保持物理边。还有 `center`。`justify` 在行内盒的摆放上按 `start` 处理，同时像 CSS 一样继承给下面的文字，让段落两端对齐（见 [文本引擎](text-engine.md#cjk-行决策211)）。

`white-space: pre` 在量测里保留换行和空格。

`align-items: baseline` 读取同一份 intrinsic metrics。文本节点优先使用
`nana-text` 保留的首行和末行 baseline；宿主只提供 ascent 时才使用该
fallback。替换内容和 Custom 节点以 block 轴末端作为明确 fallback。Flex、Grid
和 IFC 不各自估算一套基线。

IFC 的行里，原子按基线对齐（`vertical-align: baseline`，见上文「替换内容」的基线一段）：
inline-block 取末行基线（行盒在后代里时取后代末个行盒的实际位置），替换内容、`overflow` 非
visible 和没有行盒的原子取底外边距边，行从块的 strut 起算，行高是对齐后的跨度。原生段落和普通
块级流的行共用这条规则。

flex 行的保留计划记下每个按基线对齐的子项当时用的基线。下一趟里，子项的尺寸没变
但基线变了（固定尺寸的按钮里字号变了），这一行照样重新对齐，不沿用旧位置。

**浮动子集。** `float: left | right` 把盒子从块流里拿出来。同侧多个浮动按几何并排。放不下就折到下一行。

流内的 `clear`，和浮动自身的 `clear`，都用折行之后的占用底边。不是单盒的预排高度。

IFC 行盒按当前行与**兄弟**浮动外边距盒相交的左右 inset 缩窄。这是 shrink-to-avoid-float。一行里放不下的原子行内项折到下一行。缩短后仍放不下，就把行盒下移到最近那个占用浮动的底边之下。

流内**块级**边框盒不会缩窄去绕开浮动。这和 CSS 里非 BFC 的块一致。

这不是完整排除。祖先浮动不侵入子块的 IFC。没有 `shape-outside`。

flex 项和 grid 项上的 float，按 CSS 被块化，然后忽略。

**定位。** 你可以用 `relative`、脱流的 `absolute`、相对窗口的 `fixed`，以及文档流内的 `sticky`。`sticky` 在滚动投影之后才贴住。它不写回 Runtime 的 `LayoutBox`。

`absolute` 的包含块和 CSS 相同（CSS 2.1 §10.1）：

- 最近的定位祖先（`relative`、`absolute`、`fixed`、`sticky`）的 **padding 盒**。中间的静态盒子跳过，它们的 padding 不算。
- 没有定位祖先时，是初始包含块：文档原点上、视口大小的矩形。

所以想让 absolute 子项贴着某个盒子，那个盒子要写 `position: relative`。`inset: 0` 盖住它的 padding 盒，不是内容盒。

**clip 链（#201）。** 溢出容器（`overflow` 不是 `visible` 的盒子）只裁剪、滚动这样的后代：这个容器是后代的包含块，或者在包含块之上（CSS Overflow 3、CSS 2.1 §11.1.1）。所以：

- `absolute` 盒子的包含块在一个静态滚动容器之外时，它不随那个容器滚动，也不被它裁剪；包含块在容器里面时照常。
- `fixed` 盒子和它下面的一切不受任何祖先裁剪、滚动。

每个盒子发布的 `LayoutResult` 记着这条链：`clip` 是最近一个够得着它的溢出容器，链沿那个容器自己的 `clip` 往上；`clip_parent` 是盒子取裁剪和滚动的节点，一般是父级，逃出溢出容器的 `absolute` 盒子则是它逃出的最外层容器的父级，`fixed` 盒子是 `None`。`UiWorld::layout_clip_chain` 按发布的结果列出整条链。被跨过的祖先仍把变换传给它，但不传任何裁剪和滚动。这里和 CSS 有一处不同：被跨过的祖先上的 `clip-path` 和组件铬的裁剪（EmptyState、对话框）也不作用于它，CSS 的 `clip-path` 会。

Scene 抽取、命中索引、无障碍和 Select 菜单的可用空间都按 `clip_parent` 挂盒子，不各自沿祖先推导。要按上方的盒子截一块区域的遍历（菜单的可用空间、无障碍里 EmptyState 与对话框的铬裁剪）都走 `UiWorld::clip_ancestors`，即沿发布的 `clip_parent` 往上：`fixed` 盒子（字段自己或它上方的一个）到此为止，`fixed` 的字段打开的菜单取整个窗口（#205 门禁 `issue205_menu_room_and_chrome_clips_follow_the_clip_chain`）。命中索引里逃出的盒子挂在 `clip_parent` 的条目下，滚动时它逃出的那个容器的平移不带上它，它覆盖的点按绘制顺序排先后。无障碍节点的 `bounds` 仍是整个盒子，`clip` 是链上各溢出容器按此刻的滚动留给它的那块视口，链上没有容器时为 `None`；画出来的、指针点得到的就是 `bounds` 落在 `clip` 里的部分。滚动仍然不跑布局。祖先改变定位或开始、停止裁剪时，下面记着的链跟着重新发布。

门禁在 `world/issue201.rs` 的 `clip_chain` 模块：定位的页面里有静态滚动容器，里面一张定位卡片带一个静态滚动容器，放一行流内盒子、一个以卡片为包含块的 `absolute` 盒子和一个 `fixed` 盒子；卡片后面一个静态裁剪盒放一个以页面为包含块的 `absolute` 盒子。断言各自的 `clip` / `clip_parent`；两个容器滚到几组偏移，每组不跑布局，无障碍的位置和裁剪、指针命中（就地打补丁的命中索引和重建的都算）都按链；Select 菜单的空间同样只被链上的容器截；卡片和里层容器改变定位、溢出时与冷布局逐项相同（含无障碍的 `clip`），就地打补丁的命中索引（先让滚动平移活着，再变链）在整页网格上的命中和候选与重建的相同；被跨过的祖先的变换（样式上的和合成器动画呈现的）照样作用于逃出的盒子；一个树序在后、盖住逃出盒子的兄弟接住指针，不论索引怎样排它们。`nana-ui-scene` 的 `runtime_document_clip_chain_tests.rs` 走真实的 `RuntimeDocument` 帧，逐个比较 Scene 画出的位置和裁剪、无障碍的 `bounds` / `clip`、命中和剔除，滚动和链变化后都相同。

滚动范围也按这条链算（CSS Overflow 3：盒子只算进它包含块链上的 scrollable overflow）。`absolute` 盒子算进它包含块的范围，不算进它逃出的静态滚动容器；`fixed` 盒子和它下面的一切不算进任何容器。见下文「溢出」。门禁在 `world/issue201.rs` 的 `scroll_range` 模块：静态滚动容器里放一个以页面为包含块、远在下方的 `absolute` 盒子和一个更远的 `fixed` 盒子（里面再放一个 `absolute` 盒子），容器的可滚距离、`LayoutResult::overflow`、`scroll_extent` 和偏移夹取都不含它们，`absolute` 盒子算进够得着它的页面；容器改成 `relative`、盒子在 `absolute` 与 `fixed` 之间切换时范围随链移动，与冷布局逐项相同（含度量和夹取后的偏移）；两个容器滚动时不跑布局，范围不变。`world/scroll_bounds.rs` 的 `out_of_flow_boxes_count_where_their_clip_chain_reaches_them` 在索引层面对照按 clip 链走的全遍历和新建的索引：定位翻转、子树在包含块之间搬动、停放后接回、停放时顶层开始裁剪、触发式菜单打开（菜单项变 `fixed`）再关上。

增量布局按包含块追踪：包含块挪动或改变尺寸时，挂在它上面的盒子会重放，即使中间的静态盒子这一趟没被放过；只有中间的静态盒子移动时，盒子不动。祖先开始或停止成为包含块时，下面的盒子改挂到新的包含块，发布的 `LayoutResult::containing_block` 也跟着更新。门禁在 `world/issue200.rs`：每一步都和冷布局逐个比对盒子、结果、命中和无障碍。

`sticky` 相对最近的滚动视口解析 inset，并受正常流包含块约束：到父容器末端会释放。没有 inset 的轴照常滚动。嵌套 Sticky 从父级的呈现位置继续计算；嵌套滚动容器各自拥有视口。Runtime 的命中、无障碍和 Scene 绘制共用同一个 placement 投影，滚动不触发内容 measure、layout 或文本塑形。改变包含块尺寸或重新挂载时，只失效其登记的 Sticky 分支。

`fixed` 的包含块是它所在文档的视口：原点在窗口左上角，大小是这个文档上次布局用的视口。祖先是不是定位盒子、有没有 padding 都不算；祖先的滚动和裁剪也不作用于它，命中和无障碍同样按窗口上的位置读它的盒子。没有定位祖先的 `absolute` 用的初始包含块也是这个文档的视口。一个 `AppContext` 里有几个文档时，每个文档记着自己的视口（`UiWorld::document_viewport`）：一个文档改变视口，只重放它自己的 `fixed`、`vw` / `vh` 盒子和初始包含块上的盒子，另一个文档不跑布局，盒子、结果和记下的视口都不变。门禁 `issue200_fixed_boxes_hang_from_their_own_windows_viewport`：两个视口不同的文档，各有一个藏在 `relative` 卡片和带 padding 的静态盒子里、宽 25% 的 `fixed` 盒子，一个初始包含块上的 `absolute` 盒子，一个滚动列表里的 `fixed` 盒子；列表滚动、两个文档轮流改变大小，每一步盒子、命中和无障碍范围都在自己的视口上，另一个文档不动，最后与冷布局相同。

一个 `AppContext` 承载几个文档时，每个文档的帧用 `AppContext::take_system_work_for(document)` 取工作（`RuntimeDocument` 就是这样取的）：样式、文本这些全局工作照常取走，布局 seed 只取这个文档的，另一个文档的编辑留在待处理表里，等它自己的帧。直接用 `take_system_work` 一次取完再交给某个文档的布局也不会丢：那一趟会把别的文档的 seed 放回待处理表。门禁 `issue200_an_edit_pending_in_one_window_waits_for_that_windows_frame`：第二个文档的盒子变宽后先画第一个文档，第一个文档一趟布局都不跑；第二个文档自己的帧才放下这次编辑；一次全部取完、全部交给第一个文档时，编辑同样留到第二个文档的帧。

`fixed` 只适合普通节点贴在视口上。产品浮层仍走控件。不要自己用 `fixed` 搭对话框。

**锚点定位（#200）。** `fixed` 盒子可以带 `LayoutStyle::anchor`（`AnchorSpec`）。它写明锚点矩形、首选侧、对齐、间距，还可以给一块可用区域。布局按盒子量出来的尺寸、对着真实视口放它，用的是 `resolve_anchored_surface`：

- 首选侧放得下，就放首选侧；放不下而对侧放得下，就翻到对侧；
- 再沿两条轴收进可用区域。可用区域是视口和 `room` 的交集。

这是 CSS anchor positioning 里「翻面再收进来」的那一部分。所有锚定的浮层共用这一个解析器：ContextMenu、AnchoredActionMenu、触发菜单、HoverCard、Popover，以及 Select 的下拉。Select 两侧都放不下时，取宽的一侧并截短。

窗口改变大小时，布局直接重放这些表面，不用重新投影组件。命中和无障碍读的都是布局给出的盒子。这个字段没有 CSS 写法，由控件写。门禁在 `world/issue200.rs`。

Select、Dropdown、SearchDropdown 的菜单不是布局盒子，是字段的组件几何：按字段的盒子和「菜单看得见的空间」（`SelectMenuRoom`：窗口，再扣掉字段 clip 链上的裁剪，按此刻的滚动位置）用同一个解析器摆放。Scene 画的、`select_option_at` 选的、命中索引里字段那一项带的菜单表面，都从这一份几何来。空间由字段投影时记下；每一趟布局结束后（滚动保留之后），引擎再为文档里开着的菜单按刚发布的布局算一次空间，变了才写回，所以窗口改变大小、字段或外层容器被挪动时，菜单不用等字段重新投影就重新翻面、截高，命中索引随之重建字段那一项。只有滚动不跑布局：页面滚动时菜单跟着字段走，到下一次变化再重新摆放，画和命中仍是同一份。门禁 `issue200_a_select_menu_follows_the_viewport_and_hits_where_it_paints`：窗口变矮再变高，菜单翻上翻下，每一步画在某点的那一行就是指针选中的一行、命中给字段的一点，旧位置不再命中，与冷布局得到的菜单相同。字段（`ComboBox`）的范围是它的布局盒子；菜单打开时，字段下有一个列表框，每个露出来的行一个选项节点，范围同样取自这份几何（行与菜单视口、此刻的「看得见的空间」相交，再经字段的变换），所以读屏读到的、画出来的和指针选中的是同一行，见 [Select 的读屏](../components/select.md#读屏)。门禁在同一个文件：`issue200_a_select_listbox_*`、`issue200_an_open_select_menu_reads_as_a_listbox_of_its_rows` 等。

脱流盒子的高度没写，也没同时写 `top` 和 `bottom` 时，按内容算。它里面按高度填满的子项（`Fill`、百分比高度）没有确定的高度可依，按内容处理。不再取包含块的高度。包含块的高度由这些盒子撑出来时（按内容量高的虚拟列表行），那样会每一趟布局都长高一截，帧永远稳定不下来。

这条只管它里面的子项。盒子自己的百分比 `min-height` / `max-height` 仍按包含块的高度兑现：`fixed` 是视口，`absolute` 是它的包含块。脱流盒子的包含块总有确定的高度（CSS 2.1 §10.7）。所以 `position: fixed; max-height: 80%` 的卡片按内容长高，最高到视口的 80%。不用改写成 `80vh`。

写了 `aspect-ratio` 的脱流盒子，高度按 `left` / `right` 和宽度的上下限留下的宽度换算，和 flex、grid、行内里一样：`width: 120px; max-width: 60px; aspect-ratio: 1.5` 得 60×40，`left: 0; right: 480px` 在 600 宽的包含块里得 120×80（#382，门禁 `world/issue382.rs` 的 A，css-parity `T-P21`）。同时写了 `top` 和 `bottom` 的盒子，高度仍取两个 inset 之间。

这几条所有 context 都一样（CSS Sizing 4 §5.1）。比例在 `box-sizing` 指定的盒子之间换算：`border-box; padding: 6px; width: 120px; max-width: 60px; aspect-ratio: 1.5` 的边框盒是 60×40，`content-box; padding: 6px; width: 120px` 的内容盒是 120×80、边框盒 132×92。换算出的高度再受盒子自己的 `min-height` / `max-height` 约束：上例加 `min-height: 50px` 得 60×50。流内容器的 auto 高度读的也是这个高度（#393，门禁 `world/issue382.rs` 的 A，css-parity `T-S23`）。

**层叠上下文子集。** Scene 按 `(z_index, document_order)` 排序。它把隔离组当成一层。这些组是：`opacity` 介于 0 和 1 的组、`isolation: isolate`，以及 `position` 非 static 且写了 `z-index`。

高 z 的子项不会画到组外、后出现的兄弟之上。

**例外：** `position: fixed` 的表面画在根层叠上下文。Popover、ActionMenu、HoverCard 的弹出层属于这一类。它们不被父级隔离组裁进卡片里。

弹出层的内容由 Runtime 定成 viewport-fixed。抽取给 Scene 时也按这份样式走。所以祖先的 overflow 裁剪、滚动和变换都不作用于它。

卡面是触发节点画的那块底。它同样不带触发器的变换和裁剪。它和弹出内容排在根层叠上下文的同一层（`MENU_OVERLAY_Z_INDEX`）。位置在触发器之上，也在页面其余内容之上，包括后出现、`z-index` 更高的兄弟。它在弹出内容之下。

重叠处的命中排序用同一条前缀切断。否则后面的兄弟卡会抢走菜单点击。卡面本身也在这一层接指针。落在条目之间或内边距上的按下，留在菜单里。不穿到底下的页面。也不当成按触发器去开合菜单。

Select、Dropdown、SearchDropdown 打开的菜单也排在根层叠上下文。层级是 `MENU_OVERLAY_Z_INDEX`，字段自己的 `z-index` 更高时取它，同层按字段的文档顺序排。所以它画在后出现的兄弟组之上，重叠处命中也归它。和弹出层不同，它不是 viewport-fixed：它带着字段的变换，仍受字段外层的裁剪。菜单摆放时只用看得见的那一块，见 [Select 的菜单](../components/select.md#菜单)。

不要靠你的应用给整张卡抬 `z_index`。

这不是完整的 CSS Appendix E。负 z 分层、float 和 inline 层、transform 单独成层等，未全做。

命中仍走树结构。组内的 z 只在兄弟之间比。

**`display: contents`。** 子节点提升到父级的格式化上下文。该节点自己没有盒子。

**文字。** 你可以用字号、字重、字体、行高、字距、颜色。

`line-clamp` 和 `-webkit-line-clamp` 限制行数，并打省略号。

`text-decoration: underline | line-through` 由 Scene 在文本盒上描线。

`text-emphasis`、`text-emphasis-style`、`text-emphasis-position`、`text-emphasis-color`（以及 `-webkit-` 前缀）给每个字加着重号：`dot`、`circle`、`double-circle`、`triangle`、`sesame` 加 `filled` / `open`，或一个字符串（只取第一个字）。位置是 `over` 或 `under`（中文常用 `under`），默认 `over right`。三者都继承；简写设样式和颜色，不碰位置。记号是半字号的普通字形，标点和空白不加；行盒按记号实际高度长高。改样式或位置只重新布局，改颜色只重画。见 [text-engine.md](text-engine.md) 的「着重号」。

`font-feature-settings` 和 `font-kerning` 写入 `nana-text` 的 OpenType features。

`font-variation-settings` 把已经声明的轴交给 shaper。`wght` 仍并进 `font-weight`。字体没有该轴，则**该轴** fail-closed。不把标签改写成 `wght`。也不塞进 `FontFeatures`。

`user-select: text | none | auto | all | contain` 写入 `LayoutStyle`。它继承，和 `cursor` 同类。`auto` 下，普通 `TextContent` 不可选。`text` 允许文档级拖选复制。`all` 单击即选中该文本节点的全文，不必拖。`contain` 可以选中，但选区留在该子树内，不延伸到邻居。`none` 不进选区。子节点显式写 `text`，可以再打开。

这不是第二套 TextInput。

作者的 `::selection` 和 `::-moz-selection` 只兑现 `background`、`background-color`、`color`，画到文档选区高亮。其余属性 fail-closed。这不是完整的 CSS Highlight API。未写时，选中底仍用主题的 `accent_soft`。

`word-break: keep-all` 会跳过。`line-break: strict|loose`、`text-spacing-trim`、`text-autospace`、`text-justify`、`text-wrap-style` 交给文本引擎的 CJK 行决策（#211），都会继承。字距是近似。

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

`mask-image` 和 `-webkit-mask-image` 可以是线性或径向渐变的 alpha，或 `url()` 纹理。GPU 最多 8 个 mask 色标。两种都只取 alpha，色标和纹理的颜色不参与，即 CSS 默认的 `mask-mode: match-source`：`linear-gradient(90deg, transparent, black)` 从左到右由隐到显。`mask-mode: luminance` 不支持。

mask 在节点的布局盒上求值，只作用于节点自己画的东西，不作用于子树：

- 节点自己的 quad（背景填充；边框和 `box-shadow` 不受影响）和 HostTexture 的内容。
- 渐变 mask 还作用于 `.painter` 录下的全部命令：填充、描边、阴影、文字、图标和图片，坐标同样是节点内坐标。所以自绘的形状和节点背景按同一条渐变淡出。超出布局盒的部分（自绘阴影、外发光）取盒边上的值，不会被裁掉。这一点和 CSS 的 `mask-clip: border-box` 不同，是有意的：方向性的光晕要能伸出盒外。
- 有 mask 时，自绘里的矩形不再走 quad 快路径，一律三角化后由 path shader 乘上 mask。文字、图标和图片各画进一个图层，图层合成前乘上 mask。
- `url()` mask 不作用于自绘，那部分照常不加 mask 画出，并计入指标 `runtime.paint.mask_unsupported`。

子节点和节点自己的文字都不受 mask 影响。CSS 那种对整棵子树的遮罩，要先把子树隔离成一个图层，NanaUI 没有做。需要遮住子节点时，把 mask 放到子节点自己身上。

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

`backdrop-filter: blur()` 是逐节点的 dest 采样模糊。`blur(r)` 的 `r` 和 CSS 一样是高斯的标准差。它做旋转映射，也做祖先的 inset 和 polygon clip。不是整窗的 Mica 或 Acrylic。

`<img>` 的 `object-fit: cover | contain | fill | none | scale-down` 经 cascade 写入 `PaintStyle::object_fit` 和 `content_image` 的 fit；`<video>` 与 `<gpu>` 把它落到 `CustomRenderNode.fit`。三者由画家用同一个 `ContentFit::fitted_size` 求尺寸（见上文「替换内容」）。HTML attr 是 presentational hint。样式表压过 attr。inline 再压过样式表。

**命中。** `pointer-events` 是 `auto` 或 `none`，按 CSS **继承**。

父级是 `none` 时，没写该属性的子孙，used 值也是 `none`，不可点。只有显式 `auto` 的子节点重新成为目标。点在父盒上、但没落在那个 `auto` 子上，会穿透。

退出命中在提交时就生效。设成 `none`、关掉 `InteractionState::pointer_events`、挪到 `none` 的父级下或已停放的分支里，提交之后、下一帧重建命中索引之前的命中都会穿透到下层。重新变得可点要等下一帧解析样式。

未知关键字 fail closed。它不改已经指定的值。`inherit` 和 `unset` 清掉指定值。

平面 3D 命中，和 Quad、Text、Icon、HostTexture 的绘制，共用同一份单应逆变换。不按仿射盒命中。也不按梯形 AABB 的空角命中。

**溢出。** `overflow`、`overflow-x`、`overflow-y` 可以是 `visible`、`hidden`、`clip`、`auto`、`scroll`。

`hidden`、`clip`、`auto`、`scroll` 都会裁剪绘制，也会裁剪命中。裁剪和滚动沿 clip 链作用于后代，见「定位」。

两轴都裁剪时，后代的绘制按盒子自己的 `border-radius` 裁圆角。裁的是边框盒。四角不等时取最小的那个，不超过短边的一半。这和 CSS 一致。盒子自己的背景和边框仍只按直角边框盒裁。不被同一个圆角再裁一次。命中仍按直角。

`overflow: auto|scroll` 的滚动权威是 Runtime 的 `ScrollOffset`。滚轮走这条路径。滚动不写回 `LayoutBox`。

偏移保留小数。画面按设备像素吸附。见 [Runtime 与 Scene](runtime-scene.md)。

滚动起点在 start 边。这是 CSSOM View 的 scrolling area。`direction: rtl` 的行、`vertical-rl` 的块轴、`row-reverse` 和 `column-reverse` 从右或下开始排。初始视图就是右或下边。向左或向上溢出的内容，用负偏移滚到。和 CSSOM 一样，偏移从 0 往负走。越过 start 边另一侧的内容，滚不到。

滚动范围只算滚动容器够得着的盒子，即 clip 链经过它的盒子（见「定位」里的「clip 链」）：包含块在容器之外的 `absolute` 盒子算进它的包含块，`fixed` 盒子哪里都不算。内容索引把 `absolute` 盒子的范围带过它和包含块之间的盒子，落进包含块；带上去的范围只取决于它来自的子树，所以子树搬家时索引不用重建。会裁剪或滚动的盒子，`LayoutResult::overflow` 也只并入 `clip_parent` 是自己的子项；其他盒子照旧并入所有子项。滚动范围只算到最近的裁剪盒为止，和 CSS 的 scrollable overflow 一样。里层的滚动容器只把自己的盒子算进外层的范围，它里面内容的溢出不算：滚动区里套一个滚动区，外层不会因为里层的内容变长而能滚。`overflow: hidden` / `clip` 的盒子在它裁剪的那条轴上同样只算自己的盒子，没裁剪的那条轴照常算进它的内容。滚动容器两条轴都算裁剪。哪些轴算裁剪，用 `UiWorld::overflow_contained_axes` 查。

每个滚动容器都有度量。写布局盒、增删子节点，或改样式的提交结束时，Runtime 重新测量受影响的滚动容器，并把偏移夹进去。不论盒子是引擎写的，还是宿主写的。多行文本编辑器的度量随塑形结果走。`vertical-rl` 编辑器同样用负偏移滚向左边的列。

滚动路径的计数在 `WorkCounters` 上（#381），都常开，只做加法：
- `scroll_viewport_updates`：滚动容器视口尺寸变化的次数。第一次量出视口、不再滚动时丢掉视口，都算一次。存进相同的度量不算，只有内容范围变了也不算。
- `scroll_content_extent_recomputes`：内容索引里重算范围的条目数。纯滚动是 0。范围重算一旦爬上每次滚动，这一列先变。
- `scroll_clip_updates`：滚动路径重写了 clip 的节点数，即命中索引里重建的条目，不是被滚动的容器数。普通后代共用容器的一次平移，不重写；粘性分支跟着滚动重放，算它的条目；带透视的容器退回整份重建，算全部条目。clip 传播退化成 O(子树) 时，这一列跟着涨。
- 没有 `scroll_layout_reflows`。“纯滚动不排版”断言在已有的计数上：`layout_measure_nodes`、`layout_placement_nodes`、`text_shaped` 都是 0。
- 虚拟列表不另起名字。为窗口新建的行是 `virtual_rows_materialized_from_layout`；这些行的实体是 `entities_spawned`，离开窗口的行的实体是 `entities_despawned`。

门禁在 `world/issue381.rs`。页面和视口一样高，里面两个列表，各有一个带两个子节点的粘性表头和 50 行：
- 只滚动的帧：不排版，视口和范围都是 0。滚一个列表，clip 计 3，就是它的粘性分支，不含它的 50 行，也不含另一个列表的表头；两个都滚计 6；滚页面计 0。
- 视口从 600 改成 500：只有跟着视口走的页面计一次视口变化，范围重算 1 次（只有页面自己的条目），clip 是 0。存进相同的度量不计。列表里一行变高，范围变了，视口不计。
- 虚拟列表滚到窗口之外：滚动帧三项都是 0。同步后，新建行数等于新窗口的行数；下一帧新建的实体是新窗口的，删掉的是旧窗口的。范围重算不超过发布批次数乘以实体数，一万行和一百万行的集合相同。

自定义滚动条铬是 [`ScrollView`](components.md)。`overflow` 不另做一套 thumb 绘制。

**选择器。** 视图里只有 class 和 class 复合选择器生效（`.card`、`.card.active`），其余选择器给构建警告。下面是引擎本身能解析和匹配的部分。匹配要的树信息（兄弟序号、是否为空、是否选中、焦点、`:has()` 的后代位集）都由调用引擎的一方给出，引擎不自己遍历树。

引擎认得 type、class、id、属性选择器，以及组合符空格、`>`、`+`、`~`。

还有 `:root`、`:first-child`、`:last-child`、`:only-child`、`:nth-child()`、`:nth-of-type()`、`:nth-last-child()`、`:first-of-type`、`:last-of-type`。它们按调用方给的兄弟序号和个数匹配。简单的 `:not()`、`:is()`、`:where()` 也可以。

`:disabled` 和 `:not(:disabled)` 就是 `[disabled]` 的存在性匹配。

`:checked` 读调用方给的 `checked` 标志。哪些节点算选中，由调用方决定。`input:checked + label` 走已有的兄弟组合符。

`:empty` 和 `:not(:empty)` 读调用方给的 `is_empty`。引擎不自己数子节点和文字。

廉价主体 `:has(.class|#id|type)`，含逗号 OR，读调用方算好的后代位集（`has_bits`、`has_args`）。参数去重后最多 64 个。引擎不自己扫子树。

廉价主体 `:focus-within` 读调用方给的 `focus_within` 标志。

`:focus-visible` 映射为 `:focus`。没有键盘和指针的区分。这是 fail-closed：未聚焦时不匹配。

`::before`、`::after`、`::placeholder`、`::selection` 和 `::-moz-selection` 只解析和匹配，得到声明块，没有谁拿它生成盒子或画东西。

占位文字和文档选区的颜色是 Style Model 的字段：`LayoutStyle::placeholder_color`、`placeholder_opacity`，以及 `selection_background`、`selection_color`。Runtime 的 TextInput 和文档选区照着画。

带组合符的 `:has` 和 `:focus-within`，以及写在祖先上的，引擎不解析，计入 `skipped_selectors`。

**层叠。** 样式表内部先 normal，再 `!important`。然后再比特异度和源序。视图编译 class 规则时用的也是这个顺序。

视图里没有 prop 和 inline 这两层：class 规则按上面的顺序，写在元素构建出来的布局上。引擎自己的 `rebuild_layout_style` 还有这两层，顺序见下文「`@layer`」之后的几段。

`:hover`、`:focus`、`:active` 进交互桶，`matched_interactive_rules` 按调用方给的状态挑规则。它们不当布局条件。视图里是构建警告。

`transition` 在视图里编成元素的隐式动画。只认 `opacity`、`transform`、`width`、`height`、`background`（含 `background-color`），其他属性给构建警告。

`@keyframes` 和 `animation` 只解析，不播放。视图里是构建警告。

`font-variation-settings` 的过渡在视图里给构建警告。要按轴动画，用 Rust 的 `TransitionBuilder::font_axis`：每个轴一条 Layout-class track。

各属性走哪一类动画（Compositor、Paint、Layout）、fallback 和性能含义见 [架构](architecture.md)。

**增删节点时重算什么。** 什么都不重算。视图运行时不匹配选择器，增删一个子节点不会让兄弟、父节点或祖先重新级联。

视图里的元素只按自己写的 class 组合（含 `class_when` 当前打开的那些）取样式。`:first-child`、`:nth-child()`、`+`、`~`、`:empty`、`:has()` 这类要看树结构的选择器，在视图里只给构建警告。

**挂载带样式的节点要多少。** `stylesheet!`、`view!` 和 `.vue` 的 `<style>` 在构建期编译成 `StylePatch`。运行期不解析 CSS，也不逐元素匹配选择器。一个元素的 class 组合第一次出现时，从样式表里挑出它能命中的规则，得到一个 `StyleSite`。之后同一组合、同一初始布局的元素，直接复用这处已经合成好的样式。

所以挂载成本不随样式表有多少条规则变，也不随一个元素命中多少条规则变，更不随已挂载的节点数变。

实测（#348）：每行一个行容器、一个文本、一个按钮，各带一个 class，共 3 个节点。release，Apple M4，2026-10-30，负载约 3。计时包括建视图、合成样式、提交到 `UiWorld`，不包括之后的布局和绘制。

| 样式表 | 每个元素命中 | 冷挂载 4,000 行（12,001 节点） | 每行 | 再追加 500 行，每行 |
| --- | ---: | ---: | ---: | ---: |
| 10 条规则 | 1 / 4 条 | 约 37 ms | 8.9–9.0 µs | 9.2–9.3 µs |
| 100 条规则 | 1 / 34 条 | 约 37 ms | 9.0–9.1 µs | 9.4–9.5 µs |
| 1,000 条规则 | 1 / 334 条 | 约 37 ms | 9.0–9.2 µs | 9.0–9.4 µs |

同样的行不带 class，每行约 8.5 µs。往已有 0、1,000、2,000、4,000 行的列表里追加 500 行，耗时一样。

**条件 class。** 每个元素再加一个 `class_when`，所有行共用一个信号；一半规则要这个 class 打开才命中。

- 冷挂载每行 9.9–10.1 µs，追加每行 10.1–10.4 µs。比只用固定 class 多约 10%，多出的是每个元素一个绑定。
- 打开这个信号后，4,500 行共 13,500 个元素一起换样式。一次 `flush_reactive` 约 9.4 ms，每行约 2 µs。

10、100、1,000 条规则下这几个数都一样：每组「class 组合 × 打开的条件 class」只合成一次。

**`@container` 规则。** 一半规则写在一个有三个宽度区间的 `@container` 块里，每个元素因此带上一条响应式规则。冷挂载和追加每行都在 10.0–10.6 µs。

插入的行只沿自己的子树去找最近的查询容器，不再把页面上所有响应式规则扫一遍。此前每插入一行都要扫一遍，4,000 行冷挂载要 240 ms，追加每行 120 µs。门禁是 `world::responsive::tests::inserted_rows_find_their_container_without_visiting_every_rule`。

**卸载。** 整张列表（4,500 行）一次卸载，每行约 2 µs，带条件 class 时约 2.5 µs。

逐行卸载读同一个信号的行时，订阅列表在失效的订阅者占到一半时才清理一次，所以每行的成本不随行数涨。门禁是 `view::reactive::tests::disposing_readers_one_by_one_compacts_in_amortized_constant_time`。

**预算：** 挂载 4,000 个带样式的节点在 100 ms 以内。用 `cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --profile-styled-mount` 复测。

门禁 `view::style::site_tests::equal_elements_compose_once_however_many_mount` 守住"相同的元素只合成一次"：再挂 300 行相同的行，合成次数不变。

**`@import`。** 视图的构建期编译不给加载器，`<style>` 里的 `@import` 一律不加载，`nana://res/` 也一样。下面是引擎在调用方给了加载器时的行为。

它解析 `url(...)` 和引号路径。相对路径相对导入方，或相对 `FsStylesheetLoader` 的 `base`。canonicalize 之后必须落在 `base` 下。`stylesheet_base_from_href` 从样式表的地址推出 `base`。没有加载器时，`@import` 跳过。不扫进程 cwd。

给了 `PackagedStylesheetLoader` 时，`nana://res/` 打包样式表照常加载。它内部的相对 `@import` 在包内解析。

`http(s)`、`data:`、越狱、超过 1MB、带 `layer` 或 `supports()` 的 prelude，一律 fail closed。记 skip，不加载。

写在普通规则之后的 `@import`，按 CSS 忽略。写在 `@media` 块内的 `@import` 同样忽略。

`file://` 会 percent-decode。非本机 host 跳过。

循环和深度有上限，是 16。导入规则并进同一份 cascade。已经解析的导入按 canonical href 缓存。视口变化不重解析。

**`@media`。** 子集是 `min/max/width`、`min/max/height`（px）、`orientation`、`prefers-color-scheme`，以及 `screen`、`all`、`print` 类型。

条件匹配时，`ParsedStylesheet::flatten` 把规则并进 cascade。换视口或主题，用新的 `MediaEnvironment` 再 flatten 一次即可，不重扫 CSS 文本。视图里 `@media` 是构建警告。

**`@font-face`。** 引擎只解析 `font-family`、`src`、`font-weight`（含 `200 700` 这种范围），不注册字体。视图里是构建警告。

要用自己的字体，在宿主里调 [`register_host_font_face`](../../crates/nana-ui/src/nana_text.rs)：一个 family 加一段字重范围，登记成一个 face，写入共享的 `FontSystem`。和 `bundled-fonts` 是同一套库。要成组替换，用 `HostFontScope`。这不是 CSSOM 的 `FontFace`。

宿主手里已经有解析好的 face，可以交给 [`ingest_host_font_faces`](../../crates/nana-ui/src/font_face_ingest.rs)：`HostFontFaceSpec` 的 `weight` 和 `weight_end` 就是 `font-weight` 的范围，整段登记。它读 jail 内的本机文件、`nana://res/` 和 `data:`。CSS 引擎的 `css_at_rule::load_font_face_bytes` 只读文件和资源包，不认 `data:`，因为 `data:` 不是文件路径。内联字体由 ingest 自己解码。

单个字体文件的读上限是 8MB。

**`@supports`。** 它在解析期求值。匹配，就把内部规则并进同一份 cascade，所以视图里块内的 class 规则照样编译。

谓词子集是引擎已有的。`display: flex|grid|block`，以及已经解析的其它 `display` 关键字。`color` 是 `parse_css_color` 能解析的值。`width` 是 `LengthSpec::parse` 能解析的值。可以加 `not`、`and`、`or`。

未知谓词整块 fail-closed，计入 `skipped_at_rules`。例如 `selector()`、`lab()`、`display-p3`，以及未列入的属性。

**`@container`。** 引擎只把它编译成数据，不去量容器；容器尺寸由 Runtime 的响应式规则读（见上文「响应式规则（#265）」）。

prelude 是 `[名字] 条件`：
- 条件可以用 `and`、`or`、`not` 组合，同一层不混用 `and` 和 `or`。
- 特性是 `width`、`height`、`inline-size`、`block-size`，可以带 `min-` / `max-`，也可以写成区间 `(width < 480px)`、`(400px <= width < 800px)`。
- 长度只认 px，以及不带单位的 0。

每条查询编成一条轴上的半开区间。`max-width: 480px` 含 480，上界是 `480f32.next_up()`；`<` 不含边界。

元素命中的 `@container` 规则把容器尺寸切成桶，最多 16 个断点。每个桶里成立的规则按 cascade 顺序（`!important`、特异度、源序）并入，得到这个桶的整份样式。它和基础样式不同的字段就是这个桶的变体，交给 Runtime 的规则：`ResponsiveContainer::Nearest { name }`，找最近的、名字相同、回答这条轴的容器。

`container-type`（`normal`、`inline-size`、`size`）、`container-name` 和 `container` 简写写进 `LayoutStyle`。查询容器不做尺寸包含，尺寸仍随内容。

下面这些一律 fail closed，计入 `skipped_at_rules`：
- 其它单位，`aspect-ratio`、`orientation`、`style()`、`scroll-state()`；
- 一条查询里有两条轴；
- 逗号列表，只有名字，嵌套的 `@container`。

一个元素命中的规则问到两个容器或两条轴，或者断点超过 16 个时，它的 `@container` 规则全部不生效。引擎计入 `container_queries`，视图里给构建警告。

块里只有普通样式规则生效。`:hover`、伪元素、`transition`、`@keyframes`、`@font-face` 写在里面不生效。

**`@layer`。** `@layer name { }` 和匿名的 `@layer { }` 把内部规则按作者源序并进 cascade，并记下层名（`ParsedStylesheet.layer_names`）。

**没有** cascade-layer 优先级。unlayered 并不压过 layered。`!important` 也不按层反转。

`@import … layer()` 和 `supports()` 仍不加载。

引擎的 `rebuild_layout_style` 先写样式表的普通声明、prop 和 inline 的普通声明，以及 `nana-*` class hints；再写样式表 important；最后是 prop 和 inline 的 important。所以样式表的 `width:80px !important` 不会被普通的 `width:200px` prop 盖掉。

视图不走这条路，也不认 `nana-*` class hints：class 只按 `<style>` 里写的规则取样式。

自定义属性会去掉 `!important`。和普通声明同一套 `split_important_flag`。所以 `--gap: 8px !important` 经 `var(--gap)` 得到 `8px`。不是带着标志，无法解析。

## 不要指望的

这些你不要指望：

- `sideways-rl` 和 `sideways-lr`。
- 完整的 Tate-chu-yoko。
- 竖排 ruby。
- 竖排着重号（`text-emphasis-position` 的 `right` / `left` 会被解析，但竖排不画记号）。
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
- 完整 IFC。行内原子按基线对齐、行盒有 strut，但没有 `vertical-align` 的其他取值，也没有行内
  拆箱。行盒在后代里的盒子里，flex / grid 容器、等宽分行或有自动 margin、浮动的块级流不推导首末行位置，
  仍按自身字体近似。
- 轨道列表 token 形式的 `subgrid`。
- 用 `position: fixed` 当 Dialog 或 Drawer。走 Nana 的浮层。
- 自定义 `url()` 光标图。
- 未知的 `user-select` 关键字。
- 完整的 CSS Highlight API。选区只有 `selection_background` 和 `selection_color` 两个字段，CSS 的 `::selection` 在视图里是构建警告。
- `::before`、`::after` 生成盒，`content`、`attr()`、计数器、`quotes`。
- `-webkit-app-region` 和 `app-region`。任意盒不能当标题栏。拖窗只走 `AppTitleBar`，再到 `nana-window`。你写了也不会拖窗。
- `overflow:auto` 的自定义滚动条铬。走 `ScrollView`。
- `<iframe>` 加载。见 [应用内浏览器](gpu.md#应用内浏览器)。这一条未实现。
- 浏览器式 `<video>` 的解码和播放。宿主推帧和 poster 规则见 [实时画面](gpu.md)。
- 把 `<canvas>` 当成 2D 位图，或当成浏览器的 2D / WebGPU 上下文。GPU 画面走 [实时画面](gpu.md) 的 `GpuTextureView`（`"nana.host-texture"`）。那不是 Chromium 2D。
- 父级 CSS `perspective` 属性，以及 `preserve-3d`。元素自身的 `perspective()` 加 `rotateY` 已经会画。

畸形的 `font-variation-settings` 仍 fail-closed。字体没有的轴，在 shaping 时跳过，不改写成 `wght`。

Android 和 iOS **不是**产品布局目标。两个实验宿主（[移动宿主](mobile-host.md)）仍用同一份 `LayoutStyle`、Runtime 和 UiScene，没有第二套移动端布局引擎。见 [Android](android.md)、[iOS](ios.md)。 HarmonyOS / OpenHarmony 同样共用这份布局，只是编译档，见 [OHOS](ohos.md)。

`:has`、`::before`、`::after`、`::placeholder`、`::selection` 只是引擎能解析和匹配，视图里都是构建警告，见上文「选择器」。

`writing-mode: horizontal-tb | vertical-rl | vertical-lr` 已经接入逻辑边，也接入了 IFC 和 flex 行的 inline 轴。见上文「能用的」。

主题色走控件和外观设置。不要用任意业务色去改框架 token。

未知声明不进 Style Model。视图里会给构建警告。不要假设「写了就能在某处生效」。

## 和 Rust 布局的关系

CSS 解析出来的，是同一份 `LayoutStyle`。

L3 用 [`Stack`](rust-layout.md) 表达这份合同。用 `from_layout`，或用字段 builder。

真正算盒子的，是 Runtime 的 `RuntimeLayoutEngine`。产品帧走 `RuntimeDocument::flush`。

## 页面与卡片的留白

NanaUI 的容器边距归属见 [Rust 布局合同](rust-layout.md#浮层定位与容器内边距归属)。

CSS 只覆盖你声明的边。`padding: 0` 明确清零。

兄弟的 margin 和父级的 gap 相加。不自动去重，也不自动折叠。
