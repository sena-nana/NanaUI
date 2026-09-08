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
| 250 | 0.061 | 0.062 | 0.98 | 0.006 |
| 500 | 0.060 | 0.062 | 1.87 | 0.012 |
| 1,000 | 0.065 | 0.063 | 4.36 | 0.024 |
| 2,000 | 0.065 | 0.063 | 11.07 | 0.049 |

（`window` 形状。`headless` 形状的 `bare` 同样是 0.060–0.063；`reactive` 低一些，因为它
不做窗口那套语义同步。滚动版本与不滚动的相同：2,000 行 `bare-scroll` 0.065。）

四件事：

1. **`bare` 与 `listeners` 是常数**，250 到 2,000 行都是 0.06 ms 上下。事件派发不随树增长，
   而且两者相等——把事件真的送进 JS 处理器不要钱。
2. **L3 仍然是 O(节点数)**（0.006 → 0.049 ms）。于是比值随规模**下降**：2,000 行上
   Vue 只比 L3 贵 1.3 倍。Vue 有约 0.06 ms 的固定底噪，小树上比值看着大但两边都微不足道。
3. **`reactive` 是 `bare` 的 170 倍**，而且是唯一还随树增长的一项。这是写法，见下。
4. 滚动不再有惩罚。曾经有：无头路径上滚动的树每事件 1.9 ms。

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
