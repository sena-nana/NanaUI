# Runtime 与 Scene

界面真正被记住和画出来的地方。产品路径：

`UiWorld` → `ExtractedNode` → `UiScene` → `SceneWgpuPainter`

应用入口见 [开始](start.md) 和 [应用 API](application-api.md)。这篇讲这棵树内部怎么更新、怎么抽成画面、怎么交给宿主去画。

## Dependency direction

```text
Nana public/application adapters
              |
              v
       nana-ui-runtime
              |
              v
        nana-ui-scene
              |
              v
     SceneWgpuPainter（宿主 Window / Surface / Device / Queue）
```

`nana-ui-runtime` 与 `nana-ui-scene` 不依赖 WGPU 或平台 GPU API。
`scripts/check-engine-boundary.py` 保持 Runtime / Scene 对绘制后端中立。

## Retained authority

`UiWorld` 是 identity、document ownership、hierarchy、node kind、canonical/computed
style/theme、text、committed text-input selection、text metrics、未滚动 layout、scroll offset、per-pointer hover/press/capture、event route、focus/IME preedit、accessibility、registered text presentation 和 render content 的唯一权威来源。对外只使用 `StableNodeId` / typed
`Entity<V>`；Bevy Entity 编码不是 ABI。

文本的特殊表现（高亮是第一个）分成两层：节点上的 `HighlightRequest` /
`TextPresentation` 是 intent 与派生结果；算法是 `UiWorld` 上按名字注册的
`TextPresenter`。扩展通过 `ExtensionRegistrar::register_presenter` 安装，因此 Vue
`flush` 不必经过 `AppContext`。Presenter 只读 committed UTF-8；IME preedit 保持单色。
Span 颜色是 `SemanticColorRole`，抽取时按当前 theme 解析。可选
`syntax-highlighting` feature 提供名为 `"highlight"` 的 syntect presenter
（`HighlightPresentation` / `TextArea::highlight`）。未知语言或未注册 presenter
时 Scene 退回单色文本。`SceneWgpuPainter` 用 paragraph spans 绘制；word wrap 由
文本 shaping 处理，不在 Scene 为每个 span 另占 primitive slot。

所有结构或 component 变更先进入 frame-local `MutationQueue`，整批验证成功后一次
commit。失败批次不发布局部 hierarchy/component 结果；despawn 后 ID 永久 tombstone，
禁止 ABA 复用。

Vue `NodeHandle` 与 `StableNodeId` 无损映射。DOM facade 只保留 namespace、attributes、
scope 等 compatibility metadata；`event_flags` 权威是 `UiWorld` `EventListeners`。
`MessageBridge` 的 hierarchy 在 renderer 消费前由 Runtime 覆盖，不能成为第二权威源。

## Incremental systems and wakeup

Runtime 以 dirty component 产生确定性的 `SystemWork`，区分 style、text shaping、
layout、input/hit-test、focus/IME、accessibility、render extraction 和 render
removal。静止 world 返回空 work，不运行无关 system，也不要求持续 redraw。canonical
`RuntimeDocument::flush` 在一次 frame transaction 内调用 host text shaper，并由
backend-neutral `RuntimeLayoutEngine` 根据 viewport、style 与 shaping metrics 写回 layout；
应用不手写控件坐标。低层 `flush_with` 只保留给需要替换系统执行器的 backend。
viewport 变化会在无应用 mutation 时主动触发布局；文本先进行 intrinsic shaping，再按
resolved content box 执行 wrap/ellipsis shaping，auto-height 不会被首轮单行高度错误钳制。
frame system 失败时，所有已消费 work 会恢复到 scheduler，Scene 与 Accessibility delta
在 frame settle 前不发布，重试不会丢失 dirty/removal 事务。
Accessibility 增量事务包含同 generation 的 updated nodes 与 stable-ID removals；subtree
删除同时更新存活父节点的 children，平台 adapter 不维护另一棵权威语义树。
Desktop hosted window 在首次 show 前创建 AccessKit adapter，并在应用处理前转发每个
window event。平台投影缓存仅用于生成增量 update；它不接受业务 mutation。默认程序不声明
动作，只有显式启用的 `RuntimeProgram::accessibility_action` 才能暴露已接通的操作；Vue 当前
把 enabled Focus/Click 按 stable ID 送回 retained focus 与既有 Bridge/DOM 事件链。

局部 mutation 只传播到语义受影响的 node/subtree/ancestor；传播遇到已有相同 dirty
状态即停止。动画以 Runtime-owned stable ID 注册，host 显式传入单调时间并消费最近
deadline；Runtime 不创建计时线程。due sample 本身不伪造 render dirty，consumer 仅对
实际属性结果提交 mutation。动画、实时 GPU 内容和普通 UI wakeup 是独立 cadence，实时
source 不得强制整个 Runtime 全量更新。
Scene host 通过 `RuntimeAnimationClock` 将 duration epoch 映射为
`Instant`；adapter 不创建 timer，也不替应用决定 sample 是否需要 redraw。
应用级 sampled state、外部 runtime pump 与 retry backoff 使用
`RuntimeProgram::next_wakeup/wake`；host 将该 deadline 与所有 document animation deadline
取最早值。这条路径不依赖 redraw，`wake` 只有在业务状态实际变化时才请求窗口更新。

Workspace 的持久布局、viewport/scale、resize drag 与 collapse transition 由
`nana-ui-core::WorkspaceModel` 单独持有，并只接受显式 `Duration` 时间。静态 geometry
直接借用布局；只有 active transition 才构造瞬时 extent snapshot。`WorkspaceController`
只做 Instant→Duration 与指针/`WorkspaceAction` 到 `WorkspaceMutation` 的转换。Gallery 与产品
消费 `WorkspaceModel` / `WorkspaceMutation`，不得再在 adapter 中保存第二份 workspace 状态或动画。

Sidebar disclosure 使用 backend-neutral `ExpansionState`，以显式 `Duration` 连续采样和反向；
section state 只换算 host clock 并决定是否订阅 frame。SplitPane 的持久约束、size、focus、
hover 与 absolute-delta resize 则统一由 `SplitPaneModel` / `SplitPaneMutation` 持有；现有
`SplitPaneController` 只负责把 host point/key event 转成该 mutation。
Dock insertion dwell 也使用 controller-owned monotonic `Duration` epoch；pending target 不保存
host `Instant`，frame subscription 发出明确的 `AdvanceDragDwell`，不能再借 hover 消息伪装
时间推进。`DockUpdate.changed` 只表示需持久化的 dock 树（`DockWorkspace` 投影）变化；dwell/focus/preview 与
measured surface geometry 是瞬时状态，由产生该 mutation 的 input/frame host 请求重绘，不能
借 `changed` 触发配置写入。
产品 Dock 权威是 Runtime `DockWorkspace`。Split 比例只有一套公式：
`dock_split_ratio_from_pointer` / `dock_nudge_split_ratio` /
`dock_split_child_lengths`（`nana-ui-runtime`）。`nana_ui::dock::DockController`
是 host adapter：指针/dwell/frame → `DockMutation`，几何只经 `surface_layout`，
内部 resize 调用上述 Runtime 公式，不再自算 ratio-per-pixel。持久化 JSON 是
`DockWorkspace` 的投影（字段名沿用既有 `DockLayout` JSON）。
`DockController::layout_json` / restore 经该产品树转换，不另存一套 split
算法。`DockLayout` 仍是 adapter 的 live tree 加上 monitor clamp / item-spec，
不是第二条 live dock。`DockAction` 不是产品路径。Workspace / Split / Dock
不保留第二条 resize 规则。
`DockController::surface_layout` 是 adapter 的确定性几何出口：同一份
`DockLayout` 经 Runtime 子长度公式投影 active item content bounds、tab group 与带
stable path 的 splitter hit bounds，并区分主窗口 28px dock chrome 与 floating
window 36px native title bar。Runtime 指针 resize 写回 `DockWorkspace`；HostTexture
consumer 不得再实现 split ratio、divider 或 chrome offset 算法。

## Application API

应用入口手册见 [`application-api.md`](application-api.md)。

`Entity<V>`、`View`、`AppContext`、`ViewContext` 与 `AppContext::mount` 提供 typed
state/read/update/remove、keyed 子树组装、closure event、typed action、registered text
presenters 和 staged extension install，不暴露 ECS World。一次 context
update 汇集为一个 mutation commit。插件通过 `register_component` 与
`register_activation` 加入指针激活，不必改 `activate_node`。Vue bind 的常见语义走
`SemanticSpec`；其余属性走 `SemanticSpec::attr`。`Task`/`Subscription` 只包装标准
Future/Stream；executor、waker 和取消生命周期由 host adapter 拥有。
`RuntimeProgram::Message` 留给跨窗口/GPU/持久化，不是每个 Button press 的总线。

Pointer/wheel/keyboard 的稳定事件、modifier、pointer phase/type 与 disposition 位于
`nana-ui-platform`；winit 只负责 adapter conversion。平台输入不得通过 renderer
类型进入 Runtime 或 Vue semantic event path。`nana-ui::RuntimeInputAdapter` 将稳定 wheel
delta 路由到命中层级最近且仍可滚动的 ScrollView，并从当前 focus 派生 Table navigation；
只有实际状态变化才返回 `prevent_default`，调用方据此决定是否继续交给 Scene host。
Platform 的 `fetch` / `clipboard` 是默认开启但可独立关闭的 capability features，基础
window/input/IME contract 不应因 TLS 或系统 clipboard toolchain 无法跨目标编译。

`ComponentView` 在 closure event 全部交付后把最终 state 增量投影到 UiWorld。内建
`Text`、`Button`、`TextInput`/`TextArea`、`Checkbox`、`Switch`、`Slider`、`TabList`/`Tab`、`ScrollView`、`List`、`Table`/`Row`/`Cell`、`OverlayHost`、`Dialog`、`Menu`/`MenuItem`、`Tooltip`、`SearchDropdown`、`CommandPalette` 与 typed events 是 Runtime 合同。TextInput/TextArea/SearchDropdown/CommandPalette 共用 committed UTF-8 selection/IME state，SearchDropdown 仅在打开时持有编辑状态，CommandPalette 始终可编辑；accessibility 显式区分 multiline；ScrollView 只拥有配置，offset 与 measured `ScrollMetrics` 只存在 Runtime；
字段未变化时不提交 mutation。控件目录见 [控件](components.md)。

内建与插件控件共用 `AppContext` 上的 `ComponentRegistry`。`AppContext::new` 安装
`NanaBuiltinComponents`（`nana.builtin`）；插件通过同一套
`ExtensionRegistrar::register_component` 写入。稳定身份是 `ComponentTypeId`
（`nana.button` / `app.bilibili-user-card`），Vue tag（含 `nana-` 前缀）与 L3
`create_component<C>` 都解析到该表。`bind` 只把通用 UI 投影进 `UiWorld`；业务
state 留在 `AppContext.views`。未注册自定义 tag 仍按 HTML downlevel 落到 Column。
这与 Vue `NativeComponentRegistry`（JS host 组件工厂表，不是 Runtime
`ComponentTypeId` ABI）不是同一条路径。要加控件时：

| 目标 | 注册到 |
|------|--------|
| 进入布局 / 命中 / Scene 的新语义控件 | `ExtensionRegistrar::register_component`（Runtime `ComponentRegistry`）+ Vue `nana-*` tag |
| 仅 JS props / 事件 / 命令白名单 | `NativeComponentRegistry` + `Nana.components.call` |

禁止只扩展 `WidgetKind`，或只注册其中一张表却期望另一条路径生效。
动态 dylib 与公开 Bevy Entity 仍不在 ABI 内。
OverlayHost typed view 只拥有样式；exclusive active 与 focus restore 只存在 UiWorld。切换
active 时非活跃直属 subtree 从 layout/input/render/accessibility 排除，modal overlay 限制
焦点范围，旧 subtree 的 pointer capture 自动释放；非法 reparent 原子拒绝，active overlay
销毁自动清理引用并恢复仍有效的原焦点。
组件文本的 content-box padding、line-height、wrap/ellipsis 和水平/垂直 anchor 是
Runtime/Scene contract；backend 不得从 element tag 推测这些语义。

Variable-height virtual list/table geometry 位于 `nana-ui-core`。Fenwick 索引
使二维 visible window、range extent 与单项 measurement/column update 保持 O(log n)；具体 item
构建、滚动输入和 painter 由 component/backend 消费该窗口，不能在 core 复制 retained tree。scroll mutation 不重排 layout，只让后代 input/render 失效；scrollport clip 保持 viewport 坐标，后代 transform 叠加负 offset。
keyed list/table materialization 共用 revision-fenced prepare/commit plan；`AppContext` 以单次
Runtime commit 完成 visible mount、unmount 与最终顺序，成功后才发布 application-owned
key/entity 映射。重叠 row/column key 保留 Entity，非可见数据不进入 retained tree；物化前
必须验证 materializer key、typed view、entity mapping 与目标 List/Table 直属层级完全一致，
禁止跨容器误删实体。

## Render extraction and Scene

Runtime 只输出 `ExtractedNode` delta 与 tombstone removal。`UiScene` 保存稳定 primitive
cache，并表达 Quad、Text、Custom、content bounds/text placement、affine transform、clip chain、累计 opacity、
z-index 和 document order。普通局部更新不重建 hierarchy order 或无关 primitive；
hierarchy 改变时才重算 document order。

Vue compatibility 的 `ScrollOffsetStore` 只排队 Scene-host scroll command，不保存状态；程序化滚动和 viewport `on_scroll` feedback 都提交 Runtime offset/metrics。每个 VueHost 独立拥有 `LayoutBoxStore`，它只保存该窗口 JS 查询所需的 paint-phase geometry，不得跨窗口共享。`begin_frame` 不清 boxes/transforms；滚动只进 `views` overlay，不 `record()` 滚动几何、不写回 Runtime `LayoutBox`。Vue host op 进入 `PendingHostOps`，`flush_host_frame` 才 commit。`gpu_slots` 权威是 Runtime `CustomRenderNode`；`event_flags` 权威是 `UiWorld` `EventListeners`；`attrs` 仍是 DOM/CSS facade，不复制树拓扑。`NanaTreeDocument` / `MessageBridge` / `LayoutBoxStore` 三个 facade 仍在。

`StandardVisual` 将 checkbox/switch/slider 的 indicator、track、fill、thumb 作为有限 backend-neutral render content；它与标签文本分别解析前景，不由 backend 识别 tag。`CustomRenderNode` 是一等布局/Scene 公民：只有 renderer/resource/revision opaque key，不携带 backend object，与 Quad/Text 一样参与 clip、z-index 和 document order。
`RenderGraph` 将 external resource preparation、连续标准 primitive 与 custom node 编译为
独立 pass，显式注册 target/resource access，并通过 hazard dependency 保留生产、采样与 Scene
顺序。同一 resource 在一帧出现冲突 renderer/revision 时整图拒绝编译。Scene host
将 Draw 与 `nana.host-texture` 的 InvokeCustom 映射为同一 dest pass 内按序切换 pipeline；
无法加入该 pass 的 `SceneGpuRenderer` 才结束主 pass 再 `render()`。
`SceneResourceProducer` 则执行 `PrepareExternal`；每个 preparation pass 使用独立 host-owned
encoder，成功后立即由同一 Queue 有序提交，因此后续 producer 失败不会让先前 producer
滞留在未提交状态。业务 GPU 内容不得 CPU
readback、Base64/图片编码或额外子窗口后伪装成共享合成。

## Scene host

`RuntimeProgram` / `run_runtime` 是 Rust 应用的宿主合同：应用只提供
`RuntimeDocument`、UiScene、平台事件处理与可选 HostTexture / SceneGpu renderer registry。
`run_runtime` 直接进入 `run_runtime_scene`（Nana-owned winit + `SceneWgpuPainter`）。
该路径按 `winit::window::WindowId` 映射 `nana_ui_platform::WindowId`，主窗口留在同一个
`HostedGpuContext`，辅助窗口用 `create_surface` 共享 Device/Queue。`WindowCommand::Open` /
`Close` / `Focus` / `SetTitle` / `Move` / `SetBounds` / `SetFullscreen` / `SetMinimized` /
`SetMaximized` / `SetAlwaysOnTop` 作用在目标窗口；关闭主窗口退出，关闭辅助窗口拆除
surface/AccessKit 并发送 `WindowEvent::Closed`。Runtime 先消费 IME，再把同一
`WindowEvent::Ime` 交给 `RuntimeProgram::window_event`（程序不得再写入 Runtime）。
每窗独立 IME request 与 AccessKit adapter，adapter 在首次 show 前创建。

当前 `nana-ui` 通过 `SceneWgpuPainter` 绘制 Runtime / UiScene；Vue 兼容路径的
`scene-view` / `hosted` 接入同一 Scene host。Android 见 [`android.md`](android.md)。
无法忠实表达的 affine / text / custom primitive 显式失败。
