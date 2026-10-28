# Android 实验宿主

Android（`platform/android`）是仓库内的**实验性 ARM64 宿主路径**，并非当前正式产品交付目标，也不是第二套产品渲染内核——请勿将其写入面向用户的跨平台承诺。

比它低一级的 HarmonyOS / OpenHarmony 编译档见 [OHOS](ohos.md)：只承诺能编译，没有仓库内宿主。

在 Android 宿主上，界面同样进入 `nana-ui-runtime`（`UiWorld`）与 `nana-ui-scene`（`UiScene`），并由同一个 `SceneWgpuPainter` 绘制；窗口与 Vulkan 图形设备由宿主的 Android `Activity` 持有（不调用桌面端的 `run_runtime`）。

## 窗口内容

整个 Surface 是一份 Runtime 文档，由 [移动宿主](mobile-host.md)（`nana-mobile-host`）持有，iOS 宿主也用同一层。`GameActivity` 把 Surface 铺到状态栏、导航栏和软键盘下面。文档根节点铺满主题背景色，按四边 inset（`WindowInsets` 的 `systemBars | displayCutout` 与 `ime`）留出内边距，横屏时左右两边的刘海和导航栏同样避让。Primary 之外的触摸交给系统。

窗口里挂的是两个宿主共用的测量页（`nana-mobile-probe`）。Android 在原型里没有接摄像头，打开「CPU 预览」后，宿主以 30 fps 生成 640×480 的帧并上传，用来测上传这条路径的开销。

清单的 `configChanges` 含 `uiMode|fontScale|density`，这些变化不重建 Activity，深浅色和字号随系统更新。

## 帧调度与生命周期

宿主循环按 `FrameDemand` 决定怎么等：

- 需要出帧时，每一轮都出帧，由 Fifo 交换链按 vsync 限速。
- 有截止时间时，`poll_events` 睡到截止时间。
- 空闲时，`poll_events(None)` 一直阻塞，直到有输入、生命周期事件，或视图任务经 `AndroidAppWaker` 唤醒。

不再有 16 ms 轮询和 500 ms 心跳帧。

原计划用 `AChoreographer_postFrameCallback64` 对齐 vsync。但 android-activity 0.6 的 `poll_events` 碰到 `ALooper_pollOnce` 返回的回调结果时，只记一条错误就忽略，每帧都会刷一条错误日志，所以改成现在这种交换链限速的方式。对齐 vsync 起点的延迟优化留到以后。

`TerminateWindow` 只丢掉 surface，文档、状态和 Device 都保留；下一次 `InitWindow` 用同一个 Device 接上新的 surface。

## Phase 2 平台合同（基于 `GameActivity`）

相较于早期的 `NativeActivity`，当前 Android 宿主基于 `android-activity` 的 `game-activity` feature 构建（`NanaActivity` 继承自 `GameActivity`，从而具备 `InputConnection` 与 `GameTextInput`）：

- **输入法（IME）**：组字更新映射为 `CompositionInput::Update`，提交映射为 `Commit`，删除上下文映射为 `DeleteSurrounding`；它们与触摸、按键事件一样，由文档的输入源盖上序列号后统一走 `AppContext::route_input`，不维护第二套 Android 文本状态机。`EditorInfo` 从当前聚焦的编辑器镜像 `password` 与 `multiline` 标志（`PlatformCapabilities::android_mvp().ime = true` 仅表示该通道已接通，不等同于桌面级完整验收）。
- **系统剪贴板**：通过 JNI 调用系统 `ClipboardManager` 实现 `ClipboardHost`，并接入 `default_shared_clipboard()` 的 Android 分支；文档的 `HostServices` 经 `read_shared_clipboard` / `write_shared_clipboard` 使用这一后端（`clipboard = true`）。
- **应用数据目录**：`app_data_dir` 在 Android 上直接返回沙箱内的 `Context.getFilesDir()`（忽略 `app_id` 参数）；由宿主自行调用 `FileStore::open` 后注入，框架不私自写盘。
- **无障碍（TalkBack）**：接入 `Click`、`Focus`、`SetValue`、`SetSelection` 动作投影后，`Button`、`Switch` 与 `TextInput` 可被激活（`TextInput` 的 `Click` 会请求焦点以拉起软键盘；滚动与虚拟列表投影留待后续阶段）。

## 交叉编译与 APK 打包

本机交叉编译与桌面端运行的 `nana-android-host --lib` 单元测试仅验证 Rust 侧的动作队列、输入/IME 映射与无障碍投影合同，**不能替代真机上的中日韩输入法候选框、系统剪贴板或 TalkBack 触摸浏览验收**。

```bash
# 1. 仅做 ARM64 交叉编译检查：依次 check nana-ui-core、nana-ui-platform、nana-ui --lib、nana-mobile-host 与 nana-android-host
./scripts/check-android-arm64.sh

# 2. 构建 dist 档并打包可安装的 APK（需配置 Android NDK 与 GameActivity 4.4.0 Gradle 包装，勿开启 prefab）
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
```

构建注意事项：
- 打包 APK 必须使用 `--dist` 配置：`dev` 构建会在 `.so` 中内嵌约 `390 MB` 的 DWARF 调试信息与 `46 MB` 符号表，而 `package-android-host-apk.sh` 对 stripped `.so` 设有 `60 MiB` 体积上限。APK 内的 `.so` 采用未压缩方式存储并配合 `extractNativeLibs="false"`，安装后不再解压出第二份副本。更多工程细节见 `platform/android/README.md`。

## 真机验收清单

在真机安装 APK 后需逐项核对以下行为。2026-09-16 曾在 Galaxy Z Fold4 Android 16 上通过前四项（见 #83），当时挂的还是改用移动宿主之前的外观页；换成测量页后需要重新验收：

| 验收项 | 期望行为 | 证据要求 |
| --- | --- | --- |
| **软键盘唤起与收起** | 点击搜索框弹出软键盘，失焦后自动收起；页面底边随 IME inset 抬升至键盘上方 | 真机或模拟器 |
| **系统栏避让** | 页面顶端在状态栏下方，底端在导航栏上方，横屏时左右避让；系统栏下面是主题背景色 | 真机，含刘海 / 挖孔屏 |
| **后台与恢复** | 切到后台再回来，搜索框里的文字、开关和计数都还在 | 真机或模拟器 |
| **静止不出帧** | 停止操作后不再出帧（`adb shell dumpsys gfxinfo` 的帧数不再增长） | 真机 |
| **深浅色与字号** | 跟随系统深色模式与系统字号 | 真机 |
| **CJK 输入法候选** | 拼音/笔画候选词选定后正确写入 Runtime `TextInput`（宿主暂未提供 `CursorAnchorInfo`，候选条停留在键盘顶端） | **必须使用真机输入法**，不得用 `adb shell input keyevent` 替代 |
| **系统剪贴板互通** | `TextInput` 内复制/粘贴与系统 `ClipboardManager` 互通 | 真机 |
| **TalkBack 读屏交互** | `Button` 激活计数、`Switch` 状态切换、`TextInput` 聚焦并进入编辑 | **必须由真人在真机上做 TalkBack 手势**（`adb` 注入的 tap 会绕过无障碍触摸浏览） |
| **触摸滚动** | 网格按 `ScrollPhysics::Android` 甩动，到边缘停住，不越界 | 真机 |
| **滚动与虚拟列表的无障碍** | 尚未接入 Android 侧滚动无障碍动作 | 留待后续阶段 |
