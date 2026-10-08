# 应用 API

这篇给你查入口。你第一次写应用，先看 [快速开始](../guide/quick-start.md) 和 [框架如何运行](how-it-works.md)。签名以 rustdoc 为准。这篇不复制每一份类型。

## 你该依赖什么

| 消费方 | crate / 包 | 入口 |
| --- | --- | --- |
| 新的桌面界面 | `nana-ui`（feature `hosted`） | `nana_ui::runtime`、`ApplicationState`、`RuntimeApplication`、`NanaApplication::builder` / `run_runtime` |
| 窗口设置 / 输入类型 | 通常经 `nana-ui` 再导出；需要时直接 `nana-ui-platform`（输入合同本身在 `nana-ui-input`） | `WindowDescriptor`、`WindowHandle`、`CanonicalInputEvent`、`InputPayload`、`InputEndpoint`、`HostServices` |
| Vue 宿主 | `nana-ui-vue` + `nana-js-v8` | `nana_ui_vue::prelude`（`VueRuntimeProgram::run`） |
| Vue 控件 | `@nanaui/nanavue-components` | `NanaButton` 等 |
| Vue renderer | `@nanaui/nanavue-runtime` | `createApp()` |

不要直接依赖 `nana-ui-devtools`、`nana-css-parity` 来画产品界面。前者是无头调试。后者是 CSS 对照测试。

### 输入：一条路由

长期 conformance 矩阵与可重复命令见 [Issue 243 输入 conformance](input-conformance.md)。

所有输入只有一种形态：`CanonicalInputEvent`（`nana-ui-input`。经 `nana-ui-platform` 和
`nana-ui` 再导出）。原生窗口、Vue、devtools、Android 和测试都在自己的边界把原生事件降级成
它一次。之后只走一条路由：

```text
宿主降级 → InputSequencer 盖戳（source / device / generation / sequence / timestamp）
        → InputEndpoint（有界；相邻 pointer move 与 wheel 合并）
        → AppContext::drain_input / route_input（Runtime，按 source 绑定的 document）
        → RoutedEvent → 宿主 → RuntimeProgram::input_event(RoutedInput)
```

- **负载**：`Pointer`、`PointerEnter` / `PointerLeave`、`Wheel`、`Key`（物理键为 W3C
  `code`。如 `KeyA`。逻辑键为布局解析后的名称）、`Text(CommittedText)`、`Composition`、
  `Focus` 以及设备与 source 的连接/断开。一次按键和它输入的文本是两个事件。文本带着按键的
  sequence：按键被控件处理（快捷键、焦点切换、提交表单、终端已发出字节）时。路由丢弃这段
  文本。不会再插入。IME 组字只经 `Composition`。从不由按键名推导。应用按键策略
  （`AppContext::on_key`）和 keymap（`KeymapLayer::resolve_key`）读的就是这一份
  `KeyInput`：物理键留在 `physical`，快捷键按 `logical` 匹配，修饰键四位原样抄进
  存储用的 `KeyModifiers`。公开的键盘事件只有这一份。`FileDrag(FileDragInput)`
  是从窗口外拖入的文件（悬停、放下、取消）。由路由交给登记的放置目标。
  `RoutedInput::pointer_hit` 是它所在或落下的目标。程序想在整个窗口接收文件。就在
  `input_event` 里读 `FileDragKind::Drop` 的路径。
- **绑定**：`AppContext::bind_input_source(source, generation, document)` 返回
  `Result`。generation 回退或同一 generation 换 document 都会被拒绝（`InputBindError`）。
  `unbind_input_source` 先让仍被按住或捕获的指针走一次与显式 `PointerCancel` 相同的取消。
  再撤销捕获与悬停。窗口关闭后重开会用更新的 generation。迟到的旧事件不会被接受。
- **路由状态属于 context**：每个 source 的指针身份、顺序、断连标记、光标与文本输入槽位都
  存在它所绑定的 `AppContext` 里。两个窗口不会共享指针、IME 所有者或光标。事件的时间取自
  它自己的时间戳（Runtime 动画时钟域）。tooltip 延迟、悬停探测和多击判定都用这个时间。
- **焦点**：窗口失焦只取消该 source 按住与捕获的指针和它正在悬停的文件拖放。document 的焦点控件保持不变。重新
  获得焦点时把文本输入状态再交给宿主一次。只有 source 断开且再没有其他获得焦点的 source
  驱动同一 document 时。才清除 document 焦点。
- **命中**：未捕获的指针事件只做一次命中查询（`UiWorld::hit_test_queries` 计数）。overlay
  路由、悬停、诊断 hover、handle 探测、光标和 `RoutedInput::pointer_hit` 共用这一个结果。
  被捕获的指针事件不做命中查询。稳态指针移动（包括每次都切换悬停的移动）不分配内存。
- **drain**：`drain_input` 把事件从端点取出后再路由。任何事件都不会卡在队首。原生宿主对
  状态转换（按下、抬起、按键、文本、组字、焦点）立即 drain。指针移动和滚轮在事件循环本轮
  末尾或下一次重绘前 drain。因此程序每轮看到的是合并后的一个 move。
- **结果**：`InputRouteOutcome` 给出 `handled`、`prevent_default`、`pointer_hit` 和
  `invalidated_work`。`handled` 与 `prevent_default` 相互独立：阻塞型 overlay 可以阻止宿主
  默认行为而不表示控件处理了事件。`AppContext::input_counters()` 给出路由、拒绝、悬停变化、
  光标与文本输入更新次数。以及被丢弃的文本数。命中查询、焦点与捕获变化只进诊断指标。

宿主能力走 `HostServices`（同样在 `nana-ui-input`）：

- **光标**与**文本输入（IME）**是“最新值”槽位。不是请求队列：Runtime 只在值变化时调用
  `set_cursor(CursorIcon)` 和 `set_text_input(Option<&TextInputContext>)`。窗口重新获得焦点
  时再发一次文本输入状态。`TextInputContext` 携带用途（普通、密码、终端）、候选框锚点（插入
  符。逻辑坐标）和选区附近最多 `SurroundingText::MAX_BYTES`（4000 字节）的文本窗口。字段再
  大也只复制这个窗口。密码与终端不提供周围文本。原生宿主把 Runtime 的光标与窗口边框缩放
  光标、程序的 `WindowCursor` 覆盖合成后再设给窗口。
- **剪贴板**是唯一有返回值的调用。在复制/剪切/粘贴快捷键在按键链中原来的位置同步发生：应用
  的按键策略（`on_key`）和终端先看到按键。宿主不等待忙碌的后端。返回 `HostServiceError::Busy`。
  剪贴板拒绝写入时。剪切不删除文本。原生宿主整个进程只打开一次系统剪贴板。
- `HeadlessInput` 是没有窗口的输入源（devtools、测试、离屏测试工具）。自带
  `HeadlessHostServices`。保存宿主本应显示的光标与 IME 状态和一个私有剪贴板。
  `UnsupportedHostServices` 给不需要这些能力的宿主。

仍在范围之外、没有用无头结果代替的：远程 source 的认证/重放/限流。跨线程 endpoint 的
同步与进程级内存配额。canonical 负载的正式 FFI/序列化版本（Rust enum 布局不是 wire 格式）。
嵌套 NanaUI 的坐标变换（#251。随 #247 的真实嵌入生产者再加）。以及真实 Windows IME、
Android、XR 与 accessibility provider 的设备验收。

新代码从 `nana_ui::runtime` 引入控件。crate 根控件兼容面已删除。`runtime::internal` 仅给 Gallery 和宿主适配器、迁移检查使用，已隐藏文档，不是第二套产品 API。`runtime::host` 是 Scene / GPU slot 类型。`runtime::perf` 是帧计数。不是视图状态。

### 根级兼容入口迁移

| 旧入口 | 新入口 | 状态 |
| --- | --- | --- |
| `nana_ui::Button`、`Text`、`Workspace` 等根级控件 | 已删除 | 使用 `nana_ui::runtime::{Button, Text, Workspace, ...}` |
| `nana_ui::Textarea` | 已删除 | 使用 `nana_ui::runtime::TextArea` |
| `nana_ui::components::*` | 兼容面（隐藏文档） | 新代码使用 `nana_ui::runtime::*`；旧路径仅用于迁移和目录检查 |
| `AppContext::world_mut()` | 已删除 | 使用 `commit_mutations`；仅内部宿主/适配器使用 `compat_world_mut()` |
| `nana_ui::dock::*` | `nana_ui::runtime::{Dock, DockWorkspace, ...}` + 宿主适配器 | Dock 树只属于 Runtime |

`world_mut` 与 `Textarea` 兼容入口已删除。后续新增能力只能进入 `runtime`
或明确的宿主兼容模块。

### 能力权威

每项只有一个 owner 和一条推荐调用。Menu/Action 的转换与 dispatch 由 #287 验收，Theme 由 #108 / #110 验收，推荐写法的迁移指南由 #291 验收。

| 能力 | Owner 与推荐调用 | 保留入口 | 删除条件 |
| --- | --- | --- | --- |
| 组件更新 | `update_component` / `set_component` / `create_component` | `create_view`（`doc(hidden)` host primitive，不跑 component project） | 宿主适配器不再需要未投影的占位节点 |
| CSS 颜色 | `nana-ui-css` 解析字符串。PaintScript 只接受 semantic role 与 typed RGBA | 调用方传入的 color parser | 不在 Runtime 再写一套 CSS parser |
| 虚拟列表 / 表 / 树 | `materialize_virtual_*_retained_in` | legacy `materialize_virtual_*`（`doc(hidden)` + deprecated，不与 retained 混用） | 仓库外不再有未放置调用。本仓库的新调用由 `scripts/check-api-convergence.py` 拒绝 |
| 查询编辑 | `TextInputState`，经 `query_text()` / `set_query()` | 无第二份 query 字段 | 查询权威保持这一份 |
| 虚拟行身份 | `each_virtual::Unit::Row { columns, keys }` | 无哈希身份 | 行身份保持结构化 key |
| 事件订阅 | 宿主自己的 stream | `Subscription<T>`（`doc(hidden)` + deprecated） | 无产品调用后删除。本仓库的新 `Subscription::new` 由同一脚本拒绝 |
| GPU surface | `HostedGpuShared::resize_surface` / `prepare_surface_frame` | `HostedGpuSurface::resize` / `prepare_frame`（`doc(hidden)` advanced） | 仍有宿主把 surface 与 device 拆开持有时保留 |
| 默认 GPU renderer | `scene_gpu_renderers`：`None` 与空表都不安装演示 renderer。需要时显式返回 `default_scene_gpu_renderers()` | 无隐式 `"gpu-view"` | 演示 painter 保持 opt-in |

`ActionDescriptor` 只有一个。定义在 Runtime（`nana_ui::runtime`。`nana_ui` 再导出同一个类型）：keymap 读 `id` / `enabled` / `when`。命令面板另外读 `label` / `category` / `keywords`。只绑快捷键的宿主用 `ActionDescriptor::new(id)`。要进面板的用 `ActionDescriptor::labeled(id, label)`。`ActionRegistry` 同样只有一个。按注册顺序保序。`search` / `available` 供面板检索。

Vue 产品窗口需要 `nana-ui-vue` 的 `hosted`（隐含 `scene-view`。把 UiScene 交给 `SceneWgpuPainter`）。没有 `scene-view` 的构建只做 flush / 对照。不画产品帧。

## Cargo feature

`nana-ui` 默认打开 `wgpu-backends`（仅在启用 `gpu` 时生效）。需要自选图形后端的应用使用 `default-features = false`，再按职责打开：

| feature | 作用 |
| --- | --- |
| `hosted` | `run_runtime`、winit 0.31-line（workspace git pin，非 crates.io 0.31.0）、AccessKit；隐含 `gpu` |
| `gpu` | `SceneWgpuPainter`、`HostTexture`、`GpuTextureView`、`GpuView` |
| `bundled-fonts` | 嵌入 Noto Sans SC |
| `components` | 下面组件族的聚合 |
| `full` | fonts + components + hosted + syntax-highlighting |
| `calendar` / `charts` / `controls` / `graph-canvas` / `image-viewer` / `rich-text` | 打开对应 runtime/scene `cfg` 并再导出该族。未启用则该族不参与编译。历史 no-op 别名（`overlays`、`selects`、`qr-code` 等）已删除，改用 `components` / `full` |
| `syntax-highlighting` | `TextArea` 的 `"highlight"` presenter |
| `accesskit-tree` | `AccessTreeProjector` 的 TreeUpdate 投影导出，给自接平台适配器的宿主（如 Android `accesskit_android`）；`hosted` 已隐含 |

Cargo 不会因你写了 `CalendarHeatmap` 就自动打开 `calendar`。

## 应用身份、路径与诊断

进程级设置走 `NanaApplication::builder(ApplicationIdentity)`：解析并发布 `ApplicationPaths`（`NanaApplication::paths()` 随处可取）。可选 `.diagnostics(DiagnosticsConfig::default())` 打开结构化诊断。然后 `.run::<P>(WindowDescriptor)`。`run_runtime` 仍可直接用。只是不设路径、不开诊断。业务不要自己拼 `./runtime/...` 或平台目录。从 `ApplicationPaths` 取逻辑位置（`runtime_resources`、`data`、`config`、`cache`、`logs`、`crash` 等）。`app_data_dir` 保留。在桌面平台的安装布局下与 `ApplicationPaths::data` 相同（便携版的 data 在 `<root>/data`）。详见 [诊断与应用路径](diagnostics.md)。

打包后的应用用 `nana_ui_platform::application_identity!` 声明身份：它会在二进制里留下身份标记。packager 与 validator 都拿它比对。开启 `packaged-resources` feature 后。调用 `.resource_packs(ResourcePackOptions)` 会读取 package manifest。并把 `.nrpack` 资源包挂载到 `nana://res/<逻辑路径>`。样式、`@font-face`、`url()` 都能直接引用。安装版和便携版里。没有显式 base 的相对 `url()` 以 `runtime_resources` 为基准。不再以 CWD 为基准。详见 [打包与分发](packaging.md)。

## RuntimeProgram

普通 Rust 应用优先实现 `ApplicationState`。由 `RuntimeApplication<State>` 管理
每个窗口的 `RuntimeDocument` 和资源注册表。最小可运行程序见
`crates/nana-ui/examples/application-counter.rs`。
下列低层合同仍用于 Vue 和自行管理窗口/文档的嵌入式宿主。

只从 Rust 类型创建控件（`mount` / 声明式视图 / `.vue`）的应用。可以设 `const BUILTINS: BuiltinComponents = BuiltinComponents::Typed;`：内置控件只注册标识。程序没创建过的控件不会被链接进来。Vue 等按标签构造控件的宿主保持默认的 `Full`。详见 [声明式视图](reactive-view.md#控件级摇树builtincomponentstyped)。

`ApplicationWindow::demand` 与低层 `RuntimeProgram::frame_demand` 使用同一
`FrameDemand`：默认 `OnDemand`。单次截止时间 `At(Instant)`。持续刷新
`Continuous(NonZeroU32)`。120 代表请求 120Hz。不是显示器刷新率保证。
宿主会把 `UiScene::compositor_needs_tick` 合并进该窗口的 present cadence。
compositor overlay 不改写程序自己的 `frame_demand`。也不唤醒无关窗口。
device/surface 丢失后宿主调用 `RuntimeDocument::set_surface_generation`。
资源更新使用 `TextureSlot`。详细变更和当前验收范围见
[高刷新重构](../../archive/docs-notes/high-refresh-refactor.md)。

应用实现这个 trait。再 `run_runtime::<App>(WindowDescriptor::new("…"))`。

| 方法 | 职责 |
| --- | --- |
| `initialize` | 建程序实例；可返回要在第一帧 `update` 的消息。被调用的时刻就是启动的 `UiReady`，只建第一屏，重活交给任务，见 [两阶段启动](startup.md) |
| `startup_takeover` | 可选；`initialize` 建的文档是否立即接管 Early Splash（默认 `Immediate`），`Deferred` 则等 `context.startup().take_over(ticket)` |
| `startup_changed` | 可选；启动阶段变化（请求、撤回、交接完成） |
| `with_document` / `with_document_mut` | 按 `WindowId` 在访问闭包中交出 `RuntimeDocument` |
| `update` | 宿主级消息；保持便宜 |
| `theme` | 返回注册表解析后的 `Arc<CompiledTheme>`；Light/Dark 是预制主题，身份由 `ThemeId` 表示 |
| `window_material_mode` | 可选；默认实色 |
| `host_textures` | 默认；slot → `HostTexture` |
| `prepare_window_frame` | flush 前准备纹理。窗口遮挡或最小化时 `FrameDemand` 到期仍会调用，包括 0 维；不 flush、不 present。producer encode 仅在尺寸可画时跑 |
| `window_frame_presented` | present 后释放旧资源。隐藏 GPU tick 不调用 |
| `scene_gpu_renderers` | 高级。`None` = 不注册自定义 renderer（不会隐式安装演示 `"gpu-view"`）；空表 = 明确没有自定义 renderer。需要内置演示 painter 时显式返回 `default_scene_gpu_renderers()` |
| `scene_resource_producers` | 高级。按图离屏；第一次可忽略 |
| `bind_window` | present 之后填内容 |
| `rebuild_gpu` | 设备丢失后重绑资源 |
| `window_event` | 窗口生命周期。系统关闭与标题栏关闭按钮都只发 `CloseRequested`，宿主不自行关窗；返回 `WindowCommand::Close(id)` 或 `exit()` 才关闭。默认实现立即关闭，需要先保存的程序可稍后再返回 |
| 通过 `WindowHandle::set_menu_bar` 设置菜单 | 原生菜单栏；选中项用 `take_menu_activations()` 每帧 drain，见 [窗口](window.md#菜单栏) |
| `input_event` | Runtime 派发之后的原始输入，唯一的输入钩子。参数 `RoutedInput` 同时带 `event`、`pointer_hit`（仅指针与滚轮）和 `disposition`；已消费事件仍派发，应用快捷键应检查 `disposition.prevent_default` |
| `next_wakeup` / `wake` | 与重绘无关的定时工作 |
| `host_failure` | 宿主已从该错误恢复；默认忽略 |

`RuntimeProgramContext` 提供 `window_id`、`geometry`、`gpu()`、`material()`、`dispatch`、`run_task`、`startup()`（启动记录与接管请求）。原生窗口句柄不穿过这条边界。

`ApplicationState` 透传下列钩子。`RuntimeApplication<State>` 原样转给 `RuntimeProgram`，所以透明叠加窗口、需要合成器路径的应用不必再手写一层 `RuntimeProgram`：

| `ApplicationState` 方法 | 对应 `RuntimeProgram` 钩子 |
| --- | --- |
| `gpu_backend_policy()`（关联函数） | `gpu_backend_policy` |
| `startup_window_material_mode()`（关联函数） | `startup_window_material_mode` |
| `window_material_mode_for(&self, id)` | `window_material_mode_for` |
| `appearance_backdrop_opacity_for(&self, id)` | `appearance_backdrop_opacity_for` |
| `input_event(&mut self, id, input, windows, cx)` | `input_event`，多给一份全部窗口的 `ApplicationWindow` |
| `next_wakeup(&self)` / `wake(&mut self, now, windows, cx)` | `next_wakeup` / `wake` |
| `close_requested(&mut self, id, windows, cx)` | `window_event` 的 `CloseRequested`。默认回 `WindowCommand::Close(id)`；回答里不带它，窗口就留着（先确认、收到托盘），决定后再从 `update` 关 |

富文本：`nana_ui::runtime::rich` 是值的词汇（`RichText`、`RichSpanStyle`、`RichTextStroke`、`RichTextShadow`、Style Model 的 `PaintColor`）。`RichTextView` 组件或 `MutationQueue::set_rich_text(id, rich)` 把它挂到文本节点上，按改到的层定价：塑形层重新排版，绘制层只重绘，`effect` 不碰文本。见 [RichTextView](../components/rich-text-view.md) 与 [文本引擎](text-engine.md#富文本-span)。编辑用 [RichTextEditor](../components/rich-text-editor.md)：工具栏调 `AppContext::rich_edit(entity, RichEditCommand)`，文档经 `RichTextEditorEvent::Changed` 交回应用。逐字特效和打字机揭示用 `AppContext::set_rich_presentation(node, effects, reveal)`，只是呈现，不做文本工作，只在播放期间请求帧。

`AppContext::animation_now()` 是文档的动画时钟：合成器上一次采样动画用的时间。应用自己排的呈现（打字机揭示、逐字特效的起点）读它，和合成器用同一个时钟，不用会漂移的墙钟。

`RuntimeProgramUpdate.redraw` 支持 `None`、`Window(id)`、`Windows(ids)`、`All`。
合并局部更新会保留实际窗口集合。`RuntimeRedraw::for_windows` 会排序去重。
`RuntimeRedraw` 现在持有窗口列表。只实现 `Clone`。不再实现 `Copy`。穷尽匹配需处理 `Windows`。
Vue 输入按语义变化和已挂载节点消费的 Canvas／HostTexture 版本选择窗口。

## 建树

```text
RuntimeDocument::new(DocumentId)
mount_view_root(document, || {
    column().gap(12).children(
        button("…").on_cx(|_, _: &Activate, cx| cx.dispatch_program_all(Msg)),
    )
})
mount { scope.child("key", …) }          // 动态区增删
update_component(entity, |view, _| { … }) // 改单个字段
set_component(entity, Button::new(…))     // 整体换 props，保留交互态
```

挂载视图是初次整页（一次 commit）。之后靠绑定定点更新。`mount` 是 keyed 子树协调。不是第二套渲染器。点击 handler 不要重新挂载一遍。Vue 不得用 `create_component` / `mount_view*` 分配 ID。它绑定自己已有的节点。细则见 [L3：用 Rust 建界面](l3-authoring.md)。

从应用状态整体重建一个组件时用 `set_component`。不要在 `update_component` 里写
`*view = 新的()`：后者连运行时拥有的交互态一起覆盖。表现为刷新一下菜单就收起、
过滤框光标被重置。`set_component` 走 `ComponentView::reconcile`。由每个组件决定
什么该活下来（`Select` / `Dropdown` / `SearchDropdown` 保留展开与高亮。
`SearchDropdown` 还保留用户已输入的查询与光标）。props 真的变了仍会重置交互态。

刷新时把整张列表按当前值重写一遍是可以的：组件与原值相等、闭包也没排 mutation 和事件时。
`update_component` / `set_component` 直接返回。不投影也不提交。应用不需要自己按行指纹跳过。
组件自身没变、只是它投影时读的 world 状态变了。用 `reproject_component(entity)` 重新投影。
空闭包的 `update_component` 不再有这个作用。

已经持有实体、切换时还要保留其状态的区域。可用
`reconcile_children(parent_id, &[child_id, ...]) -> Result<bool, FrameworkError>`。
它发布父节点的完整子节点顺序。省略的子树停放而不销毁。合法跨父节点移动以及
停放父节点上的装配都支持。同序返回 `false`。缺失节点、重复项、环和跨文档移动
会在同一 Runtime 事务中失败。不会先停放其他孩子。它不创建组件或替代带语义的
slots / overlay 组装接口。`mount` 仍用于按 key 构造并销毁缺席组件的动态区域。

`create_component` / `append_child` / `on` 仍是底层 primitive。

对外身份是 `StableNodeId` / `Entity<V>`。不要依赖内部实体编码。

## 扩展控件

| 目标 | 路径 |
| --- | --- |
| 进入布局、命中、Scene | `UiExtension` + `register_component`；Vue tag 为 `ComponentTypeId` 去掉 `nana.`（与 HTML 同语义用原生标签；不同语义换名） |
| 仅 JS 命令 / props 白名单 | `NativeComponentRegistry` + `Nana.components.call` |
| GPU 内容 | `GpuTextureView` + 宿主纹理；直写见 `GpuView` |
| 改一个节点长什么样 | `Painter` 挂到 `NodeStyle::painter`（`Card::painter` / `Stack::painter` / `Panel::painter`），见下 |

不支持动态 dylib。

### 节点自绘（`Painter`）

相当于 Qt 的 `paintEvent`：`paint(&self, cx)` 画在子节点下面。
`paint_over_children` 画在子节点上面。在里面调用 `cx.draw_default()` 就画出
该节点原本的内建外观（基类的 `paintEvent`）。不调用则完全替换。子节点照常布局和
绘制。负 `z-index` 的子节点也在 `paint` 之上。

`cx` 录的是命令。不是立即绘制：同一节点在（`paint_key()`、布局尺寸、主题代数、
交互状态、字体集）不变时不会重录。只移动时连三角化都复用。颜色写
`SemanticColorRole` / `SemanticColorMix`。圆角写 `RadiusTier`。阴影写
`ElevationRole`。由 `cx` 按当前主题解析。切换主题自动重录。

- 状态：`cx.state()` 给出悬停、按下、可见焦点、禁用、选中。状态变化会重录。
  不必算进 `paint_key`。悬停只看节点自己是不是命中目标。指针落在子节点上时
  父节点的 `hovered` 为假。与内建交互外观一致。
- 文字：`cx.measure_text(&text, max_width)` 用宿主当前的排版引擎测量（未接引擎时
  按 em 估算）。与 `cx.text` 画出来的一致。`PaintText` 支持折行、最多行数、
  行高、斜体和字体族（`family`，CSS `font-family` 语法；测量和绘制读同一个）。颜色可以是渐变。
- 路径：`PaintPath` 支持直线、二次 / 三次贝塞尔、`arc` 和 Canvas 语义的
  `arc_to`（任意拐角倒圆。凹角也可以）。NonZero / EvenOdd。参数与 Canvas 一样
  写成标量（`arc` 的最后一个参数是扫过的角度。与 `QPainterPath::arcTo` 相同。
  不是 Canvas 的终止角）。`PaintPath::from_svg` 从 SVG path 数据建路径。`contains` /
  `stroke_contains` 对应 Canvas `isPointInPath` / `isPointInStroke`。另有
  `bounds`、`transformed`。`cx.fill_path` / `cx.stroke_path` 带抗锯齿。
- 上色：填充、描边、圆角矩形、文字和图标都接受纯色或 `Gradient`（`linear` /
  `radial` / `conic`。任意多个色标。`Pad` / `Repeat` / `Reflect`）。渐变逐像素
  求值。色标进入 Scene 后在 linear scRGB 中按 premultiplied 方式插值；sRGB 输入只
  解码一次。超过 16 个色标时重采样。
- 描边：`StrokeStyle` 设宽度、线帽、连接、尖角限制（Canvas `miterLimit`）。
  缺省与 Canvas 一致：平头线帽、尖角连接、限制 10。
  `.dash(pattern, offset)` 是 Canvas `setLineDash` / `lineDashOffset` 语义。
  每段按线帽收尾。一条描边切出超过一万段虚线时按实线画（每段已不到一个像素）。
- 局部变换：`translate` / `scale` / `rotate` / `concat` / `set_transform`。
  `save` / `restore` 保存恢复变换、不透明度和混合方式。和 Canvas 一样。
  `restore` 也弹出 `save` 之后压入的裁剪（图层仍由 `push_layer` / `pop_layer`
  单独管）。路径、裁剪、
  渐变随变换。描边和虚线先在变换前的坐标里算好再整体变换（与 Canvas 一致）。
  阴影的偏移和模糊不随变换。文字、图标、图片和 `draw_default()` 的内建外观也
  跟随当前变换。
- 不透明度与混合：`set_opacity`（Canvas `globalAlpha`。逐条命令生效）、
  `set_blend(mode)`。以及 `push_layer(opacity, blend)` / `pop_layer` 把一组命令
  先画进图层再整体合成。重叠部分不会叠加透明度。混合方式是完整的 CSS
  `mix-blend-mode` 集合（`multiply`、`screen`、`overlay`、`darken`、`lighten`、
  `color-dodge`、`color-burn`、`hard-light`、`soft-light`、`difference`、
  `exclusion`、`hue`、`saturation`、`color`、`luminosity`）。CSS 的
  `mix-blend-mode` 也随之支持这些值。
- `cx.shadow(path, ..)`：沿路径轮廓的阴影。外阴影或 `inset` 内阴影。
- `cx.push_clip(path)` / `cx.pop_clip()`：只裁剪之后录的内容。不影响子节点。
  也不会裁掉 `push_clip` 之前画的阴影。任意路径都精确：路径几何在 CPU 上切割并
  保留抗锯齿。文字、图标、图片和 `draw_default()` 的内建外观在裁剪路径是（圆角）
  矩形或不超过 8 个顶点的多边形时直接用 GPU 裁剪。其他形状画进一个按路径遮罩的
  图层。
- `cx.rounded_rect`（四角独立圆角。可带边框和阴影）、`cx.text`、`cx.icon`、
  `cx.image(rect, source, fit, radii)`（`source` 与 CSS `url()` 同源。`fit`
  同 `object-fit`。异步加载）。`cx.fill_path_with_image(path, rect, source, fit)`
  用图片填充任意路径。之后的图片怎样采样由 `cx.set_image_sampling(..)` 决定
  （随 `save` / `restore` 保存恢复）。见下文 [图片采样](#图片采样)。
- 命中：`Painter::hit_test(local, size)` 返回 `Some(false)` 的点点穿到下面。
  `hit_painted_outline()` 为真时命中区域等于录下的内容（填充、描边含虚线空段、
  圆角矩形、图片、文字框、`draw_default()` 的节点矩形。按变换和裁剪计算）。
  阴影不参与命中。与 CSS `box-shadow` 和 Canvas `isPointInPath` 一致。
  `set_opacity(0)` 之后、或不透明度为 0 的图层里画的内容也不参与命中。描边
  按到中线的距离不超过半个线宽判断。即线帽和连接一律按圆形算。命中读的是节点
  最新的录制。状态、焦点或主题变化重录后立即生效。
- 数值：含 NaN / 无穷的路径、线宽和阴影不绘制。变换后超出节点原点一百万像素的
  路径不绘制。线宽、模糊和扩散按一万像素封顶。

除了写在 `NodeStyle::painter` 上。也可以用 `AppContext::set_painter(id, …)`
（`MutationQueue::set_painter`）把 painter 挂在任意节点上：它优先于样式里的
painter。组件重写节点样式也不会把它冲掉。适合给内建组件换外观。

节点的 transform、裁剪、不透明度、层叠顺序和合成器动画照常作用于自绘内容。
挂了 painter 的节点自身构成层叠上下文。节点自己的 `overflow` 裁剪只作用于子
节点。不裁自绘的阴影。

拿不到 Rust trait 的消费方用 `PaintScript`：一份 JSON 命令列表。覆盖 `cx` 的
全部绘制命令（需要读回结果的 `measure_text` 和自定义命中逻辑除外。命中可以给
一条路径）。长度可以写成相对节点尺寸的 `"50%"`、`"100% - 12"`。颜色写语义
角色名、主题混色或 typed paint。Vue 的 `paint` 属性还会由
`nana-ui-css` 将 CSS 颜色字符串解析成同一份 typed paint；直接调用 Runtime
`PaintScript::from_json` 时请传 typed paint。路径写 SVG path 字符串。命令可按交互状态筛选。
写错的字段名直接报错（格式见 `PaintScript::from_json` 的文档）。图标按内建
图标名画。

单元测试里可以用 `PaintRecording::record(&painter, &theme, size, state)` 不借助
`UiWorld` 录一次 painter。检查它录下的命令。Vue 里把它写在任意元素
的 `paint` 属性上（JSON 字符串或对象）。包括按钮这类内建组件。解析失败时不挂
painter。原因记在 `WidgetProps::paint_error`。并以 `nana.paint` 来源的警告
送到 `VueHost::set_diagnostics` 接的诊断回调（同一元素的同一错误只报一次）。

`Panel::kind(CardKind::Flat)` / `Card::kind(CardKind::Flat)` 不画底色和边框。
适合交给 painter 自绘外框。

## JavaScript 产物形态

Vue / JS 入口交给宿主的是一份 `RuntimeArtifact`。两种形态：

| 形态 | 构造 | 说明 |
| --- | --- | --- |
| 源码 | `RuntimeArtifact::from_source(name, source)` | UTF-8 JavaScript（通常是你 Vite 打出的 IIFE）。框架在加载前用 `compose_runtime_artifact` 把 Web API shim 拼到前面。 |
| Binary Release | `RuntimeArtifact::from_v8_snapshot(name, bytes)` | V8 `SnapshotCreator::create_blob` 的快照。`is_binary_release()` 为真，**原样加载**，框架不再拼 shim。 |

因此快照必须在 `compose_runtime_artifact` **之后**编译：shim 要一起进快照。否则运行时找不到 `__nanaWebApi`。源码形态下框架会检测 `__nanaWebApi` 是否已存在。已拼过的不会重复拼。

`name` 同时是样式表解析的基准：相对 `@import` 与 `url()` 都相对它兑现（见[布局](layout.md)的 `stylesheet_base`）。

## Fetch 宿主

网络权限属于宿主。不属于页面。页面作者那一侧（`response.body` 怎么读、`text()` / `json()`、`FormData`、`ReadableStream` 的限制）见 [Vue](vue.md) 的「它提供的 Web 面」与「网络与宿主命令」。这里只说嵌入方要实现和配置的那一半。

`WebApiState::new()` 默认装的是 `NativeFetchHost::new(FetchPolicy::default())`。而默认策略**一个源都不放行**。不配白名单就是全拒。没有宿主点头。页面碰不到网络。

### FetchHost

| 方法 | 合同 |
| --- | --- |
| `fetch` | 必须实现。收 `FetchRequest`（url / method / headers / 完整正文），还 `FetchResponse` 或 `FetchError` |
| `fetch_cancellable` | 多收一个 `FetchCancellation`。默认实现先 `check()` 再调 `fetch`；后端可中断就应该在连接、跟随重定向和读正文的循环里都观察它 |
| `fetch_streaming` | 把响应写进 `FetchSink`。默认实现回落到 `fetch_cancellable`，整份正文当成一块发出——只实现了 `fetch` 的宿主照常能用，只是不会流 |
| `policy` | 必须实现。上限、超时、重定向次数和 worker 数都从这里读 |

`FetchSink` 两个回调：`head(FetchHead)` 恰好调用一次。且在任何 `chunk(&[u8])` 之前。任一返回 `Err` 就地中止这次传输。消费方已经走了（或者它自己有上限）。就不必再为没人要的字节付钱。`FetchHead` 是除正文外的全部响应信息。`FetchHead::with_body` 把它和读完的正文合回 `FetchResponse`。

响应上限取自**执行这次请求的宿主自己声明的 `policy()`**。`fetch_streaming` 的默认回落实现也照它判：超了报 `ResponseTooLarge`。一块都不进 sink。否则只实现 `fetch` 的宿主会把自己 `FetchPolicy` 里写的上限整个漏掉。

### FetchPolicy

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| 允许的源 | 空集 = 全拒 | 精确 **origin** 比较，不是路径前缀 |
| `timeout` | 30 秒 | 一次请求的总预算，跨重定向共享，不是每跳一份 |
| `max_request_bytes` | 16 MiB | 请求正文，发出前查 |
| `max_response_bytes` | 16 MiB | 响应正文；流式下按**累计**字节算，不因为分块就放行更大的正文 |
| `max_redirects` | 5 | 超过报 `Redirect` |
| `worker_count` | 4 | 见下面的 worker 边界 |

源用 `allow_origin` / `with_allowed_origin` 登记。格式 `scheme://host[:port]`。按 URL origin 的 ascii 序列化存：带路径、query 或 fragment 的写法报 `InvalidRequest`。非 http/https 报 `Policy`。放行 `https://example.com` 就放行了它下面的所有路径。但**不**包括 `https://api.example.com`。子域名是另一个 origin。要单独登记。`authorize(&Url)` 是执行点。`allowed_origins()` 可回读。

### 内置 NativeFetchHost 做了什么

阻塞式 `ureq` 实现。必须跑在 UI / JS 线程之外（`nana-ui-web-api` 提供这条 worker 边界）。它在每一跳之前执行策略：

- 请求正文超 `max_request_bytes` → `RequestTooLarge`。不发。
- 响应先看 `Content-Length`。超了直接 `ResponseTooLarge`。正文一个字节都不进 sink。读的过程中再按累计字节判一次。超了当场断流。
- 每一跳都重新 `authorize`。重定向目标不在白名单里照样拒。跨源时 `authorization` / `proxy-authorization` 被摘掉。303 以及 POST 的 301 / 302 转成 GET。清空正文与 `content-length` / `content-type`。
- `set-cookie` 不会出现在交给 JS 的响应头里。
- 超时是整次请求的总预算：每跳按已用时间扣减。扣光报 `Timeout`。

错误按 `FetchErrorKind` 分类：`Policy`、`InvalidRequest`、`Network`、`Timeout`、`RequestTooLarge`、`ResponseTooLarge`、`Redirect`、`Cancelled`、`Unsupported`。

### worker 边界

`worker_count`（至少 1）条阻塞线程调 `fetch_streaming`。任务队列长 `worker_count * 2`。队列满时 `fetchStart` 抛「fetch worker queue is full」。不会阻塞发起请求的那条线程。响应头、分块和错误都只在帧泵里回到引擎线程。`FetchHost` 实现不碰 JS。

### 装上去

| 入口 | 用法 |
| --- | --- |
| `MountOptions.fetch_host` | Vue 宿主的常规入口；`mount_vue_as_nana` 用它建带该 host 的 web-api 状态 |
| `shared_fetch_host(host)` | 把自己的 `FetchHost` 实现包成 `SharedFetchHost`（`Arc<dyn FetchHost>`） |
| `WebApiState::with_fetch_host` / `with_fetch_host_and_local_storage` / `shared_web_api_state_with_fetch` | 自己管 web-api 状态的嵌入式宿主 |
| `RuntimeProgram::resource_fetch_host(id)` / `ApplicationWindow.fetch_host` | 该窗口文档的引擎资源出口（`url()` 图片等）。Vue 宿主自动返回该窗口 web-api 的 `fetch()` host；纯 Rust 应用按窗口填写，缺省为 `None`（拒绝远程图） |
| `SceneWgpuPainter::set_resource_fetch_host` / `NanaVueApp::fetch_host` | 自己持有 painter 的嵌入式宿主：每次绘制某个文档前，把该文档的 host 交给 painter |

```rust
let policy = FetchPolicy::default().with_allowed_origin("https://api.example.com")?;
let app = mount_vue_as_nana(MountOptions {
    fetch_host: Some(shared_fetch_host(NativeFetchHost::new(policy))),
    ..MountOptions::default()
});
```

自己实现 `FetchHost` 最少写 `fetch` 和 `policy`。要让页面能边到边读。再实现 `fetch_streaming`。

### 这条路上明确没有的

- **cookie**：JS 侧带 `cookie` / `set-cookie` 请求头直接 `TypeError`。响应里的 `set-cookie` 被丢弃。没有 cookie jar。
- **CORS / preflight**：没有浏览器同源模型。也不会发 `OPTIONS` 预检。唯一的门是宿主白名单。
- **cache**：没有 HTTP 缓存层。`cache` 选项在 JS 侧被拒。
- **请求侧流式正文**：正文在发给宿主之前已经是完整字节。`duplex` 在 JS 的拒绝列表里。响应侧的流式见上面的 `FetchSink`。

引擎自己的资源出口走的也是这条路。`url()` 图片、`<img src>`、`mask-image`、`border-image` 与 JS `fetch()` 共用同一个 `FetchHost` 和同一份 `FetchPolicy`：同样的 origin 白名单、同样逐跳复核重定向、同样跨源摘授权头。**没装 host 就一张远程图都不取**。`data:`、`file:` 与 jail 内的相对路径不受影响。这份 host 按文档而非按进程：同一进程里多次 mount、多个窗口共用一个 painter 时。每个文档的图片只经过它自己的 host。缓存结果也按 host 分开。不会拿到另一份策略放行的图。取消在 painter 析构与图片连续 120 帧无人引用时触发。终结的是网络等待。不是已经开始的解码。`@font-face` 没有网络传输。只认 `local()`、`data:` 和 jail 内的本机文件。

## 性能上你不用手写的

挂载视图（`mount_view*`）把整棵子树收成一次 commit。mutation 提交后 Runtime 自己调度脏工作。无变更不刷帧。大列表走 `materialize_virtual_*`。GPU 换纹理升 generation。不重建布局。

### 图片采样

`url()` 图片（`<img src>`、`background-image`、`cx.image`）默认按**实际绘制的设备像素**准备：
painter 在每次全新 prepare 里记下每张图画成多大（`ContentFit` / `background-size` 之后的
尺寸 × 节点变换 × 缩放因子。与 HostTexture 的 `painted_extent` 同一口径）。同一张图被多处、
多个窗口使用时取最大的那个。后台线程用 CatmullRom 在线性光、预乘 alpha 下把原图缩到这个尺寸。
只上传这一层。按双线性采样。大图显示成小图不再走样。显存按显示尺寸计。尺寸跟随有滞回：
变大立即重采样。变小要等较小的尺寸稳定 2 秒。`max(6px, 6%)` 以内的变化和任一边小于 16px
的需求（折叠、动画中）不触发。布局与 `fit` 仍按图片的原始尺寸计算。重采样不改变占位。

同一张图同时以差别很大的尺寸出现、或尺寸持续变化（缩放动画、可缩放预览）时。改用 mip 链：

```rust
BackgroundImage::url_with_fit(url, BackgroundImageFit::Cover)
    .with_sampling(ImageSampling::Mipmap);  // CSS 图层 / <img> 内容
cx.set_image_sampling(ImageSampling::Mipmap); // Painter 里之后的 cx.image
```

`ImageSampling::Mipmap` 保留解码尺寸。在后台生成完整 mip 链。三线性采样
（`MipmapFilterMode::Linear`）。同一 URL 的两种采样是两份独立的缓存项。

本地图片（文件、`data:`、`nana://res/`）第一次出现时仍在帧上同步解码并先按解码尺寸显示。
重采样或 mip 链在后台完成后替换。远程图片在后台线程里取回、解码并直接准备到目标尺寸。
重采样时复用已取回的字节。不再请求网络。自接事件循环的宿主照旧用 `set_image_waker` /
`has_pending_images()` 等待这些后台结果。

HostTexture 的像素归宿主：默认仍只采样第 0 层。宿主按 `painted_extent` 准备尺寸（见
[实时画面](gpu.md#按实际绘制像素准备内容)）。宿主自己上传了 mip 链时。在 `GpuTextureView` /
`Thumbnail` / `Avatar` 上用 `.sampling(ImageSampling::Mipmap)`（`CustomRenderNode::with_sampling`）切到三线性采样。
Vue 目前没有对应的 CSS 属性。`<img>` 只走默认重采样。

消息有两个入口。按类型选：`dispatch_program` **按 Rust 类型只保留最后一条**。适合「后一条取代前一条」的状态消息（resize、主题变了、请求重绘）。`dispatch_program_all` 按派发顺序全部送达。业务消息通常是一个 `enum`。那就是**同一个类型**。用 `dispatch_program` 会让同一帧内的两次点击塌成一次、悄悄丢掉第一次。这种情况用 `dispatch_program_all`。两者都在下一帧进入 `update`。

控件需要先于默认编辑处理按键时。用 `AppContext::on_key` 或 `on_view_key` 注册一个策略。后者读取当前保留的控件值。返回 `true` 表示消费。重复注册替换旧策略。删除视图会移除策略。输入路由在浮层处理后、终端与默认编辑前调用 `dispatch_focused_key`。只投递按键（提交的文本不经过策略）。只投递给当前文档中已挂载且未禁用的焦点节点。IME 组合期间跳过业务策略。策略消费的按键不再插入它所输入的文本。

应用改写编辑器文本（`update_component`、`set_component`、`mount` 或直接提交 `SetTextInput`）时。只要字节变了。该编辑器的 undo/redo 随这次提交清空（直接写 `UiWorld` 的。在下一次使用时清空）：载入另一份文档后 Ctrl+Z 不会退回上一份。写回编辑器自己报告的值、或重建出相同文本。不算改写。日志保留。从应用侧（`update_component` 等）做的发送后清空、格式化写回同样会清掉用户的撤销历史。要让它成为用户可撤销的一步（格式快捷键、补全）。改用 `edit_text_area(entity, range, text)` / `edit_text_input(…)`：把 `range` 替换成 `text`。走用户自己编辑的同一条路径。记成独立的一步。不并入前后的连续输入。编辑器只读、禁用或正在输入法组字时与用户输入一样被拒绝。`TextInput` 的长度上限照样生效。范围碰到的 atom 整体替换。包括用户自己的光标在内的每个光标都随编辑平移、不会被移到编辑处。唯一的例外是正好停在插入点上的光标会移到插入文本之后。与打字一致（在光标处补全）。它不是在 snippet 占位里打字。联动占位不跟随。进行中的 snippet 会话像任何值变化一样被重映射或结束。超出 `TextInput` 长度上限时整体拒绝、不截断。替换成相同文本不算编辑、不发变更事件。范围越界或不在字素边界上返回错误。编辑器自己的 `TextChanged` 处理器（`on` 注册）在事件投递中改写编辑器（转大写、过滤字符）。算这个编辑器接收输入的一部分：这一步记下处理器改写后的文本。照常可撤销。不改变任何文本的一步不保留。发送后清空时。已发送的草稿能否撤销回来由应用决定：不希望撤回时。在发送时调用 `clear_text_history(node)`。只有 `TextInput` 与 `TextArea` 记撤销日志：数字框、搜索下拉、命令面板、右键菜单的输入框还带着文本以外的状态（已提交的数值、筛选结果）。只还原文本会让两者对不上。所以它们不记日志。`can_undo_text` 对它们恒为假。

保留编辑器绑定到另一个任务、文件或草稿身份时。即使文本相同也应调用 `clear_text_history(node)`。清除原对象的 undo/redo。它不改变文本、选区或正在进行的 IME。业务对象身份和是否允许重绑定仍由应用判断。

自接指针的组件可用 `UiWorld::pointer_layout_position` 将窗口坐标转换到布局坐标。反向用 `layout_pointer_position`。两者使用当前命中投影。包括祖先滚动和透视变换。无投影、已 park 或不可逆变换返回 `None`。输入之前应由既有帧流程刷新布局与命中投影。

## Rust 虚拟列表和树

需要真实滚动占位与屏外编辑保留时。将 List 放在 ScrollView 内。使用
`materialize_virtual_list_retained_in`。树使用 `materialize_virtual_tree_retained_in`
和只含展开行的 `VirtualTreeLayout`。列表示例（`list` 已挂载。`items` 跨帧保存）：

`materialize_virtual_list[_in]`、`materialize_virtual_tree[_in]` 和
`materialize_virtual_table[_in]` 是旧的非定位兼容入口，已隐藏并标记为 deprecated。
它们只负责可见 key 的挂载和卸载，调用方必须自己安排位置、内容总高度以及焦点/IME
保留；同一个 `Virtual*Items` 实例不能与 retained 入口混用。新代码统一使用下面的
retained 入口，旧入口只给尚未迁移的宿主保留过渡期。

```rust
cx.materialize_virtual_list_retained_in(
    list, &mut items, &layout,
    VirtualViewport::vertical(offset, height, overscan),
    &[], // 额外业务编辑会话；焦点与 Runtime IME 自动保留
    |index| model.key_at(index),
    |key| model.index_of_key(key),
    |index, _key| TextInput::new(model.value_at(index)),
)?;
```

滚动驱动的窗口用 `sync_virtual_list_retained_in(scroll, list, items, layout, overscan, fingerprint, …)`
（树/表是 `sync_virtual_tree_retained_in` / `sync_virtual_table_retained_in`）。它从
ScrollView 读当前 `ScrollOffset` 和视口。用 `window_for`（含 overscan）判断 range。
range、数据 fingerprint 与活动焦点/IME 状态都没变就不提交 Runtime mutation。仍走平移。
fingerprint 由应用按 key 序列、数据版本和 retained keys 计算。同长度同 extent 的 key 重排必须改变它。
range 跨过一行才挂新行、卸旧行。焦点或 IME 失效时会立即释放自动保留项。
在 `prepare` / 帧钩子里调用。不要绑在 `ScrollChanged` 上（会漏惯性、布局钳位、flush 后偏移）。
数据 key 序列、版本或 `retained_keys` 变化时更新 fingerprint 后继续调用 sync。行高或布局变化
先更新 `layout` 再物化。封面、事件、翻页继续放在 `on_mount` / 行差集里。框架不拉图。

每行要绑事件时用 `materialize_virtual_list_retained_with`（或 `sync_virtual_list_retained_with`）。
它多收一个 `on_mount` 回调：**只**为本次新建的行调用一次（滚回已挂载的行不会重复调用）。
在提交之后执行。可以直接 `cx.on(entity, ...)`。滚走释放的行连同 handler 一起释放。

行高由内容决定（换行、`line_clamp`、展开）时不要在应用里估算字宽：用
`VirtualListItems::measured()` 建 items。改调 `sync_virtual_list_measured_with(scroll, list, items,
&mut layout, …)`。行容器不再被钉成 `layout` 里的高度。而是按内容排版。每次同步先从上一次布局读回
已挂载行的真实高度写进 `layout`（`measure_anchored`）。视口顶上那一行保持不动。它上面的行量出来
更高时。滚动偏移跟着加上差值。而不是把内容往下推。`layout` 里的值只是还没量过的行的估算。
新挂上的行要等下一次布局后才量得到：`items.pending_measure()` 为真时再要一帧。
列表不必是整个滚动内容：两个 sync 都按上一次布局里列表相对 ScrollView 的位置扣掉它上方的内容（页头、内边距）。视口只有压到列表的那一截才算窗口。
「展开」这类入口只在文字真的被截断时出现：布局后 `cx.text_truncated(text)` 给出 `line_clamp`
（或高度）是否丢掉了行。不要按字数阈值猜。

项身份与内容按 key 保持。框架在组件外放置一个非命中容器以维护逻辑位置。
并管理 List 的完整内容高度。数据重排必须提供最新逆索引。删除、折叠或 key
不匹配会释放对应项。离屏导航先查询目标偏移并物化。布局发布后调用 `scroll_into_view(scroll, target, margin)`
（已在视口内的目标不动容器。只按最小距离滚动。虚拟化行仍需先物化才有布局盒）。
再将焦点移到目标项内的具体控件。业务持久化草稿和选择仍按 key 保存。
旧物化入口仅协调身份。不要与定位入口混用于同一个非空 `items`。

## Rust 冻结表格

`materialize_virtual_table_retained_in(table, items, layout, viewport, frozen, retained_cells,
row_key_at, row_index_of, column_key_at, column_index_of, build_row, build_cell)` 使用同一
`VirtualTableLayout` 和 key 数据模型。`frozen` 按 `[列数, 行数]` 排列。`retained_cells`
是额外保留的 `(行 key, 列 key)`。焦点及 Runtime IME 所在单元格自动保留。
框架管理表格内容尺寸、行列绝对位置、冻结变换和冻结区域顺序。未指定单元格背景时使用
Surface。应用维护最新两轴逆索引。数据变化调用 `materialize_virtual_table_retained_in`。
滚动用 `sync_virtual_table_retained_in(..., overscan, fingerprint, frozen, ...)`。正文与冻结 range、
fingerprint 和活动焦点/IME 状态都不变时不挂/卸单元格。冻结前缀
仍按当前偏移更新变换。不要再覆盖受管理的变换。
离屏导航先 `reveal_cell_with_frozen`。物化、发布布局、滚动。再聚焦目标内的控件。
列卸载会释放单元格的全部嵌套视图和订阅。持久选择和草稿仍由应用按业务 key 保存。

## 非目标

- 完整浏览器、Tauri、裸 `@vue/runtime-dom` 产物、把整窗 WebView 当产品 UI
- 第二套 Device / Queue、CPU 回读伪装零拷贝
- 不要让控件拿窗口句柄。不要在 UI 画完后把原生 WebView 盖在 Surface 上
- 以 crate 根控件表或 Vue 的 DOM facade 定义新的框架合同

应用内打开网页使用 `runtime::BrowserView` 的宿主原生内容例外。当前只有
macOS `WKWebView`。Windows/Linux 明确不可用。`nana-ui` 没有独立的
`browser` feature。

需要把网页当画面素材（直播挂件、叠加层）时用无头 `WebSurface`：
`RuntimeProgram::web_surface_requests` 声明页面，帧交给应用自己的 sink。它不在
UI 树里，也不是 WebView 产品壳。合同见 [GPU 参考](gpu.md#无头网页画面websurface)。


### 多文档布局缓存

`RetainedLayoutCache` 按 `DocumentId` 隔离布局框、测量结果、隔离容器位置和已解析
padding。同一 `UiWorld` 交替布局多个文档时。一个文档的全量布局不会清空其他文档。
独立调用 `RuntimeLayoutEngine` 的宿主在文档关闭后调用
`RetainedLayoutCache::remove_document(document)` 释放该文档缓存。对空文档布局也会释放缓存。
`AppContext` 在删除、停放或 detach 最后一个文档根节点时自动释放布局缓存。
停放仍保留控件状态。重新插入后重新建立布局缓存。


保留测量结果按节点失效：内容或样式影响布局时。该节点及真实依赖祖先的全部历史
约束失效。避免切回旧视口时恢复过期尺寸。每节点跨帧最多保留两组约束。超过容量只会
重新测量。单次布局内的测量缓存不受此限制。低层宿主删除节点后可调用
`RetainedLayoutCache::remove_node(document, id)` 立即释放条目。AppContext 的删除
事务自动完成这一步。其他节点缓存不受影响。


### 局部布局后的滚动指标

局部布局仅为实际重排节点及其祖先 ScrollView 发布滚动指标。重复目标合并。并保持
文档顺序。其他文档、已停放节点及无关子树不参与目标选择。全量布局仍执行完整发布。
`restore_scroll_anchor` 会将容器加入下一次布局工作。确保没有样式变化时也能在测量后
恢复锚点。受影响滚动容器的内容范围由 UiWorld 的惰性布局边界索引计算。首次查询构建子树索引。
普通几何写回只更新变更节点及祖先的最大边界。结构变化重建对应父级的子节点聚合。
索引不包含绘制阴影、滤镜或滚动偏移。保持既有布局滚动范围语义。删除节点同步释放条目。

### Workspace 借用已有控件作为区域

`WorkspaceRegionSlot::new` 表示 Workspace 管理的结构区域。使用 Generic 语义且自身不接收指针。
如果直接将已有控件作为区域表面。使用 `WorkspaceRegionSlot::borrowed`：Workspace 仅投影区域布局和表面样式。
控件继续持有当前输入及无障碍语义。重新装配不会恢复旧的 label、disabled 或 focusable 快照。
`DesktopShell` 对直接借入的 ScrollView 自动选择此合同。普通结构区域保留原行为。
创建 Workspace 后须调用 `AppContext::assemble_workspace` 建立区域轨道。再进行布局和命中索引构建。

### 隐藏层级中的无障碍语义

`UiWorld::project_accessibility` 与增量投影会保留连接可见控件所需的结构祖先。
`visibility:hidden` 容器输出中性的 Generic 节点。其自身标签、值、描述、原角色及
交互状态均不对外提供。显式 `visibility:visible` 的孩子继续拥有完整的父子链和语义。
隐藏叶节点及不生成布局盒的子树不进入投影。变回可见后恢复当前业务状态中的语义。

启用 `accesskit-tree` 的嵌入式宿主可使用 `AccessTreeProjector` 消费完整节点或
`AccessibilityDelta`。隐藏结构容器对应没有点击/聚焦操作的 GenericContainer。
孩子的操作与焦点保留。Workspace/Dock 的持久化格式不受这项投影变化影响。

原生 hosted 适配器额外持有一个稳定的 Window 无障碍根。挂载、替换或清空文档。
以及焦点进入普通控件时。都不会替换窗口提供者的根身份。Runtime 节点仍保留自己的
角色与稳定 ID。无控件聚焦时。原生焦点回退到窗口根。不借用隐藏的结构容器。
这层窗口包装不改变 `AccessTreeProjector` 的嵌入式投影合同。

原生适配器共享当前保留投影。平台激活或再次激活时按需构造已发布状态的完整快照。更新和激活
共用同一份已提交状态。进入原生事件回调前释放投影锁。`AccessTreeProjector::new`
也只建立保留状态。嵌入式宿主需要首次完整树时显式调用 `full_update()`。
纯标签、值、焦点和边界更新复用根及文本运行索引。焦点按稳定 ID 增量维护。
结构或文本输入角色变化仍进行必要的重新检查。
静态 `Text` 的可访问内容映射到 AccessKit `Label.value`。让原生 UIA 的 Name 属性
包含文本。普通控件仍分别使用 label 和 value。编辑控件的名称与输入值不会混合。

hosted 宿主在布局收敛后暂存无障碍变化。在成功呈现后、应用的 `presented` / 窗口
绑定回调之前发布。未呈现期间的一批变化保留增量。累积多批变化时。仅标记恢复后
需要当前文档的完整快照。避免中间删除或重挂载使批次拼接失真。空闲重试不会覆盖
已有变化。各窗口独立保留待发布状态。关闭窗口不会转而修改主窗口的队列。

编码失败后。自驱 surface 的 `HostedGpuContext` 消费者（`wgpu-interop`）先丢弃这一帧的
`FrameContext` 和引用 Surface 帧的 view。再调用 `discard_frame`（主窗口）或
`discard_surface_frame`（辅助窗口）。下次获取时仅重建受影响的 Surface。继续使用已有的
`GpuContext`。不提交失败帧。也不通知生产者提交成功。painter 画过的目标随帧的丢弃自动
回滚。标准 Runtime 宿主自动处理这条路径。

新建主窗口和辅助窗口的无障碍适配器先只提供稳定的 Window 根。不预先读取文档。
首次成功呈现后才发布内容。应用在挂载前已经调用 `flush`。也不会使未呈现控件提前
进入原生树。冷启动没有基础树。首批局部增量需要补一次当前文档快照。有效的显式
完整快照可直接使用。首次发布后的局部更新继续沿用增量合同。

`RuntimeProgramContext<Message>` 可直接克隆并移入后台任务。仅要求 `Message: Send`。
不要求消息实现 `Clone`。克隆共享宿主 GPU 资源、消息入口与任务队列。后台任务通过
`dispatch` 唤醒宿主并提交消息。业务状态仍由应用更新回调处理。

## 窗口服务

`RuntimeProgramContext::windows()` 提供线程安全的 `WindowService`。`window()` 提供当前窗口的 `WindowHandle`。窗口描述统一为 `WindowDescriptor`。只放一幅画面的窗口用 `content_aspect_ratio` 锁定客户区宽高比，框架负责拖边、最小尺寸与换比例，见 [内容宽高比](window.md#内容宽高比)。`ApplicationState::build` 在每扇窗口发布前执行。构建失败不显示窗口。窗口事件可在 `ApplicationState::window_event` 中观察。详见 [窗口](window.md#多窗口) 的完成语义、embedded 接口和迁移说明。

窗口关闭时。`RuntimeApplication` 先移除窗口文档。再调用 `ApplicationState::window_closed` 清理应用状态。最后调用 `window_event(Closed)`。关闭通知的观察者因此看到已完成清理的状态。创建失败回滚仍调用清理钩子。但不发送成功或关闭事件。
