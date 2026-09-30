# 一帧

一帧做的是把已经在 `UiWorld` 里的状态画上你的 Surface。路径不换：应用状态进入 `nana-ui-runtime` 的 `UiWorld`，flush 得到 `nana-ui-scene` 的 `UiScene`，`nana-ui` 的 `SceneWgpuPainter` 画进宿主 Surface。

Window、Surface、`GpuContext`，以及这一帧的 `FrameContext`，都属于宿主。`run_runtime` 可以代你做这层宿主。`SceneWgpuPainter` 只往这个 `FrameContext` 里录。它不另开 Device，也不把画面读回 CPU。

`run_runtime` 内部，每扇需要重绘的窗口大致按这个顺序走。全文在 [框架如何运行](../reference/how-it-works.md)。

```text
1. 消化 dispatch_program 的消息 → RuntimeProgram::update
2. prepare_window_frame          → 你把最新纹理准备好
3. RuntimeDocument::flush        → 样式、文字、布局、命中、抽取
4. 获取 Surface；外部资源生产    → FramePlan，同一宿主 encoder，失败整帧丢弃
5. SceneWgpuPainter::paint_target → 可见操作按文档顺序画进目标
6. queue.submit + submitted + present → 每目标一份 NanaUI 提交
7. window_frame_presented        → 现在才能丢掉上一帧的纹理
8. bind_window                   → 需要的话再填内容
```

下面按消息、纹理、flush、present 说明这张表。遮挡和零尺寸是这张表的例外，不是插在中间的新步骤。

## 消息

控件点击不要放进 `update`。用 `on` / `observe`。需要开窗或换纹理时，在闭包里 `dispatch_program`。这条消息下一帧才进入 `update`。`update` 只处理宿主级消息，保持便宜。页面内容不要在这里填，放到 present 之后的 `bind_window`。

结构变更先进入这一帧的 mutation 队列。整批验证成功才一次 commit。失败不发布半棵树。销毁后的 ID 不再复用。

Vue 也不在调用的那一刻改树。host op 进待提交队列，`PendingHostOps` 在帧边界由 `flush_host_frame` 提交。DOM facade 不复制树拓扑。`event_flags` 的权威是 `UiWorld` 的 `EventListeners`。GPU 槽的权威是 Runtime 的 `CustomRenderNode`。

## 纹理

`prepare_window_frame` 在 flush 之前。你在这里把这一帧要采样的纹理准备好。换纹理升 generation，不要为了换内容去改 Runtime 节点。细节在 [实时画面](./gpu.md)。

窗口被遮挡、最小化，或尺寸为零（即使它仍可见）时，`FrameDemand` 到期仍会调用 `prepare_window_frame`。这一拍不 flush 界面、不获取 Surface、不 present，也不调用 `window_frame_presented`。尺寸还可以画时，外部资源生产者在一个不带 Surface 的 `FrameContext` 上编码，并立刻提交。这里的 `submitted()` 只表示这次编码已经入队，不是采样这张纹理的界面帧已经呈现。尺寸为 0 时仍然 prepare，但不跑这次编码。

不能 present 的时候也会走到 prepare。不要假定后面一定有 Surface。上一帧的纹理要等到真正的 `window_frame_presented` 之后才能释放。

## flush

`RuntimeDocument::flush` 在一次帧事务里做样式、文字、布局、命中和抽取。它调用宿主文字整形（`NanaTextShaper`），再由 `RuntimeLayoutEngine` 按 viewport、样式和 shaping 写回布局。你不要自己再跑一套布局，也不要把控件坐标写进树。

系统失败时，已经消费的工作回到调度器。Scene 和无障碍增量在 settle 之前不发布。

没有变更时，flush 是空转。宿主不该为了空转去刷一整帧。动画、实时画面和普通界面的唤醒是分开的。一块纹理在动，不该迫使整棵 `UiWorld` 全量更新。

viewport 变了，即使没有应用这边的 mutation，也会触发布局。脏的是 document root，加上 `position: fixed` 和 `vw` / `vh` 节点。未移动的子树复用保留的缓存。

滚动不写回 Runtime 的布局盒。JS 查询用的盒子是绘制阶段的投影。`LayoutBoxStore` 是这份投影，不是另一份布局权威。

`UiWorld` 保存的是逻辑状态。过渡中的透明度和变换留在 `PresentationStore` 上，按节点和属性去查。绘制、命中、焦点和无障碍在需要时，按同一个时间戳取呈现值。查询走 `presentation_pair` / `applied_value()`。对外身份是 `DocumentId`、`StableNodeId`，以及类型化的 `Entity<V>`。内部怎么存节点，不是 API。

同一条轨道、同一个时间戳，求值是确定的。spring 和 decay 按绝对时间解析，不靠上一帧的积分。`width` / `height` 走布局，采样写成像素，不改成 scale。Quad 上的合成轨道由着色器按这个时间求值。文字、图标、Mesh 和 HostTexture 仍用 CPU 上的呈现值。

::: warning
不要把合成采样写回基础样式。那样会把过渡中的瞬时值当成逻辑状态。呈现停在 overlay 上。
:::

`transform` / `opacity` 这类合成轨道不得每帧改 `UiWorld`。描述符在开始、改目标和取消时写入。稳态帧只推进时间。完成靠 deadline，不靠逐帧采样才知道结束。产品 present 不把画面读回 CPU。

flush 把变更抽成 `ExtractedNode`，`UiScene` 收下这份绘制图。`GpuTextureView` 和 `GpuView` 跟其他节点一样进入文档顺序。合成类的 overlay 不写进抽取节点的逻辑样式。只有时间在走，不得因此重新抽取。

## present

flush 之后，宿主获取 Surface。按图生产的外部资源录进宿主这一帧的 `FrameContext`，和界面绘制合在同一次提交里。编码或绘制失败时，整帧丢弃。生产者不能自己提交，也不能提前报告完成。

`SceneWgpuPainter::paint_target` 把可见操作按文档顺序画进目标。实时纹理在它该出现的位置采样，不攒到帧尾。

提交和丢弃只由 `FrameContext` 负责。每个目标一份 NanaUI 提交，然后 present。`submit` 结清这一帧。帧被丢掉时，painter 登记过的保留写入会回滚。不要在丢弃之后还假定内容已经上屏。前一帧画过某个目标、还没提交也没丢弃时再画它，会得到 `ScenePaintError::TargetInFlight`。

`window_frame_presented` 之后，才能丢掉上一帧仍可能被采样的纹理。需要再填页面内容时，放在随后的 `bind_window`。

静态窗口保持按需刷新。只有该窗口自己的合成轨道需要连续 present 时，才接到轻量的 `FrameDemand::Continuous`。这不强制 Vue 补丁，也不跑全局的样式和布局。采样仍不写回基础样式。

最小化或遮挡期间，合成器不去追隐藏的 GPU 帧。窗口回来之后按绝对时间求值。设备或 Surface 重建之后，宿主调用 `UiScene::set_surface_generation`。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/architecture/gpu">
    <p class="next-step-link">实时画面</p>
    <p class="next-step-caption">纹理怎样作为节点留在树上。</p>
  </a>
  <a class="next-step" href="/reference/how-it-works">
    <p class="next-step-link">框架如何运行</p>
    <p class="next-step-caption">状态放哪、谁拥有窗口和设备。</p>
  </a>
  <a class="next-step" href="/reference/runtime-scene">
    <p class="next-step-link">Runtime 与 Scene</p>
    <p class="next-step-caption">flush 怎样抽出绘制图。</p>
  </a>
</div>
