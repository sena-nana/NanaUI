# NanaUI Agent 入口规范

## 仓库边界

- NanaUI 的产品保留/渲染合同是 Runtime（`nana-ui-runtime`）与 UiScene
  （`nana-ui-scene`）。产品路径：`UiWorld` → `ExtractedNode` → `UiScene` →
  `SceneWgpuPainter`。宿主拥有 Window、Surface、Device 与 Queue。仓内
  `engine/iced` 与 `engine/gpui-scenario-bench` 已从仓库移除，不是 `nana-*`
  编译依赖，不是应用编程模型，也不是当前绘制后端。Vue + JS 是一等 L1/L2
  消费方；不要把 WebView 当作产品 UI 路径。消费应用业务仍在框架外。
- `crates/nana-ui` 负责主题、控件、Shell、Workspace 和 GPU 内容插槽；
  `crates/nana-window` 独立负责平台窗口材质，普通控件不得访问窗口句柄。
- 消费应用拥有业务状态、配置存储和 Region 内容；NanaUI 只提供通用状态与合同。
- 宿主拥有 Window、Surface、Device 与 Queue。`SceneWgpuPainter` 注入该 GPU
  上下文；禁止第二套 Device/Queue、正式路径 CPU 回读或伪零拷贝。Live2D 仍走
  HostTexture 槽位，禁止写成已直通 Scene pass。
- Vue ECS 折入已落地（#6 为历史动机）：`gpu_slots` 权威是 Runtime
  `CustomRenderNode`，`event_flags` 权威是 `UiWorld` `EventListeners`，`attrs`
  仍是 DOM/CSS facade 且不复制树拓扑。host op 进 `PendingHostOps`，
  `flush_host_frame` 才 commit。`LayoutBoxStore` 增量投影，滚动不写回 Runtime
  `LayoutBox`。三个 facade 仍在。插件 ABI 仍未做，不阻塞产品路径。
- 依赖版本以 `Cargo.toml`、`Cargo.lock` 和实际依赖图为准，并保持单一 WGPU
  主版本。
- 视觉改动以 LiliaUI 的真实源码、令牌、状态和快照为依据，不凭印象近似。

## 项目级 Skills

- `$nanaui-workspace-ui`：Workspace、布局、Sidebar、Settings、主题、控件和浮层。
- `$nanaui-gpu-integration`：GPU View、宿主纹理、渲染调度和依赖收敛。
- `$nanaui-window-materials`：原生窗口材质及回退。
- `$nanaui-validation`：功能、构建、快照、平台与性能验证。

## 硬约束

- 修复根因，优先最小且清晰的设计，禁止补丁式修复和无价值抽象。
- 禁止在 UI 显示技术说明、路线图占位、Agent 文案或未接入的操作。
- 只为功能变化添加行为测试；不硬匹配日志、固定文案或私有实现。
- 不覆盖已有修改；在当前分支工作，不创建临时工作树或默认创建 PR，用户确认后
  再提交。
- 按 `$nanaui-validation` 选择与改动相称的验证；编译通过不能替代真实 GPU、
  消费者或目标平台证据。
