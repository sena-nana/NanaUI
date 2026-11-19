# 性能

没有变更时，`flush` 是空转，大约 0.0001 ms，不随树的大小变化。宿主此时不应该交出一帧空画面。

动画、实时 GPU 和普通 UI 的唤醒是分开的。一块画面在动，不该迫使整棵 Runtime 全量更新。合成器轨道在稳态只推进时间，不写回 `UiWorld`。静态窗口保持 `FrameDemand::OnDemand`。窗口被遮挡或最小化时，宿主仍按程序自己的 `FrameDemand` 调用 `prepare_window_frame`，这时不 flush，不获取 Surface，也不 present。

`width` 和 `height` 走布局。每一次采样都会再布局一次。不要把它们做成缩放。

需要重绘的窗口大致走这些步：消化 `dispatch_program` 的消息，`prepare_window_frame`，`RuntimeDocument::flush`，获取 Surface，`SceneWgpuPainter::paint_target`，submit 和 present，然后才是 `window_frame_presented`。合成器自己要 tick 时，只把该窗口接到 `FrameDemand::Continuous`。它不强制样式或布局重算。0 尺寸仍然 prepare；producer 的编码只在尺寸可画时才跑。

## 重构后的基准快照

在五阶段架构基线重置之后，Apple M4 `release` / `dist` 实测读数如下（完整口径与复现命令见 [资源预算](../../reference/resource-budgets.md) 与 [Taffy 对照基准](../../reference/taffy-reference.md)）：

- **栈安全、零 `stacker`**：测量（`measure_node`）与视图建树（`ViewBuilder::run_trampoline`）均在堆上显式工作栈迭代执行（`0 B` 测量每层栈），放置按 `recursion::SPAN` 与 `Boundary` 队列截断；`500` 层深树整帧最低线程栈为 **`52 KiB`**（Taffy 为 `900 KiB`）。
- **节点常驻内存**：`NodeRecord <= 152 B`、`Hierarchy == 24 B` 配合分页槽位 `Slots` 与共享 `Arc<NodeStyle>`，10k 已布局 `Stack` `634 B`、`stack-depth wide` `21,001` 盒实测 **`943 B / 盒`**（Taffy `1,506 B / 盒`）。
- **局部增量与零开销观测**：`100k` 节点局部帧 `p95` 耗时 `30.2 µs`，与 `1k` 节点（`25.8 µs`）之比为 **`1.17×`**；`WorkCounters` 与 `HeadlessTurn::work` 在不增加单节点常驻内存的前提下守住增量计数。
- **预编译着色器与分发体积**：Apple 默认加载预编译压缩 MSL（常驻 `naga` IR `0 B`），其余平台加载紧致压缩 `NAGA1` IR；配合 `[profile.dist]`，`reactive-sfc` 可执行体积从 `15.80 MiB` 降至 **`5.56 MiB`**（`-64.8%`）。

## 一条指针事件

派发本身是常数。测量是 macOS / Apple Silicon、`--release`、60 次预热加 400 次移动、`window` 形状的 P50。生产窗口走 `prepare_window_frame`。`headless` 形状走 `flush_scene_frame`，生产窗口不走那条路。

Rust L3 在修好按组件类型的索引之后，250 到 2,000 行每个规模都是 0.0003 ms（修之前 2,000 行是 0.0819 ms）。

## 真的改过的帧

上面的常数成立，是因为那棵树什么都没变。一旦有改动，窗口帧要做的工作就回来了。纯绘制按改动量走；牵动布局的帧曾经会扫到文档根，也会对每个兄弟再解析一次样式，这两处已经修掉。

欠下的工作本身是常数时，代价也是常数：502 到 8,002 个节点都在 0.008–0.011 ms。下面每一行都要移位时，仍然是 O(节点数)。

长列表用 `each_virtual`：5 万行挂载加布局加出窗口 2.4 ms，一次建完的 `each` 是 175 ms，每行常驻约 2.3 KB。写法见 [列表](../essentials/list.md)。

帧上要和 UI 对齐，就走宿主这一次提交：

- 不为界面再 `request_device` 一次
- 不让实时内容用另一套 Queue 提交
- 不把 GPU 画面读回 CPU 再贴回去
- 不在 UI 画完之后往 Surface 上再盖一层
- 控件不拿窗口句柄去调系统 API

新代码从 `nana_ui::runtime` 进入。同一条字符串 slot 上的 `GpuTextureView` 仍在这棵树里参与布局、裁剪和命中。

无头对照把 `--shape window` 换成 `--shape headless`。滚动版本再加 `--scroll`。行数要和生成 bundle 时的行数一致。

这些数字来自一台机器上的 hover，不是 CI 门禁。时钟曾经被冻在 0，那时 2,000 行的 L3 记成 0.0242 ms，比后来的 0.0819 ms 乐观大约 3.4 倍。现在每个事件按 16 ms 推进，上面的 0.0003 ms 是索引修好之后的数。

测量命令和分段计时在 [一个指针事件的成本](../../reference/input-cost.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/accessibility">
    <p class="next-step-link">无障碍</p>
    <p class="next-step-caption">隐藏节点怎样留在树上，名字从哪来。</p>
  </a>
  <a class="next-step" href="/guide/essentials/list">
    <p class="next-step-link">列表</p>
    <p class="next-step-caption">视口之外的行不必一次建出来。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">进出场走合成器，稳态不写回逻辑样式。</p>
  </a>
</div>
