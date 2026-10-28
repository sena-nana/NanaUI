# iOS 实验宿主

iOS 宿主（`platform/ios`）是一条**实验性**宿主路径，和 [Android](android.md) 一样不是产品交付目标。它和 Android 共用 [移动宿主](mobile-host.md)那一层。

## 结构

| 路径 | 作用 |
| --- | --- |
| `platform/ios/src/lib.rs` | `nana-ios-host`：staticlib，对外提供 C ABI，内部包着 `MobileRuntime` + `MobileRenderer` + 测量页 |
| `platform/ios/include/nana_ios_host.h` | C 头文件。所有调用都在主线程；几何默认是物理像素，带 logical 字样的是逻辑像素；文本偏移是 UTF-8 字节 |
| `platform/ios/App/NanaProbe.xcodeproj` | 最小 Swift 外壳。「Build Rust host」构建阶段调用 `build-rust.sh`，按平台和配置构建 staticlib（Debug 用 `dev`，Release 用 `dist`） |
| `App/NanaProbe/NanaView.swift` | 以 `CAMetalLayer` 为底层的视图：`CADisplayLink` 按 `FrameDemand` 暂停和恢复（ProMotion 最高 120 Hz）；触摸带合并采样；从 trait collection 读深浅色和 Dynamic Type；进入后台时 detach |
| `App/NanaProbe/NanaTextProxy.swift` | 混合层①：`UITextInput` 文本代理，加 `UITextInteraction` |
| `App/NanaProbe/ProbeViewController.swift`、`ProbeCamera.swift` | 混合层③：前置摄像头 640×480 的 `AVCaptureVideoPreviewLayer` 垫在透明表面下，按测量页预览区的位置排布；也可以切到 CPU 上传路径 |
| `App/NanaProbe/ProbeMetrics.swift` | 帧间隔和 tick CPU 时间的 P50/P99（每 240 帧记一次日志）、冷启动到首帧的时间，以及 Instruments 用的 signpost |

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

网格的触摸拖动滚动依赖 runtime 的触摸滚动（`ScrollPhysics`）。这项改动单独进行，合入之前网格只能用滚动条滚动。
