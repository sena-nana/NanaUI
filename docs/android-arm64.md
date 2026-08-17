# Android ARM64：无 WebView 的 Vue/QuickJS 宿主

Android 仍是实验路径，不是 NanaUI 当前产品目标。该槽继续使用 Rust-owned
QuickJS、Nana Style Model 与 Iced/WGPU 实验 slot，不创建系统 WebView，也不把
桌面 `run_runtime` / `SceneWgpuPainter` 产品环搬到 Android。V8 不是 Android
必需依赖；平台 MVP 以 QuickJS 交叉编译为门禁。

## 边界

- Activity/宿主拥有窗口、Surface、Device 与 Queue；
- Vue/JS 只驱动 Custom Renderer 和语义树；
- `nana-ui-platform` 的 Fetch 类型在 Android 可编译，真实网络仍必须由应用创建
  `NativeFetchHost` 并显式授权 origin；
- 默认 `FetchPolicy` 为空白名单，不授权任何网络 origin；
- Android clipboard/IME 尚无真实后端时必须继续报告 unsupported；
- 不引入 WebView、Tauri mobile 插件或第二套 GPU 上下文。

`ureq 3.x` 使用 rustls，不依赖桌面 WebView。阻塞 HTTP 始终在线程池执行，Promise
只在 QuickJS/引擎线程由 `pump_frame` 结算。

## 交叉编译门禁

```bash
./scripts/check-android-arm64.sh
```

脚本负责检查 Android target/NDK 并构建无 WebView 的 QuickJS 宿主。交叉编译通过只
证明依赖和 ABI 可构建，不等于真实设备上的 Surface、输入、IME、TLS 信任库、网络
权限或性能验收；这些仍需目标设备证据。
