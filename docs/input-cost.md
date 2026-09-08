# 一个指针事件的成本

给要**选路径**的人，以及要继续优化 Vue 输入路径的人。测量代码是
[`crates/nana-ui-devtools/src/bin/nana-hover-benchmark.rs`](../crates/nana-ui-devtools/src/bin/nana-hover-benchmark.rs)，
原始报告在 [`performance-data/vue-hover-2026-09-08/`](performance-data/vue-hover-2026-09-08/)。

结论先说：**Vue 每个指针事件现在是常数约 0.062 ms，不随树的大小变化。** 在 2,000 节点上
它是 Rust L3 的 1.3 倍——L3 自己还是 O(节点数)。指针密集不再是选型理由。

剩下的唯一大项是一种**写法**：hover 处理器改动参与渲染的状态时，一个 render function
拥有的整列都会重 patch。

## 测了什么

四棵形状相同的树：一列定高（24 px）的行，指针在列内逐行下移并回绕。Vue 侧三个变体，
Rust L3 一个；另有把行放进滚动容器的 `-scroll` 版本。

| 变体 | 每行有什么 |
| --- | --- |
| `bare` | 没有任何指针处理器 |
| `listeners` | 每行一个 `onPointerenter`，只写一个不参与渲染的计数器 |
| `reactive` | 每行一个 `onPointerenter`，写一个行样式读取的 `ref` |
| `reactive-components` | 同样的可见行为，但每行是自己的组件、拥有自己的 hover `ref`——文档推荐的写法 |
| `runtime-l3` | 无处理器。L3 的 hover 工作与是否注册处理器无关 |

四棵树在**同一个进程**里依次建立并驱动，计时取在进程内。

**两种形状，`--shape` 选。** `window` 跑 `VueHost::prepare_window_frame`，也就是真实窗口
重绘跑的那套；`headless` 跑 `flush_scene_frame`，那是 Agent 会话的替身。这个区分是必须的
——`flush_scene_frame` 全仓只有三个调用方，**生产窗口不走它**，只量它会得出一条没有
窗口会走的路径的结论。此前几轮就是这么错的。

## 结果

macOS / Apple Silicon，`--release`，60 次预热 + 400 次计时移动，P50（ms）：

| 行数 | `bare` | `listeners` | `reactive` | `reactive-components` | L3 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 0.056 | 0.057 | 0.78 | 0.61 | 0.0003 |
| 500 | 0.057 | 0.057 | 1.47 | 1.09 | 0.0003 |
| 1,000 | 0.060 | 0.058 | 3.35 | 2.23 | 0.0003 |
| 2,000 | 0.061 | 0.058 | 8.63 | 5.27 | 0.0003 |

（`window` 形状。`headless` 形状的 `bare` 同样是 0.060–0.063；`reactive` 低一些，因为它
不做窗口那套语义同步。滚动版本与不滚动的相同：2,000 行 `bare-scroll` 0.065。）

四件事：

1. **`bare` 与 `listeners` 是常数**，250 到 2,000 行都是 0.06 ms 上下。事件派发不随树增长，
   而且两者相等——把事件真的送进 JS 处理器不要钱。
2. **L3 现在也是常数**（0.0003 ms，任何规模）。两条路都不再随树增长；Vue 的固定底噪约
   0.06 ms，是 L3 的 200 倍，但绝对值仍在十分之一毫秒以内。
3. **`reactive` 是 `bare` 的 140 倍**，而且是唯一还随树增长的一项。到这一步它才真的只剩写法，
   见下——在此之前它两次被误判成"写法问题"，两次都还有框架侧的账没结。
4. 滚动不再有惩罚。曾经有：无头路径上滚动的树每事件 1.9 ms。
5. **L3 也已经收成常数。** 见下一节。

## 怎么走到这里的

四轮改动，每轮都先量后改：

| 改动 | 作用 | 2,000 行效果 |
| --- | --- | --- |
| `resolve_layout` 按 revision 挡（`8157c65`） | 每事件七段全树遍历重算同一个结果 | 无头 1.444 → 0.379 ms |
| `sync_svg_rasters` 走索引（`1929d24`） | 每窗口帧最多四次全节点扫描 + 每节点一次 String 分配 | 生产 0.583 → 0.062 ms |
| `patchProp` 的 style 按值比较（`707d161`） | 内联 style 对象每次渲染都是新引用，每行一次跨界进 CSS 级联 | `reactive` 60.3 → 10.9 ms |
| `flush_scene_frame` 按 Scene instance 挡（`b52c9b8`） | 每帧重录每个绘制盒，每节点三把锁 | 无头 0.258 → 0.063；滚动 1.903 → 0.064 |

一路上被数据推翻的判断，留在这里因为它们都是容易再犯的：

- "按 `EventListeners` 剪掉无人监听的事件" —— 收益为零。无处理器的 `bare` 与每行都有
  处理器的 `listeners` 耗时相同，事件派发本来就不是成本。
- "Vue 比 L3 贵 80–107 倍" —— 测量假象。那次经 `AgentSession::pointer` 驱动，它在
  `dispatch_pointer` 已经泵过一帧之后又泵一帧，而 L3 侧没有对应物。
- "reactive 是写法问题，框架侧改不掉" —— 不对。渲染器拿到了 Vue 给的 `prev` 却没用它。
- "`resolve_layout` 幂等" —— 不。挂载后第一趟只写 0 尺寸，第二趟才投影绘制几何；旧代码
  靠每帧都跑才收敛，第一版门把文档冻在收敛途中。它现在在一次调用内跑到不动点再记状态。

## L3 侧曾经的 O(节点数)：已收成常数

Vue 压成常数之后，剩下线性的那条是 Rust L3。三个黑盒判别先把它框住：不是 `flush`
（空闲 flush 0.0001 ms 且不随规模变）、不是 hover 转换（同一行再悬停一样贵）、
不是命中目标查找（指针移到树外一样贵）。也就是每个指针事件都跑、与命中和转换都无关的
一段全树工作。

分段计时找出三处，全是同一个形状——**"这个文档里的每个 X" 用扫描 `document_order` 加过滤
来回答**：

| 处 | 谁在问 | 每事件成本（2,000 行） |
| --- | --- | ---: |
| `active_runtime_overlays` | `route_overlay_pointer`：有没有浮层挡住指针 | 0.0233 ms |
| `split_handle_near` | 指针是否在分割手柄的 6px 松弛内 | 与下一行合计 0.0246 ms |
| `sync_split_handle_hover` | 释放文档里所有 hover 中的分割面板 | 同上 |
| `clear_calendar_heatmap_hover` | 清掉所有日历热力图的 hover | 余下全部 |

第一处的索引本来就存在（`UiWorld::overlay_hosts_by_document`，注释写着 "so cost tracks
host count, not world size"），只是那个函数没用。后三处没有索引。

**修法：把索引放在 `component_type` 旁边。** `UiWorld` 现在维护
`nodes_by_component`，在 `SetComponentType` 应用处和 despawn 处更新。放在 world 而不是
`AppContext`，是因为只有那里是唯一权威：语义绑定路径（`finish_semantic_binding`）根本不经过
`stamp_component_type`，索引挂在框架层就会漏。

**结果：L3 每指针事件 0.0819 → 0.0003 ms，任何规模都一样**（250 到 2,000 行全是 0.0003）。
不再随节点数增长。

正确性证据是 Gallery 的 559 张像素门禁全匹配——它走的正是这条纯 Rust 路径，有真实的对话框、
菜单、抽屉、分割面板和 tooltip。

## 修掉的一个测量盲区：基准以前把时钟冻在 0

`RuntimeAgentSession` 用的是 `RuntimeInputAdapter::dispatch`，那个便捷方法把 `now` 传成
`Duration::ZERO`（带时钟的是 `dispatch_at`）。于是所有按时间节流的路径**只放行第一次**：
`split_handle_near` 前面有 8 ms 节流，`now` 恒为 0 意味着第一次之后永远 false，它的全文档
扫描全程只跑了一遍。

后果不只是少测了一处：**冻结时钟下的 L3 数字整体偏乐观 3.4 倍**（2,000 行 0.0242 vs 真实
0.0819）。会话现在按每个事件推进一帧（16 ms），用固定步长而不是墙钟，脚本化的会话仍然可复现。

这也让无头会话第一次能触发 tooltip 延时这类行为——对 `$nanaui-agent-debug` 是能力增加，
不是副作用。

## 还剩什么：任何真的改动过的帧都是 O(节点数)

上面那张表最容易被误读的一行是 `bare` 的 0.06 ms 常数。它常数，是因为那棵树**什么都没变**。
一旦有任何东西变了，窗口帧就回到 O(总节点数)——`reactive` 与 `reactive-components` 的
settle 几乎一样（2,000 行 2.58 vs 2.42 ms），而 `bare` 的是 0.0005。

一路分段计时下去，2,000 行、每事件只有 **2 个** widget 变脏时：

| 层 | 每帧 | 占比 |
| --- | ---: | ---: |
| `VueHost::sync_semantics` | 2.53 ms | 100% |
| └ `sync_semantics_from_bridge` | 2.03 | 80% |
| 　└ `apply_semantic_styles` | 1.54 | 61% |
| 　　└ **`flush_runtime_systems`** | **1.51** | **60%** |
| 　└ `prepare_semantic_styles` | 0.52 | 21% |
| └ `reparent_orphans` | 0.25 | 10% |
| └ `sync_layout_containing_blocks` | 0.15 | 6% |
| └ `sync_sidebar_footer_into_document` | 0.08 | 3% |

**Vue 层的增量已经在工作了。** 351 次同步里只有 1 次是全量（挂载那次），其余 350 次每次只
投影 2 个 widget——`prepare_semantic_styles` 的 dirty 路径没问题。`apply_semantic_styles` 里
`commit_extra` 0.013、`adopt_runtime_allocated_ids` 0.020，也都不是问题。

**成本落在 `flush_runtime_systems`，也就是 Runtime 自己的帧。** 2 个节点脏，2,000 节点的树
上要 1.51 ms。对照：同一棵树上什么都不脏时，Runtime 的空闲 flush 是 0.0001 ms 且不随规模变
——它的"没活干就退出"挡得很好，"有活干"的那条路是 O(总节点数)。

这不是新发现，是仓库自己记着的未决门禁：`high-refresh-performance.md` 里
"Runtime 5,000 节点首次系统处理 P95 40.9 ms / 门禁 8 ms"、"标准布局 P95 51.9 ms / 门禁 8 ms"
一直没过，Issue #8 的阈值也没放宽。这里的贡献只是指出：**它就是 Vue 侧"改动过的帧"成本的
主体**，而 Vue 层之上已经没什么可省的了。

### 所以框架对写法中立了吗：Vue 层是，Runtime 层还不是

`reactive` 和 `reactive-components` 的 settle 相差 6%（2.58 vs 2.42），因为两者都只脏了 2 个
widget，Vue 层照此收费。**这一层已经中立了。** 剩下的差别在 dispatch，那是 Vue 自己的 diff，
是写法真正该负的账。

但两者的 settle 都是 2.4 ms 而不是 0.002 ms——Runtime 按树大小而不是按改动量收费，于是**写对
的那种也照样被罚**。要让框架真正对写法中立，缺口在 Runtime 的脏帧路径，不在 Vue 层。

## `reactive` vs `reactive-components`：拆组件值多少

文档一直建议"把行拆成各自的组件，一个 `ref` 变化只 patch 一行"。给它量一个数：

**2,000 行 8.63 → 5.27 ms，省 39%。仍然是 O(节点数)。**

省下的是 Vue 侧的 vnode diff（dispatch 6.13 → 2.90）。没省下的是 settle——因为上一节那处
与 Vue 怎么写无关。所以这条建议是真的，但它把一个"超过半帧"变成"三分之一帧"，不是变成常数；
真正的常数要等 `sync_semantics` 收窄。

`reactive` 这个变体不是稻草人：模板里的 `v-for` 编译出来就是"一个 render function 返回 N 个
vnode"，不生成子组件，所以 `<div v-for="row in 2000" :style="…">` 与它同形。

### 它三次抓出框架侧的账

把它当成"写法问题"是我在这份文档里犯过的错，每次都还有框架的账没结：

| 发现 | 每事件跨界（2,000 行） | 效果 |
| --- | --- | --- |
| `patchProp` 的 style 按引用而非按值比较 | 2,000 次，每次进 CSS 级联 | 60.3 → 10.9 ms |
| 监听器换了闭包身份就重新告知宿主 | 2,000 次，每次入队一条 world mutation | 10.9 → 8.97 ms |
| `sync_semantics` 之下的 `flush_runtime_systems` | 0 次跨界，但 2 个脏节点要 1.5 ms | 未修，见上节 |

第二处尤其能说明问题：JS 侧的 invoker 模式**已经**正确地避免了监听器变动
（`existing.value = handler`，不碰 `addEventListener`），然后紧接着还是
`hostCall("patchProp", [nid, key, true])`——把一件宿主早就知道的事又说了一遍。

## 怎么复现

```bash
for mode in bare listeners reactive bare-scroll listeners-scroll reactive-scroll; do
  node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs $mode 2000
done
cargo build --release -p nana-ui-devtools --features agent-bin --bin nana-hover-benchmark
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60 --shape window
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60 --shape headless --scroll
```

Vue 侧 bundle 落在 `target/`（不是 `dist/`）：它们是按需重生成的测量输入，不是 CI 要逐字节
比对的已提交 fixture。行数要和 `--rows` 一致，否则一边在重复 hover 同一行、另一边在换行。

## 正确性怎么保的

这些改动全是"跳过一次本该重算的工作"，而那类改动的失败方式是安静的。守它们的是
`crates/nana-ui-vue/src/host/frame.rs` 里的**等价性哈内斯**：同一脚本（挂载、空闲帧、滚动、
追加节点、改视口、改样式）在开门与关门的两个 host 上逐步并行推进，每步之后比较文档快照
**和 JS 实际读回的绘制几何**。它不需要 V8 也不需要打包产物，所以跑在默认的
`cargo test --workspace` 里。

第一版哈内斯只比了 `BoxSnapshot`，把两个故意改坏的门都放过去了——那读的是文档的布局盒，
而 scene 门决定的是要不要重填绘制盒 store。**一个从不失败的等价性测试比没有更糟**，所以
它是拿故意改坏的门验过的。

它抓的是行为差异，不是最优性：一个只少省一次工作、结果相同的门不会被它判失败。

## 边界

- 单机单次，没有跨机器复现，也没有进 CI 的性能门禁。
- 只测了 hover。点击、滚动、键盘走同一个 `pump_frame`，所以同样受益，但没测。
- Vue 侧仍**没有像素基线门禁**（gallery 的 559 张走纯 Rust 路径）。这四轮的像素证据来自
  `vue-sfc-compat` 的三个验收脚本和 Agent tier 的截图/语义交叉验证。
