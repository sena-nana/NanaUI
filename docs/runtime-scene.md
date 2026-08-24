# Runtime 与 Scene

这篇给**改保留树或绘制抽取**的人。应用开发看 [框架如何运行](how-it-works.md)。入口类型看 [应用 API](application-api.md)。

产品路径：`UiWorld` → `ExtractedNode` → `UiScene` → `SceneWgpuPainter`。

`nana-ui-runtime` 与 `nana-ui-scene` 不依赖 WGPU。Scene crate 不持有应用状态、窗口或 Device。

## 权威

`UiWorld` 是 identity、document 归属、层级、节点类型、样式、文本、已提交选区、未滚动 layout、scroll offset、指针 hover/press/capture、事件路由、焦点 / IME、无障碍、text presenter 和 render content 的唯一权威。对外只用 `StableNodeId` / `Entity<V>`。内部存储换实现不能作废 JS handle、诊断或持久化数据。

结构变更先进入帧内 mutation 队列，整批验证成功后一次 commit。失败不发布局部层级结果；销毁后的 ID 不再复用。

Vue 节点 handle 与 `StableNodeId` 对应。DOM facade 只留兼容元数据，不能成为第二权威源。

## 增量

Runtime 按脏组件产生确定性工作：样式、文字、布局、命中、焦点 / IME、无障碍、抽取。静止 world 返回空工作，不要求持续 redraw。

`RuntimeDocument::flush` 在一次帧事务里调用宿主 `TextShaper`，由 `RuntimeLayoutEngine` 按 viewport、样式和 shaping 写回 layout。viewport 变化在无应用 mutation 时也会触发布局。系统失败时已消费工作回到调度器，Scene 与无障碍增量在 settle 前不发布。

局部 mutation 只传播到语义受影响的节点；遇到已有相同脏状态即停。动画以 Runtime 持有的稳定 ID 注册，宿主传入单调时间；Runtime 不建计时线程。动画、实时 GPU 和普通 UI 唤醒分开。

`WorkspaceModel` / `SplitPaneModel` / `DockWorkspace` 各自持有持久布局，只接受显式 `Duration`。host adapter 做 Instant → Duration 与指针转换，不另存一份产品状态。

## 抽取与绘制

flush 将变更抽成 `ExtractedNode` 增量，`UiScene::apply_delta` 更新绘制图。`CustomRenderNode` 是一等抽取字段：`GpuTextureView` / `GpuView` 与 Button 一样进入 document order。

`SceneWgpuPainter` 注入宿主 Device / Queue，在当前 dest pass 按节点顺序编码。HostTexture 不攒到帧尾，不为每个 GPU 槽单独开 pass。含 HostTexture / 自定义 GPU 节点的帧使用 `sample_count = 1`；没有 GPU 节点的帧可以用 4x MSAA 画方块和网格，文字在 resolve 之后画。不要在自定义节点两侧反复 resolve。外部 `SceneResourceProducer` 在采样前用同一 Queue 提交。冲突 revision 拒绝整帧。

无障碍增量带同一 generation 的更新节点与稳定 ID 删除。平台 adapter 不维护另一棵权威语义树。默认程序不声明无障碍动作；只有显式接通的 `RuntimeProgram::accessibility_action` 才暴露。

## 文本呈现

`HighlightRequest` / `TextPresentation` 是 intent；算法是按名注册的 `TextPresenter`。扩展经 `ExtensionRegistrar::register_presenter` 安装。Presenter 只读已提交 UTF-8；IME preedit 保持单色。未知语言或未注册 presenter 时 Scene 退回单色文本。
