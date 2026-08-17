# Nana Runtime / UiScene contract

本文定义 NanaUI 独立于 compatibility engine 的稳定架构合同。阶段证据见
`issue-7-phase-2.md` 至 `issue-7-phase-9.md`；本文只记录长期不变量。

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
 compatibility/native backend
```

`nana-ui-runtime` 与 `nana-ui-scene` 不依赖 Iced、WGPU 或平台 GPU API。Iced-derived
code 不得反向依赖 Nana packages。`scripts/check-engine-boundary.py` 持续检查这两个
方向。

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
时 Scene 退回单色文本。Iced compatibility 用 paragraph spans 绘制；word wrap 由
Iced paragraph 处理，不在 Scene 为每个 span 另占 primitive slot。

所有结构或 component 变更先进入 frame-local `MutationQueue`，整批验证成功后一次
commit。失败批次不发布局部 hierarchy/component 结果；despawn 后 ID 永久 tombstone，
禁止 ABA 复用。

Vue `NodeHandle` 与 `StableNodeId` 无损映射。DOM facade 只保留 namespace、attributes、
scope/event flags 等 compatibility metadata；`MessageBridge` 的 hierarchy 在 renderer
消费前由 Runtime 覆盖，不能成为第二权威源。

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
动作，只有显式启用的 `HostedProgram::accessibility_action` 才能暴露已接通的操作；Vue 当前
把 enabled Focus/Click 按 stable ID 送回 retained focus 与既有 Bridge/DOM 事件链。

局部 mutation 只传播到语义受影响的 node/subtree/ancestor；传播遇到已有相同 dirty
状态即停止。动画以 Runtime-owned stable ID 注册，host 显式传入单调时间并消费最近
deadline；Runtime 不创建计时线程。due sample 本身不伪造 render dirty，consumer 仅对
实际属性结果提交 mutation。动画、实时 GPU 内容和普通 UI wakeup 是独立 cadence，实时
source 不得强制整个 Runtime 全量更新。
Iced hosted compatibility 通过 `RuntimeAnimationClock` 将 duration epoch 映射为
`Instant`；adapter 不创建 timer，也不替应用决定 sample 是否需要 redraw。
应用级 sampled state、外部 runtime pump 与 retry backoff 使用
`RuntimeProgram::next_wakeup/wake`；host 将该 deadline 与所有 document animation deadline
取最早值。这条路径不依赖 redraw，`wake` 只有在业务状态实际变化时才请求窗口更新。

Workspace 的持久布局、viewport/scale、resize drag 与 collapse transition 由
`nana-ui-core::WorkspaceModel` 单独持有，并只接受显式 `Duration` 时间。静态 geometry
直接借用布局；只有 active transition 才构造瞬时 extent snapshot。`WorkspaceController`
保留 Iced event/frame subscription 和 view conversion 作为 compatibility adapter，稳定状态与
mutation API 是 backend-neutral `WorkspaceModel` / `WorkspaceMutation`，不得再在 adapter 中保存
第二份 workspace 状态或动画。

Sidebar disclosure 使用 backend-neutral `ExpansionState`，以显式 `Duration` 连续采样和反向；
Iced section state 只换算 host clock 并决定是否订阅 frame。SplitPane 的持久约束、size、focus、
hover 与 absolute-delta resize 则统一由 `SplitPaneModel` / `SplitPaneMutation` 持有；现有
`SplitPaneController` 只负责把 Iced point/key event 转成该 mutation。
Dock insertion dwell 也使用 controller-owned monotonic `Duration` epoch；pending target 不保存
Iced `Instant`，frame subscription 发出明确的 `AdvanceDragDwell`，不能再借 hover 消息伪装
时间推进。`DockUpdate.changed` 只表示需持久化的 `DockLayout` 变化；dwell/focus/preview 与
measured surface geometry 是瞬时状态，由产生该 mutation 的 input/frame host 请求重绘，不能
借 `changed` 触发配置写入。
Dock 的稳定状态入口是 `DockMutation` + `LogicalPoint` 与 `update_mutation[_at]`；active
drag/resize 内部也只保存 logical point、scalar delta 和 `Duration`。旧 `DockAction` 的
Iced `Point`、widget/subscription/view 仅是 compatibility adapter。三套 Workspace/Split/Dock
曾共用但现已无消费者的 Iced `ResizeDrag` 已删除，避免保留第二条 resize 规则。
`DockController::surface_layout` 是 retained consumer 的确定性几何出口：同一份
`DockLayout` 产生 active item content bounds、tab group 与带 stable path 的 splitter hit
bounds，并区分主窗口 28px dock chrome 与 floating window 36px native title bar。Runtime
或 HostTexture consumer 不得再次实现 split ratio、divider 或 chrome offset 算法。

## Application API

`Entity<V>`、`View`、`AppContext` 与 `ViewContext` 提供 typed state/read/update/remove、
closure event、typed action、registered text presenters 和 staged extension install，不暴露 ECS World。一次 context
update 汇集为一个 mutation commit。`Task`/`Subscription` 只包装标准 Future/Stream；
executor、waker 和取消生命周期由 host adapter 拥有。

Pointer/wheel/keyboard 的稳定事件、modifier、pointer phase/type 与 disposition 位于
`nana-ui-platform`；winit/Iced 只负责 adapter conversion。平台输入不得通过 renderer
类型进入 Runtime 或 Vue semantic event path。`nana-ui::RuntimeInputAdapter` 将稳定 wheel
delta 路由到命中层级最近且仍可滚动的 ScrollView，并从当前 focus 派生 Table navigation；
只有实际状态变化才返回 `prevent_default`，调用方据此决定是否继续交给 compatibility backend。
Platform 的 `fetch` / `clipboard` 是默认开启但可独立关闭的 capability features，基础
window/input/IME contract 不应因 TLS 或系统 clipboard toolchain 无法跨目标编译。

`ComponentView` 在 closure event 全部交付后把最终 state 增量投影到 UiWorld。内建
`Text`、`Button`、`TextInput`/`TextArea`、`Checkbox`、`Switch`、`Slider`、`TabList`/`Tab`、`ScrollView`、`List`、`Table`/`Row`/`Cell`、`OverlayHost`、`Dialog`、`Menu`/`MenuItem`、`Tooltip`、`SearchDropdown`、`CommandPalette` 与 typed events 不暴露 Iced；TextInput/TextArea/SearchDropdown/CommandPalette 共用 committed UTF-8 selection/IME state，SearchDropdown 仅在打开时持有编辑状态，CommandPalette 始终可编辑；accessibility 显式区分 multiline；ScrollView 只拥有配置，offset 与 measured `ScrollMetrics` 只存在 Runtime；
字段未变化时不提交 mutation。它们是后续 compatibility component migration 的稳定
入口，不代表现有完整组件 painter 已经迁移。
OverlayHost typed view 只拥有样式；exclusive active 与 focus restore 只存在 UiWorld。切换
active 时非活跃直属 subtree 从 layout/input/render/accessibility 排除，modal overlay 限制
焦点范围，旧 subtree 的 pointer capture 自动释放；非法 reparent 原子拒绝，active overlay
销毁自动清理引用并恢复仍有效的原焦点。
组件文本的 content-box padding、line-height、wrap/ellipsis 和水平/垂直 anchor 是
Runtime/Scene contract；backend 不得从 element tag 推测这些语义。

Variable-height virtual list/table geometry 位于 `nana-ui-core`，不依赖 Iced。Fenwick 索引
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

Vue compatibility 的 `ScrollOffsetStore` 只排队 Iced command，不保存状态；程序化滚动和 Iced `on_scroll` feedback 都提交 Runtime offset/metrics。每个 VueHost 独立拥有 `LayoutBoxStore`，它只保存该窗口 JS 查询所需的 paint-phase geometry，不得跨窗口共享或把滚动后的坐标写回 Runtime layout。

`StandardVisual` 将 checkbox/switch/slider 的 indicator、track、fill、thumb 作为有限 backend-neutral render content；它与标签文本分别解析前景，不由 backend 识别 tag。`CustomRenderNode` 只有 renderer/resource/revision opaque key，不携带 backend object。
`RenderGraph` 将 external resource preparation、连续标准 primitive 与 custom node 编译为
独立 pass，显式注册 target/resource access，并通过 hazard dependency 保留生产、采样与 Scene
顺序。同一 resource 在一帧出现冲突 renderer/revision 时整图拒绝编译。Iced compatibility
executor 将 Draw 映射为标准 painter；`SceneGpuRenderer` 的 InvokeCustom 直接取得同一
Device/Queue、当前 CommandEncoder 与 target，能够在图内编码 custom pass。
`SceneResourceProducer` 则执行 `PrepareExternal`；每个 preparation pass 使用独立 host-owned
encoder，成功后立即由同一 Queue 有序提交，因此后续 producer 失败不会让先前 producer
滞留在未提交状态；`nana.host-texture` 使用这条图管理兼容路径。业务 GPU 内容不得 CPU
readback、Base64/图片编码或额外子窗口后伪装成共享合成。

## Compatibility backend

`RuntimeProgram` / `run_runtime` 是 Rust 应用的 canonical host contract：应用只提供
`RuntimeDocument`、UiScene、平台事件处理与可选 HostTexture / SceneGpu renderer registry，不返回
`iced::Element`。`run_runtime` 直接进入 `run_runtime_scene`（Nana-owned winit + `SceneWgpuPainter`）。
该路径按 `winit::window::WindowId` 映射 `nana_ui_platform::WindowId`，主窗口留在同一个
`HostedGpuContext`，辅助窗口用 `create_surface` 共享 Device/Queue。`WindowCommand::Open` /
`Close` / `Focus` / `SetTitle` / `Move` / `SetBounds` / `SetFullscreen` / `SetMinimized` /
`SetMaximized` / `SetAlwaysOnTop` 作用在目标窗口；关闭主窗口退出，关闭辅助窗口拆除
surface/AccessKit 并发送 `WindowEvent::Closed`。Runtime 先消费 IME，再把同一
`WindowEvent::Ime` 交给 `RuntimeProgram::window_event`（程序不得再写入 Runtime）。
每窗独立 IME request 与 AccessKit adapter，adapter 在首次 show 前创建。

2026-08-17 本机 macOS 证据（`runtime-host-fixture` 经 `run_runtime`）：

- `cargo build -p runtime-host-fixture --locked` 通过；进程 `./target/debug/runtime-host-fixture` pid 14136 保持运行。
- `CGWindowListCopyWindowInfo` 同时看到 titled 窗口 `NanaUI fixture`（480×252）与
  `NanaUI fixture tool`（360×212）。fixture 在 primary `Ready` 时发出
  `WindowCommand::Open`，按钮 Activate 也会再发同一命令。
- 本 agent 的 `osascript` / `AXIsProcessTrusted()` 为 false，`AXUIElement` 返回 -25211
  （TCC 未授权辅助访问），因此本轮未从 System Events 或 Accessibility Inspector 读到
  `AXTextField`/`AXButton`。fixture 主文档含 `TextInput` + `Button`，辅助文档含
  `Button`，角色投影仍是 AccessKit `TextInput`/`Button`。
- Windows/Linux 实机未跑，不记为通过。

当前 `nana-ui` 通过 `SceneWgpuPainter` 绘制 Runtime/UiScene；`nana-ui-vue` 的
`iced-view` / `hosted` feature 接入同一 Scene host，而不是 `iced::Element` 树。
Android 不属于 NanaUI 当前产品范围；未来移动端必须由 Android 原生组件拥有平台
生命周期、IME、accessibility 与原生控件，NanaUI 仅作为嵌入渲染内容参与混合合成，
不直接调用 Android API。无法忠实表达的 affine/text/custom primitive 显式失败。
