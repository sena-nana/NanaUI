# Android ARM64：无 WebView 的 Vue/V8 宿主

Android 仍是实验路径，不是 NanaUI 当前产品目标。该槽使用 Rust-owned
V8（桌面 smoke）、Nana Style Model，以及与桌面同类的 Runtime / UiScene /
`SceneWgpuPainter` 实验绘制（Activity 仍拥有窗口与 WGPU，不调用 winit
`run_runtime`）。不创建系统 WebView。

crates.io `v8` 150.4.0（rusty_v8 谱系）不为 `aarch64-linux-android` 发布预编译
静态库。本仓库用 GitHub Actions 从源码打包该 target（workflow `Package V8`），
产物命名与 rusty_v8 发布档案一致。

没有 `RUSTY_V8_ARCHIVE` 时，交叉编译脚本以 `--no-default-features` 构建宿主
（不链接 V8）。提供档案后脚本链接 V8 并构建完整 `nana-android-host`。桌面
`cargo test -p nana-android-host` 仍跑 V8 engine smoke。这不是完整 Android
产品，也不承诺 IME / AX / CJK。

## 边界

- Activity/宿主拥有窗口、Surface、Device 与 Queue；
- Vue/JS 只驱动 Custom Renderer 和语义树；
- `nana-ui-platform` 的 Fetch 类型在 Android 可编译，真实网络仍必须由应用创建
  `NativeFetchHost` 并显式授权 origin；
- 默认 `FetchPolicy` 为空白名单，不授权任何网络 origin；
- Android clipboard/IME/AX 尚无真实后端时必须继续报告 unsupported；
  NativeActivity KeyEvent 可进入 Runtime 文本，这不是软键盘 IME；
- 不引入 WebView、Tauri mobile 插件或第二套 GPU 上下文。

`ureq 3.x` 使用 rustls，不依赖桌面 WebView。阻塞 HTTP 始终在线程池执行，Promise
只在引擎线程由 `pump_frame` 结算。

## 交叉编译门禁

```bash
./scripts/check-android-arm64.sh
```

脚本负责检查 Android target/NDK 并构建无 WebView 的宿主。未设置
`RUSTY_V8_ARCHIVE` 时不链接 V8。交叉编译通过只证明依赖和 ABI 可构建，不等于真实
设备上的 Surface、输入、IME、TLS 信任库、网络权限或性能验收；这些仍需目标设备
证据。

V8 档案由手动 workflow `Package V8` 从源码打出（`librusty_v8_release_*.a.gz` +
`src_binding_release_*.rs`）。消费：

```bash
gh release download rusty-v8-v150.4.0 --dir dist/v8
source scripts/use-v8-prebuilt.sh dist/v8
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build
```
