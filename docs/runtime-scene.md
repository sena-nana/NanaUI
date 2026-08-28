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

`RuntimeDocument::flush` 在一次帧事务里调用宿主 `TextShaper`，由 `RuntimeLayoutEngine` 按 viewport、样式和 shaping 写回 layout。viewport 变化在无应用 mutation 时也会触发布局，且只跑一次全量 `layout_document`。系统失败时已消费工作回到调度器，Scene 与无障碍增量在 settle 前不发布。

局部 mutation 只传播到语义受影响的节点；遇到已有相同脏状态即停。动画以 Runtime 持有的稳定 ID 注册，宿主传入单调时间；Runtime 不建计时线程。动画、实时 GPU 和普通 UI 唤醒分开。

`WorkspaceModel` / `SplitPaneModel` / `DockWorkspace` 各自持有持久布局，只接受显式 `Duration`。host adapter 做 Instant → Duration 与指针转换，不另存一份产品状态。

## 抽取与绘制

flush 将变更抽成 `ExtractedNode` 增量，`UiScene::apply_delta` 更新绘制图。`CustomRenderNode` 是一等抽取字段：`GpuTextureView`（默认）与 `GpuView` 都和 Button 一样进入 document order。

`SceneWgpuPainter` 注入宿主 Device / Queue，在当前 dest pass 按节点顺序编码。HostTexture 不攒到帧尾，不为每个 GPU 槽单独开 pass。含 HostTexture / 自定义 GPU 节点的帧使用 `sample_count = 1`；没有 GPU 节点的帧可以用 4x MSAA 画方块和网格，文字在 resolve 之后画。不要在自定义节点两侧反复 resolve。高级的 `SceneResourceProducer` 在采样前用同一 Queue 提交。冲突 revision 拒绝整帧。

无障碍增量带同一 generation 的更新节点与稳定 ID 删除。平台 adapter 不维护另一棵权威语义树。默认程序不声明无障碍动作；只有显式接通的 `RuntimeProgram::accessibility_action` 才暴露。

### CSS clip-path

祖先 `clip-path` 进入 `UiScene::ClipRegion` 链并投影为 `FragmentClip`：

- **GPU scissor** 仍只用轴对齐包围盒；精确 clip 走顶点 `FragmentClip`（Quad / Mesh / Text / HostTexture）或 **dest opacity-group 合成**（外层旋转 clip、**`clip-path: polygon(...)`**、以及需要与 MSAA 交错的多层 rounded overflow）。
- **`clip-path: inset(... round R)`**：圆角半径写入 `FragmentClip.corner_radius`；非平移 transform 下仍保留 SDF 圆角（不再在旋转时清零 radius）。
- **`clip-path: polygon(...)`**：Scene 存 AABB + 局部顶点；**自身 quad** 在 fragment 做点内多边形测试；**子项 / 文本 / HostTexture** 通过 dest-group 在合成 pass 做 winding 多边形测试（非 AABB-only）。
- **HostTexture**：祖先 inset-round overflow clip 的 `corner_radius` 经 `clip_inv_ef.z` 传入 host-texture shader，与 quad 共用 rounded-box SDF。

### CSS filter drop-shadow

`filter: drop-shadow(offset-x offset-y blur color)` 走 dest 合成组：子树先画进 group 层，合成时采样该层 **alpha 轮廓**（UV 按 offset 平移），再用与元素 `filter: blur()` 相同的 5×5 核模糊并着色，然后 source-over 到原图之下。不是 `box-shadow` 的 rounded-box SDF quad，也不新开 backdrop ping-pong pipeline。blur 半径 cap 16px。多层 `drop-shadow`、spread、`inset` 仍 fail closed。未知 `filter` 函数仍整表 fail closed。

### CSS backdrop-filter

`backdrop-filter: blur(Npx)` 是**逐节点**效果：在绘制该节点填充之前，从当前 dest 纹理（`sample_count = 1` 的 `dest.color` 或 opacity-group 层）采样其 bounds（按 blur 核扩展）背后的**已绘制**内容（document order 中排在该节点之前的 Quad / HostTexture / 自定义 GPU 槽等），经 separable Gaussian 模糊后再合成回节点区域（圆角 / clip-path / mask-image 仍生效），最后才画半透明 fill / gradient。这与 Windows `nana-window` 整窗 Mica/Acrylic 或 Appearance `backdrop_*` **无关**。含 HostTexture / 自定义 GPU 节点 / dest 组 / backdrop-filter 的帧走 interleaved dest（`sample_count = 1`）；仅含 CSS `url()` 的 quad 仍走 4× MSAA。**任意 affine transform**（含旋转）的节点：copy/blur 区域用变换后 AABB，composite 顶点在逻辑 quad UV 上应用 `quad_abcd`/`quad_ef` 映射到 dest 像素再采样模糊纹理。opacity group 内 backdrop 从该 group 层采样，而非 dest 根纹理。composite pass 的 ancestor `FragmentClip`（含 inset-round 与 polygon）与 quad 共用同一套 `inside_fragment_clip`。

## 文本呈现

`HighlightRequest` / `TextPresentation` 是 intent；算法是按名注册的 `TextPresenter`。扩展经 `ExtensionRegistrar::register_presenter` 安装。Presenter 只读已提交 UTF-8；IME preedit 保持单色。未知语言或未注册 presenter 时 Scene 退回单色文本。
