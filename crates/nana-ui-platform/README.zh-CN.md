# nana-ui-platform

[English](README.md) · 简体中文

NanaUI 与宿主平台之间不含原生句柄的那一层：窗口合同、路径拼写、应用目录，以及剪贴板、HTTP(S)、WebSocket 和文件持久化这些宿主服务。

`WindowDescriptor`、`WindowEvent`、`WindowId` 描述开窗申请和结果，只有数据。`path` 模块（应用里是 `nana_ui::path`）负责共享路径拼写：`normalize_path` 不碰磁盘，`canonicalize_path` 只在 I/O 线程调用，`display_path` 生成的文字只用于显示，不能写回设置或用来打开文件。`ApplicationPaths::resolve` 把数据、配置、缓存、日志等目录映射到当前平台。它还原样再导出 [`nana-ui-input`](../nana-ui-input) 的输入合同。

原生窗口、系统材质和标题栏的执行在 [`nana-window`](../nana-window)，输入路由在 `nana-ui-runtime`。

## Features

- `clipboard`（默认开启）：系统剪贴板，桌面上基于 `arboard`。
- `fetch`（可选）：`FetchHost` 和基于 `ureq` 的 `NativeFetchHost`。默认不放行任何源，由应用的 `FetchPolicy` 决定。
- `ws`：`WebSocketHost` 和基于 `tungstenite` 的 `NativeWebSocketHost`。
- `experimental-android`：实验性 Android 宿主的平台能力。

## 测试

```bash
cargo test -p nana-ui-platform
cargo test -p nana-ui-platform --features ws
```

其余组合见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。真实剪贴板、网络和原生窗口行为要在目标平台上验证。

更多内容见 [窗口参考](../../docs/reference/window.md) 和 [窗口与平台](../../docs/learn/window-platform.md)。
