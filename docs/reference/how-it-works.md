# 框架如何运行

这篇是 NanaUI 的心智模型。读完之后，你应能判断三件事：状态放哪，一帧里谁在做事，实时画面怎么进树。

函数签名见 [应用 API](application-api.md)。实现细节见 [Runtime 与 Scene](runtime-scene.md)。

## 三个事实

1. **界面是一棵保留树。** `Text`、`Button`、侧栏、对话框、一块实时画面，都是节点。框架记住这棵树，做布局、命中、焦点和绘制。你不写控件的像素坐标。
2. **窗口和 GPU 是你的。** 你的应用拥有 Window、Surface、Device 和 Queue。`run_runtime` 代你做的那层宿主也一样。NanaUI 画进去。它不另开一套设备，也不把画面拷到 CPU 再贴回 GPU。
3. **实时画面是树上的成员。** 它不是洞，也不是覆盖层。它占据布局位置，会被裁切，会被挡住，也可以被点。合成顺序就是文档顺序。

## 谁拥有什么

```text
应用
  业务状态、配置、鉴权
  每个 Region / Dock pane 里放什么
  这一帧着色器 / 视口画成哪张纹理
  窗口恢复可由应用自管，或通过 `persist_key` 交给 NanaUI 的
  `ViewStateStore`（物理 backend 由宿主注入）

NanaUI
  控件语义与交互
  布局、滚动、命中、焦点、IME、无障碍增量
  把树抽成 UiScene，画进你的 Surface

nana-window
  系统材质（Vibrancy / Mica / Acrylic）和标题栏拖拽 / 客户区 chrome
  普通控件拿不到 HWND / NSWindow
```

`Workspace`、`Dock`、`Settings` 提供桌面壳的结构。结构是区域、分隔、折叠和 Tab。区域里的文档、资源列表和预览内容，仍是你的应用的。

## 你怎么接到这棵树上

普通 Rust 应用实现 `ApplicationState`，通过 `RuntimeApplication<State>` 调用 `run_runtime`。容器默认处理文档路由、GPU 注册表和窗口生命周期。

完整示例是 `crates/nana-ui/examples/application-counter.rs`。

嵌入式宿主和 Vue 适配继续实现低层的 `RuntimeProgram`：

```text
initialize     建 RuntimeDocument，build { child / on }
with_document / with_document_mut
               按 WindowId 在访问闭包中交出那棵树
update         只处理宿主级消息（开窗、换 GPU、持久化）
               按钮点击不要走这里，用 on / observe
host_textures / prepare_window_frame / window_frame_presented
               默认：把已有纹理挂上树，flush 前更新，present 后再释放
scene_gpu_renderers / scene_resource_producers
               高级：第一次接入可忽略，见 gpu.md
```

控件交互写成 `AppContext::on(button, |_, Activate, cx| { ... })`。

需要开窗或换纹理时，在闭包里 `cx.dispatch_program(Message)`。下一帧进入 `update`。

`update` 保持便宜。把页面内容填进树，放在 `bind_window`，也就是 present 之后。

完整程序见 [开始](start.md)。

## 一帧

`run_runtime` 内部（`run_runtime_scene`）对每扇需要重绘的窗口，大致走这些步：

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

没有变更时，flush 是空转。宿主不应空刷。

动画、实时 GPU 和普通 UI 的唤醒是分开的。一块实时画面在动，不该迫使整棵 Runtime 全量更新。

Motion 意图在 `nana-ui-core::motion`。同一条 track，加上同一个时间戳，CPU 可以确定性地求值。spring 和 decay 按绝对时间解析。它们不依赖上一帧的积分。

逻辑目标状态在 `UiWorld`。呈现值在 `PresentationStore` 的 overlay 里，按 node 加 property 按需查询。

Rust L3 用 `node.transition().opacity().transform().duration().ease()`、`Spring::to`，以及 `Timeline::parallel` 或 `sequence`，编译进这条 IR。

`width` 和 `height` 走 Layout-class CPU。不要改成 scale。

字体轴用 `.font_axis(*b"wdth", 125.0)`。也可以用 `Spring` 的 `.font_axis(tag)`，或 `Timeline` 里的 `FontAxis(tag)` track。它们和 CSS `font-variation-settings` 的 transition、`@keyframes` 编译成同一种逐轴 track。采样叠到计算样式的轴上。文字按 #88 的脏图重新 shaping。

L3 transition 结束后保持终点，直到该属性下一次被写成不同的值。

列表 FLIP 是显式的 presentation transform。写成 `node.flip(first, last)`，或 Vue TransitionGroup 的 `setPaintTransform`。逻辑框停在 Last。视觉从 Invert 回到 identity。

Compositor track 的 `MotionDescriptor` 在 start、retarget、cancel 时写入 generational slab。稳态只推进时间。

`SceneWgpuPainter` 把 descriptor 放到 storage buffer。Quad shader 的 `evaluate()` 复现同一套闭合解。非 Quad 的 primitive 仍用 CPU overlay。产品 present 不回读。

CPU 和 Layout 动画走 `next_animation_deadline`。`UiScene::compositor_needs_tick` 只把**该窗口**接到轻量的 `FrameDemand::Continuous` present。它不强制 Vue patch，不强制 `UiWorld` 的全局 schedule，也不强制 Style 或 Layout。

静态窗口保持 `OnDemand`。最小化或被遮挡时，compositor 不按 Hidden-GPU 追帧。恢复后按绝对时间求值。

device 或 surface 重建之后，宿主调用 `UiScene::set_surface_generation`。完成事件同样靠 deadline。不靠逐帧的 CPU sample。

默认执行类由 `AnimatableProperty::animation_class()` 决定。组件不能改：

| 属性 | 默认 Class | 执行路径 | fallback / 性能含义 |
| --- | --- | --- | --- |
| `transform` / `opacity` | Compositor | overlay；Quad 走 GPU `evaluate()`；非 Quad 走 CPU overlay | 稳态不写 `UiWorld`；非 Quad 仍是 CPU presentation |
| `clip` / `clip-path` | Compositor | overlay；shader 目前只绑 transform/opacity 的 `motion_id` | 稳态不写 `UiWorld`；clip 呈现仍 CPU overlay |
| `shader-parameter` | Compositor | overlay；需注册 typed codec | 无默认 Quad GPU 路径 |
| `color` / `background` / `blur` / `filter` / `shadow` | Paint | CPU 插值 | 可能每 sample 脏 paint/extract；不是 filter GPU |
| `width` / `height` / `padding` / `margin` | Layout | CPU layout | 每 sample layout；不要偷成 scale |
| `font-size` / `font-axis` | Layout | 非 compositor（排版 / 绘制也会受影响） | [#85](https://github.com/sena-nana/NanaUI/issues/85) 不强制 GPU |
| `display` | Discrete | snap | 不插值 |

`#8` 的 `animations_considered` 和 `animation_deadlines_scanned` 稀疏门禁仍然有效。compositor-only 的稳态结构门禁见 [`perf/README.md`](../../perf/README.md) 的 `compositor-steady`。

开发诊断走 `AnimatableProperty::diagnostic_hint()`。文档不硬编码整句。

`FrameDemand` 指定按需、截止时间，或持续刷新。

窗口被遮挡或最小化时，宿主仍按程序自己的 `FrameDemand` 调用 `prepare_window_frame`。这时不 flush，不获取 Surface，也不 present。

0 维仍然 prepare。producer 的 encode 只在尺寸可画时才跑。

纹理内容更新时，通过 `HostTextureRegistry::slot` 取得的 `TextureSlot`，通知引用该资源的窗口。

已落地的范围和性能证据见 [高刷新重构](../../archive/docs-notes/high-refresh-refactor.md)。

你的应用**不要**自己跑一套布局，也不要把控件坐标写进树。`flush` 会调宿主的文字整形（`NanaTextShaper`）和 `RuntimeLayoutEngine`。

对外身份是 `DocumentId` 和 `StableNodeId`，以及类型化的 `Entity<V>`。内部节点存储不是 API。

## 实时画面怎么成为节点

默认做法：画面画到可采样纹理，树上挂 `GpuTextureView`，用**同一字符串 slot** 在 `host_textures()` 登记。它和 Button 一样被布局、裁剪和命中。

多层就是相邻的几张 `GpuTextureView`。没有中间纹理时才用 `GpuView`。那是 `u64` slot，不是 registry 键。

`<video data-nana-video>` 走 `video:{id}` 这条 HostTexture。有槽时不叠 poster。

按图离屏见 [gpu.md](gpu.md#按图离屏)。换纹理时升 generation。不要拆节点。细则见 [实时画面](gpu.md)。

## Vue 是输入，不是另一套窗口

Rust 控件、Vue 的 HTML 1:1 控件、`nana-*` 组件，以及有限的 HTML/CSS 子集，写的是同一套样式模型。模型是 token、语义和布局。它们进同一棵 `UiWorld`。

CSS 只有一个引擎，就是 `nana-ui-css`。Vue 路径在运行时用它。L3 视图的 `<style>` 和 `css!` 在构建时用它，编译成 Style Model 数据。

```text
Rust  build / create_component ──┐
Vue   button / input / ul / table / nana-*  ─┼─► UiWorld ─► UiScene ─► SceneWgpuPainter
Vue   div + CSS 子集                         ─┘
```

没有整窗 WebView 壳。`createApp()` 把 Vue 3 的 Custom Renderer 接到宿主。JavaScript 跑在嵌入的 V8 里。

Vue + JS 与 Rust L3 共用 Runtime，也共用组件合同。见 [Vue](vue.md)。

应用内网页内容只能通过明确的 `runtime::BrowserView` 宿主例外接入。Runtime 节点负责布局、可访问性和生命周期。原生浏览器由宿主创建。当前只有 macOS 后端，且不进入离屏的 Runtime/Scene 截图。

## 不要做的

这些是合同，不是风格建议：

- 为界面再 `request_device` 一次，或让实时内容用另一套 Queue 提交，却期望和 UI 对齐
- 把 GPU 画面读回 CPU、编码成图，再当图标贴回去
- 在 UI 画完之后，再往 Surface 上盖一层实时画面
- 让控件拿窗口句柄去调系统 API
- 把整窗 WebView 当成 NanaUI 的壳，或在 UI 画完后把原生 WebView 盖在窗口上
- 把 crate 根上的旧控件再导出，或把 `nana_ui::dock::*` 适配器，当成第二套产品 API。新代码从 `nana_ui::runtime` 进。产品 Dock 是 Runtime 的 `Dock` 和 `DockWorkspace`
