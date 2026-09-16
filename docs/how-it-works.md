# 框架如何运行

这篇是 NanaUI 的心智模型。读完应能判断：状态放哪、一帧谁在做事、实时画面怎么进树。函数签名见 [应用 API](application-api.md)；实现细节见 [Runtime 与 Scene](runtime-scene.md)。

## 三个事实

1. **界面是一棵保留树。** `Text`、`Button`、侧栏、对话框、一块实时画面都是节点。框架记住这棵树，做布局、命中、焦点和绘制。你不写控件的像素坐标。
2. **窗口和 GPU 是你的。** 应用（或 `run_runtime` 代你做的那层宿主）拥有 Window、Surface、Device、Queue。NanaUI 画进去，不另开一套设备，也不把画面拷到 CPU 再贴回 GPU。
3. **实时画面是树上的成员，不是洞，也不是覆盖层。** 它占据布局位置，被裁切、被挡住、可以被点。合成顺序就是文档顺序。

## 谁拥有什么

```text
应用
  业务状态、配置、鉴权
  每个 Region / Dock pane 里放什么
  这一帧着色器 / 视口画成哪张纹理
  窗口恢复（位置、最大化、上次所在屏）

NanaUI
  控件语义与交互
  布局、滚动、命中、焦点、IME、无障碍增量
  把树抽成 UiScene，画进你的 Surface

nana-window
  系统材质（Vibrancy / Mica / Acrylic）和标题栏拖拽 / 客户区 chrome
  普通控件拿不到 HWND / NSWindow
```

`Workspace`、`Dock`、`Settings` 提供桌面壳的**结构**（区域、分隔、折叠、Tab）。区域里的文档、资源列表、预览内容仍是应用的。

## 你怎么接到这棵树上

普通 Rust 应用实现 `ApplicationState`，通过 `RuntimeApplication<State>` 调用
`run_runtime`；容器默认处理文档路由、GPU 注册表和窗口生命周期。
完整示例是 `crates/nana-ui/examples/application-counter.rs`。
嵌入式宿主和 Vue 适配继续实现低层 `RuntimeProgram`：

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

控件交互：`AppContext::on(button, |_, Activate, cx| { ... })`。需要开窗或换纹理时，在闭包里 `cx.dispatch_program(Message)`，下一帧进入 `update`。`update` 保持便宜；把页面内容填进树放在 `bind_window`（present 之后）。

完整程序见 [开始](start.md)。

## 一帧

`run_runtime` 内部（`run_runtime_scene`）每扇需要重绘的窗口大致走：

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

无变更时 flush 是空转，宿主不应空刷。动画、实时 GPU、普通 UI 的唤醒是分开的：一块实时画面在动，不该迫使整棵 Runtime 全量更新。Motion 意图在 `nana-ui-core::motion`：同一 track + 同一时间戳，CPU 可确定性求值；spring / decay 按绝对时间解析，不依赖上一帧积分。逻辑目标状态在 `UiWorld`，呈现值在 `PresentationStore` overlay，按 node + property 按需查询。Rust L3 用 `node.transition().opacity().transform().duration().ease()`、`Spring::to` 和 `Timeline::parallel` / `sequence` 编译进这条 IR。`width` / `height` 走 Layout-class CPU，不改成 scale。列表 FLIP 是显式 presentation transform：`node.flip(first, last)` 或 Vue TransitionGroup `setPaintTransform`，逻辑框停在 Last，视觉从 Invert 回到 identity。Compositor track 的 `MotionDescriptor` 在 start/retarget/cancel 时写入 generational slab，稳态只推进时间。`SceneWgpuPainter` 把 descriptor 放到 storage buffer，Quad shader `evaluate()` 复现同一闭合解；非 Quad primitive 仍用 CPU overlay。产品 present 不回读。

CPU/Layout 动画走 `next_animation_deadline`；`UiScene::compositor_needs_tick` 只把**该窗口**接到轻量 `FrameDemand::Continuous` present，不强制 Vue patch、UiWorld 全局 schedule 或 Style/Layout。静态窗口保持 `OnDemand`。最小化/遮挡时 compositor 不 Hidden-GPU 追帧，恢复后按绝对时间求值。device/surface 重建后宿主调用 `UiScene::set_surface_generation`。完成事件同样靠 deadline，不靠逐帧 CPU sample。

默认执行类由 `AnimatableProperty::animation_class()` 决定，组件不能改：

| 属性 | 默认 Class | 执行路径 | fallback / 性能含义 |
| --- | --- | --- | --- |
| `transform` / `opacity` | Compositor | overlay；Quad 走 GPU `evaluate()`；非 Quad 走 CPU overlay | 稳态不写 `UiWorld`；非 Quad 仍是 CPU presentation |
| `clip` / `clip-path` | Compositor | overlay；shader 目前只绑 transform/opacity 的 `motion_id` | 稳态不写 `UiWorld`；clip 呈现仍 CPU overlay |
| `shader-parameter` | Compositor | overlay；需注册 typed codec | 无默认 Quad GPU 路径 |
| `color` / `background` / `blur` / `filter` / `shadow` | Paint | CPU 插值 | 可能每 sample 脏 paint/extract；不是 filter GPU |
| `width` / `height` / `padding` / `margin` | Layout | CPU layout | 每 sample layout；不要偷成 scale |
| `font-size` / `font-axis` | Layout | 非 compositor（排版 / 绘制也会受影响） | [#85](https://github.com/sena-nana/NanaUI/issues/85) 不强制 GPU |
| `display` | Discrete | snap | 不插值 |

`#8` 的 `animations_considered` / `animation_deadlines_scanned` 稀疏门禁仍有效。compositor-only 稳态结构门禁见 [`perf/README.md`](../perf/README.md) 的 `compositor-steady`。开发诊断走 `AnimatableProperty::diagnostic_hint()`，文档不硬编码整句。

`FrameDemand` 指定按需、截止时间或持续刷新。窗口被遮挡或最小化时宿主仍按
程序自己的 `FrameDemand` 调用 `prepare_window_frame`，不 flush、不获取 Surface、不 present。
0 维仍 prepare，producer encode 只在尺寸可画时跑。纹理内容更新通过
`HostTextureRegistry::slot` 取得的 `TextureSlot` 通知引用该资源的窗口。
已落地范围与性能证据见 [高刷新重构](high-refresh-refactor.md)。

应用**不要**自己跑一套布局或把控件坐标写进树。`flush` 会调宿主文字整形（`NanaTextShaper`）和 `RuntimeLayoutEngine`。

对外身份是 `DocumentId` / `StableNodeId`（以及类型化的 `Entity<V>`）。内部节点存储不是 API。

## 实时画面怎么成为节点

默认：画面画到可采样纹理，树上挂 `GpuTextureView`，用**同一字符串 slot** 在 `host_textures()` 登记。和 Button 一样被布局、裁剪、命中。

多层就是相邻的几张 `GpuTextureView`。没有中间纹理才用 `GpuView`（`u64` slot，不是 registry 键）。`<video data-nana-video>` 走 `video:{id}` HostTexture，有槽时不叠 poster。按图离屏见 [gpu.md](gpu.md#按图离屏)。换纹理升 generation，不要拆节点。细则见 [实时画面](gpu.md)。

## Vue 是输入，不是另一套窗口

Rust 控件、Vue 的 HTML 1:1 控件 / `nana-*` 组件、以及有限的 HTML/CSS 子集，写的是**同一套样式模型**（token + 语义 + 布局），进**同一棵** `UiWorld`。

```text
Rust  build / create_component ──┐
Vue   button / input / ul / table / nana-*  ─┼─► UiWorld ─► UiScene ─► SceneWgpuPainter
Vue   div + CSS 子集                         ─┘
```

没有 WebView 壳。`createApp()` 把 Vue 3 的 Custom Renderer 接到宿主；JavaScript 跑在嵌入的 V8 里。Vue + JS 与 Rust L3 共用 Runtime 和组件合同。见 [Vue](vue.md)。应用内打开网页是另一件事，目前未实现，见 [应用内浏览器](gpu.md#应用内浏览器)。

## 不要做的

这些是合同，不是风格建议：

- 为界面再 `request_device` 一次，或让实时内容用另一套 Queue 提交却期望和 UI 对齐
- 把 GPU 画面读回 CPU、编码成图，再当图标贴回去
- 在 UI 画完之后再往 Surface 上盖一层实时画面
- 让控件拿窗口句柄去调系统 API
- 把整窗 WebView 当成 NanaUI 的壳，或在 UI 画完后把原生 WebView 盖在窗口上
- 把 crate 根上的旧控件再导出、或 `nana_ui::dock::*` 适配器，当成第二套产品 API（新代码从 `nana_ui::runtime` 进；产品 Dock 是 Runtime 的 `Dock` / `DockWorkspace`）
