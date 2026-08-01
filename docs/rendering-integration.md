# WGPU 集成边界

Issue #1 要求 NanaUI 能与 NanaShader Runtime 共享宿主的 WGPU 资源。标准 Gallery
验证 UI 工作区结构，`gpu-view-demo` 验证 Iced renderer 内的自定义 WGPU
primitive；`hosted-gpu-demo` 进一步验证宿主控制窗口、事件循环和唯一 WGPU
上下文。它们没有对 NanaShader 做任何改动，也不把等价场景验证冒充真实
NanaShader/Live2D 接入。

当前宿主 Demo 已按以下职责运行：

```text
winit Window / EventLoop（宿主）
          │
          └── wgpu Device / Queue / Surface（宿主唯一拥有）
                    ├── NanaShader Runtime
                    └── NanaUI 工作区与 GPU 内容区域
```

宿主负责窗口生命周期、Surface 配置和渲染时序；NanaUI 负责布局、输入、普通控件以及内容区域的逻辑/物理像素矩形。

`hosted-gpu-demo` 只调用一次 `Adapter::request_device`。直接 WGPU 场景使用宿主持有的 Device/Queue 创建管线、更新 uniform，并渲染到同时具有 `RENDER_ATTACHMENT` 与 `TEXTURE_BINDING` 用途的纹理；`GpuTextureView` 直接采样宿主 `TextureView`，`iced_wgpu::Engine` 接收同一 Device/Queue 的克隆句柄并合成到相同 Surface。场景刷新由 NanaUI 按钮消息驱动，事件循环使用 `ControlFlow::Wait`，没有第二套 Device、CPU 回读、图片编码或持续帧订阅。

交互式宿主通过 `HostedUiRenderer::push_window_event` 入队原生窗口事件并请求重绘；在 `RedrawRequested` 中先调用 `update`、处理产生的消息，再用最新应用状态调用 `render` 和 present。连续且相邻的鼠标移动只保留最新位置，但不会跨越按下、释放、触摸、键盘或窗口事件合并。Surface 仍由宿主配置；低延迟交互窗口推荐将 `desired_maximum_frame_latency` 设为 `1`。

`HostTexture` 以稳定 ID 和 generation 包装引用计数的 WGPU `TextureView`。宿主替换或 resize 纹理时递增 generation，NanaUI 只重建对应 bind group；未出现的纹理实例在帧末从 pipeline cache 清除。当前合同接收可过滤的二维 float 纹理，并使用预乘 Alpha 合成。

`GpuView` 的 `prepare` 直接取得 Iced WGPU renderer 当前的 `Device`、`Queue` 与 `Viewport`；每个实例按稳定 ID 缓存 uniform buffer/bind group。`Inline` 模式复用 Iced 当前的 RenderPass，`Standalone` 模式使用 Iced 同一帧的 CommandEncoder 与目标纹理创建独立 Pass，两者共享 RenderPipeline，但不会共享实例数据。它不创建中间纹理、不进行 CPU 回读或图片编码。未出现在下一帧的实例会从 pipeline cache 移除。

`RenderSlot` 是单个内容插槽的公共几何合同：逻辑边界按 scale factor 取 floor/ceil，确保物理 viewport/scissor 覆盖完整边缘，并可裁剪到目标纹理。`WorkspaceGeometry` 则为所有稳定 Region 输出布局快照。独立 Demo crate 的 `GalleryState::subscription` 只在 loading 或布局动画实际运行时创建定时订阅；`gpu-view-demo` 只在窗口、输入或状态变化时触发重绘。

当前已经覆盖复用现有 RenderPass 的简单内容路径、用同一 CommandEncoder 创建独立 Pass 的组合路径、宿主创建 `winit::Window`/`Surface`/`Device`/`Queue` 后注入 Iced renderer，以及宿主纹理直显。真实 Live2D/NanaShader 内容接入与同一复杂渲染图中的时序整合仍未完成，不能用等价渐变场景作为这些验收项的替代证据。

`hosted-gpu-demo` 通过 `nana-window` 接入 macOS Vibrancy 和 Windows Mica/Acrylic，并在无原生能力时切换为不透明主题背景；平台矩阵与限制见 `window-materials.md`。`transparent-window` 仍只使用 Iced 标准入口的透明/模糊设置，不作为原生材质验收证据。

Hosted runner 与标准 Iced 示例共享 `AppTitleBar`、`WindowChromeState` 和
`WindowChromeAction`。Runner 仍拥有 Winit Window，并直接执行拖拽、最小化、
最大化/还原和关闭；macOS 仅在空白父区域收到按下事件时通过 `nana-window` 的
无状态桥接启动原生拖拽，交互子控件会先消费事件。这一适配没有改变 Surface、
Device、Queue、纹理或同帧提交路径。
