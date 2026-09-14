# 消费方升级记录（2026-09-14）

本轮在 NanaUI 补齐 NanaLive #75 依赖、但属于框架的窗口与最新帧能力。编码、Sink、音频、转场仍在 NanaLive。`url()` 图片按文档 fetch host 分桶的修复已在 `7b374e470` 落地（NanaUI #75）。

冻结基线以各应用当时的 git pin 为准。NanaLive 本次一并迁移；LiliaBilibili、LiliaCode、NanaShader 等下次升 pin 时按下面适配。

## API 变化

### 窗口

- 新增 `DisplayId` / `DisplayInfo` / `WindowService::displays()` / `EmbeddedRuntime::displays`。
- `WindowCommand::SetFullscreen` 从 `fullscreen: bool` 改为 `fullscreen: Option<FullscreenRequest>`。删除 `SetSimpleFullscreen` 与 `WindowHandle::set_simple_fullscreen`。
- macOS 不切 Space：`FullscreenRequest { mode: FullscreenMode::Simple, display: None }`。
- 有效全屏、置顶、当前显示器通过 `WindowEvent::ModeChanged` 上报。`Ready` 后必有一次初始状态。
- `WindowDescriptor::fullscreen` 在窗口可见后应用；目标屏不存在时退回窗口模式。
- `WindowLevel` 下沉到 `nana_ui_platform`，`nana_ui` 再导出。
- Vue JS 接口不变。`windowGeometry().fullscreen` 与 `alwaysOnTop` 改为宿主观察值，不再在请求时乐观写入。

### 最新帧

- 新 crate `nana-frame-exchange`：生产端 `FrameExchange` / `FrameInbox` / `FrameLease`，只依赖 wgpu。
- `nana-ui`（`gpu` feature）再导出，并提供窗口侧 `FrameBinding`。
- 这是 GPU 内复制，不是零拷贝。UI 线程不得等待 GPU 完成；lease 必须等到 `window_frame_presented` 再释放。

## 各应用

| 应用 | 适配 |
| --- | --- |
| NanaLive | 本次迁移：升 pin、订阅 `ModeChanged` 更新 chrome、display worker 改用 `FrameExchange<(u64, u64)>`，预览改用 `FrameBinding` |
| LiliaBilibili | 下次升 pin：`crates/liliabilibili/src/app/presentation.rs` 与 `crates/bilibili-app/src/presentation.rs` 的 `SetFullscreen { fullscreen: bool }` 改为 `fullscreen: on.then(FullscreenRequest::default)`；`WindowEvent` match 增加 `ModeChanged`（或继续用 `_`） |
| 其他 | 若直接构造 `WindowDescriptor`，补 `fullscreen: None` 或改用 `Default`；`WindowCommand` 从 `nana_ui_platform::host` 导入 |

公开合同见 [窗口](window.md)、[实时画面](gpu.md)。
