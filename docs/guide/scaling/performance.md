# 性能

没有变更时，`flush` 是空转，大约 0.0001 ms，不随树的大小变化。宿主此时不应该交出一帧空画面。

动画、实时 GPU 和普通 UI 的唤醒是分开的。一块画面在动，不该迫使整棵 Runtime 全量更新。合成器轨道在稳态只推进时间，不写回 `UiWorld`。静态窗口保持 `FrameDemand::OnDemand`。窗口被遮挡或最小化时，宿主仍按程序自己的 `FrameDemand` 调用 `prepare_window_frame`，这时不 flush，不获取 Surface，也不 present。

`width` 和 `height` 走布局。每一次采样都会再布局一次。不要把它们做成缩放。

需要重绘的窗口大致走这些步：消化 `dispatch_program` 的消息，`prepare_window_frame`，`RuntimeDocument::flush`，获取 Surface，`SceneWgpuPainter::paint_target`，submit 和 present，然后才是 `window_frame_presented`。合成器自己要 tick 时，只把该窗口接到 `FrameDemand::Continuous`。它不强制 Vue 再 patch，也不强制样式或布局。0 尺寸仍然 prepare；producer 的编码只在尺寸可画时才跑。

## 一条指针事件

派发本身是常数。测量是 macOS / Apple Silicon、`--release`、60 次预热加 400 次移动、`window` 形状的 P50。生产窗口走 `prepare_window_frame`。`headless` 形状走 `flush_scene_frame`，生产窗口不走那条路。

| 行数 | bare | listeners | reactive | reactive-components | L3 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 0.056 | 0.057 | 0.78 | 0.61 | 0.0003 |
| 500 | 0.057 | 0.057 | 1.47 | 1.09 | 0.0003 |
| 1,000 | 0.060 | 0.058 | 3.35 | 2.23 | 0.0003 |
| 2,000 | 0.061 | 0.058 | 8.63 | 5.27 | 0.0003 |

`bare` 没有指针处理器。`listeners` 每行一个处理器，只改一个不参与渲染的计数。两者一样，把事件送进 JS 不要钱。单位是毫秒。

Rust L3 在修好按组件类型的索引之后，每个规模都是 0.0003 ms（修之前 2,000 行是 0.0819 ms）。Vue 这条路的固定底噪约 0.06 ms，大约是 L3 的 200 倍，绝对值仍在十分之一毫秒以内。滚动不再另加一笔：无头路径上曾经到过 1.9 ms，现在 2,000 行的 `bare-scroll` 是 0.065 ms。

贵的是处理器改了参与渲染的状态，而且整列行属于同一个渲染函数。同一张表里，2,000 行的 `reactive` 是 8.63 ms，大约是 `bare` 的 140 倍。把每行拆成自己的组件（`reactive-components`）是 5.27 ms，省 39%，仍然随行数增长。模板里的 `v-for` 编译出来就是一个函数返回 N 个节点，和 `reactive` 同形。

## 真的改过的帧

上面的常数成立，是因为那棵树什么都没变。一旦有改动，窗口帧要做的工作就回来了。后来把「脏的是绘制还是布局」拆开量过：纯绘制按改动量走；牵动布局的帧曾经会扫到文档根，也会对每个兄弟再解析一次样式。这两处修完之后，2,000 行 `reactive` 的每事件从 8.87 ms 到 4.78 ms，settle 从 2.66 ms 到 0.73 ms；`reactive-components` 从 5.45 ms 到 1.79 ms。`bare` 和 `listeners` 不变。

欠下的工作本身是常数时，代价也是常数：502 到 8,002 个节点都在 0.008–0.011 ms。下面每一行都要移位时，仍然是 O(节点数)。

投影的集合可以是增量的，花费的时间仍可能随文档增长。一次 hover 的 settle 里，后来量到大约 73% 在 Vue 层，其中四处随文档变长：`try_bind_registered_component`、`find_sidebar_reparent_host`、`sync_layout_containing_blocks`，以及交给 Runtime 的脏工作 `flush_runtime_systems`。早期「Vue 层已经按改动量收费、不必再看」以及「这笔账就是全量布局门禁」两句，后续测量都改掉了。全量通道上的那两项门禁在增量修复前后基本不动。

长列表用 `each_virtual`：5 万行挂载加布局加出窗口 2.4 ms，一次建完的 `each` 是 175 ms，每行常驻约 2.3 KB。写法见 [列表](../essentials/list.md)。

帧上要和 UI 对齐，就走宿主这一次提交：

- 不为界面再 `request_device` 一次
- 不让实时内容用另一套 Queue 提交
- 不把 GPU 画面读回 CPU 再贴回去
- 不在 UI 画完之后往 Surface 上再盖一层
- 控件不拿窗口句柄去调系统 API

新代码从 `nana_ui::runtime` 进入。同一条字符串 slot 上的 `GpuTextureView` 仍在这棵树里参与布局、裁剪和命中。

无头对照把 `--shape window` 换成 `--shape headless`。滚动版本再加 `--scroll`。行数要和生成 bundle 时的行数一致。

这些数字来自一台机器上的 hover，不是 CI 门禁。Gallery 的 559 张像素门禁守的是这条纯 Rust 路径。时钟曾经被冻在 0，那时 2,000 行的 L3 记成 0.0242 ms，比后来的 0.0819 ms 乐观大约 3.4 倍。现在每个事件按 16 ms 推进，表里的 0.0003 ms 是索引修好之后的数。

复现时先按模式生成 bundle，再跑：

```bash
cargo build --release -p nana-ui-devtools --features agent-bin --bin nana-hover-benchmark
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60 --shape window
```

分段计时、四种模式的构建脚本，以及后来改掉的结论，在 [怎么工作](../../reference/how-it-works.md) 与 [一个指针事件的成本](../../reference/input-cost.md)。

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
