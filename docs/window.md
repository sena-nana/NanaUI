# 窗口

Windows 自绘窗口按钮采用 NanaUI 图标按钮外观：28×28、控件圆角、垂直居中；按钮间距 2px，按钮组左右各留 6px。普通按钮使用常规悬停/按下颜色，关闭按钮使用危险色。按钮的实际区域处理窗口操作，周围留白不扩展为系统式整块按钮；macOS 原生红黄绿按钮保持平台行为。

NanaUI 画的是桌面窗口：标题栏、图标、系统材质、多窗口都按桌面软件来，不按浏览器来。

`run_runtime(RuntimeWindowSettings::new("标题"))` 会创建主窗口、唯一 GPU 上下文，并开始事件循环。`RuntimeWindowSettings` 就是 `nana_ui_platform::WindowSettings`。

## 标题栏

默认自绘标题栏：左侧内容、中间标题、右侧窗口按钮。空白处拖动窗口；按钮先吃到指针，不会被拖走。

叠加在舞台上的标题栏仍使用 `AppTitleBar`，通过 `leading / center / trailing / controls` 布局槽承载文字和操作，并调用 `assemble_app_title_bar`。`transparent(true)` 仅移除栏背景，保留内容与命中，不需要开启整窗透明。`drag_enabled(false)` 禁止栏内空白与文字启动拖窗，并在后续输入时取消已按下但未完成的手势；隐藏或卸载标题栏也会取消。默认分别为 `false`、`true`。全屏保留业务入口时设置 `drag_enabled(false)` 和 `show_window_controls(false)`；不要另外添加顶边坐标拖动逻辑。语义入口支持 `transparent` 和 `drag-enabled`。标题为空且没有 center 内容时不保留中间占位，右侧按内容宽度保留空间，左侧使用剩余宽度并可收缩；有标题或 center 时保持左右对称布局。

- macOS：透明标题栏 + full-size content，NanaUI 画 36px 标题栏，左侧给系统红黄绿留 78px。系统默认只把红黄绿放在标准标题栏高度内居中，`prepare_client_chrome` 会按标题栏高度平移按钮容器，使其在 36px 内居中。
- Windows / Linux：关掉系统 decorations，由 `AppTitleBar` 画最小化、最大化、关闭，控件组贴标题栏右缘。

自绘 chrome 可拖窗口客户区最外 8px 缩放（四边与四角）。系统 caption、最大化、全屏、`resizable: false` 交给平台边框或禁用，不叠第二套命中。

没有自绘标题栏的窗口设 `WindowSettings::system_caption(true)`，避免 Windows 无框窗口失去关闭按钮。

关闭 / 最小化 / 最大化是窗口动作。控件发出语义（`WindowChromeAction`），Scene host 去执行。普通控件拿不到窗口句柄。L1 CSS `-webkit-app-region` / `app-region` 不是拖拽合同：任意盒写 `drag` 也不会变成 caption。

`WindowChromeState` 绑在明确的 `WindowId` 上。单窗口可用默认入口（只认收到的第一扇窗）；多窗口在 `WindowCommand::Open` 拿到 ID 后用 `for_window` 各建一份。关窗后不会自动接管别的窗口。

### Windows 客户端绘制标题栏契约

Windows 上有两条互斥的 chrome 路径，由 `WindowSettings::system_caption` 与 `transparent` 决定，实现见 `windows_scene_chrome`：

| 设置 | 系统边框 | 阴影 / 圆角 | `WS_EX_NOREDIRECTIONBITMAP` |
| --- | --- | --- | --- |
| `system_caption: true` | 开（系统标题栏与缩放） | 系统默认 | 仅透明窗口打开 |
| `system_caption: false` 且不透明 | 关，由 `AppTitleBar` 画 Minimize / Maximize / Close | 圆角（Windows 11 DWM 阴影）；不用 winit `undecorated_shadow`（会把客户区顶边内缩 1px，标题栏盖不住） | 关 |
| `system_caption: false` 且 `transparent: true` | 关 | 无自绘阴影（避免 DWM 合成冲突） | 开 |

不透明自绘窗关掉 decorations 且不用 `undecorated_shadow`：winit 用 `WM_NCCALCSIZE` 把客户区铺到窗口外沿，创建后不再清 `WS_CAPTION`（`SetWindowPos(SWP_FRAMECHANGED)` 会在 `can_create_surfaces` 里卡住 UI 线程）。透明自绘窗仍清 caption，避免 DWM 合成留下系统边框。自定义标题栏按钮宽高均为 `WINDOW_CONTROL_WIDTH`，在高 `TITLE_BAR_HEIGHT` 的 controls 槽内垂直居中，按钮组两侧保留 `WINDOW_CONTROL_PADDING`。

命中顺序（逻辑像素，已含当前 `scale_factor`）：

1. 自绘窗口按钮（AccessKit 名称 `Minimize`、`Maximize`/`Restore`、`Close`）优先，不启动拖拽；客户区最外 8px 缩放区域与实际按钮相交时同样让位；按钮之外的边角继续支持窗口缩放。
2. 标题栏空白处按下后移动超过 4px 才发出 `WindowChromeAction::Drag`；Scene host 调用 `nana_window::drag_custom_title_bar`，失败再 `winit::drag_window`。
3. 无系统 caption、可缩放、未最大化、非全屏时，客户区最外 `RESIZE_HANDLE_SIZE`（8px）走 `LiveFrameResize`（macOS `setFrame`、Windows `SetWindowPos`），不进入系统嵌套 size-move 循环；系统 caption 窗口不叠第二套缩放命中。

### 实时缩放

客户区拖动边框时，指针移动直接改窗口矩形，事件循环继续跑，`SurfaceResized` 同步几何并请求下一帧。画帧时若物理尺寸或 present 策略变了才 `surface.configure`；同尺寸跳过。Windows 系统边框缩放仍可能走 `WM_ENTERSIZEMOVE`；`LiveSizeMove` 在那段时间用 `Mailbox`/`Immediate` present。透明窗口走同一条路径。

DPI 与多显示器：指针、拖拽与缩放都用逻辑坐标；物理像素只用于 Surface。窗口位置由宿主记录，创建前按当前显示器工作区 clamp（原屏断开则主屏居中）。模态辅助窗在 Windows 上 `with_owner_window` 绑定父 HWND。

IME：焦点进可编辑字段时 `Window::request_ime_update(Enable)` 一次（hint / purpose、caret 盒、非密码的 surrounding text）。之后 caret、purpose 或 surrounding 变化走 `Update`；能力集变了先 `Disable` 再 `Enable`；失焦 `Disable`。候选框相对 caret，不相对系统非客户区。AccessKit 增量更新与视觉几何同一套 layout box；composition 期间不得出现悬空 `parent_and_index`。

透明 Alpha（`settings.transparent`）强制 `MaterialEffect::Transparent`，不会改试 Mica / Acrylic。失败只能回不透明实色，并带 `MaterialFallback`。真机入口：`vue-hosted-acceptance --chrome-probe`、`--input-probe`、`--hybrid --windows`，以及 `nana-ui` 的 `transparent-window` 示例。

## 菜单栏

原生应用菜单栏是**唯一画不进界面树**的桌面 chrome：macOS 上它属于应用而不是窗口，住在系统菜单条里。所以它在 `nana-window`，用一份平台中立的模型描述（`MenuBar` / `Menu` / `MenuEntry` / `MenuShortcut`，模型本身在 `nana-ui-core`，纯数据）。

应用**声明**菜单，宿主**安装**它：发 `WindowCommand::SetMenuBar { id, bar }`。普通控件拿不到窗口句柄，而 Windows 的菜单属于窗口，所以安装必须由持有窗口的 Scene host 做——和 `SetIcon` 同一条路。

选中项通过 `take_menu_activations()` 回来：每帧 drain 一次，拿到的是 `MenuEntry::Item` 的 `id`。**框架只报告用户选了哪一项，这个 id 是什么意思仍由应用决定**，与 `SecondaryPress` 同一原则。参照 `examples/component-gallery`：菜单 id 被映射成和界面操作完全相同的业务消息。

平台支持不对等，`menu_bar_support()` 如实上报，不假装：

| 平台 | 结果 | 说明 |
| --- | --- | --- |
| macOS | `System` | 系统菜单条。第一个菜单落在应用菜单位置，放应用级命令。无需窗口，也可直接用 `install_application_menu_bar` |
| Windows | `InWindow` | 窗口内的 `HMENU`。选中经 `WM_COMMAND`，由 `SetWindowSubclass` 挂的钩子取回 |
| 其它 | `Unavailable` | 什么都不装。把这些命令放进界面里 |

`installed_menu_bar()` 读回平台实际持有的菜单（macOS），供宿主自检；`crates/nana-window/examples/menu-probe.rs` 就是用它做真机验收的。

菜单是整体替换：再调一次 `SetMenuBar` 换掉整条。没有增量条目 API——重建一个菜单很便宜，而跨三个平台做 diff 不便宜。

## 文件对话框

系统文件对话框需要父窗口句柄——macOS 挂成 sheet,Windows 需要 owner HWND——而句柄只在宿主层。所以对话框和菜单栏走同一条路:模型在 `nana-ui-core`(`FileDialogRequest` / `FileDialogResult` / `FileFilter`),打开由宿主经 `WindowCommand::OpenFileDialog { id, request }` 执行。控件仍然拿不到句柄:`PathField` 只发 `BrowseRequested`,由应用翻译成一个请求。

结果是**异步**的：宿主通过 `WindowEvent::FileDialogCompleted { id, result }` 回流并主动唤醒事件循环。`id` 是窗口身份，`result.id` 是应用的 `u64` 请求身份；应用保存请求对应的业务对象和编辑基线，再消费结果。没有全局结果队列，也无需每帧轮询。

每个窗口只能有一个活动对话框。拒绝通过独立 `WindowEvent::FileDialogRejected { id, request_id, error }` 回流。第二个不同身份的请求收到 `FileDialogError::Busy`，不会覆盖第一个请求；重复活动身份收到 `DuplicateRequest`，消费方保留原 pending，不把拒绝当作该活动请求完成。窗口关闭时活动请求收到 `WindowClosed`。宿主为每次打开分配内部 token，关闭后晚到的回调（包括窗口或请求 ID 重用）不会完成新请求，每个接受的请求只完成一次。宿主同时持有原生会话句柄，关闭时结束 macOS sheet、关闭 Windows worker 的 picker 或取消 portal/zenity；保留父句柄的 worker 不会留下可见孤儿窗口。

**取消不是错误**：`result.error` 为 `None` 且 `paths` 为空。可观察的平台/线程错误通过 `FileDialogError::Platform` 返回；不支持的目标返回 `Unavailable`。rfd 本身只返回 `Option`，其 `None` 保持取消语义，不能据此推断系统失败。`PickFolders` 和 `OpenFiles` 返回多个路径，其余返回单路径或取消。过滤器、初始目录和保存文件名保留在请求中。

| 平台 | 执行方式 |
| --- | --- |
| macOS | 主线程 `NSOpenPanel` / `NSSavePanel` sheet，回调完成；支持文件、多个文件、目录、多个目录和保存 |
| Windows | 独立 rfd 工作线程持有父窗口，支持文件/目录的单选与多选及保存，不阻塞宿主渲染 |
| Linux | 独立 portal 工作线程，带父窗口标识；响应在打开前订阅并按实际返回的 request path 关联，兼容旧 portal；不可用时沿用可取消并回收子进程的 zenity fallback |
| 其它 | 返回 `Unavailable`，不静默悬挂 |

Linux portal 返回 URI 数组，可保留路径中的换行。zenity fallback 多选采用换行分隔的 CLI 输出，文件名本身包含换行时无法无歧义拆分；单选只剥离一个协议结尾换行，保留实际文件名。该 fallback 多选边界不能作为任意路径支持通过的依据。

`describe_configured_dialog(&request)` 读回平台实际配置（标题、起始目录、扩展名），不呈现对话框；`crates/nana-window/examples/file-dialog-probe.rs` 检查这部分配置。真实交互使用 `crates/nana-ui/examples/hosted-file-dialog-probe.rs`：在应用窗口内覆盖五种选择、重复与忙碌拒绝、取消、窗口退出，并观察对话框打开时持续 `window_frame_presented`。配置检查和交叉编译不能代替各平台原生交互验收。

## 图标

任务栏、exe、Dock 上的图标是应用身份，不是界面里的 `Icon` 字形。

- Rust：`register_application_icon`，或 `WindowSettings::icon` / `WindowCommand::SetIcon`
- 未设置时用默认几何标记，不要把它当品牌
- Windows exe 可在 `build.rs` 里 `nana_app_icon::embed_windows()`
- macOS Dock：`nana_window::set_application_icon_png`；`.app` 用 `nana-package-app`

## 打包

发布产物用 `dist` 档，不是 `release`，更不是 `debug`：

```bash
cargo build -p component-gallery --bin component-gallery --profile dist
cargo run -p nana-app-icon --bin nana-package-app --   --exe target/dist/component-gallery --name NanaUI   --identifier dev.nanaui.gallery --out target/dist
```

`dist` = `release` + `lto = "fat"` + `codegen-units = 1` + `strip = "symbols"` +
`panic = "abort"`。`release` 保持原样，CI 和 benchmark 继续快速迭代。

`nana-package-app` 默认再跑一次 `strip -x`（`--no-strip` 关掉），并在可执行文件里
探到 debug-assertions 字符串时警告——曾经有一个 108 MB 的 `.app` 就是误打了 debug
构建，其中 55 MB 是符号表。

用 `scripts/report-artifact-size.py <artifact>` 逐段核对体积，它同时数出二进制里内嵌
了几份字体，并在发现 debug 构建时报警。

## 材质

通过 `RuntimeProgram::window_material_mode` 申请**一种**系统效果。Appearance 设置在宿主提供时可选 Mica / Acrylic / Vibrancy；失败回实色，并给出原因，不会改试另一种。

| 平台 | 可申请 | 失败时 |
| --- | --- | --- |
| macOS 10.10+ | 指定的 Vibrancy / UnderWindowBackground | 不透明主题背景 |
| Windows 11 | 指定的 Mica **或** Acrylic | 不透明主题背景 |
| Windows 10 1809+ | 指定的 Acrylic | 不透明主题背景 |
| Linux | 无系统模糊 API | 不透明主题背景 |

`Translucent` 只开窗口透明，不等于模糊。透明窗口和系统模糊是两件事。

当前 macOS 在 GPU 窗口上申请 Vibrancy 可能拿不到系统效果（金属层会盖住系统材质）。Windows 的透明客户区和 Mica / Acrylic 以真机为准。编译通过不等于那台机器上看起来对。对照 `crates/nana-ui/examples/transparent-window.rs`。

原生材质由 `nana-window` 执行：`apply_system_material` / `apply_hosted_system_material`。`run_runtime` 会给主窗口和每个工具窗口分别应用、刷新和清理。主题或材质切换会先清掉旧效果再按当前请求重试。设备恢复后按当前请求重新应用。native 成功时，侧栏/主区/标题栏的覆盖色来自 Runtime Style Model（`ThemeTokens::with_backdrop`），不是整窗清屏。

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


### 独立透明工具窗

`WindowSettings::focus_on_show = false` 让首次显示不抢占前台焦点；默认 `true` 保持原行为。工具层可组合 `transparent = true`、`always_on_top = true` 与非模态 `WindowRole::Tool`。不需要 `DesktopShell` 才能使用边缘缩放。

`WindowCommand::Open` 成功发出 `WindowEvent::Ready`；创建失败发出 `OpenFailed { id, error }`，应用应撤销创建中状态。`SetMousePassthrough { id, enabled }` 通过原生窗口命中测试实现穿透，每次都回报 `MousePassthroughChanged { id, enabled, result }`（未知窗口也回报失败）。应用收到成功确认后才显示锁定状态，并保留另一窗口的解除穿透入口。

`RuntimeProgram::window_material_mode_for(id)` 与 `appearance_backdrop_opacity_for(id)` 默认调用现有全局方法，允许主窗和透明工具窗分别配置。宿主在创建、外观变化、Surface 恢复时都按目标窗口调用；背景透明不改变前景文字的不透明度。纯透明窗口的内容背景由应用 Runtime 节点绘制。

`WindowSettings::constrain_to_work_area = true` 用于完整恢复工具窗：部分出屏的位置也会校正，尺寸超出屏幕时会缩小。Windows 使用扣除任务栏的原生工作区；其他平台当前回退显示器边界。默认 `false` 保留原有“与任意屏幕有交集即保留位置”的行为。

原生验收探针（会短暂移动鼠标到探针自身窗口）：

```powershell
cargo build -p nana-ui --example desktop-overlay-probe --features hosted,bundled-fonts --locked
python scripts/validate-desktop-overlay.py
```

探针验证主窗 Solid/Opaque 与工具窗 Transparent/PreMultiplied、首次不抢焦点、创建失败反馈、穿透开关反馈、实际鼠标 1→0→1 路由及透明区域与关闭后的底层屏幕像素一致。结果写入 `target/desktop-overlay-native.json`；它不替代具体产品布局的视觉验收。
