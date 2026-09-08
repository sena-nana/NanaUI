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
| `runtime-l3` | 无处理器。L3 的 hover 工作与是否注册处理器无关 |

四棵树在**同一个进程**里依次建立并驱动，计时取在进程内。

**两种形状，`--shape` 选。** `window` 跑 `VueHost::prepare_window_frame`，也就是真实窗口
重绘跑的那套；`headless` 跑 `flush_scene_frame`，那是 Agent 会话的替身。这个区分是必须的
——`flush_scene_frame` 全仓只有三个调用方，**生产窗口不走它**，只量它会得出一条没有
窗口会走的路径的结论。此前几轮就是这么错的。

## 结果

macOS / Apple Silicon，`--release`，60 次预热 + 400 次计时移动，P50（ms）：

| 行数 | `bare` | `listeners` | `reactive` | L3 |
| ---: | ---: | ---: | ---: | ---: |
| 250 | 0.061 | 0.062 | 0.98 | 0.003 |
| 500 | 0.061 | 0.062 | 1.87 | 0.006 |
| 1,000 | 0.065 | 0.063 | 4.36 | 0.012 |
| 2,000 | 0.064 | 0.063 | 11.07 | 0.024 |

（`window` 形状。`headless` 形状的 `bare` 同样是 0.060–0.063；`reactive` 低一些，因为它
不做窗口那套语义同步。滚动版本与不滚动的相同：2,000 行 `bare-scroll` 0.065。）

四件事：

1. **`bare` 与 `listeners` 是常数**，250 到 2,000 行都是 0.06 ms 上下。事件派发不随树增长，
   而且两者相等——把事件真的送进 JS 处理器不要钱。
2. **L3 仍然是 O(节点数)**（0.003 → 0.024 ms，已经砍掉一半，还剩一处）。于是比值随规模
   **下降**：2,000 行上 Vue 比 L3 贵 2.7 倍。Vue 有约 0.06 ms 的固定底噪，小树上比值看着大
   但两边都微不足道。
3. **`reactive` 是 `bare` 的 170 倍**，而且是 Vue 侧唯一还随树增长的一项。这是写法，见下。
4. 滚动不再有惩罚。曾经有：无头路径上滚动的树每事件 1.9 ms。
5. **现在轮到 L3 是那条线性的。** 见下一节。

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

## L3 侧的 O(节点数)

把 Vue 压成常数之后，剩下线性的那条是 Rust L3。三个黑盒判别先把它框住（都用真计时）：

| 问题 | 做法 | 结果 |
| --- | --- | --- |
| 是 `flush` 吗？ | `hover_xy` 之后再空跑一次 `flush` | **不是**。空闲 flush 0.0001 ms，不随规模变——Runtime 自己的帧门挡得很好 |
| 是 hover 转换吗？ | 同一行再悬停一次 | **不是**。0.0473 vs 换行的 0.0476 |
| 是命中目标查找吗？ | 指针移到树外面 (-500, -500) | **不是**。0.0470，一样贵 |

也就是说：每个指针事件都跑、与是否命中和是否换目标都无关的一段全树工作。再往里分段计时，
是两处，各占一半。

### 已修：`active_runtime_overlays` 每事件重建全文档反查表

`route_overlay_pointer` 在每个指针事件上问"有没有浮层挡住指针"，而
`active_runtime_overlays` 回答这个问题的方式是把整个 `document_order` 收集成 Vec，再建一张
全量的 id → 位置 HashMap，然后遍历。一棵 2,000 节点、零浮层的树上，这是每事件两次全量分配，
只为得到一个空列表。

索引本来就存在：`UiWorld::overlay_hosts_by_document`，它的文档注释写着 "Overlay validation
iterates this instead of the entity index so cost tracks host count, not world size"。这个函数
没有用它。改成从索引出发，并在没有浮层时直接返回。

`route_overlay_pointer` 0.0233 → 0.0001 ms（常数）。L3 每事件 2,000 行 0.0489 → 0.0242（−50%）。
Gallery 的 559 张像素门禁全匹配——那套快照走的正是这条纯 Rust 路径，是这次改动的正确性证据。

### 未修：`split_handle_near` 的全文档回退扫描

剩下的一半在 `split_pane.rs` 的 `split_handle_near`：先试指针目标再走祖先链，都没命中就
**回退到 `document_order` 全扫**，找 6px 松弛范围内的分割手柄。一棵没有分割面板的树上，
每次指针移动都跑这一遍。有一个 8 ms 的时间节流（`begin_split_hover_probe`），但那是节流不是
消除。

修法与 overlay 那条同形：需要一份按文档维护的分割面板索引。目前没有——`is_split_pane` 是
逐节点的视图 downcast，`views` 也没有按类型索引。所以这条没做：它需要先决定索引挂在哪、
在哪注册与注销，而那是设计决定不是改一行。

### 一个测量陷阱

`AppContext::last_frame_profile()` 和 `last_work_counters()` **保留最后一次非空闲的值**
（`finish_frame_profile` 里 `if profile.any_stage_ran()`）。在空闲帧上读它们会拿到挂载帧的
数据——第一次量就是这样，读出"每事件 TextShape 12.3 ms"，比实测的整个事件还大 250 倍。

## 还剩什么：`reactive`

`reactive` 在 2,000 行上仍要 11.07 ms（超过半帧），其中 8.27 在 dispatch、2.80 在 settle。
成因是**一个 render function 拥有整列**：hover 改一个参与渲染的 `ref`，Vue 就重建全部
2,000 个 vnode。剩下的成本是 Vue 自己的 diff，加上每行一个新箭头函数导致的监听器 patch
（那些确实变了——闭包身份每次渲染都不同），加上随之而来的语义同步。

框架侧能做的已经做了：值相同的 style 不再跨界。**剩下的归应用**：

- 把行拆成各自的组件，一个 `ref` 变化只 patch 一行；
- 或者让 hover 只改不参与渲染的状态（`listeners` 变体就是这样，它是常数成本）；
- 处理器提到渲染之外缓存住身份，能再去掉每行一次的监听器 patch。

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
