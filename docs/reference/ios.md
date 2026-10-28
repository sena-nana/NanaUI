# iOS 实验宿主

iOS 宿主（`platform/ios`）是一条**实验性**宿主路径，和 [Android](android.md) 一样不是产品交付目标。它和 Android 共用 [移动宿主](mobile-host.md)那一层。

## 结构

| 路径 | 作用 |
| --- | --- |
| `platform/ios/src/lib.rs` | `nana-ios-host`：rlib，提供宿主的 C ABI（生命周期、触摸、帧、文本代理、无障碍），内部包着 `MobileRuntime` 和 `MobileRenderer` |
| `platform/ios/include/nana_ios_host.h` | 通用 C 头文件。所有调用都在主线程；几何默认是物理像素，带 logical 字样的是逻辑像素；文本偏移是 UTF-8 字节 |
| `platform/ios/Shell/NanaView.swift` | 以 `CAMetalLayer` 为底层的视图：`CADisplayLink` 按 `FrameDemand` 暂停和恢复（ProMotion 最高 120 Hz）；触摸带合并采样；从 trait collection 读深浅色和 Dynamic Type；进入后台时 detach；触感反馈；VoiceOver 容器。`afterTick`、`onTick`、`onIdle` 留给 App 排布原生层和统计帧数据 |
| `platform/ios/Shell/NanaTextProxy.swift` | 混合层①：`UITextInput` 文本代理，加 `UITextInteraction` |
| `platform/ios/probe/` | `nana-ios-probe`：测量页的 staticlib，用 `export_ios_app!` 把测量页放进宿主，自带 `nana_probe_*` 调用（`include/nana_ios_probe.h`） |
| `platform/ios/App/NanaProbe.xcodeproj` | 测量页的 Swift 外壳。「Build Rust host」构建阶段调用 `build-rust.sh`，按平台和配置构建 staticlib（Debug 用 `dev`，Release 用 `dist`） |
| `App/NanaProbe/ProbeViewController.swift`、`ProbeCamera.swift` | 混合层③：前置摄像头 640×480 的 `AVCaptureVideoPreviewLayer` 垫在透明表面下，按测量页预览区的位置排布；也可以切到 CPU 上传路径 |
| `App/NanaProbe/ProbeMetrics.swift` | 帧间隔和 tick CPU 时间的 P50/P99（每 240 帧记一次日志）、冷启动到首帧的时间，以及 Instruments 用的 signpost |

### 在宿主上做一个 App

宿主是 rlib，Xcode target 只链接 App 自己那一个 staticlib：

1. App 的 crate 设为 `crate-type = ["staticlib", "rlib"]`，依赖 `nana-ios-host` 和 `nana-mobile-host`。
2. 实现 `IosApp`：
   - `mount` 挂上 App 的视图，传进来的 `Shell` 用来请求触感（`Haptic`）和唤醒；
   - `before_frame` 在每帧准备之前运行，例如更新宿主纹理。
3. 用 `export_ios_app!(MyApp::new())` 导出 `nana_ios_host_new`。Swift 外壳对 `nana_ios_host_*` 的调用不用改。
4. App 自己的 C 调用用 `with_app::<MyApp, _>(host, |parts| ...)` 拿到 App、`MobileRuntime` 和 `MobileRenderer`，声明在 App 自己的头文件里（头文件 include `nana_ios_host.h`）。
5. Xcode 工程：
   - 引用 `Shell/` 里的两个 Swift 文件；
   - 「Build Rust host」阶段设 `NANA_RUST_PACKAGE`、`NANA_RUST_LIB`，不在这个仓库里时再设 `NANA_CARGO_MANIFEST`；
   - `OTHER_LDFLAGS` 链接 `-l<库名>`。

### 无障碍

宿主把文档的无障碍树压平成 VoiceOver 会停留的节点：有对应角色的节点，以及带标签的节点。

- 外壳调 `nana_ios_host_accessibility_count` 取快照，再逐个读节点（角色、状态位、逻辑像素框、滑条数值）和文字（标签、值、提示）。
- 操作通过 `nana_ios_host_accessibility_action`，有激活、增减、聚焦。
- `NanaView` 把这些节点做成 `UIAccessibilityElement`；VoiceOver 打开时，节点数一变就发 `layoutChanged`。

## 文本代理

编辑器获得焦点后，宿主经回调通知外壳。外壳把代理视图盖在编辑器上，让代理成为第一响应者，并挂上 `UITextInteraction(for: .editable)`。从这时起，落在编辑器上的点按、长按和拖动都交给 UIKit 的文本手势。

| UIKit 调用 | 宿主调用 |
| --- | --- |
| `setMarkedText` / `unmarkText` / `insertText` / `deleteBackward` | `set_marked_text` / `commit_text` / `delete_surrounding` |
| 设置 `selectedTextRange` | `set_selection` |
| `caretRect(for:)` / `firstRect(for:)` / `selectionRects(for:)` | `text_caret_rect` |
| `closestPosition(to:)` | `text_offset_at` |

UIKit 按 UTF-16 偏移计数，代理负责和 UTF-8 字节偏移互相换算。回调在宿主的输入路由内部运行，不能同步回调宿主，外壳一律派发到主队列再处理。

已知限制：

- `UITextInteraction` 会画自己的光标和选区高亮，NanaUI 编辑器的光标同时也在画，所以会出现两个光标。原型没有处理这一点。
- `selectionRects` 只按单行计算。

## 构建

```bash
# 交叉检查（真机目标；--sim 换成模拟器目标，需要先 rustup target add aarch64-apple-ios-sim）
./scripts/check-ios-arm64.sh

# 不签名构建一次，确认工程能编过、能链接
xcodebuild -project platform/ios/App/NanaProbe.xcodeproj -target NanaProbe \
  -configuration Release -sdk iphoneos CODE_SIGNING_ALLOWED=NO build
```

装到真机时，在 Xcode 里给 NanaProbe target 选择 Development Team，然后用 Release 配置运行。Debug 的 staticlib 带完整 DWARF，有几百 MB，不能拿来测体积和性能。

外壳走 UIScene 生命周期（`SceneDelegate` 建窗口，进后台看 `UIScene` 的通知）：iOS 27 SDK 编出的 App 不采用场景生命周期就直接启动失败。

### 模拟器

```bash
# rustup target 装在构建时用的那个工具链上（NanaUI 用 stable）
rustup +stable target add aarch64-apple-ios-sim
xcodebuild -project platform/ios/App/NanaProbe.xcodeproj -target NanaProbe \
  -configuration Debug -sdk iphonesimulator -arch arm64 CODE_SIGN_IDENTITY=- build
xcrun simctl install booted build/Debug-iphonesimulator/NanaProbe.app
# --console-pty 把宿主的 stderr（tick 失败、panic）接到终端
xcrun simctl launch --console-pty booted dev.nana.probe
```

模拟器上能验证功能：首帧、触摸滚动与甩动、开关与滑条、拼音组字与候选上屏、退格、进后台再回来、深浅色切换，以及空闲时不出帧。它不能验证的是：

- **相机**：模拟器没有摄像头，预览层停在黑色底，CPU 预览收不到帧。
- **性能**：帧间隔和 CPU 数字不能作为证据。
- **GPU 能力**：模拟器的 Metal 只提供 Apple2 家族，wgpu 据此只给 15 个跨阶段变量（真机 A11 起是 31）。场景着色器按 15 的上限写，所以模拟器上也能跑；新增着色器输出时要守住这个上限。

## 验收

下面这些都要在真机上，和一个最小的 SwiftUI 对照 App（同样的网格，加 `AVCaptureVideoPreviewLayer`）背靠背比较。模拟器的数字不算证据。

| 项 | 达标线 | 怎么量 |
| --- | --- | --- |
| 冷启动到首帧 | ≤ 500 ms | `first frame` 日志（从进程启动算起） |
| 甩动网格时的帧间隔 | 120 Hz 下 P99 ≤ 8.3 ms | `frames=… interval p99` 日志，加 Instruments 的 Animation Hitches |
| 静止 | 10 秒内 CPU 约为 0，没有 tick | Xcode CPU gauge；日志里没有新的帧统计 |
| 首次效果的卡顿 | 首次进入页面、首次出现每种效果时，没有超过 16 ms 的帧 | signpost 中 `tick` 的耗时 |
| 预览功耗 | 原生预览层、NanaUI 垫底预览、CPU 上传三种方式，各开 10 分钟比较功耗和温度 | Xcode Energy gauge、Instruments Power Profiler |
| 体积 | 记录 IPA 增量 | Release 归档 |
| 手感 | 中文输入法组字、长按选区、放大镜、编辑菜单都可用；滑动和回弹与 SwiftUI 版并排对比 | 真人操作 |

网格的触摸拖动滚动走 runtime 的触摸滚动，iOS 宿主选 `ScrollPhysics::Ios`：每毫秒按 0.998 减速，拖过边缘有橡皮筋效果，松手后回弹。合同见 [runtime-scene.md](runtime-scene.md#触摸滚动与-scrollphysics)。手感上要和 SwiftUI 版并排比的是：甩动多远停下、回弹的力度，以及从按下到开始滚动的 8 逻辑像素 slop。
