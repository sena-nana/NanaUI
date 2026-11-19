---
title: 布局引擎
---

# 布局引擎内部：`RuntimeLayoutEngine`

本章拆解 `nana-ui-runtime` 的布局引擎：计算样式如何进入测量、容器如何选择格式化上下文、增量布局靠什么机制做到「只重排受影响的子树」、文本测量与盒布局如何反馈收敛，以及为什么滚动偏移绝不改写布局盒。读完后你能搞懂：`layout_document_with_frontier` 一次增量遍历的完整路径；`LayoutFrontier` 依赖图与各类失效种子的传播规则；`IntrinsicMetrics` 与 `LayoutResult` 两个输出合同的分工；以及视口缩放（resize）、文本折行反馈与滚动各自触发的精确失效范围。

## 职责与边界

- **无状态算法所有者 + 跨帧保留缓存**：`RuntimeLayoutEngine` 本身是零字段单元结构（`#[derive(Default, Clone, Copy)]`，`crates/nana-ui-runtime/src/layout_engine.rs:88-89`），消费 `UiWorld` 中已解析的 `LayoutStyle` 与文本整形度量，产出原子的布局写回；跨帧状态全部存放在按文档隔离的 `RetainedLayoutCache`（`layout_engine.rs:719`）中。
- **输入边界**：运行时不解析 CSS；进入布局引擎的是 Style Model 数据（`nana_ui_core::LayoutStyle`，挂在 `NodeRecord.resolved_layout` 上）。
- **输出边界**：面向绘制、命中测试、无障碍与滚动的规范几何权威是不可变的 `LayoutResult`（`crates/nana-ui-runtime/src/layout_result.rs:113`，Issue #203）；节点上的 `LayoutBox` 只是其 `bounds` 的兼容投影。
- **通过 `MutationQueue` 写回**：布局引擎不直接绕过世界篡改节点表，而是仅为几何实际发生变化的盒子入队 `MutationQueue::write_layout`，并在整批提交后统一发布 `LayoutResult`。

## 依赖与邻居

| 邻居模块 / Crate | 协作关系 |
| --- | --- |
| `nana-ui-core` | 提供 `LayoutStyle`、`LengthSpec`、`FlexDirection` / `GridTrack` / `GridPlacement`、失效分类 `LayoutInvalidation` / `LayoutDependencyFootprint`，以及动态自适应布局的 `solve_deficit` / `EnvelopeShape` |
| `nana-text` | 提供文本叶子节点的固有宽度与行排版度量；引擎通过 `TextLayoutCache` 与 `UiWorld` 侧表读取 |
| `UiWorld` | 提供输入树与写回目标；增量失效种子由 `commit` 生成（`pending_layout_invalidations`） |
| `tools/css-parity` `measure_layout` | 通过 `RuntimeLayoutEngine::layout_style_tree` 直接复用同一套算法（`layout_engine.rs:85-87`） |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `RuntimeLayoutEngine` | `crates/nana-ui-runtime/src/layout_engine.rs:89` | 布局算法入口（全量 / 增量 / 样式树测量） |
| `LayoutViewport` | `crates/nana-ui-runtime/src/layout_engine.rs:50` | 宿主传入的逻辑视口尺寸（`width` / `height`，构造时钳制为非负有限值） |
| `EstablishedContext` | `crates/nana-ui-runtime/src/layout_engine/context.rs:13` | 容器为其流内子节点建立的格式化上下文（`Overlay` / `Layers` / `Grid` / `Flex` / `Inline`）与 `grid_2d` 标志 |
| `IntrinsicMetrics` | `crates/nana-ui-runtime/src/intrinsic_cache.rs:57` | 子树固有尺寸贡献：`min/max inline·block`、`preferred`、首末基线、宽高比、`truncatable`、`generation` |
| `UsedSize` | `crates/nana-ui-runtime/src/intrinsic_cache.rs:21` | 在特定包含块约束下解析出的主轴与交叉轴已用尺寸 |
| `LayoutFrontierSeed` | `crates/nana-ui-runtime/src/layout_frontier.rs:19` | 增量种子：目标节点 + `LayoutInvalidation`（失效类别、字段掩码、依赖足迹） |
| `LayoutDependencyGraph` | `crates/nana-ui-runtime/src/layout_frontier.rs:61` | 保留依赖图：`parents` / `children` / `contexts` 三张双向邻接表 + 布局隔离集 |
| `LayoutFrontier` | `crates/nana-ui-runtime/src/layout_frontier.rs:318` | 单次增量遍的 `measure` / `placement` / `context` / `writing` / `scroll` 工作集 |
| `RetainedLayoutCache` | `crates/nana-ui-runtime/src/layout_engine.rs:719` | 跨帧布局记忆：每文档的盒子快照、内蕴度量缓存、容器放置计划与测量计划 |
| `LayoutResult` | `crates/nana-ui-runtime/src/layout_result.rs:113` | 单节点规范几何快照（border/padding/content box、基线、滚动范围、子片段、裁剪引用、代际） |
| `TextLayoutKey` / `TextLayoutCache` | `crates/nana-ui-runtime/src/text_layout_cache.rs:17` / `:89` | 容量为 2,048 条的有界 FIFO 文本度量缓存（键内包含宿主 `FontGeneration`） |
| `Extent` / `MaxTree` | `crates/nana-ui-runtime/src/world/scroll_bounds.rs:13` / `:60` | 滚动容器内容范围的区间并集与 $O(n)$ 线段树聚合 |

## 内部结构

### 1. 样式解析与格式化上下文选择

`UiWorld::resolve_style`（`crates/nana-ui-runtime/src/world/style.rs:87`）自根向下解析计算样式，对无本地覆盖的子节点直接共享父节点的 `Arc<ComputedStyle>`（`publish_inherited_computed_style`，`:12`）；布局引擎读取的是叠入了响应式容器桶与Layout类动画后的 `motion_layout(id, &resolved_layout)`（`:106-108`）。

格式化上下文由 `establish_context`（`crates/nana-ui-runtime/src/layout_engine/context.rs:29`）统一判定，已收敛为纯五上下文模型：`Overlay` → `Layers` → `Grid` → `Flex` → `Inline`（旧式 CSS `Block` / `FlowRoot` / `InlineBlock` 布局路径已移除）。测量与放置两个阶段调用同一个函数，确保对同一容器绝不会选出两套不同的算法。各上下文求解器与收集器按模块拆分：

- **流内子项收集（`flow.rs:24`）**：`collect_flow_children_reporting` 收集流内子列表（排除 `Overlay` / `PositionSpec::Absolute|Fixed` 脱离流子节点），并上报 `display: contents` 等后代结构依赖；
- **Flexbox（`flex.rs:25`）**：`distribute_flex_main` 执行主轴空间分配与交叉轴拉伸；
- **Grid 与独立 `subgrid`（`grid.rs:23`）**：`Grid2DLayout` 求解二维显式轨道，并支持独立轴向或双向 `subgrid`（`InheritedGridTracks`，`:16`）；
- **行内折行装箱（`inline.rs:6`）**：`pack_wrap_lines` 处理行内级原子盒换行；
- **动态自适应布局（`dynamic.rs:1-18`，Issues #209/#212/#213）**：配置了 `AdaptationProfile::solve_overflow` 的容器在单行空间不足时，通过 `DynamicPass::envelope` 与 `solve_deficit` 按代价从低到高向弹性子项借出空间，且包络（`RetainedEnvelope`）跨 resize 保留复用。

### 2. 两阶段遍历、无界深度栈控与三层跨帧缓存（`measure_node` → `place_node`）

每遍布局由自底向上的**内蕴测量（`intrinsic_size` / 显式堆工作栈迭代的 `measure_node`，`measure.rs:993`）**与自顶向下的**节点放置（`place_node`，`placement.rs:6`）**组成。`measure_node` 全程运行在显式 `Vec` 堆工作栈上（`0 B` 递归栈增长、零 `stacker` 依赖）；放置阶段则通过 `recursion::SPAN` 与 `Boundary` 队列（`recursion.rs:18-28`）分段展开，500 层嵌套实测峰值栈仅 `52 KiB`。`RetainedLayoutCache`（按文档隔离）通过三层缓存避免重复计算：

1. **跨上下文共享的内蕴事实缓存（`intrinsic_cache.rs:1-13`）**：缓存键只包含子树内容/样式身份与**约束类别**，不包含外层容器的格式化上下文类型——因此无论是 `Flex`、`Grid`、`Layers` 还是 `Inline` 父容器询问同一个子树的固有尺寸，都能直接命中同一条缓存。
2. **容器放置计划（`ContainerPlan`）与测量计划（`MeasurePlan`）**：`place_node` 重放已缓存的容器计划，通过 `PlanCheck`（`placement.rs:33-43`）判定 `Unchanged`（直接跳过）、`ChangedFrom(idx)`（从首个变化子项起重放）或 `Stale`（全量重算）。容器计划遵守 **Gate E（一容器一计划，重复编辑原地替换而非追加，`layout_engine.rs:454-458`）**；测量计划按 `(容器, 约束槽)` **合并而非覆盖**（因为同一容器常在单遍内被主轴和交叉轴两种约束测量，`:480-491`），并对本遍未量到的槽位主动检查 `plan_missed_change`（`:493-509`）。
3. **同源全量快照直通**：若请求全量布局但 `layout_source_epoch` 与视口均未变化，直接整块返回上一次的输出快照（`:214-246`）。

### 3. 增量前沿：`LayoutDependencyGraph` 与 `LayoutFrontier`

增量入口 `layout_document_with_frontier`（`layout_engine.rs:150`）不采用粗暴的「脏节点一路传染到文档根」，而是基于按依赖足迹（`LayoutDependencyFootprint`）加权的依赖图传播：

- `LayoutDependencyGraph`（`layout_frontier.rs:61`）维护父子双向边（`add_parent_dependency_split` 将上行度量依赖与下行约束依赖分开记账，防止仅影响位置 `placement-only` 的种子把整条祖先链拖进重测，`:103-116`）、同上下文兄弟边（Flex 行 / Grid 轨道，`:120`）与布局隔离边界（`:140`）。
- 只有当子节点的输出变化（由 `LayoutResult::metric_delta` 分类为 `INTRINSIC_INLINE/BLOCK`、`BASELINE`、`USED_SIZE`、`PLACEMENT`，`layout_result.rs:205`）命中了父边实际声明消费的足迹时，失效才会继续向外扩散。
- **视口缩放（Viewport Resize）精确播种**：每个盒子记录自己实际读取的视口轴 `viewport_basis`（`vw` 只记宽、`vh` 只记高、`position: fixed` 记双轴、普通盒子记 `0`，`layout_engine.rs:64-77`）。当窗口仅改变高度时，`UiWorld::viewport_resize_seeds`（`world.rs:3800`）不会把只用了 `vw` 或不依赖视口高度的子树拉入重排。

### 4. 文本测量反馈环与滚动范围模型

布局定出容器宽度后可能改变多行文本的折行结果，进而改变容器高度。`RuntimeDocument::flush` 将这一「布局 → scoped 重整形 → 再布局」的反馈环限制在至多 `POST_LAYOUT_SHAPE_PASSES = 4` 轮（`runtime_document.rs:48`），一旦没有产生新的布局种子即刻收敛。

对于滚动容器（`world/scroll_bounds.rs:1-4`）：**可滚动内容范围（scroll extent）是纯布局产物（由 `MaxTree` 线段树以 $O(n)$ 聚合后代盒子并集），而滚动偏移（`scroll_offset`）是绘制与命中变换**。调用 `scroll_to_clamped`（`world/mutation.rs:2758`）只更新 `NodeRecord.scroll_offset` 与命中索引滚动偏移，并标记 `DirtyMask::INPUT | DirtyMask::RENDER`，**完全不改写后代节点的 `LayoutBox` 或 `LayoutResult`**。

## 协作与数据流

```text
commit（SetStyle / Insert / …）──生成──▶ LayoutFrontierSeed（类型化失效种子）
RuntimeDocument::flush ──调用──▶ AppContext::layout_document_with_frontier
    ├─ 合并待处理种子 +（若视口变化）补充 viewport_resize_seeds
    ├─ 沿 LayoutDependencyGraph 按足迹闭包求得 LayoutFrontier
    ├─ intrinsic_size（命中内蕴缓存 / 合并测量计划）→ place_node_scoped（重放容器计划）
    ├─ 仅对几何变化的盒子入队 write_layout → commit → publish_layout_results
    └─ scoped 文本重整形反馈（≤ 4 轮，无新种子即收敛）
输出：不可变的 LayoutResult（供 Scene 抽取、命中测试、无障碍与滚动共同消费）
```

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-runtime/src/layout_engine.rs` | 引擎总入口、`LayoutViewport`、`RetainedLayoutCache`、Gate E 计划发布 |
| `crates/nana-ui-runtime/src/layout_engine/context.rs` | `establish_context`：格式化上下文判定 |
| `crates/nana-ui-runtime/src/layout_engine/measure.rs` | `intrinsic_size` 与显式堆工作栈迭代测量 `measure_node` |
| `crates/nana-ui-runtime/src/layout_engine/placement.rs` | `place_node`、容器计划重放与 `PlanCheck` |
| `crates/nana-ui-runtime/src/layout_engine/recursion.rs` | `SPAN` 与 `Boundary` 分段放置栈预算控制（500 层 `52 KiB`） |
| `crates/nana-ui-runtime/src/layout_engine/{flow,flex,grid,inline,overlay,dynamic}.rs` | 流内收集、Flexbox、二维 Grid / 独立 `subgrid`、行内换行、`Overlay` 与动态自适应求解器 |
| `crates/nana-ui-runtime/src/layout_frontier.rs` | `LayoutFrontierSeed`、`LayoutDependencyGraph` 与 `LayoutFrontier` |
| `crates/nana-ui-runtime/src/intrinsic_cache.rs` | `IntrinsicMetrics`、`UsedSize` 与跨上下文共享缓存键 |
| `crates/nana-ui-runtime/src/text_layout_cache.rs` | 有界 FIFO 文本度量缓存 |
| `crates/nana-ui-runtime/src/layout_result.rs` | `LayoutResult`、`LayoutFragment` 与 `metric_delta` |
| `crates/nana-ui-runtime/src/world/scroll_bounds.rs` | `Extent` 与 `MaxTree` 滚动内容范围聚合 |

## 延伸阅读

- [保留树：UiWorld](/learn/runtime-tree)：布局失效种子从何而来（`commit`），布局完成后由谁消费（`flush`）
- [文本引擎](/learn/text-engine)：`nana-text` 如何将整形与行排版解耦以配合窗口缩放
- [布局结果合同（参考）](/reference/layout-result) · [布局（Rust 路径）](/reference/rust-layout)
