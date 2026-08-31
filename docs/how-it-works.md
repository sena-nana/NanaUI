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

新应用实现 `RuntimeProgram`，调用 `run_runtime`。

```text
initialize     建 RuntimeDocument，build { child / on }
document()     按 WindowId 交出那棵树
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
4. 外部资源生产（若有）          → 同一 Device/Queue，提交在 Scene 采样之前
5. SceneWgpuPainter::paint       → 按文档顺序画进 Surface
6. queue.submit + present
7. window_frame_presented        → 现在才能丢掉上一帧的纹理
8. bind_window                   → 需要的话再填内容
```

无变更时 flush 是空转，宿主不应空刷。动画、实时 GPU、普通 UI 的唤醒是分开的：一块实时画面在动，不该迫使整棵 Runtime 全量更新。

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

没有 WebView 壳。`createApp()` 把 Vue 3 的 Custom Renderer 接到宿主；JavaScript 跑在嵌入的 V8 里。这是迁已有界面的路，不是新产品的默认写法。见 [Vue](vue.md)。应用内打开网页是另一件事，目前未实现，见 [应用内浏览器](gpu.md#应用内浏览器)。

## 不要做的

这些是合同，不是风格建议：

- 为界面再 `request_device` 一次，或让实时内容用另一套 Queue 提交却期望和 UI 对齐
- 把 GPU 画面读回 CPU、编码成图，再当图标贴回去
- 在 UI 画完之后再往 Surface 上盖一层实时画面
- 让控件拿窗口句柄去调系统 API
- 把整窗 WebView 当成 NanaUI 的壳，或在 UI 画完后把原生 WebView 盖在窗口上
- 把 crate 根上的旧控件再导出、或 `nana_ui::dock::*` 适配器，当成第二套产品 API（新代码从 `nana_ui::runtime` 进；产品 Dock 是 Runtime 的 `Dock` / `DockWorkspace`）
