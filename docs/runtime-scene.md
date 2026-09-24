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

`RuntimeDocument::flush` 在一次帧事务里调用宿主 `TextShaper`，由 `RuntimeLayoutEngine` 按 viewport、样式和 shaping 写回 layout。viewport 变化在无应用 mutation 时也会触发布局：脏集合是 document roots 加上 `position: fixed` / `vw` / `vh` 节点，未移动的子树复用 retained cache，不丢整棵树。系统失败时已消费工作回到调度器，Scene 与无障碍增量在 settle 前不发布。

局部 mutation 只传播到语义受影响的节点；遇到已有相同脏状态即停。动画以 Runtime 持有的稳定 ID 注册，宿主传入单调时间；Runtime 不建计时线程。`AnimationSpec` 是 Motion IR 的 timing 子集，进度经 `evaluate_progress` 求值，不另开一套 timeline。Rust L3 `ViewContext::transition` / `node().motion(Spring::to)` / `node().timeline(Timeline::parallel|sequence)` 编译为同一 `AnimationSpec`。内建 hover / switch / spinner / surface / skeleton / sidebar / workspace / loading 不再维护平行逐帧时钟：compositor-safe 属性走 overlay，switch thumb 与 sidebar 高度保持 Layout，完成事件走 deadline。`advance_animations` 仍按稀疏 deadline 采样：CPU/Layout/Paint 类 track 用 `frame_interval`；compositor-safe presentation track 只用 start / completion deadline（`animations_considered` / `animation_deadlines_scanned` 语义不变）。查询走 `UiWorld::presentation_pair` / `applied_value()`，禁止每帧把 transform/opacity 写进 `UiWorld`。Compositor overlay 只给 `AnimationClass::Compositor`；对应 `MotionDescriptor` 用 generational handle 绑定，稳态求值不重建 slab。动画、实时 GPU 和普通 UI 唤醒分开：`compositor_needs_tick` 接到该窗口轻量 present，不进入 CPU animation deadline。多窗口只因自身 active motion 被 redraw；device/surface 重建后 `set_surface_generation`。

`WorkspaceModel` / `SplitPaneModel` / `DockWorkspace` 各自持有持久布局，只接受显式 `Duration`。host adapter 做 Instant → Duration 与指针转换，不另存一份产品状态。

### 动画属性分类

逻辑 / base 在 `UiWorld`，呈现值在 `PresentationStore`。完成事件走 deadline，不靠逐帧 CPU sample。默认 Class 是 `AnimatableProperty::animation_class()`，组件不能改。没有 filter GPU、也没有 Mesh GPU evaluate。

| 属性 | 默认 Class | 执行路径 | fallback / 性能含义 |
| --- | --- | --- | --- |
| `transform` / `opacity` | Compositor | overlay；Quad（含 QuadBatch / QuadColorBatch）剥 overlay 走 GPU `evaluate()`；Text / Icon / Mesh / HostTexture 走 CPU overlay | 稳态不写 `UiWorld`；非 Quad 仍是 CPU presentation |
| `clip` / `clip-path` | Compositor | overlay；`compositor_gpu_motion_ids` 目前只绑 transform/opacity | 稳态不写 `UiWorld`；clip 呈现仍 CPU overlay |
| `shader-parameter` | Compositor | overlay；需注册 typed codec | 无默认 Quad GPU 路径 |
| `color` / `background` / `blur` / `filter` / `shadow` | Paint | CPU 插值（`frame_interval` 稀疏采样） | 可能每 sample 脏 paint/extract；不是 filter GPU |
| `width` / `height` / `padding` / `margin` | Layout | CPU layout，采样写 px 并脏 LAYOUT | 每 sample layout；不要偷成 scale |
| `font-size` / `font-axis` | Layout | 非 compositor（排版 / 绘制也会受影响） | [#85](https://github.com/sena-nana/NanaUI/issues/85) 不强制 GPU |
| `display` | Discrete | `snap_discrete`：结束前保持 from，结束时 rest | 不插值 |

`#8` 的 `animations_considered` / `animation_deadlines_scanned` 语义不变。compositor-only 稳态结构门禁见 [`perf/README.md`](../perf/README.md) 的 `compositor-steady`。开发诊断：`AnimatableProperty::diagnostic_hint()` 与 `UiWorld::inspect_motion()`；hint 文案以代码为准，文档不硬编码整句。

## 抽取与绘制

flush 将变更抽成 `ExtractedNode` 增量，`UiScene::apply_delta` 更新绘制图。`CustomRenderNode` 是一等抽取字段：`GpuTextureView`（默认）与 `GpuView` 都和 Button 一样进入 document order。

Compositor-class overlay（`transform` / `opacity` / `clip` / `shader-parameter`）不写进 `ExtractedNode` 的逻辑样式。Scene 用 `UiScene::apply_presentation` 从 live `PresentationStore` / `MotionDescriptorStore` 引用绑定 `CompositorLayer`（flush 不得每帧 clone slab）。层 identity 是节点 `StableNodeId`；子树 primitive 只在 topology/primitive 变化时 invalidate（`cache_generation`）；仅时间推进不得 re-extract。Promotion 阈值：`LAYER_PROMOTE_HOLD` 16ms，`LAYER_DEMOTE_HOLD` 120ms（overlay 结束后 2ms～119ms 仍保持层，满 120ms 才 demote）。`clear_compositor_layer_request` 经 extract 把 `request_layer=false` 写回 scene，摘掉 `requested`，再走 demote hold。`OpacityGroup` / `FilterGroup` 仍是 dest 隔离组，不是 motion layer。Layer 上的 `CompositorMotionBinding { track_id, index, generation }` 对齐 D 的 `MotionHandle`（`generation == 0` 表示无 live descriptor）。Painter 按 `presentation_epoch` 失效 dest 缓存；有 live GPU descriptor 时跳过 dest blit 复用，稳态帧只写 shared time uniform，不重传整表。**只有** Quad 类 primitive 剥 overlay 并用 `motion.wgsl` `evaluate()`；Text / Icon / Mesh / HostTexture 仍用 CPU presentation，避免剥掉 overlay 却无法 GPU 还原。width/height 等 Layout 类 overlay 不 promote。

Layout-class（`width` / `height` / `padding` / `margin`）走 CPU：每帧采样写 px 并脏 LAYOUT，子树量测与命中用真实几何。FLIP（`FlipRect` / `layout_flip_*` / Vue `setPaintTransform`）在 Last 布局已提交后只播 compositor translate；逻辑 `LayoutBox` 停在 Last，视觉从 Invert 回到 identity。命中跟随 presentation transform。L3 为 `node().flip(first, last)`；可选 `animate_size()` 另开 Layout-class 宽高，不是 scale。Vue TransitionGroup move 与这条 FLIP track 共用同一 Motion IR。

画笔把图元和它每个裁剪的**最终纯平移**吸附到设备像素（`clip::paint_transform`）；旋转、缩放、透视不动。滚动偏移、CSS translate 和冻结行列的反向平移合成后才吸附，所以同一滚动下的 quad、图标、文字、路径网格、HostTexture、CustomRender 与裁剪边挪同样多的整像素（自带 GPU transform 动画的 quad 的叠加量除外）。`ScrollOffset`、hit test 与无障碍边界保留小数，与画面相差不到半个设备像素。触控板的小数横向滚动因此不再让文字重新解析（#223，见[文本引擎](text-engine.md)）。多行编辑器的滚动不是滚动容器的平移，沿 x 的那部分由 `ComponentGeometry::TextInput::scroll` 交给 scene，值、光标、选区与其上的标记画成平移，同样吸附。

`SceneWgpuPainter` 注入宿主 Device / Queue，在当前 dest pass 按节点顺序编码。HostTexture 不攒到帧尾，不为每个 GPU 槽单独开 pass。含 HostTexture / 自定义 GPU 节点的帧使用 `sample_count = 1`；没有 GPU 节点的帧可以用 4x MSAA 画方块和网格，文字在 resolve 之后画。不要在自定义节点两侧反复 resolve。高级的 `SceneResourceProducer` 在采样前用同一 Queue 提交。冲突 revision 拒绝整帧。

无障碍增量带同一 generation 的更新节点与稳定 ID 删除。平台 adapter 不维护另一棵权威语义树。默认程序不声明无障碍动作；只有显式接通的 `RuntimeProgram::accessibility_action` 才暴露。

### CSS clip-path

祖先 `clip-path` 进入 `UiScene::ClipRegion` 链并投影为 `FragmentClip`：

- **GPU scissor** 仍只用轴对齐包围盒；精确 clip 走顶点 `FragmentClip`（Quad / Mesh / Text / HostTexture）或 **dest opacity-group 合成**（外层旋转 clip、**`clip-path: polygon(...)`**、以及需要与 MSAA 交错的多层 rounded overflow）。
- **`clip-path: inset(... round R)`**：圆角半径写入 `FragmentClip.corner_radius`；非平移 transform 下仍保留 SDF 圆角（不再在旋转时清零 radius）。
- **`clip-path: polygon(...)`**：Scene 存 AABB + 局部顶点；**自身 quad** 在 fragment 做点内多边形测试；**子项 / 文本 / HostTexture** 通过 dest-group 在合成 pass 做 winding 多边形测试（非 AABB-only）。
- **HostTexture**：祖先 inset-round overflow clip 的 `corner_radius` 经 `clip_inv_ef.z` 传入 host-texture shader，与 quad 共用 rounded-box SDF。
- **圆角 `overflow`**：两轴都裁剪的祖先按自身 `border-radius`（四角取最小）生成带 `corner_radius` 的 `ClipRegion`，后代的 Quad / Text / Mesh / HostTexture 都走同一条 fragment clip；节点自己的表面（slot 0）只挂直角版本，避免对已经是圆角的填充和边框再做一次抗锯齿。只有一层圆角时留在顶点属性里，多层嵌套才进 dest 合成组。

### CSS filter drop-shadow

`filter: drop-shadow(offset-x offset-y blur color)` 走 dest 合成组：子树先画进 group 层，合成时采样该层 **alpha 轮廓**（UV 按 offset 平移），再用与元素 `filter: blur()` 相同的 5×5 核模糊并着色，然后 source-over 到原图之下。不是 `box-shadow` 的 rounded-box SDF quad，也不新开 backdrop ping-pong pipeline。blur 半径 cap 16px。多层 `drop-shadow`、spread、`inset` 仍 fail closed。未知 `filter` 函数仍整表 fail closed。

### CSS backdrop-filter

`backdrop-filter: blur(Npx)` 是**逐节点**效果：在绘制该节点填充之前，从当前 dest 纹理（`sample_count = 1` 的 `dest.color` 或 opacity-group 层）采样其 bounds（按 blur 核扩展）背后的**已绘制**内容（document order 中排在该节点之前的 Quad / HostTexture / 自定义 GPU 槽等），经 separable Gaussian 模糊后再合成回节点区域（圆角 / clip-path / mask-image 仍生效），最后才画半透明 fill / gradient。这与 Windows `nana-window` 整窗 Mica/Acrylic 或 Appearance `backdrop_*` **无关**。含 HostTexture / 自定义 GPU 节点 / dest 组 / backdrop-filter 的帧走 interleaved dest（`sample_count = 1`）；仅含 CSS `url()` 的 quad 仍走 4× MSAA。**任意 affine transform**（含旋转）的节点：copy/blur 区域用变换后 AABB，composite 顶点在逻辑 quad UV 上应用 `quad_abcd`/`quad_ef` 映射到 dest 像素再采样模糊纹理。opacity group 内 backdrop 从该 group 层采样，而非 dest 根纹理。composite pass 的 ancestor `FragmentClip`（含 inset-round 与 polygon）与 quad 共用同一套 `inside_fragment_clip`。

## 文本呈现

`HighlightRequest` / `TextPresentation` 是 intent；算法是按名注册的 `TextPresenter`。扩展经 `ExtensionRegistrar::register_presenter` 安装。Presenter 只读已提交 UTF-8；IME preedit 保持单色。未知语言或未注册 presenter 时 Scene 退回单色文本。
