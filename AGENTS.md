# NanaUI Agent 入口规范

## 仓库边界

- NanaUI 的产品保留/渲染合同是 Runtime（`nana-ui-runtime`）与 UiScene
  （`nana-ui-scene`）。产品路径：`UiWorld` → `ExtractedNode` → `UiScene` →
  `SceneWgpuPainter`。宿主拥有 Window、Surface、Device 与 Queue。Vue + JS
  是一等 L1/L2 消费方；不要把 WebView 当作产品 UI 路径。消费应用业务仍在
  框架外。
- `crates/nana-ui-core` 持有主题令牌、Style Model、`WorkspaceModel` 和几何。
  `crates/nana-ui-runtime` 持有保留树、内建控件、Shell、Workspace、Dock 和
  GPU 槽（`CustomRenderNode` / `gpu_slots`）。`crates/nana-ui` 是宿主适配器：
  `run_runtime`、crate-root 再导出、`SceneWgpuPainter`。
- `crates/nana-window` 负责系统材质以及标题栏拖拽 / 客户区 chrome / 缩放桥；
  普通控件不得访问窗口句柄。叠加标题栏仍走 `AppTitleBar` 透明模式与布局槽；
  全屏由消费方关闭标题栏拖动和窗口按钮，保留业务槽内容。
  Windows 窗口按钮使用框架统一圆角图标按钮样式与实际按钮命中区域。
- 消费应用拥有业务状态、配置存储和 Region 内容；NanaUI 只提供通用状态与合同。
- 非模态任务浮层使用 `OverlayHost` + `Panel`，共享 Runtime 关闭生命周期与焦点恢复；业务导航、固定策略与视口预留由消费应用持有，不借用模态 Dialog/Menu 的语义。
- 宿主拥有 Window、Surface、Device 与 Queue。`SceneWgpuPainter` 注入该 GPU
  上下文；禁止第二套 Device/Queue、正式路径 CPU 回读或伪零拷贝。GPU 内容是
  一等 Scene 节点（`CustomRenderNode`）：与 Button/Text 一样参与布局、裁剪、
  命中和 document order。`nana.host-texture` 在该节点的顺序位置采样，不得攒到
  帧尾。Live2D 仍在框架外，把 1..N 层映射为普通 HostTexture 节点；禁止写成
  Cubism 直写 Surface，也禁止引入 `Live2DNode`。
- Vue ECS 折入已落地（#6 为历史动机）：`gpu_slots` 权威是 Runtime
  `CustomRenderNode`，`event_flags` 权威是 `UiWorld` `EventListeners`，`attrs`
  仍是 DOM/CSS facade 且不复制树拓扑。host op 进 `PendingHostOps`，
  `flush_host_frame` 才 commit。`PendingHostOps` 镜像、`WidgetKind`、`ComponentSupport`
  都不是第二套实例化 ABI。`LayoutBoxStore` 增量投影，滚动不写回 Runtime
  `LayoutBox`。三个 facade 仍在；`nana-ui-web-api` 是 L1 兼容层，不是第四套绘制核。
  内建与插件控件走同一份 `ComponentRegistry` / `register_component`；Vue tag 与 L3
  `create_component` 都解析 `ComponentTypeId`。`NativeComponentRegistry` 是
  JS host 原生组件路径，不是这条 Runtime ABI。
- 依赖版本以 `Cargo.toml`、`Cargo.lock` 和实际依赖图为准，并保持单一 WGPU
  主版本。
- 视觉改动以 LiliaUI 的真实源码、令牌、状态和快照为依据，不凭印象近似。

## 项目级 Skills

- `$nanaui-workspace-ui`：Workspace、布局、Sidebar、Settings、主题、控件和浮层。
- `$nanaui-gpu-integration`：GPU View、宿主纹理、渲染调度和依赖收敛。
- `$nanaui-window-materials`：原生窗口材质及回退。
- `$nanaui-validation`：功能、构建、快照、平台与性能验证。
- `$nanaui-agent-debug`：无头截图 / a11y / 点击。调试产品 Vue 或 Runtime
  应用时直接调用 `nana-agent-session`、`VueAgentSession` 或
  `RuntimeAgentSession`，不要靠日志猜界面。

## 硬约束

- 修复根因，优先最小且清晰的设计，禁止补丁式修复和无价值抽象。
- 禁止在 UI 显示技术说明、路线图占位、Agent 文案或未接入的操作。
- 只为功能变化添加行为测试；不硬匹配日志、固定文案或私有实现。
- 不覆盖已有修改；在当前分支工作，不创建临时工作树或默认创建 PR，用户确认后
  再提交。
- 按 `$nanaui-validation` 选择与改动相称的验证；编译通过不能替代真实 GPU、
  消费者或目标平台证据。
