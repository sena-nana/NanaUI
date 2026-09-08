# NanaUI

NanaUI 是一套 **保留式原生桌面 UI 运行时**。应用打开窗口、持有一份 WGPU 设备；框架在这棵界面树上做布局、命中和合成，并把结果画进宿主的 Surface。

按钮、文字、侧栏和实时画面（着色器、预览视口、离屏纹理）是同一棵树上的节点：同一套排版、裁剪、点击和 document order。没有浏览器内核，也没有盖在实时画面上的一层 HUD。

面向同时需要桌面壳层和同窗实时画面的应用。把现有网站嵌进窗口，不是它要解决的问题。

## 与其他框架的差别

桌面 UI 常见三条路。NanaUI 不走其中任何一条，差在合同，不差在口号。

| | 典型做法 | NanaUI |
| --- | --- | --- |
| **网页套壳**（Electron、Tauri、Wry） | 窗口里跑 Chromium / WebView；实时画面是另一个进程或另一套合成 | 无 WebView。Vue 若使用，是 Custom Renderer，写入同一棵原生树 |
| **游戏引擎 HUD** | 画面是主世界，界面浮在上面；裁剪、命中、桌面窗口语义是后加的 | 画面是树上的节点，和 Button 一样参与布局、裁剪和点击 |
| **即时模式 UI** | 每帧重建界面；嵌入容易，桌面壳层和增量布局不是一等公民 | 保留树。无变更不刷帧；应用不手写控件坐标 |
| **框架自管 GPU 的保留式 UI** | 框架持有 Device / 绘制后端；外部画面往往是「洞」或帧尾贴图 | 宿主持有 Window / Surface / Device / Queue。GPU 内容是 document order 里的节点 |

再精确一点：

**相对 Electron / Tauri。** 它们优化的是「用 Web 技术出桌面窗口」。NanaUI 将 Vue + JS 作为一等 L1/L2 消费入口：Vue 3 的一个子集经 Custom Renderer 落到 `UiWorld`，不是把 `@vue/runtime-dom` 产物丢进 WebView。没有 Tauri 的窗口 / 插件 / invoke 协议，也没有浏览器 CORS、Cookie、Service Worker。

**相对游戏引擎。** 引擎优化的是场景和相机。产品壳（标题栏、侧栏、设置、Dock、系统材质）是后加层。NanaUI 相反：壳是一等界面；实时画面由应用画到纹理，再作为普通节点挂上树。不要绕过界面树去直写窗口 Surface。

**相对即时模式 UI。** NanaUI 是保留式：`RuntimeDocument` 上的控件投影到 `UiWorld`，布局由运行时计算。控件事件走 `on` / `observe`；`RuntimeProgram::update` 只处理跨窗口、GPU、持久化这类宿主级消息，不是每一下点击的总线。

**相对框架自管 GPU 的保留式 UI。** 关键差别是所有权：NanaUI **不**再申请第二套 Device / Queue，也不把实时画面读回 CPU 再贴回去。`SceneWgpuPainter` 注入宿主已有的 GPU 上下文，在主 pass 里按节点顺序采样 `HostTexture`。现场直写用 `GpuView`。

## 运行模型

应用开发只需要记住这一条路径：

```text
应用状态
    │  build / on / 换纹理
    ▼
RuntimeDocument  （UiWorld：树、样式、布局、命中、焦点）
    │  flush
    ▼
UiScene          （与后端无关的绘制图）
    │  SceneWgpuPainter
    ▼
宿主 Surface     （你的 Window / Device / Queue）
```

- **你拥有：** 业务状态、配置存储、每个区域里放什么、这一帧实时画面画什么、窗口恢复信息。
- **框架拥有：** 控件语义、布局、命中、焦点 / IME、无障碍增量、把树画进你的 Surface。
- **窗口句柄** 不到达普通控件。系统材质走 `nana-window`；控件只发出「请关闭 / 最小化」这类语义。

完整说明见 [框架如何运行](docs/how-it-works.md)。

## 什么时候用

适合：桌面产品需要原生窗口质感，并且着色器、预览视口或其它实时画面必须和面板、对话框住在同一棵界面里。

不适合：把网站原样放进窗口；只要即时模式调试面板；要完整浏览器或 Tauri 插件生态。

### Rust 还是 Vue

Rust `nana_ui::runtime` 是默认作者面。Vue 是一等的 L1/L2 入口，不是过渡层——两者写同一棵 `UiWorld`，布局、文字、命中、绘制只有一份 Rust 实现，所以这个选择不影响绘制稳态。按**界面表面**切，不按团队语言偏好切：

- **Rust L3**：应用壳、标题栏、Workspace / Dock、多窗口；持有 `Entity<V>`、GPU 槽或宿主纹理的地方；大规模虚拟列表 / 表格 / 树。
- **Vue L1/L2**：已有 Web 界面的迁移；以及视觉迭代频繁的内容面（设置页、表单、面板）。Vue 改一行到看见约 200 ms，Rust 是重编译 + 重启（实测量级见 [开发期热重载](docs/hot-reload.md)）。

**指针密集不再是选型理由**：实测两条路的每个指针事件都是常数，不随树增长——Vue 约 0.062 ms，L3 约 0.0003 ms，绝对值都在 0.1 ms 以内。真正要避开的是写法——长列表里 hover 改动参与渲染的状态会重建整列。数据见 [输入成本](docs/input-cost.md)。

两条路可以在一个产品里共存，但**不要在同一棵子树里混写**：原语支持，产品合同不支持。

## 试运行

需要 Rust 1.92+。在仓库根目录：

```bash
# 控件与工作区
cargo run -p component-gallery

# 界面与实时画面同一窗口
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts

# GpuView 演示（非默认）
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

`nana-ui` 默认 feature 为空。要出窗口至少启用 `hosted`（含 gpu 与 winit）和 `bundled-fonts`。

写第一扇窗口：[开始](docs/start.md)。文档索引：[docs/README.md](docs/README.md)。

## 许可

MIT 或 Apache-2.0，见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。
