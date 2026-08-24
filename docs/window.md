# 窗口

NanaUI 画的是桌面窗口：标题栏、图标、系统材质、多窗口都按桌面软件来，不按浏览器来。

`run_runtime(RuntimeWindowSettings::new("标题"))` 会创建主窗口、唯一 GPU 上下文，并开始事件循环。`RuntimeWindowSettings` 就是 `nana_ui_platform::WindowSettings`。

## 标题栏

默认自绘标题栏：左侧内容、中间标题、右侧窗口按钮。空白处拖动窗口；按钮先吃到指针，不会被拖走。

- macOS：透明标题栏 + full-size content，NanaUI 画 36px 标题栏，左侧给系统红黄绿留 78px。
- Windows / Linux：关掉系统 decorations，由 `AppTitleBar` 画最小化、最大化、关闭。

自绘 chrome 可拖窗口客户区最外 8px 缩放（四边与四角）。系统 caption、最大化、全屏、`resizable: false` 交给平台边框或禁用，不叠第二套命中。

没有自绘标题栏的窗口设 `WindowSettings::system_caption(true)`，避免 Windows 无框窗口失去关闭按钮。

关闭 / 最小化 / 最大化是窗口动作。控件发出语义（`WindowChromeAction`），Scene host 去执行。普通控件拿不到窗口句柄。

`WindowChromeState` 绑在明确的 `WindowId` 上。单窗口可用默认入口（只认收到的第一扇窗）；多窗口在 `WindowCommand::Open` 拿到 ID 后用 `for_window` 各建一份。关窗后不会自动接管别的窗口。

## 图标

任务栏、exe、Dock 上的图标是应用身份，不是界面里的 `Icon` 字形。

- Rust：`register_application_icon`，或 `WindowSettings::icon` / `WindowCommand::SetIcon`
- 未设置时用默认几何标记，不要把它当品牌
- Windows exe 可在 `build.rs` 里 `nana_app_icon::embed_windows()`
- macOS Dock：`nana_window::set_application_icon_png`；`.app` 用 `nana-package-app`

## 材质

通过 `RuntimeProgram::window_material_mode` 申请**一种**系统效果。失败回实色，并给出原因，不会改试另一种。

| 平台 | 可申请 | 失败时 |
| --- | --- | --- |
| macOS 10.10+ | 指定的 Vibrancy / UnderWindowBackground | 不透明主题背景 |
| Windows 11 | 指定的 Mica **或** Acrylic | 不透明主题背景 |
| Windows 10 1809+ | 指定的 Acrylic | 不透明主题背景 |
| Linux | 无系统模糊 API | 不透明主题背景 |

`Translucent` 只开窗口透明，不等于模糊。透明窗口和系统模糊是两件事。

当前 macOS 在 GPU 窗口上申请 Vibrancy 可能拿不到系统效果（金属层会盖住系统材质）。Windows 的透明客户区和 Mica / Acrylic 以真机为准。编译通过不等于那台机器上看起来对。对照 `crates/nana-ui/examples/transparent-window.rs`。

原生材质由 `nana-window` 执行：`apply_system_material` / `apply_hosted_system_material`。`run_runtime` 会给主窗口和每个工具窗口分别应用、刷新和清理。主题或材质切换会先清掉旧效果再按当前请求重试。设备恢复后按当前请求重新应用。

## 多窗口

`WindowCommand::Open { id, settings }` 再开工具窗或预览窗。它们共用同一份 Device / Queue；若走 Vue，也共用同一个 JS 引擎。关主窗口即退出；关辅助窗口只拆那一扇。

```rust
RuntimeProgramUpdate {
    redraw: RuntimeRedraw::All,
    window_commands: vec![WindowCommand::Open {
        id: TOOL,
        settings: WindowSettings {
            title: "Notes".into(),
            initial_size: (360.0, 180.0),
            minimum_size: (240.0, 120.0),
            role: WindowRole::Tool,
            parent: Some(WindowId::PRIMARY),
            system_caption: true,
            ..WindowSettings::new("Notes")
        },
    }],
    exit: false,
}
```

每扇窗一份 `RuntimeDocument`，用 `document` / `document_mut` 按 `WindowId` 交出。完整例子：`window-chrome-multi-window.rs`、`examples/runtime-host-fixture`。

窗口位置、最大化、上次开在哪块屏幕，由应用自己记。框架在创建窗口前按当前显示器工作区约束位置（原屏断开则主屏居中，DPI 变则按逻辑尺寸重算），但不替你选配置目录，也不写盘。
