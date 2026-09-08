# 一个指针事件的成本

给要**选路径**的人，以及要继续优化 Vue 输入路径的人。测量代码是
[`crates/nana-ui-devtools/src/bin/nana-hover-benchmark.rs`](../crates/nana-ui-devtools/src/bin/nana-hover-benchmark.rs)，
原始报告在 [`performance-data/vue-hover-2026-09-08/`](performance-data/vue-hover-2026-09-08/)。

结论先说：**绘制稳态两条路没有差别；输入路径 Vue 曾经贵 22–31 倍，其中约四分之三是
重复计算，已经挡掉，现在是 8–16 倍。** 剩下的差距不再是选型的量级门槛，但仍随节点数
线性增长。

## 测了什么

三棵形状完全相同的树：一列定高（24 px）的行，无滚动容器，指针在列内逐行下移并回绕，
所以没有任何东西会重排。Vue 侧三个变体，Rust L3 一个：

| 变体 | 每行有什么 |
| --- | --- |
| `bare` | 没有任何指针处理器 |
| `listeners` | 每行一个 `onPointerenter`，只写一个不参与渲染的计数器 |
| `reactive` | 每行一个 `onPointerenter`，写一个行样式读取的 `ref` |
| `runtime-l3` | 无处理器。L3 的 hover 工作与是否注册处理器无关 |

三棵树在**同一个进程**里依次建立并驱动，计时取在进程内，所以既没有跨机器状态差异，
也没有把 stdio 往返算进每事件的数字里。

两侧都在 `AgentSession` **下面一层**驱动，各付一次派发加一次沉降：Vue 走
`dispatch_pointer`（结尾自带一次 `pump_frame`）再 `flush_scene_frame`，也就是窗口重绘
做的事；Rust 走 `hover_xy`（派发加 `flush`）。这一层对齐是必须的，两个方向都会错：

- 从 `AgentSession::pointer` 测，会多算 Vue 一次 `pump_frame` 和一次窗口只在 bridge
  revision 变了才做的 `semantic_snapshot`，比值虚高约 4 倍。
- 省掉 `flush_scene_frame`，绘制盒缓存永远是空的，`resolve_layout` 会一直走
  「还没绘制过」的分支——真实应用只在首帧前走那条。

## 结果

macOS / Apple Silicon，`--release`，每档 60 次预热 + 400 次计时移动，取 P50（ms）：

| 行数 | `bare` 修复前 | `bare` 修复后 | 降幅 | `listeners` | `reactive` | L3 | 现比值 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 0.198 | **0.103** | 48% | 0.104 | 1.98 | 0.0065 | 16x |
| 500 | 0.342 | **0.142** | 58% | 0.146 | 3.93 | 0.0122 | 12x |
| 1,000 | 0.634 | **0.230** | 64% | 0.231 | 8.35 | 0.0241 | 9.5x |
| 2,000 | 1.444 | **0.389** | 73% | 0.396 | 18.73 | 0.0483 | 8.1x |

挂载（2,000 行）：Vue 144–162 ms，L3 32 ms，约 5x。挂载不在这次修复范围内。

三件事：

1. **`listeners` 和 `bare` 在三位小数上相同。** 把事件真的送进 JS 处理器不要钱。
   "按 `EventListeners` 剪掉无人监听的事件" 这个优化方向**收益是零**——一个处理器都没有
   的 `bare` 已经在付同样的价。
2. **两条路都还是 O(节点数)。** 比值从 250 行的 16x 收敛到 2,000 行的 8x，说明修掉的那部分
   比留下的部分增长更快。
3. **`reactive` 是 `bare` 的约 40 倍，且没有改善。** 一个 render function 拥有全部行时，
   hover 改一个 `ref` 会让 Vue patch 整列。成因和一条可行的框架侧修法见
   [§`reactive` 的成因](#reactive-的成因每行两次-patchprop而-style-从不比较)。

放进帧预算看：2,000 行时 Vue 一次鼠标移动 `bare` 占 16.67 ms 的 2.3%（修复前 8.7%），
`reactive` 仍要 18.7 ms——超过一整帧。

## 修了什么

`pump_frame` 每次泵都调 `VueHost::resolve_layout`，而每个指针事件泵一次。它下面是七段
全树遍历。2,000 行时的分段计时（ms/事件，总事件 1.44 ms）：

| 段 | 耗时 |
| --- | ---: |
| `sync_cascaded_layout_into_runtime` | 0.341 |
| `reparent_orphans` | 0.240 |
| `flush_host_frame` | 0.190 |
| `sync_layout_containing_blocks` | 0.121 |
| `sync_sidebar_footer_into_document` | 0.083 |
| store `retain` + `snapshot` | 0.084 |
| `apply_layout_boxes` + `reapply_scroll_translations` | 0.041 |
| **合计 `resolve_layout`** | **1.042** |

没有单个主犯——七段都是 O(n)，而指针移动不改变布局，所以七段都在重算上一次的结果。
所以门开在入口：`resolve_layout` 现在按
`(LayoutBoxStore::revision, bridge.revision(), 逻辑视口)` 判断，三者都没变就整段跳过。

**`LayoutBoxStore::revision` 是这把钥匙的关键一半。** Scene 每帧会把每个可见节点重新
`record` 一遍，不管它动没动，所以"被写过"每帧都为真、毫无信息量。store 现在只在写入
**真的改变了内容**时才自增；`enqueue_layout_if_changed` 早就在按节点做同样的比较，只是
把结论丢掉了。

### 一个必须先解决的问题：它本来就不是幂等的

第一版门是错的，测试当场抓到：`resolve_layout` 跑一趟和跑两趟结果不同。挂载后第一趟只
把节点写成 0 尺寸，第二趟才把绘制几何投影进去——**旧代码之所以能得到正确结果，只是因为
它每帧都在跑**，而一道门会把文档冻在收敛途中。

所以现在 `resolve_layout` 在**一次调用内**跑到不动点再记录状态，收敛信号是
`apply_layout_boxes` 有没有真的改写任何节点的布局（上限 8 趟，与 `nana_ui_scene` 的
`MAX_FRAME_PASSES` 一致）。最坏情况与改动前做一样多的活，稳态一趟不做。

### 顺带确认的一条契约

绘制盒**只填补引擎没产出的盒子，或扩大引擎报小了的范围，从不重定位**
（`write_layout_boxes` 的 `overwrite = false` 分支）。第一版回归测试断言"移动过的盒子必须
到达文档"，那是在断言一个不存在的契约——测试错了，不是代码错了。现在测的是扩大范围。

## 还剩什么：一次事件的两半

基准现在把每个 Vue 事件拆成 `dispatch`（指针进入这一层：命中、DOM 事件送进 JS、随之而来的
patch、一次 host 帧泵）和 `settle`（`flush_scene_frame`：提交 host op、跑 Runtime 系统、
重录每个绘制盒）。2,000 行、P50 ms：

| 模式 | 总计 | dispatch | settle |
| --- | ---: | ---: | ---: |
| `bare` | 0.379 | **0.061** | 0.315 |
| `listeners` | 0.375 | 0.061 | 0.312 |
| `reactive` | 15.20 | **14.75** | 0.458 |

**dispatch 是常数。** 250 → 2,000 行全都是 0.055–0.061 ms。事件本身的派发路径不随树增长，
之前那 22–31 倍里没有一分是"把事件送进 JS"。

### O(n) 全在 `flush_scene_frame`

`settle` 是唯一还随树线性增长的部分（0.039 → 0.077 → 0.152 → 0.315 ms）。它内部分三段，
2,000 行 ms/事件：

| 段 | 耗时 |
| --- | ---: |
| `doc.flush_host_frame()` + `report_commit_rejections` | 0.135 |
| `flush_runtime_scene`（`RuntimeDocument::flush`） | **0.0002** |
| `document_order` 遍历 + `node_bounds` + `layout_boxes.record` | 0.184 |

Runtime 自己的 flush 在无脏数据时**已经几乎免费**——它自己挡得很好。剩下两段是 Vue 层
自己的全树遍历：每帧把每个节点的绘制盒重新收集一遍（一个全量 `Vec`）再逐个 `record`，
而 `record` 每次取三把锁，2,000 行就是每事件 6,000 次加解锁，产出与上一帧完全相同的值。

同一把钥匙适用：`LayoutBoxStore::revision` 现在已经能回答"这一帧录进去的东西变了没有"，
所以这段可以按 Scene 有没有实际变化跳过。没做，因为它不在这次改动范围内。

L3 的 `no-handler` 同样是 O(n)（0.0063 → 0.0447 ms），来自它自己的 `flush`。两边剩下的
线性成本是同一类东西，这也是比值只有 8 倍而不是更大的原因。

### `reactive` 的成因：每行两次 `patchProp`，而 `style` 从不比较

`reactive` 的 14.75 ms 全在 dispatch 内。在 V8↔Rust 边界上计数，2,000 行**每个指针事件
发生 4,001 次 `patchProp`**，合计 10.8 ms（其余约 7 ms 在 JS 侧：Vue 对 2,000 个 vnode 的
diff 加上跨界值转换）。

一次 hover 只有一行的颜色变了，为什么是 4,001 次？因为 render function 每次都重建整列，
而每行有两个 prop 的**引用**每次都是新的：

- `style` 是一个新的对象字面量；
- `onPointerenter` 是一个新的箭头函数。

Vue 的 `patchProp` 按引用判断变化，于是两个都对所有 2,000 行触发。把处理器提到渲染外
（缓存住身份）验证过：调用数 4,001 → 2,001，但耗时只降 9%——**贵的是 `style` 那一半**，
每次约 4.9 µs，而监听器那半只有 0.5 µs。

`style` 贵是因为它进 CSS 级联。而
[`createNanaRenderer.js`](../packages/nanavue-runtime/src/createNanaRenderer.js) 的
`patchProp` 拿到了 Vue 给的 `prev`，**却在 style 分支里没有用它**：清洗完就无条件
`hostCall("patchProp", ...)`。

所以这不只是写法问题——**渲染器可以按值比较而不是按引用**：清洗后的声明与上次送出的相同
就不过界。那会让"内联 style 对象 + 长列表"这一整类应用的这项成本消失。这条还没做。

应用侧同时仍然值得做的：把行拆成各自的组件，这样一个 `ref` 变化只 patch 一行而不是整列。
## 怎么复现

```bash
for mode in bare listeners reactive; do
  node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs $mode 2000
done
cargo build --release -p nana-ui-devtools --features agent-bin --bin nana-hover-benchmark
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60
```

Vue 侧 bundle 落在 `target/`（不是 `dist/`）：它们是按需重生成的测量输入，不是 CI 要逐字节
比对的已提交 fixture。行数要和 `--rows` 一致，否则一边在重复 hover 同一行、另一边在换行。

## 边界

- 单机单次，没有跨机器复现，也没有进 CI 门禁。
- 只测了 hover。点击、滚动、键盘走同一个 `pump_frame`，所以同样受益，但没测。
- 修复的正确性证据是 workspace 2,911 项测试、Agent tier 40 项、以及 `vue-sfc-compat` 的
  虚拟列表 / 冻结表格 / 焦点保持三个验收脚本，全部通过；Gallery 的 559 张像素门禁测的是
  纯 Rust 路径，碰不到这段代码。
- `nana-vue-runtime-benchmark` 仍然不 import V8，测的是 Vue 层的 Rust 那一半；这个二进制
  是目前唯一测过 JS↔Rust 边界的。
