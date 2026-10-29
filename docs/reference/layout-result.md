# 布局结果合同（LayoutResult）

`UiWorld` 为每个完成布局的节点发布一份不可变的 `LayoutResult`。它是面向绘制与输入的盒子几何权威；节点上的 `layout_box` 只是其 `bounds` 字段的兼容投影。

不生成盒子的节点不发布结果：`hidden` 或 `display: none` 的节点、关着的菜单里的项、不活动的叠层分支、停放的子树，以及它们里面的节点。节点变成这样时，旧结果在下一次发布时撤掉，与冷布局相同（Issue #386，门禁在 `world/issue386.rs`）。触发式菜单关上后，退场动画还在播放时面板仍在，条目照旧按打开时布局、算进触发器的 `overflow`；退场结束后的那一帧撤掉它们，与冷布局的关闭菜单相同（Issue #387，门禁在 `world/issue387.rs`）。

## 结构与代际

每份 `LayoutResult` 携带：

- 边框盒（border box）、内边距盒（padding box）与内容盒（content box）；
- 首行基线（first baseline）与末行基线（last baseline）；
- 溢出范围（overflow extent）与滚动范围（scroll extent）；
- 子节点放置坐标（child placements）与类型化的布局片段/部件（typed layout fragments / parts）；
- clip 链（`clip`、`clip_parent`）与包含块（containing-block）引用、依赖元数据；
- 产生该结果的单调递增布局代际（`layout_generation`）。

Runtime 帧更新（`RuntimeFrameUpdate`）对外暴露这一 `layout_generation`，使 Scene 提取、命中测试、无障碍投影和滚动消费方能够对齐各自的投影版本，而不必依赖粒度更粗的 `UiWorld` 变更代际（mutation generation）。

`clip` 是最近一个够得着本节点的溢出容器，链沿该容器自己的 `clip` 往上；`clip_parent` 是本节点取裁剪和滚动的节点：一般是父级，包含块在静态溢出容器之外的 `absolute` 节点是它逃出的最外层容器的父级，`fixed` 节点为 `None`（Issue #201，规则见 [布局（CSS 子集）](layout.md) 的「clip 链」）。Scene 提取、命中测试与无障碍投影都按 `clip_parent` 挂节点，被跨过的祖先只传变换，不传裁剪和滚动；无障碍节点的 `clip` 是这条链留下的视口区域。祖先改变定位或开始、停止裁剪时，下面节点的这两项随下一次发布更新。滚动范围按同一条链算：会裁剪或滚动的节点，`overflow` 只并入 `clip_parent` 是它的子项，`scroll_extent` 和滚动度量只含它够得着的后代，逃出它的 `absolute` 盒子算进它们的包含块，`fixed` 盒子不算进任何节点。

`ExtractedNode::layout_result` 共享这一不可变快照。Scene 的 Sticky placement 从中读取包含块、视口和滚动范围，不重新测量内容；滚动偏移仍读取当前的 `ExtractedNode::scroll_offset`，不会把呈现位置写回快照。

`layout_generation` 独立于世界变更代际：**只有当布局输出实际变化时它才递增**。因此悬停（hover）、不透明度（opacity）、变换（transform）及其他纯绘制改动不会触发任何基于布局派生的几何重建。影响布局的写入会在下一次发布结果前清除受影响节点、后代及依赖祖先的缓存，而拓扑变更仍保持稳定的节点身份（`StableNodeId`）。

## 格式化上下文与子片段

`formatting_context` 记录该节点排列其子节点（或自身文本）时实际采用的格式化上下文（Issue #197），并决定每个子片段的具体类型：

| `formatting_context` | 子片段类型 |
| --- | --- |
| `Flex` | `FlexChildPlacement` |
| `Grid` | `GridChildPlacement` |
| `Inline` | `InlineAtomic` |
| `Flow` / `Overlay` | `ChildPlacement` |

在布局尚未记录上下文之前，仅由节点的 `display` 给出暂定上下文。若某次上下文切换碰巧让所有盒子留在原位置，仍然会发布一份新的 `LayoutResult`——因为上下文和片段类型已经改变；子节点的身份保持不变。

measure 和 placement 的 child flow、脱流及浮动分类直接消费 `ParticipationKind`；检查器读取同一个分类。流参与方式改变会失效父级的 contribution/placement plan，即便 child identity 和自身尺寸都未改变。仅由父级 context/constraint 变化触及的子项仍可复用兼容约束下的 intrinsic facts；直接内容变更及其 measure 祖先继续失效，跨 context 计数按 Flex/Grid/Inline 等实际 context 记录。

## 与 ComponentGeometry 的关系

`ComponentGeometry` 仍是组件语义部件与非盒式绘制数据（例如缩放拖柄、图表路径、编辑器装饰线）的兼容表面。Button、Chip、ActionMenuItem 的文字、图标、hint、spinner 盒子与 Checkbox 的指示框、标签，Switch 的轨道、标签、hint，StatusBadge / ValidationMessage 的圆点、标签，SelectionOption 的圆环（`Indicator`）、图标、标签，裸 Toast 的记号、标题、说明（`Detail`）、关闭目标与装配后 Toast 的记号，菜单触发器的标签与图标由 Foundation 的轻量 parts 发布，几何投影读取 `LayoutResult.parts`，不再次分配位置。其余组件的分类与待迁移边界见[组件布局审计](component-layout.md)。

组件内容部件求解时放在内联的定长槽里（`layout_engine::parts::ContentParts`，`CONTENT_PART_SLOTS` 为四格：Toast 的指示、标题、说明、关闭按钮恰好四个），不为节点分配；发布到 `LayoutResult.parts` 的只有存在的部件，所以槽数既不改变已发布结果的形状，也不增加节点常驻内存。读部件的地方（几何投影、段落预算、native 文本复用判定）都按部件类型取，不按槽位。

## 参见

- [布局引擎（架构学习）](/learn/runtime-layout) · [Runtime 与 Scene](runtime-scene.md) · [布局（Rust）](rust-layout.md) · [布局（CSS 子集）](layout.md)

Native Inline 的 TextLine/TextRun/InlineAtomic fragments 携带逻辑 source byte range；`LayoutResult.participation` 随父 context 更新，供 Scene 消费。详见 [InlineContent](inline-content.md)。

组件部件补充：ListItem 的 `Note` / `Detail`、LabeledValue 的 `Label` / `Value` 与 Card 的 `Header` / `Spinner` 均在同一发布批次中生成。Card 的 primary `TextContent` 与 `Header` 指向同一标题预算，普通 `ComponentContent` 指向 body；renderer 不能从 body 的位置重新推算标题。标题的 native baseline 同时供 intrinsic 测量和 LayoutResult 使用。
