# Runtime 的脏帧：一次改动要付多少钱

`input-cost.md` 最后停在一句话上：Vue 侧"改动过的帧"的成本主体是
`RuntimeDocument::flush`，2,000 节点的树上，**2 个** widget 变脏要 1.51 ms；同一棵树什么
都不脏时是 0.0001 ms。也就是说"没活干就早退"挡得住，"有活干"那条路按树大小收费。

这份文档把那条曲线量完，并结掉其中两笔账。测量代码是
[`crates/nana-ui-scene/src/bin/nana-dirty-frame-benchmark.rs`](../crates/nana-ui-scene/src/bin/nana-dirty-frame-benchmark.rs)
（feature `benchmark`，纯 Rust，不需要 V8），原始报告在
[`performance-data/runtime-dirty-frame-2026-09-08/`](performance-data/runtime-dirty-frame-2026-09-08/)。

结论先说：

- **纯绘制的改动本来就是 O(改动量)**，与树多大无关。它不是问题，之前没人量过。
  （注意：Vue 的 hover **不**走这一档，见"端到端"一节。）
- **一旦改动牵动布局，帧就变成 O(总节点数)**，而且工作计数器全部显示"只碰了 3 个节点"。
  两处原因，都已修：无障碍投影按**被调度的布局脏集**播种，以及布局引擎对每个兄弟节点
  重复解析 effective style。2,002 个节点上 0.58 → 0.20 ms（**2.9 倍**），8,002 个节点上
  2.69 → 0.87 ms（**3.1 倍**）。
- 端到端：2,000 行的 Vue `reactive` hover 每事件 8.87 → 6.66 ms，其中 settle
  2.66 → 1.64 ms（**降 38%**）。
- **布局引擎的兄弟扫描**：第二、三轮解决掉了。现在凡是"真正欠的工作是常数"的情况，代价
  就是常数——改动被含在定尺寸子树里、以及尾部子节点改尺寸，502→8,002 个节点都在
  0.008–0.011 ms。只有"下面每一行真的要移位"那种仍是 O(N)，那是应付的账。
- **Vue 的 settle 花在哪，量出来了**：73% 在 Vue 层。再往下追，用"跑 500 行和 2,000 行
  看哪些随文档增长"钉死了性质——**每个指针事件有 4 处 O(文档)**，而投影集合和调用次数
  都是常数。最大一处是 `try_bind_registered_component`：调用次数恒为 5，每次却 O(文档)。
- **内容驱动尺寸的容器**：第五轮补上了 `ContainerPlan` 的测量侧对应物 `MeasurePlan`。
  8,002 个节点上 1.41 → 0.011 ms（**128 倍**）。但**端到端一开始是 0**——真实 Vue 树里
  "默认的列"有两种拼法，每帧把计划打死。追下去发现根因是 `Stack::from_semantic` 给
  `nana.column` 播了一个**不改变任何布局**的 `direction` 种子，与 CSS cascade 每帧互相
  覆盖 2,003 个节点。改成比较布局等价、并去掉那个种子之后：`reactive` 每事件
  4.96 → 3.85（**−22%**）、settle 0.763 → 0.409（**−46%**），
  `reactive-components` 1.87 → 1.04（**−44%**）、settle 0.642 → 0.331（**−48%**），
  一次只改颜色的 hover 的布局阶段**归零**。再顺着两个规模的分段追下去，还发现
  `bind_semantic_slots` 每事件为那个 2,000 孩子的容器扫一遍 `data-slot`，**而它绑的
  `Stack` 根本不读 slot**；把"这个组件读不读 slot"声明进注册表之后，settle 再降到
  **0.303 / 0.281**。
- 附带发现：**13 个 `FrameStage` 漏计了一次大回流帧的 45%**。已修，但这让 Extract 的历史
  基线不可比，见最后一节。

## 一个数据点不够：必须同时扫"脏了几个"和"一共几个"

只有 N=2 的那一个数所以定位不了，是因为它把两件事混在一起。基准扫的是二维网格：
`rows ∈ {250, 500, 1000, 2000, 4000}`（每行一个 div 加一个文本子节点，即 2N+2 个节点）
× `dirty ∈ {1, 2, 8, 32, 128}`，每帧对这些行做一次改动，读 `last_frame_profile()` 的
13 个分段和 `last_work_counters()`。

### 两个必须分开的形状

**改动的种类**（`--shape`）：

- `paint`：换一个背景语义色。脏 STYLE 与 RENDER，不调度布局。hover 高亮在 Runtime 边界上
  就是这个。
- `layout`：改行高。脏 STYLE **并且**调度布局，而布局失效会向祖先传播。

**改动的位置**（`--position`）——这一条是我第一版基准的错误，必须写下来：

第一版把脏行放在文档**开头**。改第 0 行的高度，它下面每一行都真的要下移，所以
O(总节点数) 在那里是**应付的账**，量出来什么也不说明。改**末尾**的行则什么都不移动：
此时任何仍随文档增长的成本都是过度失效。

`tail` 与 `head` 两列因此必须分开读，下面所有"过度失效"的结论都来自 `tail`。

### 一个会骗人的读数

`last_frame_profile()` 和 `last_work_counters()` **保留最后一次非空闲的值**。在空闲帧上读
它们会拿到挂载帧的数据。基准因此在每个采样上断言 `!update.is_idle()`。

## 结果一：纯绘制的改动早就是 O(改动量)

`paint`，p50 ms，行数从 250 到 4,000（502 到 8,002 个节点）：

| 脏节点 | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.0018 | 0.0018 | 0.0018 | 0.0019 | 0.0018 |
| 2 | 0.0029 | 0.0030 | 0.0030 | 0.0030 | 0.0030 |
| 8 | 0.0103 | 0.0106 | 0.0100 | 0.0105 | 0.0112 |
| 32 | 0.0400 | 0.0418 | 0.0436 | 0.0410 | 0.0435 |
| 128 | 0.1659 | 0.1645 | 0.1638 | 0.1687 | 0.1725 |

横着读全是常数，竖着读是正比。**这条路径本来就对了**，本轮没有改动它，前后差异在噪声内
（0.95–1.09 倍）。这也说明"有活干就 O(N)"的说法太粗：得看是什么活。

## 结果二：牵动布局的改动是 O(总节点数)，而计数器看不见

`layout` + `tail`，1 个脏行——**下面什么都不移动**：

| 节点 | 修复前 | 修复后 | 工作计数器（前后相同） |
| ---: | ---: | ---: | --- |
| 502 | 0.201 | 0.101 | style=1 layout=4 hit=3 a11y=3 render=5 |
| 1,002 | 0.319 | 0.120 | 同上 |
| 2,002 | 0.618 | 0.204 | 同上 |
| 4,002 | 1.310 | 0.412 | 同上 |
| 8,002 | 3.083 | 0.891 | 同上 |

修复前那一列每翻一倍节点就翻一倍时间，而**每一个工作计数器都是常数**。
`runtime-work-invariants` 那批不变量测试全绿，因为它们盯的就是这些计数器。

分段计时把 1 个脏行、2,002 节点的那一帧拆成两半，各占一半：

| 分段 | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Layout | 0.073 | 0.120 | 0.246 | 0.527 | 1.292 |
| Accessibility | 0.079 | 0.127 | 0.256 | 0.529 | 1.117 |
| 其余全部 | <0.002 | <0.002 | <0.002 | <0.002 | <0.005 |

### 第一笔账：无障碍投影按被调度的布局脏集播种

`project_accessibility_delta` 把子树展开的种子取自
`work.input_hit_test ∪ work.transform ∪ work.layout`，然后从每个种子**递归遍历所有子节点**。

问题在第三项。布局失效向祖先传播，所以**任何**一片叶子改尺寸都会把文档根放进
`work.layout`；从根展开子树，就是遍历整个文档。基准直接从 `flush` 的返回值读到了实证：

```
502 个节点的文档，1 个脏行 → projected a11y = 502   （计数器说 3）
8,002 个节点的文档，1 个脏行 → projected a11y = 8,002 （计数器还是 3）
```

计数器看不见它，是因为 `accessibility_nodes_updated` 报的是**被调度的** ACCESSIBILITY 集合
（`schedule.rs`：`self.accessibility.len()`），而过度展开发生在投影函数**内部**。旁边的
render 路径同一帧只投影了 4 个节点，是对的。

**同一个文件里 40 行之外就写着不该这么做。** `RuntimeDocument::apply_hit_test_work` 的注释：

> 命中测试用的是 INPUT 集合，也就是布局回写恰好标在"盒子真的动了"的那些节点上。
> 被调度的布局节点是**故意**不用的：布局失效会向祖先传播，拿它去打补丁会让任何一次叶子
> 改尺寸都从文档根开始重建。

无障碍的种子犯的正是这条注释警告的错。修复是删掉 `.chain(&work.layout)`：真正动了盒子的
节点已经从另一条路到齐了——布局回写提交 `WriteLayout`，而 `WriteLayout` 恰好在那些节点上
标记 `INPUT | RENDER | ACCESSIBILITY`，包括被推移的后代。

修复后：1 个脏行投影 2 个节点，任何规模都一样。

### 第二笔账：每个兄弟节点的 effective style 被重复解析

剩下的一半在 `RuntimeLayoutEngine::layout_document_scoped` 内部（`layout_document_observed`
的四个子阶段里，tooltip 定位、回写提交、滚动指标三项都是常数，全部成本在引擎）。

采样剖析显示，容器重排 N 个子节点时，热点是 `UiWorld::effective_layout_style`——它在
`distribute_flex_main`、基线折叠、交叉轴折叠里**各被调用一次**，每次都是若干次哈希查找加
一个 `Arc` clone（隐藏或被 overlay 托管的节点还要 `Arc::make_mut` 复制整个 `LayoutStyle`）。

`LayoutInputMap::style()` 本来就是这层缓存，但它对没有 materialize 的节点直接穿透到
`world.layout_style(id)`，而 scoped 通道**不做 prefetch**，于是每个未改动的兄弟节点每帧都
重新解析三四遍。加一个整趟有效的 memo（`layout_document_scoped` 全程只借用 `&UiWorld`，
所以这不是近似，是精确等价）把引擎从 0.58 降到 0.40 ms（2,000 行）。

## 端到端：Vue 的 hover 帧确实收到了这笔钱

上面都是纯 Rust 的微基准。回到最初那个测量——`nana-hover-benchmark`，2,000 行，
`--shape window`，同一台机器上两个二进制前后各跑一次：

| 变体 | 每事件 hover 前 | 后 | settle 前 | settle 后 |
| --- | ---: | ---: | ---: | ---: |
| `bare` | 0.0623 | 0.0646 | 0.0005 | 0.0005 |
| `listeners` | 0.0618 | 0.0622 | 0.0005 | 0.0005 |
| `reactive` | 8.872 | **6.660** | 2.664 | **1.638** |
| `reactive-components` | 5.449 | **3.388** | 2.485 | **1.537** |
| `runtime-l3` | 0.0003 | 0.0003 | — | — |

`settle`（就是 `sync_semantics` 之下的 `flush_runtime_systems`）**降了 38%**，两个 reactive
变体都是。`bare` 与 `listeners` 纹丝不动，说明常数路径没有被拖累。

这也顺带回答了一个我本来会猜错的问题：Vue 的 hover 在 CSS 级联出口写的是一个新的
`NodeStyle`，所以它落在**牵动布局**的那一档，而不是上面 `paint` 那一档——否则这两处修复
在 Vue 路径上根本不会生效。"hover 只是换个背景色"在 JS 那头成立，在 Runtime 边界上不成立。

"before" 那两个 settle 数（2.66 / 2.48）与 `input-cost.md` 记的 2.58 / 2.42 吻合，说明这次
的对照组复现了原来的基线。

## 正确性怎么保的

两个改动都属于"跳过本该做的工作"，失败是静默的，所以两个回归测试都**先拿故意改坏的实现
验过它们会失败**：

| 测试 | 守什么 | 在坏实现上 |
| --- | --- | --- |
| `accessibility_delta_seeds_from_moved_boxes_not_scheduled_layout` | 一行改动的 delta 必须有界 | 恢复 `.chain(&work.layout)` → `projected 402 of 402 nodes` 失败 |
| `scrolling_publishes_the_moved_subtree_and_matches_a_full_projection` | 种子不能收窄过头 | 去掉 `input_hit_test` 种子 → `incremental delta diverged from a full projection` 失败 |

第二个是等价性而不是抽查：滚动只在滚动容器**自己**身上标 INPUT，后代保留 `LayoutBox`
且不带 ACCESSIBILITY 位，但它们在视口空间里都动了——这正是子树展开存在的理由。测试逐次
把增量 delta 应用到一棵保留树上，每次与**全量投影**比较。种子收窄过头会让宿主拿着整个
滚动子树的过期边界，而计数器依然干净。

两个测试都已加进 `.github/workflows/ci.yml` 的 `runtime-work-invariants` 名单。

其余门禁：`ui-snapshots` 559/559 MATCH（纯 Rust 路径的像素基准）、
`cargo test --workspace --all-targets` 2,916 项全过（原 2,914，新增 2）、
`cargo clippy --workspace --all-targets -- -D warnings` 干净。

## 第二轮：把布局引擎的兄弟扫描也去掉

上面那节的结论是"剩下的 O(总节点数) 在布局引擎的兄弟扫描本身"。这一轮把它拆成两半，
解决了其中占主导的那一半。

### 两处改动

**一、定尺寸的容器不需要测量孩子。** `intrinsic_size_scoped` 无条件测量全部子节点，再用它们
算出 `default_width` / `default_height`——而这两个值只通过 `unwrap_or` 消费。当节点自己的
width 和 height 都能从样式解析出确定值时，它们从头到尾没被读过。

这正是"改动过的帧 = O(文档)"的机制：布局失效向祖先传播，所以一次编辑会把它上面**每一层**
容器都放进变更闭包，每一层都丢掉自己缓存的 intrinsic 再重测全部孩子——为了得到一个自己
样式早就定死的尺寸。

**二、容器缓存自己对子节点的摆放（`ContainerPlan`）。** 一个容器对 in-flow 子节点的摆放，是
它自身输入加上每个子节点的样式与 intrinsic 的纯函数。这些都没变时，上一次的结果依然成立，
于是这一趟只需要下降到变更闭包真正触及的那几个子节点。

只有闭包内的子节点需要复检：影响布局的 `set_style` 会 `mark_subtree(LAYOUT)`，而那正是它
进入闭包的原因。`UiWorld::children_layout_style_is_local` 挡住三处例外——祖先能在不碰子节点
的情况下改变它的 effective style（`detached` 非空、父节点是 overlay host、父节点是菜单面），
报 false 只是让调用方走慢路径，永远安全。

计划的查找由**闭包驱动**（`affected_entries` 在按 id 排序的索引上二分），不是遍历子节点列表
——否则快路径本身还是 O(子节点数)，那正是它要消掉的成本。

### 结果

p50 ms，1 个脏节点，`dirty-frame-after-round2.json`：

| 形状 | 502 | 1,002 | 2,002 | 4,002 | 8,002 | |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `paint` | 0.0018 | 0.0017 | 0.0018 | 0.0018 | 0.0017 | 本来就是常数 |
| **`nested`** | **0.0115** | **0.0111** | **0.0114** | **0.0124** | **0.0115** | **本轮变成常数** |
| `layout` tail | 0.052 | 0.093 | 0.181 | 0.352 | 0.755 | 仍是 O(N)，快 2.9–3.6 倍 |
| `layout` head | 1.18 | 2.40 | 4.99 | 11.36 | 28.77 | 应付的回流，基本不变 |

`nested` 是**改动被含住**的形状：定尺寸行内部的一次编辑。整条祖先链照样被标脏（脏集会说
"整根脊柱都变了"），而真正欠的工作是常数。本轮之前它是 0.084（250 行）到 0.71 ms
（4,000 行）；现在 502 到 8,002 个节点全是 0.011 ms。**这一档已经按改动量收费。**

### 第三轮补上：子节点改尺寸

顺序累加的容器里，第 k 个子节点改尺寸只会让 k 之后的子节点整体平移，`[0, k)` 一步都不动。
`ContainerPlan` 因此多存两样东西：每个子节点的 `cursor_before`（主轴前缀和），以及一个
`sequential` 标记——记录当时这个容器是不是纯粹的从左到右累加。

`sequential` 排除掉一切会让兄弟之间互相耦合的东西：换行、会分配剩余空间的 `justify-content`、
反向流、grid 轨道、auto 主轴 margin、baseline/center/end 的交叉轴对齐，以及任何 grow/shrink
再分配。最后一条是**从数据判定**的，不是从样式推理的：记录时逐个检查"用到的主轴尺寸是否
等于 intrinsic"，相等就说明这一趟没有分配过剩余空间。

命中时只重放 `[k, n)`：前缀原样保留，`entries` 通过 `RefCell` 就地更新那一段，所以尾部编辑
是 O(1) 而不是 O(N) 的克隆。重放遇到任何它表达不了的东西（新出现的 auto margin、
非 Start/Stretch 对齐、opt-in 的 grow/shrink）就返回 false，调用方退回完整重排。

**结果：`layout` tail 也变成常数了。**

| 形状 | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `paint` tail / head | 0.0025 | 0.0025 | 0.0020 | 0.0017 | 0.0018 |
| `nested` tail | 0.0099 | 0.0096 | 0.0099 | 0.0109 | 0.0099 |
| **`layout` tail** | **0.0085** | **0.0083** | **0.0082** | **0.0095** | **0.0085** |
| `layout` head | 1.11 | 2.28 | 4.77 | 10.90 | 27.53 |

`layout` tail 从最初的 2.69 ms（8,002 节点）到 0.0085 ms，**约 300 倍，且不随规模变化**。

最后一行仍然是 O(N)，但它**不是过度失效**：计数器显示 `hit=8001 a11y=8001 render=8003`，
也就是 8,000 个盒子真的动了。改第一行会让下面每一行下移，这是应付的账。

**所以现在的性质是：凡是"真正欠的工作是常数"的情况，代价就是常数。**

### 三处守卫都验过会失败

重放是这轮唯一一处重新实现了容器逐子节点算术的地方，所以每一处守卫都拿故意改坏的实现
验过：

| 拿掉什么 | 差分哈内斯的报错 |
| --- | --- |
| 后缀平移（沿用缓存 origin） | `column-plain row 12 height 26: diverged at StableNodeId(41)` |
| 对齐守卫（放行 End/Center/Baseline） | `column-plain align-self-only edit: diverged at StableNodeId(15)` |
| grow/shrink 守卫 | `column-plain grow-only edit: diverged at StableNodeId(26)` |

第三条不是我预先想到的——是哈内斯在第一次跑重放时直接抓出来的：计划记录时容器没有剩余
空间，grow 因此不起作用，`used == intrinsic` 成立；随后一次编辑加上 `flex-grow` 就开始分配
了。"从数据判定没有再分配"对**当时**成立，对**将来**不成立，所以 grow/shrink 必须按样式
另外排除掉。

## Vue 的 settle 到底花在哪：量出来了

前两轮反复出现同一个问题——Runtime 变快而 Vue 的 settle 不动，然后靠猜。这轮给
`nana-ui-vue` 加了 `benchmark` 门控的阶段计时（`frame_profile`），把一次 hover 事件拆开。
2,000 行 `reactive`，`--shape window`，每事件均值 ms：

| 阶段 | dispatch | settle |
| --- | ---: | ---: |
| `sync_semantics` | — | **1.376** |
| 　├ **`prepare_semantic_styles`** | — | **0.526（38%）** |
| 　├ `reparent_orphans` | — | 0.253（19%） |
| 　├ `sync_layout_containing_blocks` | — | 0.142（10%） |
| 　├ `sync_sidebar_footer` | — | 0.084（6%） |
| 　└ `apply_semantic_styles` | — | 0.351（26%） |
| 　　└ 其中 `flush_runtime_systems` | — | 0.317 |
| `resolve_layout` | — | **0.0001** |
| `flush_runtime_systems`（全部调用） | 0.406（2 次） | 0.317（5 次） |
| 投影的 widget 数 | — | 5 |

（`prepare_semantic_styles` + `apply_semantic_styles` 合起来就是
`sync_semantics_from_bridge` 的 0.896。）

三件事：

1. **`resolve_layout` 是免费的**（0.0001 ms）。它有一个最多 8 趟的收敛循环，看着像嫌疑犯，
   实测每事件只跑 1 趟且不做事。
2. **settle 的 73% 在 Vue 层，不在 Runtime。** Runtime 那部分是 `flush_runtime_systems`
   的 0.317 ms（26%）。
3. **`flush_runtime_systems` 一次指针事件被调用 7 次**（dispatch 2 次 + settle 5 次），
   合计 0.72 ms。本轮把单次脏帧压到 0.01 ms 量级之后它从 1.30 降到 0.72，但**没有归因到
   具体是哪几次、各自脏了什么**——按基准的量级，7 次干净的脏帧应该只有 0.07 ms。

### 继续追下去：Vue 层每个指针事件付 4 处 O(文档)

把可疑的几处逐层加计时之后，**猜错了两次、量对了一次**：

- `Arc::new(widget.props.layout.clone())`——每个 widget 每帧克隆一个 115 字段的
  `LayoutStyle`，看着很贵。**实测 0.0036 ms，可以忽略。**
- 无障碍状态的构造与比较。**实测 0.0004 ms。**
- **`project_migrating_component` 是 `prepare_semantic_styles` 的 0.492/0.498，占 98%**，
  其中几乎全部在 `try_bind_registered_component`。

然后是决定性的一步：**同一套计时跑 500 行和 2,000 行**（4 倍），看哪些随文档增长。

| | 500 行 | 2,000 行 | 倍数 |
| --- | ---: | ---: | ---: |
| `sync_semantics` | 0.292 | 1.272 | **4.4×** |
| `try_bind_registered_component` | 0.113 | 0.474 | **4.2×** |
| `find_sidebar_reparent_host` | 0.028 | 0.151 | **5.4×** |
| `sync_layout_containing_blocks` | 0.018 | 0.133 | **7.3×** |
| `flush_runtime_systems`（settle） | 0.076 | 0.296 | **3.9×** |
| 投影的 widget 数 | 4.996 | 4.996 | 1.0× |
| **`try_bind_registered_component` 调用次数** | **4.996** | **4.996** | **1.0×** |

最后两行是关键：**投影集合和调用次数都是常数，时间却随文档线性增长。**

所以 `try_bind_registered_component` 不是"每个 widget 贵"，而是**每次调用 O(文档)**——
一次调用只处理一个 widget，却把整篇文档扫了一遍（2,000 行时每次约 102 µs，500 行时 23 µs）。
它每个事件被调用 5 次，合计占 settle 的 37%。**具体是函数里哪一行没有钉死**：明显的嫌疑
（`SemanticRead::get` 是哈希查找加记忆化拓扑、`COMPONENT_RULES` 按 `widget.kind` 早退、
属性构造只遍历 widget 自己的 props）都排除了，需要再加一层计时。

`find_sidebar_reparent_host` 的机制则在源码里直接看得见：它自己做一遍
`collect_reachable`（遍历全部 roots 的整棵树），再 `self.widgets.iter()` 全表扫一遍找
class name。这个 fixture 里根本没有 sidebar，所以它**每次都返回 None**，而且
dispatch 和 settle 各调用一次，合计每事件 0.38 ms。
（`reparent_orphans` 里那处被我先插桩的 `collect_reachable` 实测是 0.0000——因为
`find_sidebar_reparent_host` 返回 None 就提前 return 了，根本走不到。）

`flush_runtime_systems` 的 3.9× 也解释了上一节留下的 10 倍差：**Vue 路径上的那 7 次 flush
面对的不是基准里"1 个脏节点"的帧，而是 O(文档) 个脏节点的帧。** Vue 每个事件在往 Runtime
里灌 O(N) 的脏工作，`sync_layout_containing_blocks` 的 7.3× 是同一件事的另一面。

原来那句"最刺眼"的观察是对的，但归因是错的：**`prepare_semantic_styles` 花 0.526 ms 只投影了 5 个 widget**——每个
105 µs，而且它是这三轮里**唯一没有变过的数**（0.498 → 0.526），现在是 settle 的最大单项。`reparent_orphans` 花 0.246 ms 而这次 hover 一个孤儿都没有。这两个数字的形状和
这份文档开头那两笔账一样：**投影的集合是增量的，花的时间不是。** `input-cost.md` 里
"Vue 层已经按改动量收费了、不要再去优化 Vue 层"这句话，对**投影集合**成立，对**时间**
不成立。

（插桩本身留在仓库里：`nana-ui-vue` 的 `benchmark` feature，由 `nana-ui-devtools` 的
`agent-bin` 打开。这个归因前后被需要过三次，每次都是重新搭一遍。）

### 守它的是什么

新增一个**差分哈内斯**：13 种容器形状（gap、justify 的四种、align center/baseline、wrap、
grow、margin、auto margin、row、row-wrap）× 每种约 20 次编辑（头/中/尾改高、只改 margin、
只改 align-self、只改 order、只改 grow、改宽、改容器 gap、删行、加行），每一步之后拿
**每一个节点**与全量重算比对。

两处校验都拿故意改坏的实现验过会失败：

| 校验 | 改坏之后 |
| --- | --- |
| 子节点 intrinsic 比较 | `content_growth_under_a_child_moves_its_siblings` 报 `diverged from a full recompute` |
| 子节点样式比较 | 差分哈内斯的 `margin-only edit` 报 `diverged` |
| 整条快路径 | `scoped_layout_contained_edit_does_not_scan_siblings_as_the_document_grows` 报 `66 → 514 children` |

最后一条是**增量性**门禁，不是正确性门禁：它数容器实际测量了多少子节点，所以一个"改好了"
但其实全量重排的实现过不了它。差分哈内斯证明结果对，它证明代价小；只有两个都过才算数。

另外：像素基准 559/559 MATCH——gallery 里有真实的菜单、对话框、抽屉、分割面板和 tooltip，
正是 `children_layout_style_is_local` 挡的那些祖先派生路径。

### 端到端：Vue 这一轮几乎没动

| 构建 | `reactive` 每事件 | settle | `reactive-components` | settle |
| --- | ---: | ---: | ---: | ---: |
| 本轮之前 | 6.660 | 1.638 | 3.388 | 1.537 |
| 本轮之后 | 6.519 | 1.600 | 3.340 | 1.512 |

约 2%，在噪声量级。**Runtime 的脏帧变快了 3–60 倍，Vue 的 settle 没跟着动，说明它现在的主体
已经不在 `flush_runtime_systems` 里了。** 具体在哪没有量——这是下一步该做的第一件事，而不是
继续猜。（我在这一轮猜过一次：以为是 Vue 每帧重建 `Arc<LayoutStyle>` 导致指针比较失败，加了
按值比较的回退，实测没有变化，判断被推翻。那个回退本身是对的，留下了。）

### 把这两处修掉

**一、`snapshot.get` 换成 `snapshot.raw`**（`bind_semantic_slots` 与 `widget_icon`）。

两者都只读孩子的 `id` 加一个属性或 `kind`，却用 `SemanticRead::get` 拿完整视图——而 `get`
会为它碰到的每个 id 在 topology memo 里 `or_insert_with` 一个 children 列表。也就是说，
扫一遍 2,000 个兄弟节点，就分配 2,000 个 Vec，只为发现它们都没有 `data-slot`。

新增的 `SemanticRead::raw(id)` 直接取记录，不碰 topology。`widget_icon` 先用 `raw` 探
`kind`，只对命中的那一个孩子 `get`。

**二、没有 SidebarFrame 时整段跳过**（`reparent_orphans` / `reparent_sidebar_footer_slots`）。

`reparent_orphans` 的每条路径都需要一个 `SidebarFrame`：它只重挂孤立的 SidebarFrame，而
`reparent_sidebar_footer_slots` 需要一个可达的 SidebarFrame 才能挂 footer。**一个都没有时
整段是 no-op**——但证明这一点原来要走一遍全树再扫一遍所有 widget 的 class name，而且每个
指针事件走两次。`has_sidebar_frame()` 把它换成对枚举判别式的一次 map 扫描。

（这一条是等价改写，不是近似：前置条件在源码里可证。四个既有的 reparent 测试
——`reparent_orphans_prefers_resources_content_host`、`reparent_orphans_ignores_bare_flex_row`、
`reparent_sidebar_footer_slot_under_live_frame`、
`reparent_orphans_workspace_fallback_seeds_finite_height_cb`——都还是绿的。）

### 效果

2,000 行，每事件 settle（ms）：

| | 优化前 | 优化后 | |
| --- | ---: | ---: | --- |
| `bind_semantic_slots` | 0.421 | **0.049** | 8.6× |
| `try_bind_registered_component` | 0.490 | **0.094** | 5.2× |
| `prepare_semantic_styles` | 0.500 | **0.223** | 2.2× |
| `reparent_orphans` | 0.244 | **0.0019** | **128×** |
| 　`find_sidebar_reparent_host` | 0.164 | **0.0000** | |
| **`sync_semantics`（= settle）** | **1.330** | **0.732** | **1.8×** |

`bind_semantic_slots` 仍是 O(子节点数)（0.0113 → 0.0489，4 倍行数涨 4.3 倍），只是常数小了
8.6 倍。要变成常数需要给"哪些孩子带 `data-slot`"建索引，那要在 bridge 的 mutation 路径上
维护，本轮没做。

### 端到端：五轮累计

2,000 行，`--shape window`，p50 ms/事件（第五轮那列是同机三轮重测，见"第五轮"一节）：

| 变体 | 起点 | 第一轮 | 第三轮 | 第四轮 | 第五轮 | 累计 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `reactive` 每事件 | 8.872 | 6.660 | 6.038 | 4.783 | **3.36** | **−62%** |
| 　其中 settle | 2.664 | 1.638 | 1.330 | 0.732 | **0.176** | **−93%** |
| `reactive-components` 每事件 | 5.449 | 3.388 | 2.751 | 1.793 | **0.77** | **−86%** |
| 　其中 settle | 2.485 | 1.537 | 1.248 | 0.623 | **0.168** | **−93%** |

第五轮的同机对照是 `reactive` 每事件 5.08 → 3.49、settle 0.780 → 0.194（前四轮那几列记在
各自当时的机器状态下，跨列比较只看趋势）。这一轮里 **hover 帧的 `FrameStage::Layout`
归零**——一次只改颜色的 hover 本来就不欠任何布局。

下面这份分解是**第四轮之后**的 0.73 ms：

| | ms | % |
| --- | ---: | ---: |
| `apply_semantic_styles`（几乎全是 `flush_runtime_systems`） | 0.329 | 43% |
| `prepare_semantic_styles` | 0.223 | 29% |
| `sync_layout_containing_blocks` | 0.123 | 16% |
| `sync_sidebar_footer` | 0.079 | 10% |

### 下一条线已经钉死：内容驱动尺寸的容器仍会重测全部孩子

把 `flush_runtime_systems` 的 0.297 ms 追到底,又推翻了一个假设:**Vue 并没有往 Runtime 灌
O(文档) 的脏工作**。一次事件里它发布的是 a11y 3.99 个节点、scene 7.0 个节点、1 趟 pass,
两个规模下完全一样。是 flush 自己在常数脏集下仍然 O(N)。

再往下切(过程中我的插桩踩了这份文档开头警告过的那个坑:5 次 flush 里 4 次是空闲的,
`last_frame_profile` 在空闲帧上返回保留值,于是同一份分段被重复累加了 5 次):

| | 500 行 | 2,000 行 | |
| --- | ---: | ---: | --- |
| `FrameStage::Layout` | 0.0575 | 0.2701 | 4.7× |
| 　其中 engine | 0.0581 | 0.2699 | |
| 　tooltips / writeback / scroll metrics | <0.001 | <0.001 | 常数 |
| 变更闭包种子 | 6.99 | 6.99 | **常数** |
| 闭包(含祖先) | 6.99 | 6.99 | **常数** |
| 摆放测量的子节点 | 2.0 | 2.0 | **常数** |
| 保留缓存清扫次数 | 0 | 0 | |

闭包是常数、摆放几乎不跑、清扫不触发——O(N) 只可能在 `intrinsic_size_scoped`,而那一处
唯一没被"定尺寸短路"覆盖的情况就是**容器自己是内容驱动尺寸**。

在纯 Rust 基准里加 `nested-auto` 形状(容器 height 改成内容驱动,其余不变)复现出来:

| 形状 | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `nested`(容器定尺寸) | 0.0099 | 0.0096 | 0.0099 | 0.0109 | 0.0099 |
| **`nested-auto`(内容驱动)** | **0.153** | **0.194** | **0.325** | **0.674** | **1.340** |

脏计数器完全一样(`layout=4 hit=2 a11y=2 render=5`)。**受影响的内容驱动容器会重测它的每
一个孩子,尽管每个孩子的 intrinsic 都没变、每次都只是命中保留 memo。**

这是 `ContainerPlan` 的对称缺口:它覆盖了**摆放**,没有覆盖**测量**。修法同形——按
(容器自身输入 + 每个子节点的 style/intrinsic)缓存容器自己的 intrinsic,只复检闭包内的
子节点。下一节做掉了。

原来那句"下一个该动的是 `flush_runtime_systems` 的 0.297 ms / 5 次调用——按 Runtime 现在的单帧量级
（0.008–0.011 ms）这应该只有 0.05 ms，说明 Vue 每个事件仍在往 Runtime 灌 O(文档) 的脏工作。
`sync_layout_containing_blocks` 的 O(N) 是另一件事:`propagate_layout_containing_blocks`
每帧从每个 root 递归走整棵树,即使没有一个包含块变化。

## 第五轮:测量侧补上 `MeasurePlan`

`ContainerPlan` 的形状照搬过来:按 id 缓存容器自己的 intrinsic,记下(容器自身输入 + 每个
**直接子节点**的 style + 每个在流子节点的 intrinsic),复检由变更闭包驱动(在按 id 排序的
entries 上二分,而不是遍历子节点列表)。全部命中就直接返回上次的尺寸,不碰任何一个孩子。

一个刻意的形状差异:entries 覆盖**每一个直接子节点**,不只是在流的那些。流收集丢掉的孩子
(`display:none`、绝对定位)在容器的测量里不贡献任何东西,但它可以因为一次 `set_style` 重新
变成在流子节点——只记在流子节点的话,这次改动在 entries 里查不到,计划会被错误地沿用。
给它们记一条只有 style 的条目,比较相等即可确定它仍然不贡献。

结果(`--position tail --dirty 1`,flush p50 ms):

| 形状 | | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `nested-auto` | 之前 | 0.0858 | 0.1704 | 0.3449 | 0.6768 | 1.4062 |
| | **之后** | **0.0101** | **0.0100** | **0.0111** | **0.0113** | **0.0110** |
| `nested` | 之前 / 之后 | 0.0108 / 0.0097 | 0.0096 / 0.0094 | 0.0096 / 0.0095 | 0.0109 / 0.0106 | 0.0097 / 0.0097 |
| `layout` | 之前 / 之后 | 0.0086 / 0.0094 | 0.0085 / 0.0091 | 0.0084 / 0.0091 | 0.0095 / 0.0096 | 0.0090 / 0.0085 |
| `paint` | 之前 / 之后 | 0.0025 / 0.0017 | 0.0024 / 0.0018 | 0.0020 / 0.0017 | 0.0017 / 0.0018 | 0.0018 / 0.0019 |

8,002 个节点上 **1.41 → 0.011 ms(128 倍)**,而且和 `nested` 一样与文档大小无关了。其余三种
形状纹丝不动。`--position head` 上的 `nested-auto` 同样从 0.320 → 0.009 ms(2,000 行):那一列
改的也是行内的 label,行本身尺寸不动,所以下面没有一行需要移位,常数才是应付的账。

### 两件事是量出来的,不是设计出来的

**一、一个约束槽位不够,要两个。** 第一版给每个容器存一份计划,结果 8,002 个节点上只从
1.31 掉到 0.77 ms——**仍然是 O(N)**。计数器说得很清楚:`children_measured` 还等于行数,而
`measure_plans_reused` 是 2。加一行调试输出才看见:同一个容器**每帧被测量两次,约束不同**
——一次来自父节点自己的 intrinsic 测量(用父节点的**可用**内容框),一次来自父节点的摆放
(用父节点的**已用**内容框)。父节点是内容驱动尺寸时这两个值不同,于是一份计划被其中一次
写入、被另一次错过,每一帧,永远。

改成两个槽位,原因和 `RetainedIntrinsic` 的 `[Option<_>; 2]` 一模一样。约束的相等性由
`inputs_match` 判定,槽位查找只负责挑一份出来,所以选错槽位不会变成正确性问题。

**二、全量通道上记录计划,亏的比赚的多。** 记录一份计划要为每个孩子克隆一次 `Arc` 并给
容器排一次序。全量通道会重建整棵树的每个容器,于是这笔钱按文档大小收:
`canonical_layout_5000_nodes_ms` p50 **1.79 → 2.55 ms(+42%)**。那个基准每次迭代在 1,280 和
1,024 之间换视口宽度,所以每一帧都是 `force_full`——正是窗口拖拽时的形状。而它买到的只有
一帧:下一次增量通道本来就会把计划建起来。

所以现在只在增量通道上记录(`scope.is_some()`,这同时也排除掉 `layout_document`——css-parity
和 Vue `measure_layout` 走它,而它返回时把整张表丢掉)。代价是**全量通道之后的第一帧增量是
O(文档) 的**,那一帧要把计划建起来。改完之后:

| | 之前 | 之后 |
| --- | ---: | ---: |
| `canonical_layout_5000_nodes_ms` p50 | 1.744 / 1.803 | 1.815 / 1.823 |
| 5,000 节点首次系统处理 P95 | — | 1.985 ms |

（两列各跑两轮交替取,差异在噪声内。首次系统处理与第三轮记的 1.992 ms 一致。）

这一条**直接改变了五个测试的含义**:它们原来测的是全量之后的第一帧,也就是建计划那一帧,
于是通过得毫无意义。五个测试现在都显式先跑一次"热身"编辑,再测稳态——热身这件事本身写在
测试里,因为它是这个设计的一部分。下一节说这是怎么发现的。

### 顺带关掉 `ContainerPlan` 的同一个洞

两份计划都用"闭包里的这个 id 是不是我的某个 entry"来复检,而 entry 是按**直接子节点** id
索引的。有两种情况会让在流子节点列表不再是直接子节点的子集:

- `display: contents` 把孩子的**孩子**接进来,列表变成了计划从未记录过的孙子列表的函数;
- 块级父节点下的行内级孩子,是否被拆箱取决于**它自己的子树里有没有块**,所以任意深处的
  一次改动都能在它样式不变的情况下把它移进或移出列表。

`collect_flow_children_reporting` 现在顺带报告"这次收集有没有向下伸手",两份计划都据此
拒绝缓存。在收集时报告而不是事后检查,是因为收集本来就要走一遍每个孩子,而对产出的列表
做成员检查是平方的。

### 十处守卫都验过会失败

"跳过本该重算的工作"这类改动失败是静默的,所以每一处判断都拿故意改坏的实现跑过整套
`layout_engine` 测试:

| 改坏什么 | 抓住它的测试 |
| --- | --- |
| 不再重测闭包里的孩子(只比 style) | `content_growth_..._when_the_container_hugs` |
| 不再比孩子的 style(只比 intrinsic) | `a_container_whose_own_text_grows_remeasures_itself` |
| 忽略容器自己的 style | `flipping_a_container_to_a_row_...` |
| 忽略子节点列表身份 | `content_growth_..._when_the_container_hugs` |
| 忽略容器自己的 text metrics | `a_container_whose_own_text_grows_remeasures_itself` |
| 完全不复检孩子 | `a_display_contents_child_keeps_its_container_off_the_cached_plans` |
| 槽位不看约束就返回 | `content_growth_..._when_the_container_hugs` |
| 忽略父节点流方向 | `flipping_a_container_to_a_row_...` |
| 不排除 `display:contents` / 行内拆箱 | `a_display_contents_child_keeps_its_container_off_the_cached_plans` |
| 把拼法差异当成真的改动（关掉等价比较） | `a_default_spelled_two_ways_keeps_the_plan_but_a_real_direction_change_retires_it` |

那条等价比较有两个方向,两个都验过:`None` ↔ `Some(Column)` **必须**保住计划,
`None` → `Some(Row)` **必须**作废它——只守前者的话,"全都算相等"也能通过。

这一轮里这套沙盘跑了八遍,每次改动之后都跑。前三遍是在补测试:第一遍九条里有三条没被
抓住,说明既有测试并没有覆盖到它们。**后面它抓到的不是我预想的东西,而是我自己**:
加上"只在增量通道记录"之后再跑,十条里有三条从"抓住"变回了"没抓住"——因为那三个测试都
是全量之后只做一次增量,而那一次现在成了建计划的那一帧,计划根本没被查询过。三个测试
(以及前面两条增量性门禁)都补了显式热身。没有这套沙盘,它们会以绿色的状态守着一段从未
执行到的代码。**沙盘自己也栽过同一个跟头**:它的每处改坏原来是不带断言的字符串替换,
等价比较改掉了两处目标串之后,替换静默变成了空操作,于是两条报"没抓住"——一个改不动
目标的破坏脚本,和一个从不失败的测试是同一件事。现在每处替换都断言目标串存在。

新增的形状与测试:

- `diff_shapes()` 加了 7 种内容驱动尺寸的容器(auto 高、auto 宽高、auto + gap/margin、
  auto + align-center、auto + grow、row auto 宽、auto + content-box padding),差分哈内斯
  现在是 20 种形状 × 每种约 20 次编辑,每步之后逐节点与全量重算比对。哈内斯本身也加了
  一条断言:每种形状都必须真的复用过一次测量计划,否则它守的是空气。
- 哈内斯里"删一行"与"加一行"的**顺序换了**:`detach` 会在世界里留下一个游离节点,
  `children_layout_style_is_local` 从此对整轮返回 false,两份计划都被退休——放在它后面的
  `append` 什么也没验证。

### 端到端一开始是 0,原因是一处"同一份布局的两种拼法"

上面全部是纯 Rust 基准。**端到端跑完之后,`reactive` 的 settle 没有动,反而慢了 2–3%**
(0.767 → 0.786 ms,3 轮一致)。合成基准 128 倍、门禁全绿,真实工作负载收益为零——这一步
在任务里写着"可选",接受那个措辞是错的。

`children measured` 从 2 跳到 2,002 **不是回归**:这 2,000 次测量本来就在,只是过去只统计
摆放侧。第四轮推断的"内容驱动容器每帧重测全部孩子"在 Vue 树里确实成立;我这轮把它计出来
了,却没有消掉。

逐层排除定位到底(不是猜):2,000 孩子的容器确实每事件走 4–5 次测量循环 → `inputs_match`
全过、槽位命中、`children_layout_style_is_local` 为真 → 被逐子复检拒绝 → 106/106 是
`reason=style`,而差异字段只有一个:

```
direction: Some(Column)  !=  direction: None
```

这两个值在引擎里**完全等价**:`LayoutStyle::direction` 全仓只有一处读取,即
`used_flow_direction` 的 `unwrap_or(Column)`。而 fixture 里行的 style 只有
`height/width/flex-shrink/color`,**hover 只改 color**。

追到写入方,是**三个写者对"默认的列"用了两种拼法**,每帧互相覆盖 2,003 个节点:

| 写者 | 写什么 |
| --- | --- |
| `MessageBridge::register` | 按 widget kind 给 `direction` 播种 |
| CSS cascade → `publish_layouts` | 重新发布一份 `direction: None` 的样式 |
| Runtime `view_components::Stack` → `project_common` | 又写回 `Some(Column)` |

(栈帧:`set_style ← project_common ← bind_registerable::<Stack> ←
prepare_semantic_binding ← try_bind_registered_component ← project_migrating_component`。
这些写入**没有进入脏集**——`dirty seeds` 恒为 6.996——所以它以前不显形。)

**修法:计划比较的是"布局输入是否相同",不是"拼法是否相同"。** `layout_inputs_equal`
先做一次 `==`,不等时只放行 `used_flow_direction` 映射到同一根轴的那一对
(`None` ≡ `Some(Column)`,`Some(Row)` 仍然不等),其余字段照旧必须全等。放在测量与摆放
两侧的逐子复检、以及 `MeasurePlan::inputs_match` 上。

2,000 行,3 轮 p50 ms:

| | | settle | `FrameStage::Layout` | children measured |
| --- | --- | ---: | ---: | ---: |
| `reactive` | 本轮之前 | 0.767 | 0.285 | 2.0 |
| | 只有 `MeasurePlan` | 0.786 | 0.310 | 2002.0 |
| | **加上等价比较** | **0.630** | **0.140** | **5.4** |
| `reactive-components` | 本轮之前 | 0.646 | 0.254 | 2.0 |
| | **加上等价比较** | **0.488** | **0.096** | **5.4** |

settle **−18% / −24%**,Layout **−51% / −62%**。两处改动缺一不可:没有 `MeasurePlan`,
等价比较无处可用;没有等价比较,`MeasurePlan` 每帧被一处无关紧要的拼法差异打死。

**这条只治了缓存这一侧**,写入战本身还在。下一节把它也拆了。

### 写入战本身:一次投影播了一个不改变任何布局的种子

上一节我写过一句"它占 settle 的四成",依据是 `project_migrating_component` 0.163 ms +
`try_bind_registered_component` 0.161 ms 这两个计时器和那串栈帧。**那是没量过的归因,而且
是错的**:等价比较上线前后这两个计时器纹丝不动(0.169 → 0.167、0.167 → 0.165)。

量的方法是把那次写入直接跳掉(env 开关,只为归因),看端到端动多少:settle 0.62 → 0.40,
每事件 4.93 → 3.78,而且 `FrameStage::Layout` **归零**——一次只改颜色的 hover 本来就不欠
任何布局。所以代价是真的,只是不在我原先指的那两个计时器上。

种子在 `Stack::from_semantic`(`builtin_components.rs`):

```rust
if layout.direction.is_none() {
    match spec.type_id.as_str() {
        "nana.row"    => layout.direction = Some(Row),     // 有意义
        "nana.column" => layout.direction = Some(Column),  // 纯抖动
        _ => {}
    }
}
```

`nana.row` 那一半**是有意义的**:引擎默认的流轴是块轴,一个不声明的行会按列排。
`nana.column` 那一半**改变不了任何布局**——`used_flow_direction` 就是
`unwrap_or(Column)`——却制造了第二种拼法,于是 cascade 每帧写 `None`、投影每帧写回
`Some(Column)`,2,003 个节点上一直互相覆盖。这一次探针数是 8,098 次 `nana.column` 播种。

去掉那一半之后,2,000 行三轮 p50 ms:

| 变体 | | 每事件 | settle | `FrameStage::Layout` | children measured |
| --- | --- | ---: | ---: | ---: | ---: |
| `reactive` | 本轮之前 | 4.96 | 0.763 | 0.285 | 2.0 |
| | 只有等价比较 | 5.04 | 0.639 | 0.139 | 5.4 |
| | **去掉种子** | **3.85** | **0.409** | **0.0000** | **0.0** |
| `reactive-components` | 本轮之前 | 1.87 | 0.642 | 0.253 | 2.0 |
| | **去掉种子** | **1.04** | **0.331** | **0.0000** | **0.0** |

**每事件 −22% / −44%,settle −46% / −48%,hover 帧的布局阶段归零。**
`bare` 与 `listeners` 不变(0.062 / 0.0007)。

守它的两个测试分工明确:
`projecting_a_stack_does_not_rewrite_the_style_the_cascade_published` 断言投影**不会**去
重写 cascade 刚写下的样式(`nana.column` / `nana.box` / `nana.stack` 必须不写,`nana.row`
必须写——不写死后一条的话,"什么都不播种"也能通过);
`an_unset_flow_axis_is_a_column_but_not_a_row` 拿真实布局证明这两半一个是惰性的、一个不是。
两个都拿把种子加回去验过会失败。

**仍然剩下的**:`nana.row` 的写入是应付的,但它**同样**每帧被 cascade 覆盖(cascade 产出
`None`,投影写回 `Some(Row)`,两者不等价),所以 row 类节点还在写入战里。要修它得决定
"组件投影与作者 CSS 谁拥有这个节点的布局"——`component_owned_layout` 已经有这个概念,只是
这些节点不在里面。那是 bridge 的语义决策,没做。

### 第三块:一次没人会读的扫描

写入战修完后 settle 是 0.41 ms,最大的一块是 `prepare_semantic_styles` 0.199 ms。**先按
规矩跑两个规模**(500 行与 2,000 行),把"每次调用贵"和"每次调用 O(文档)"分开:4 倍行数
涨 4.9 倍,而调用次数全是常数(`projected_widgets` 4.998、`flush passes` 1.0)。所以是
"调用次数恒定、每次 O(文档)"——和第四轮同一个签名。

再往里切要新的分段(`PHASES` 加了三条:`projection_ids`、`prepare loop body`、
`# dirty ids in`)。结果把嫌疑人排除掉了一半:`projection_ids` 低于 0.008 ms 量级(Bridge
源那一支只走脏集的祖先链,本来就是 O(闭包)),而 **loop body 是 0.208 里的 0.207**。
循环只跑约 5 次,所以是**其中一次是 O(文档)**。最深的那片叶子是
`bind_semantic_slots` 0.099 ms,4 倍行数涨 **8.58 倍**。

它在做什么:给一个 widget 收集 `data-slot` 子节点,要把**每个孩子**探一遍。探针打出来,
在这棵树上唯一触发它的是那个 2,000 孩子的 `Column` 容器,每事件约 2.3 次。

而 `Column` / `Row` / `Box` 绑的是 `Stack`,**`Stack::from_semantic` 根本不读
`spec.slots`**。也就是说这遍扫描的结果没有任何人会读——而且"忽略 slot 的组件"和"孩子
最多的 widget"恰好是同一批。

修法是把这件事**声明出来**,而不是在宿主里按 kind 猜:`RegisterableComponent` 加
`const CONSUMES_SLOTS: bool = true`,`Stack` 覆写为 `false`,注册表按组件存下来(于是三个
布局别名 `nana.column` / `nana.row` / `nana.box` 自动继承),宿主在收集前问一句。**未注册
的类型一律答"是"**——宁可白扫一遍,也不能悄悄丢掉一个 slot 子节点。

2,000 行三轮 p50 ms:

| 变体 | | 每事件 | settle |
| --- | --- | ---: | ---: |
| `reactive` | 去掉种子后 | 3.86–4.53 | 0.405–0.447 |
| | **加上跳过扫描** | **3.68** | **0.303** |
| `reactive-components` | 去掉种子后 | 1.03–1.10 | 0.329–0.336 |
| | **加上跳过扫描** | **0.98** | **0.281** |

settle 再降 **−26% / −15%**。

守它的两个测试都拿改坏的实现验过会失败:
`stack_ignores_slots_so_declaring_it_is_honest` 直接构造带 slot 与不带 slot 的两份
`SemanticSpec`,断言 `Stack::from_semantic` 产出相同——**`CONSUMES_SLOTS = false` 是一个
宿主会照做的承诺,一旦 `from_semantic` 开始读 slot,宿主就会静默丢掉那些孩子**,所以这条
必须被检查而不只是被声明;
`the_slot_flag_reaches_every_alias_and_defaults_to_yes` 断言三个别名都继承到了该标志,
并且一个真的用 slot 的组件、以及一个注册表没见过的 id,都必须答"是"。

### 第四、五块:两次为了证明"没活干"而走全树的扫描

**`sync_sidebar_footer` 0.079 ms/事件(4.06 倍)。** 这棵树里**根本没有 sidebar**。
`sync_sidebar_footer_into_document` 第一件事是 `roots_reachable()`——走一遍整棵树、给每个
widget id 建一个 `HashSet`——只为拿去问 `reachable_sidebar_frame`。而那个函数只有在存在
`kind == SidebarFrame` 的 widget 时才可能返回 `Some`,所以一个都没有时整段是 no-op。

这正是第四轮给 `reparent_orphans` 加的那道 `has_sidebar_frame()` 守卫(对 map values 扫
一遍枚举判别式,没有可达集、没有 class name 比较),当时**没有一并加到这里**。加上之后
0.0796 → 0.0016 ms(**50 倍**),settle 0.299 → 0.219。

**`widget_icon` 0.030 ms/事件(3.90 倍)**,此时已占 `try_bind_registered_component` 的七成。
它要在孩子里找 `kind == Icon` 的那一个——又是一遍 2,000 个孩子的探测。而 `Stack` 同样不读
`spec.icon`。

这和上一节的 slot 是**同一个性质**:宿主要靠扫孩子才能算出来的 `SemanticSpec` 字段,而绑定
到的组件根本不读。所以把上一节的标志**改名并加宽**成
`READS_CHILD_DERIVED_SPEC`——一句话覆盖 `slots` 与 `icon` 两处扫描——而不是再加一个几乎
一样的布尔。测试也一并覆盖两半:`Stack::from_semantic` 对带 slot / 带 icon 的 spec 都必须
产出相同结果,两半都拿改坏的实现验过会失败。

两条合起来,2,000 行三轮 p50:

| 变体 | | 每事件 | settle |
| --- | --- | ---: | ---: |
| `reactive` | 本轮起点 | 5.08 | 0.780 |
| | **现在** | **3.49** | **0.194** |
| `reactive-components` | 本轮起点 | 1.88 | 0.652 |
| | **现在** | **0.77** | **0.168** |

### 第六块:同一个 hash 查两遍

修完前面几条,settle 0.199 ms,而 `prepare loop body` 0.072 里只有 0.019 有归属——
**0.053 无归属,涨 3 倍**。给循环里的 `snapshot.get` 单独加一条计时:0.051,涨 4.46 倍。

`SemanticRead::get` 第一次碰到一个 id 时要建它的 topology 条目(父 + 孩子)。对那个 2,000
孩子的容器,`document.live_children(id)` 先按 `document.nodes` 过滤一遍,`get` 再用
`visible = document.nodes.contains_key(..) && bridge.get(..).is_some()` 过滤一遍——**同一张
表、同一个键,每个孩子查了两次**。父节点那一侧需要完整的 `visible`(`live_parent` 不做成员
检查),孩子那一侧不需要。

去掉那一半:`snapshot.get` 0.0497 → 0.0360(**−28%**),loop body 0.0708 → 0.0569,
settle 0.188 → 0.176(三轮一致)。

**这一条差点被一次跑偏的测量埋掉。** 第一次测完我读到的是"变慢一倍"(0.051 → 0.107),
而少查一次 hash 不可能让它翻倍。改成同一批次里 A/B 交替各跑两轮,才看出真数——先前那次
是机器被占用/降频。单次读数会骗人,这份文档开头就写过一次,这里又栽了一次。

## 还剩什么

**`sync_layout_containing_blocks` 0.079 ms/事件**,现在是 settle 里最大的一块。
`propagate_layout_containing_blocks` 每帧从每个 root 走整棵树,即使一个包含块都没变。
5.09 倍/4 倍行数。

这一条我做了一半,而且**做的那一半证明了我的机制假设是错的**。原来的递归形式对每个节点
做一次 `widget.children.clone()`(每帧 N 次 Vec 分配),因为子节点循环需要 `&mut self`、
握不住那个列表的借用。改成一个工作表复用一块缓冲之后:这一段 0.096 → 0.091 ms,settle
纹丝不动。**分配不是成本,遍历才是。**(改动留下了——少 2,000 次每帧分配本身是对的——但
它不是那笔钱。)

第二次尝试是把两次 map 查找并成一次:原来每个节点查两遍——一次在
`write_containing_block` 里写、一次读它的 layout 去解析给孩子的 content box,而 widget 是
`Box` 的,每次查找都是一次 hash 探测加一次跨 cache line 的指针追。并成一次:0.091 → 0.079,
settle 0.182 → 0.168。**还是不是大头。**

### 收窄这趟走:做出来了,量到了,然后撤了

真正的修法是只从"content box 可能动过"的 widget 往下推。**做完并量过:这一段
0.079 → 0.0001 ms,settle 0.167 → 0.089(−47%)。** 然后撤掉了,理由值得写下来。

实现是一个 `containing_block_seeds` 集合,在每一处标脏的地方一并播种,由这趟走自己清空
(**不能**用 `changes.dirty`:`sync_semantics_from_bridge` 会在同一帧、这趟走之后把它抽干,
于是两者之间发生的布局改动在下一趟走时已经不见了)。加上一条 debug 断言:每次收窄走完再走
一遍全量,断言它一无所获。

这条断言当场抓出两个真 bug,都不是"漏播种":
- **视口身份**:调用方确实会传一个不定的视口,而 `viewport_changed` 只在宽高都是 `Some`
  时才成立,于是从"1120×760"切到"不定"时收窄走不知道输入变了。
- **根森林**:种子里混进了挂在挂载根**之上**的脚手架节点。从它往下推,会把它自己那个未设
  的包含块盖到整棵根子树上——全量走从不会有这个问题,因为它只从 root 出发。

修完之后,**整个 workspace 测试套件在"把播种整个关掉"的情况下依然全绿**。我又专门写了一个
差分哈内斯(9 种真实 host 改动,每次之后逐 widget 与全量走比对),对三种改坏——关掉播种、
只让 `reapply_layout_for` 不播种、不把种子限制在根森林——**全都抓不住**。

原因也清楚了:这个 bridge 里凡是能移动包含块的改动,都会走到 `changed_all()`,而
`changed_all()` 会把下一趟走强制成全量。**收窄那条路只在"本来就没活干"的时候才跑**——这多半
正是它快的原因,但"我构造不出反例"不是证明。

所以撤了。**前置条件不是那次 `set_layout()` 收口重构,而是能真正走到收窄路径、并且带着待做
布局工作的覆盖**。在此之前,这块 0.079 ms 不该动。三处沙盘和那两个 bug 都记在这里,下一个人
不必从头再摸一遍。

**`sync_sidebar_footer` 已修**,见下一节。

**内容驱动尺寸的容器,在孩子真的改尺寸时,仍然重测全部孩子。** 这一轮补的是"闭包里的孩子
都没变"那一半;另一半——容器按已缓存的每子贡献增量更新自己的聚合,也就是
`replay_sequential_suffix` 的测量侧对应物——**做了一遍又撤了**,见下。基准里新加的 `layout-auto` 形状就是这条
(内容驱动容器 + 改行高,`tail` 位置),flush p50 ms:

| | 502 | 1,002 | 2,002 | 4,002 | 8,002 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 之前 | 0.2336 | 0.4633 | 0.9339 | 1.9852 | 5.5576 |
| 之后 | 0.2444 | 0.4635 | 0.9350 | 1.9588 | 5.4317 |

本轮对它没有影响(计划每帧被正确地拒绝,然后重建;重复三轮的 engine 分段是
2.13/2.12/2.32 对 2.25/2.23/2.21 ms,两者重叠)。要把它也收成常数,需要在"平铺累加"那一支
存下每个孩子的主轴贡献与交叉轴贡献,主轴按差量更新、交叉轴按最大值更新,并在"原来的最大
值变小了"时退回全量重算(存不下第二大)。三处最大值(交叉轴、`max_content_w`、
`stacked_min_w`)都要这么处理。

### 测量侧的增量聚合:写完了,差分哈内斯是绿的,还是撤了

实现是:每个 entry 记下它对"平铺累加"的三项贡献(外主轴延展、外交叉轴延展、外宽),容器记下
三项聚合(主轴和、交叉轴最大、堆叠最大);一个孩子改尺寸时,主轴按差量修正 O(1),两个最大值
在"长过当前最大"时 O(1),而在"原本就是最大值却变小了"时放弃、退回全量重算。entry 与聚合都
放在 `RefCell` / `Cell` 后面就地更新——和 `ContainerPlan` 用 `RefCell` 是同一个理由:重建
entry 列表本身就是这份缓存要避免的那笔钱。为了不让同一段算术出现两份,把尾部的
"聚合 → 用尺寸"提成了 `finish_plain_intrinsic_size`,两条路共用。

**差分哈内斯全绿**(20 种形状 × 每种约 20 次编辑,逐节点与全量重算比对),十处沙盘也全部
仍然被抓住。但增量性门禁挂了一条:一个本该 0 次重测的用例变成了 8 次。

追下去不是算术错,是**缓存容量**:那个 hugging 容器一帧里被三种约束测量——根的可用框
(320×600)、以及摆放侧随容器自身高度变化的两种(320×163、320×160)——而槽位只有两个。
原本每次测量都会记录,两个最近的总在;加了增量修复之后,命中修复的那一次提前返回、不再记录,
于是三种约束在两个槽位里来回颠。

**撤了。** 理由和包含块那条一样:我在这一轮里没能把它收敛干净,而这是整段会话里最精细的一处
改动,放在最需要小心的模块里。真要做,前置条件是先把槽位容量这件事想清楚——要么按约束来源
(测量侧 / 摆放侧)分槽而不是按 MRU,要么让修复路径也刷新槽位次序。

**顺带修掉一个它暴露出来的真 bug**:每帧的 `nodes.measure_plans` 是**替换**而不是并入
`retained.measure_plans` 的。一个容器常常一帧被两种约束测量、却只重测其中一种(另一种由缓存
答掉、什么也不记录),替换就会把另一种的计划抹成 `None`,两个槽位于是轮流失效。改成并入。
这条在现有形状上量不出差别(`nested-auto` 本来就是平的),证据来自上面那次被撤的实验里
打出的 `have=Some([Some((320.0, 160.0)), None])`;它只会让缓存多留一条有效条目,不可能
答错,所以留下了。

**`head` 位置是超线性的，而且 O(N) 那部分是应付的账。** 改第一行会让下面每一行真的下移，
所以 O(N) 正当；但它比 O(N) 更陡——502→1.32、1,002→2.33、2,002→5.11、4,002→11.4、
8,002→28.7 ms，节点翻倍时间约乘 2.5。

**这一轮归因了。** 按阶段拆开（1,000 行对 4,000 行，即 4 倍）：

| 阶段 | 2,002 | 8,002 | 倍率 |
| --- | ---: | ---: | ---: |
| Style | 0.0010 | 0.0019 | 1.9 |
| **TextShape** | **0.180** | **2.767** | **15.4** |
| Layout | 1.09 | 5.97 | 5.5 |
| HitTest | 0.72 | 3.79 | 5.3 |
| Accessibility | 0.30 | 1.45 | 4.9 |
| Extract | 2.57 | 12.96 | 5.0 |

**超线性只有一处:`FrameStage::TextShape`,4 倍行数涨 15.4 倍(≈ N^1.96)**;其余五个阶段都在
5 倍上下(那点超出多半是工作集出 cache)。而这一帧的 `text_shaped` 工作计数器是 **0**——
一个字都没重排,阶段时间却主导了整条曲线的陡度。又一次"计数器说只碰了几个节点"。

走的是 `shape_text_for_layout_scoped(上一次布局作用域)`,head 改动的作用域就是整篇文档,
所以 O(N) 是应付的;超线性不是。**没修**,候选还没分清:每个节点在"文本为空就跳过"那一步
**之前**就克隆了一次文本 `String`(`record_string_clone` 专门记了它)、每节点一次按文本内容
做键的 shape 缓存查找、以及缓存容量不够导致的抖动。要先把这三者分开量,才谈得上改。

**全量通道之后的第一帧增量是 O(文档) 的**,见上面第二条。这是刻意换来的:全量通道本身就
是 O(文档),而在它上面记录计划会让每一帧全量都贵 42%。

### 澄清一件事：本轮没有碰那两条长期没过的门禁

`input-cost.md` 把这块成本挂到了 `high-refresh-performance.md` 记的两条未决门禁上
（"Runtime 5,000 节点首次系统处理 P95 40.9 ms"、"标准 Runtime 5,000 节点布局 P95 51.9 ms"，
门禁都是 8 ms）。量完之后这个联系**不成立**：

| | 修复前 | 修复后 |
| --- | ---: | ---: |
| 5,000 节点首次系统处理 P95 | 1.975 ms | 1.992 ms |
| 标准 5,000 节点布局 P95 | 1.763 ms | 1.847 ms |

两条纹丝不动（差异在噪声内）。原因是这两项量的是**全量/首次**通道（`force_full`），而本轮
两处修复都只作用在增量通道上：全量通道会 prefetch，`LayoutInputMap::style()` 直接命中
已 materialize 的表，memo 用不上；而首次投影的 `work.layout` 本来就是整个文档，种子怎么取
都一样。

顺带说明这台机器上跑不出那两个失败：
`python3 scripts/validate-runtime-performance.py --runtime … --framework … --vue … --scene …`
报 `Runtime performance gate: OK`，两项都在 2 ms 上下，远在 8 ms 之内。
`high-refresh-performance.md` 里 40.9 / 51.9 ms 那组数是在 CI 机器（GitHub `ubuntu-latest`）
上记的，与 Apple Silicon 本地数不可比，从这里判断不了它们现在会不会过。

## 附带：13 个 FrameStage 漏计了大回流帧的 45%

量 `head` 的时候发现，8,002 节点那一帧 flush 是 30.0 ms，而六个"跑了"的分段加起来只有
16.4 ms——**13.6 ms（45%）不属于任何 `FrameStage`**。

原因是 Extract 的计时器停得太早：

```rust
let started = Instant::now();
let extracted = world.extract_nodes(&render_dirty);
context.record_extract(&extracted);
context.time_stage_duration(FrameStage::Extract, started.elapsed());
Arc::make_mut(&mut self.scene).apply_delta(extracted, render_removed)  // ← 在计时之外
```

`apply_delta` 要走一遍抽取出的节点、重建它们的图元，并在渲染器还持有上一帧时对场景做
写时复制。把它纳入 Extract 之后，同一帧 Extract 从 1.01 变成 12.99 ms，分段覆盖率从
55% 升到 96%。

**这会让 Extract 的历史基线不可比。** 它使数字变大，但那是真实成本；Issue #8 的性能契约
以及 `validate-runtime-performance.py` 的门禁都建立在这些分段上，让近一半的帧成本隐形比
数字难看更糟。拿本轮之前记录的 Extract 数字与之后的比较是无效的，需要重新取基线。

## 怎么复现

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
# 全网格（五种 shape × 三个 position × 5 × 5）
./target/release/nana-dirty-frame-benchmark --samples 150 --warmup 30 --output report.json
# 单格，便于剖析
./target/release/nana-dirty-frame-benchmark --shape layout --position tail --rows 4000 --dirty 1
# 第五轮的那两列
./target/release/nana-dirty-frame-benchmark --position tail --dirty 1 --samples 150 --warmup 30
```

`--shape paint|layout|nested|nested-auto|layout-auto`、`--position head|tail|spread`、
`--rows`、`--dirty`、`--samples`、`--warmup`。stderr 打人读表格（含分段与
`layout_document_observed` 的四个子阶段），`--output` 写 JSON。第五轮的两份报告是
`performance-data/runtime-dirty-frame-2026-09-08/dirty-frame-measure-plan-{before,after}.json`。

全量通道的两项（`canonical_layout_5000_nodes_ms`、5,000 节点首次系统处理）来自另一个二进制：

```bash
cargo build --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-framework-benchmark --bin nana-runtime-benchmark
./target/release/nana-runtime-benchmark --output runtime.json
./target/release/nana-framework-benchmark --output framework.json \
  $(python3 perf/runners/nana/run.py --print-framework-window-args)
```

## 边界

- 单机单次（macOS / Apple Silicon，release），没有跨机器复现，也没有进 CI 的性能门禁。
- 基准的文档是一个平铺的定高列表。真实文档有嵌套、滚动容器、overlay，容器的分支会不同。
- 五种形状都不覆盖文本内容变化、结构增删、视口变化。文本增长与结构增删只在
  `layout_engine` 的差分哈内斯里覆盖（那里是正确性，不是时间）。
- 第一、二轮只动增量通道，全量/首次通道未受影响。**第五轮动了全量通道**：它现在不再记录
  测量计划，`canonical_layout_5000_nodes_ms` 因此回到 1.82 ms（记录时是 2.55 ms，不记录
  之前是 1.77 ms）。代价是全量之后的第一帧增量是 O(文档) 的。
- `MeasurePlan` 只覆盖平铺在流那一支。2D grid、grid track、IFC、`display:contents`、行内
  拆箱、菜单叠层宿主都不缓存,每帧照常重测。真实文档里这些分支占多少没有量过。
