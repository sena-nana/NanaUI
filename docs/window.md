# 窗口

Windows 自绘窗口按钮采用 NanaUI 图标按钮外观：28×28、控件圆角、垂直居中；按钮间距 2px，按钮组左右各留 6px。普通按钮使用常规悬停/按下颜色，关闭按钮使用危险色。按钮的实际区域处理窗口操作，周围留白不扩展为系统式整块按钮；macOS 原生红黄绿按钮保持平台行为。

NanaUI 画的是桌面窗口：标题栏、图标、系统材质、多窗口都按桌面软件来，不按浏览器来。

`run_runtime(WindowDescriptor::new("标题"))` 会创建主窗口、唯一 GPU 上下文，并开始事件循环。`WindowDescriptor` 就是 `nana_ui_platform::WindowDescriptor`。

## 标题栏

默认自绘标题栏：左侧内容、中间标题、右侧窗口按钮。空白处拖动窗口；按钮先吃到指针，不会被拖走。

叠加在舞台上的标题栏仍使用 `AppTitleBar`，通过 `leading / center / trailing / controls` 布局槽承载文字和操作，并调用 `assemble_app_title_bar`。`transparent(true)` 仅移除栏背景，保留内容与命中，不需要开启整窗透明。`drag_enabled(false)` 禁止栏内空白与文字启动拖窗，并在后续输入时取消已按下但未完成的手势；隐藏或卸载标题栏也会取消。默认分别为 `false`、`true`。全屏保留业务入口时设置 `drag_enabled(false)` 和 `show_window_controls(false)`；不要另外添加顶边坐标拖动逻辑。语义入口支持 `transparent` 和 `drag-enabled`。标题为空且没有 center 内容时不保留中间占位，右侧按内容宽度保留空间，左侧使用剩余宽度并可收缩；有标题或 center 时保持左右对称布局。

- macOS：透明标题栏 + full-size content，NanaUI 画 36px 标题栏，左侧给系统红黄绿留 78px。系统默认只把红黄绿放在标准标题栏高度内居中，`prepare_client_chrome` 会按标题栏高度平移按钮容器，使其在 36px 内居中。
- Windows / Linux：关掉系统 decorations，由 `AppTitleBar` 画最小化、最大化、关闭，控件组贴标题栏右缘。

自绘 chrome 可拖窗口客户区最外 8px 缩放（四边与四角）。系统 caption、最大化、全屏、`resizable: false` 交给平台边框或禁用，不叠第二套命中。

没有自绘标题栏的窗口设 `WindowDescriptor::system_caption(true)`，避免 Windows 无框窗口失去关闭按钮。

关闭 / 最小化 / 最大化是窗口动作。控件发出语义（`WindowChromeAction`），Scene host 去执行；关闭与系统关闭一样只向程序发 `CloseRequested`，由程序决定是否关闭。普通控件拿不到窗口句柄。L1 CSS `-webkit-app-region` / `app-region` 不是拖拽合同：任意盒写 `drag` 也不会变成 caption。

`WindowChromeState` 绑在明确的 `WindowId` 上。单窗口可用默认入口（只认收到的第一扇窗）；多窗口在等待 `WindowService::create_window` 获得 handle 后，用其 `id()` 调用 `for_window` 各建一份。关窗后不会自动接管别的窗口。

### Windows 客户端绘制标题栏契约

Windows 上有两条互斥的 chrome 路径，由 `WindowDescriptor::system_caption` 与 `transparent` 决定，实现见 `windows_scene_chrome`：

| 设置 | 系统边框 | 阴影 / 圆角 | `WS_EX_NOREDIRECTIONBITMAP` |
| --- | --- | --- | --- |
| `system_caption: true` | 开（系统标题栏与缩放） | 系统默认 | 仅透明窗口打开 |
| `system_caption: false` 且不透明 | 关，由 `AppTitleBar` 画 Minimize / Maximize / Close | 圆角（Windows 11 DWM 阴影）；不用 winit `undecorated_shadow`（会把客户区顶边内缩 1px，标题栏盖不住） | 关 |
| `system_caption: false` 且 `transparent: true` | 关 | 无圆角、无 DWM 描边（`DoNotRound` + `DWMWA_COLOR_NONE`），避免透明叠加层留下 HWND 矩形轮廓 | 开 |

不透明自绘窗关掉 decorations 且不用 `undecorated_shadow`：winit 用 `WM_NCCALCSIZE` 把客户区铺到窗口外沿，创建后不再清 `WS_CAPTION`（`SetWindowPos(SWP_FRAMECHANGED)` 会在 `can_create_surfaces` 里卡住 UI 线程）。透明自绘窗仍清 caption，避免 DWM 合成留下系统边框。winit 的 `WindowFlags::apply_diff` 会把 `WS_CAPTION | WS_BORDER | WS_SYSMENU` 写回 `GWL_STYLE`；透明无框窗必须在任何会改原生窗口样式的 winit 调用之后再次剥离，否则 Windows 11 会画出系统边框和三大键。自定义标题栏按钮宽高均为 `WINDOW_CONTROL_WIDTH`，在高 `TITLE_BAR_HEIGHT` 的 controls 槽内垂直居中，按钮组两侧保留 `WINDOW_CONTROL_PADDING`。

命中顺序（逻辑像素，已含当前 `scale_factor`）：

1. 自绘窗口按钮（AccessKit 名称 `Minimize`、`Maximize`/`Restore`、`Close`）优先，不启动拖拽；客户区最外 8px 缩放区域与实际按钮相交时同样让位；按钮之外的边角继续支持窗口缩放。
2. 标题栏空白处发出 `WindowChromeAction::Drag`：macOS 在按下时即交给 AppKit，且 `currentEvent` 必须仍是左键按下或拖动，否则不拖；其他平台按下后移动超过 4px 才发出。Scene host 调用 `nana_window::drag_custom_title_bar`，非 macOS 失败再 `winit::drag_window`。原生拖窗接管按键释放，拖动开始后 host 立即补发一次 `PointerPhase::Cancel`，结束按键掩码、Runtime 按压/捕获、标题栏手势与程序侧手势；`WindowService::begin_drag` 同样适用。
3. 无系统 caption、可缩放、未最大化、非全屏时，客户区最外 `RESIZE_HANDLE_SIZE`（8px）走 `LiveFrameResize`（macOS `setFrame`、Windows `SetWindowPos`），不进入系统嵌套 size-move 循环；系统 caption 窗口不叠第二套缩放命中。

窗口光标还会消费 L1 CSS `cursor` 的常用关键字：`default`、`pointer`、`text`、`move`、`grab`、`grabbing`、`not-allowed`、`crosshair`、`help`、`wait`、`progress`、`zoom-in`、`zoom-out`、`none`。该属性按 CSS 继承；未知关键字和 `url()` 光标 fail-closed。`WindowCursor` 程序化入口与上述关键字对齐，并多一个 `Automatic` 以恢复 Runtime/CSS 选择。光标优先级低于窗口边框缩放和分割/停靠/工作区 resize 手柄，高于未声明 cursor 时 TextInput 的 I 型光标；`none` 只隐藏系统光标，不加载自定义图片。

### 实时缩放

客户区拖动边框时，指针移动直接改窗口矩形，事件循环继续跑，`SurfaceResized` 同步几何并请求下一帧。画帧时若物理尺寸或 present 策略变了才 `surface.configure`；同尺寸跳过。Windows 系统边框缩放仍可能走 `WM_ENTERSIZEMOVE`。稳态帧使用 `Mailbox`（没有则 `Immediate`，再回 `AutoVsync`），避免混合刷新下 FIFO 跟主屏合成钟；`LiveSizeMove` 保持同一 present 模式并把 frame latency 提到 2。透明窗口走同一条路径。

DPI 与多显示器：指针、拖拽与缩放都用逻辑坐标；物理像素只用于 Surface。窗口位置由宿主记录，创建前按当前显示器工作区 clamp（原屏断开则主屏居中）。带 `parent` 的辅助窗（模态与非模态）在 Windows 上以 `with_owner_window` 绑定父 HWND：保持在父窗之上、随父窗最小化，且没有独立任务栏按钮。

IME：焦点进可编辑字段时 `Window::request_ime_update(Enable)` 一次（hint / purpose、caret 盒、非密码的 surrounding text）。之后 caret、purpose 或 surrounding 变化走 `Update`；能力集变了先 `Disable` 再 `Enable`；失焦 `Disable`。候选框相对 caret，不相对系统非客户区。AccessKit 增量更新与视觉几何同一套 layout box；composition 期间不得出现悬空 `parent_and_index`。

透明 Alpha（`settings.transparent`）强制 `MaterialEffect::Transparent`，不会改试 Mica / Acrylic。失败只能回不透明实色，并带 `MaterialFallback`。真机入口：`vue-hosted-acceptance --chrome-probe`、`--input-probe`、`--hybrid --windows`，以及 `nana-ui` 的 `transparent-window` 示例。

## 菜单栏

原生应用菜单栏是**唯一画不进界面树**的桌面 chrome：macOS 上它属于应用而不是窗口，住在系统菜单条里。所以它在 `nana-window`，用一份平台中立的模型描述（`MenuBar` / `Menu` / `MenuEntry` / `MenuShortcut`，模型本身在 `nana-ui-core`，纯数据）。

应用**声明**菜单，宿主**安装**它：调用 `window.set_menu_bar(bar)`。普通控件拿不到窗口句柄，而 Windows 的菜单属于窗口，所以安装必须由持有窗口的 Scene host 做——和 `SetIcon` 同一条路。

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

系统文件对话框需要父窗口句柄——macOS 挂成 sheet,Windows 需要 owner HWND——而句柄只在宿主层。所以对话框和菜单栏走同一条路:模型在 `nana-ui-core`(`FileDialogRequest` / `FileDialogResult` / `FileFilter`),应用通过 `WindowHandle::open_file_dialog` 请求，由宿主执行。控件仍然拿不到句柄:`PathField` 只发 `BrowseRequested`,由应用翻译成一个请求。

结果是**异步**的：宿主通过 `WindowEvent::FileDialogCompleted { id, result }` 回流并主动唤醒事件循环。`id` 是窗口身份，`result.id` 是应用的 `u64` 请求身份；应用保存请求对应的业务对象和编辑基线，再消费结果。没有全局结果队列，也无需每帧轮询。

每个窗口只能有一个活动对话框。拒绝通过独立 `WindowEvent::FileDialogRejected { id, request_id, error }` 回流。第二个不同身份的请求收到 `FileDialogError::Busy`，不会覆盖第一个请求；重复活动身份收到 `DuplicateRequest`，消费方保留原 pending，不把拒绝当作该活动请求完成。窗口关闭时活动请求收到 `WindowClosed`。宿主为每次打开分配内部 token，关闭后晚到的回调（包括窗口或请求 ID 重用）不会完成新请求，每个接受的请求只完成一次。宿主同时持有原生会话句柄，关闭时结束 macOS sheet、关闭 Windows worker 的 picker 或取消 portal/zenity；保留父句柄的 worker 不会留下可见孤儿窗口。

**取消不是错误**：`result.error` 为 `None` 且 `paths` 为空。可观察的平台/线程错误通过 `FileDialogError::Platform` 返回；不支持的目标返回 `Unavailable`。Windows 上用户关闭对话框是 `HRESULT_FROM_WIN32(ERROR_CANCELLED)`，其它 HRESULT 是 `Platform`，两者不混用。`PickFolders` 和 `OpenFiles` 返回多个路径，其余返回单路径或取消。过滤器、初始目录和保存文件名保留在请求中。不存在或不可访问的初始目录会被跳过，对话框落在系统默认位置，不把这种情况当成取消。

| 平台 | 执行方式 |
| --- | --- |
| macOS | 主线程 `NSOpenPanel` / `NSSavePanel` sheet，回调完成；支持文件、多个文件、目录、多个目录和保存 |
| Windows | 独立工作线程上的 `IFileOpenDialog` / `IFileSaveDialog`，父窗口为 owner HWND；`FOS_PICKFOLDERS` 选择目录；不阻塞宿主渲染 |
| Linux | 独立 portal 工作线程，带父窗口标识；响应在打开前订阅并按实际返回的 request path 关联，兼容旧 portal；不可用时沿用可取消并回收子进程的 zenity fallback |
| 其它 | 返回 `Unavailable`，不静默悬挂 |

Linux portal 返回 URI 数组，可保留路径中的换行。zenity fallback 多选采用换行分隔的 CLI 输出，文件名本身包含换行时无法无歧义拆分；单选只剥离一个协议结尾换行，保留实际文件名。该 fallback 多选边界不能作为任意路径支持通过的依据。

`describe_configured_dialog(&request)` 读回平台实际配置（标题、起始目录、扩展名），不呈现对话框；`crates/nana-window/examples/file-dialog-probe.rs` 检查这部分配置。macOS 从 AppKit panel 读回三项。Windows 起始目录来自 `GetFolder`；`IFileDialog` 没有 GetTitle / GetFileTypes，标题是 `SetTitle` 成功后的回显。目录选择不应用过滤器（因此扩展名为空），也不应用预填文件名。真实交互使用 `crates/nana-ui/examples/hosted-file-dialog-probe.rs`：在应用窗口内覆盖五种选择、重复与忙碌拒绝、取消、窗口退出，并观察对话框打开时持续 `window_frame_presented`。配置检查和交叉编译不能代替各平台原生交互验收。

## 图标

任务栏、exe、Dock 上的图标是应用身份，不是界面里的 `Icon` 字形。

- Rust：`register_application_icon`，或 `WindowDescriptor::icon` / `WindowHandle::set_icon`
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

普通应用通过 `context.windows()` 创建窗口，通过 `context.window()` 控制当前窗口。`WindowHandle` 可克隆并发送给工作线程，不持有原生窗口；所有操作排队回到窗口线程。尺寸和位置使用逻辑坐标。

```rust
// 工作线程；在异步代码中也可以用 .await。
let window = service.create_window(WindowDescriptor {
    title: "Notes".into(),
    initial_size: (480.0, 320.0),
    minimum_size: (240.0, 120.0),
    system_caption: true,
    ..Default::default()
}).wait()?;
window.set_title("Preview").wait()?;
window.set_size((640.0, 480.0)).wait()?;
window.close().wait()?;
```

创建结果仅在隐藏原生窗口、Surface、输入状态和应用文档初始化成功后完成；失败会回滚，不发送 `Ready`。`ApplicationState::build` 为每个窗口构建独立文档。自定义 `RuntimeProgram` 在 `initialize_window` 中完成构建，在 `discard_window` 中撤销失败的应用状态。成功后才注册并按 `WindowDescriptor::visible` 显示窗口。

窗口 id 由服务分配，应用用 `WindowDescriptor::tag` 声明这扇窗口是哪种文档，不要靠创建请求的顺序去对应 id。标识对宿主不透明，从 `RuntimeProgramContext::window_tag()` 读回：`initialize_window` / `ApplicationState::build` 构建文档时，以及之后该窗口的每个回调（包括 `Ready`）都能读到；窗口关闭后为 `None`，按 id 持有的状态在 `window_closed` / `discard_window` 里清理。

```rust
let character = service
    .create_window(WindowDescriptor::new("角色").tag("character"))
    .wait()?;

fn build(&mut self, window: &mut ApplicationWindow, context: &RuntimeProgramContext<Message>) -> Result<(), Error> {
    match context.window_tag() {
        Some("character") => build_character(window),
        Some("tracking") => build_tracking(window),
        _ => build_main(window),
    }
}
```

程序自选 `WindowId` + `WindowCommand::Open` 仍只是框架适配器（Vue、Dock）的通道，普通应用不需要它来区分窗口种类。

操作返回 `WindowRequest<T>`，支持 `.await`、工作线程 `.wait()` 和窗口线程非阻塞的 `try_take()`。在窗口线程调用 `.wait()` 返回 `HostThreadWait`，不阻塞事件循环。待处理窗口请求最多 1024 个，队列满时立即返回 `QueueFull`，调用方可等待已提交请求完成后重试。关闭后的 handle 返回 `WindowClosed`，宿主释放后返回 `HostStopped`；身份及世代检查防止旧请求作用于重新创建的窗口。

主窗口与附加窗口共用注册表和 Device/Queue，每个窗口独立持有 Surface、输入、IME、文档和渲染目标。同一纹理格式的窗口共用一个 Scene painter；各文档的 compositor motion 描述符按 `MotionDescriptorStore::source` 与结构 epoch 识别，切换绘制窗口时重新上传，不会读到另一窗口的动画。默认关闭一扇窗口只释放该窗口及其原生子窗口；standalone 最后一扇窗口关闭后退出。应用仍可显式返回 `RuntimeProgramUpdate::exit()` 关闭整个应用。

`ApplicationState::window_event` 和 `RuntimeProgram::window_event` 接收框架窗口事件；指针、键盘输入通过 `RuntimeProgram::input_event` 的 `RoutedInput` 接收，附带命中与处理结果，同一输入不重复派发。缩放变化通过带新 `scale_factor` 的 `Resized` 通知。

### 指针在场

`WindowEvent::PointerPresenceChanged { id, inside }` 报告悬停指针（鼠标、笔）是否在该窗口客户区内，只在变化时发送，供按指针显现 chrome 的应用使用，不需要轮询光标。输入流里的离开仍归一化为 `PointerPhase::Cancel`，与手势取消无法区分，所以不要用它推断在场。

- 触摸接触不报告在场。
- 宿主发起原生拖窗（标题栏拖动、`WindowHandle::begin_drag`）时，平台移动循环产生的离开被扣下，直到平台再次报告该指针；拖动被屏幕边缘挡住后指针离开窗口的情形，要等指针回到窗口再离开才会报告。
- 指针被捕获时，客户区外的移动不会把状态改回在场。
- `WindowHandle::set_visible(false)` 隐藏窗口时报告一次离开。
- Forward 穿透由宿主采样全局指针，在场随采样结果更新；采样不可用的平台（Wayland）报告为不在场。
- 有模态子窗口时父窗口仍报告在场，指针事件本身继续交给模态链处理。

### 显示器与全屏

`WindowService::displays()`（嵌入式宿主用 `EmbeddedRuntime::displays(event_loop)`）在窗口线程枚举当前连接的显示器，返回 `DisplayInfo`：id、名称、物理位置/尺寸、缩放、刷新率、是否主屏。Wayland 不上报主屏。`DisplayInfo::logical_bounds()` 把物理矩形换算成 `WindowDescriptor::initial_position` 使用的全局逻辑坐标；没有位置或尺寸时返回 `None`。

`DisplayId` 是本次进程里的显示器身份，跨次枚举相等。它是否在拔插后仍然指向同一块屏，取决于平台：

| 平台 | 拔掉再插回 |
| --- | --- |
| macOS | `MonitorHandle::id()` 跨重连稳定 |
| Windows / Linux | 不跨重连；重连后当作新显示器 |

没有显示器拓扑事件：winit 不提供连接/断开通知。拔屏后下一次 `displays()` 不再列出它；已经全屏在该屏上的窗口按平台落到剩余显示器。指定一块已经不存在的屏：显式 `set_fullscreen(Some(FullscreenRequest { display: Some(id), .. }))` 返回 `WindowError::InvalidParameter`，窗口状态不变；描述符上的 `WindowDescriptor::fullscreen` 则退回窗口模式（尽力而为，创建本身仍成功）。

全屏只有无边框覆盖当前视频模式，没有独占全屏（不会改显示器分辨率，避免和其他 Surface、采集工具抢显示模式）。

```rust
#[derive(Default)]
pub struct FullscreenRequest {
    pub mode: FullscreenMode,       // Borderless（默认）或 Simple
    pub display: Option<DisplayId>, // None = 窗口当前所在屏
}

pub enum FullscreenMode {
    Borderless, // 无边框，覆盖该屏当前视频模式
    Simple,     // macOS 不切 Space；其他平台等同 Borderless
}
```

`WindowHandle::set_fullscreen(None)` 退出全屏。`Simple` 只在 macOS 上报为 `WindowModeState::fullscreen = Some(Simple)`。从 macOS 原生全屏切到 Simple 时，宿主先退出原生全屏，等 `fullscreen()` 读到 `None` 再应用 Simple。

有效状态通过 `WindowEvent::ModeChanged { id, mode }` 上报，`mode` 含实际全屏模式、窗口层级和当前显示器。时机：

- 每个窗口 `Ready` 之后必发一次，作为初始状态
- 每次全屏或层级请求之后
- `Resized` / `Moved` / `ScaleFactorChanged` / `Occluded` / `RedrawRequested` 之后，如果观察结果变了

只在与上次交付不同时发送。层级是宿主上次应用的值：平台不会回报应用外的置顶变化。macOS 原生全屏动画期间读到的是目标状态。X11 下窗管自己改的全屏/置顶观察不到。`RedrawRequested` 和 `Occluded` 用来补齐异步 Borderless 完成之后的观察。

描述符全屏在窗口可见之后才应用（窗口先以隐藏状态创建并完成文档初始化）。不要用描述符表达“这块屏必须存在”，那是显式请求的合同。

`window-service-lifecycle` / `embedded-window-lifecycle` 覆盖枚举非空、指定屏全屏后收到 `ModeChanged`、置顶、`set_skip_taskbar` 的平台结果（Windows 成功，其他平台 `Unsupported`）、退出全屏、非法 `DisplayId`、描述符全屏。macOS 绿色按钮、第二块屏、拔屏，以及 Windows 多屏，需要人工核对。

### 嵌入已有宿主

`platform_host::EmbeddedRuntime` 接收宿主 `ActiveEventLoop`、proxy 和 `HostedGpuShared`，从不创建或退出宿主事件循环。宿主转发 `window_event`、`wake` 和 `about_to_wait`。`HostedGpuShared::from_device` 接入已有 Instance/Adapter/Device/Queue；不申请第二个 Device。

宿主在窗口线程回调中可用 `create_window(event_loop, descriptor)` 同步创建窗口，结果与 `WindowService::create_window` 相同，但不经过请求队列。整个进程只有一个事件循环：standalone 下再次调用 `run_runtime` 返回 `HostedRunError::EventLoop`（winit `RecreationAttempt`）；已有事件循环的宿主应改用 `EmbeddedRuntime`。

设备恢复仍归 embedded 宿主负责：宿主收到外部 Device 丢失通知后，先在窗口线程调用 `notify_device_lost()`，再转发其他窗口事件；通过 `needs_gpu_replacement()` 检查挂起状态，重建宿主设备后调用 `replace_gpu()`。替换总是切换到新 Device：每个存活窗口的 Surface 原地重绑，先释放旧交换链再配置新的——DXGI 规定一个 HWND 只能有一个交换链，无法在保留旧 Surface 作回退的同时试建新的，而设备丢失后旧 Surface 本来也无法呈现。切换后调用应用 GPU 重建回调，重绑失败的窗口按下文 Surface 故障恢复单独重试；只有所有窗口都重绑失败时才返回第一个错误供宿主上报，但切换已经发生。

`WindowHandle::with_native_handle` 将回调调度到窗口线程，仅借用回调期间有效的 raw handle。不能保存原始指针供回调结束后使用，也不会取得 `winit::Window` 所有权。

`window.effects().set_material()` 返回实际 `MaterialOutcome`；穿透通过 `set_mouse_passthrough()` 控制。`window.capture().set_protected()` 请求 macOS/Windows 的原生捕获保护，其他后端返回 `Unsupported`；这不是对所有捕获方式的保证。

完整示例：`window-service-lifecycle` 无需导入 winit；`embedded-window-lifecycle` 展示高级宿主适配。两个示例都自动验证三窗口真实呈现、按 `tag` 构建的两种附加窗口文档、跨线程控制、主窗关闭、失败回滚、子窗释放与再次创建。

### 从旧接口迁移

- `RuntimeWindowSettings` / `WindowSettings` 统一改为 `WindowDescriptor`，显式结构体初始化需添加 `visible` 或使用默认值。
- `WindowDescriptor` 新增 `tag`；显式结构体初始化补 `tag: None` 或使用 `..Default::default()`。
- 普通应用用 `WindowService` / `WindowHandle` 替代自行分配窗口 ID 和提交 `WindowCommand`。
- `WindowCommand` 从平台 crate 根导出移入 `nana_ui_platform::host`，仅供 Vue、Dock、chrome 等框架适配器使用；适配器提交的批次仍在宿主 commit 时进入同一个 WindowManager。
- 主窗口不再具有隐式退出特权；需要“关主窗退出”的产品应显式返回退出更新。
- `WindowCommand::SetFullscreen` 的参数从 `bool` 改为 `Option<FullscreenRequest>`：`None` 退出全屏，`Some` 进入。删除 `SetSimpleFullscreen`；macOS 不切 Space 的全屏改为 `FullscreenRequest { mode: FullscreenMode::Simple, display: None }`。`WindowHandle::set_simple_fullscreen` 一并删除。
- `WindowLevel` 从 `nana_ui::window_service` 下沉到 `nana_ui_platform`，`nana_ui` 再导出。全屏和置顶的有效值走 `WindowEvent::ModeChanged`，不要自己记一份请求镜像。
- LiliaBilibili `app/presentation.rs` 仍按 `SetFullscreen { fullscreen: bool }` 编译；下次升级 pin 时改为 `fullscreen: on.then(FullscreenRequest::default)`，并在 `WindowEvent` match 中处理 `ModeChanged`。

窗口几何可以交给框架：`WindowDescriptor::persist_key("main")` 会在创建前从 `localStorage` 键 `nana.window.main` 恢复位置 / 尺寸 / 最大化，并在移动、缩放后写回。全屏、最小化，以及 Win32 最小化常见的 `-32000` 原点都不写。目录由宿主选择：`app_data_dir("YourApp")` 得到平台数据目录，再 `FileStore::open` 注入 `run_runtime_with_store`（落成 `local-storage.bin`）。不注入则全程内存，进程退出即丢。Dock 布局与 `AppearanceSettings` 走同一张表（`nana.dock.*` / `nana.appearance.*`）。`Nana.storage` 的 `get` / `set` / `clear` / `remove` / `keys` 不会动这些 key。

### 独立透明工具窗

`WindowDescriptor::focus_on_show = false` 让首次显示不抢占前台焦点；默认 `true` 保持原行为。工具层可组合 `transparent = true`、`always_on_top = true` 与非模态 `WindowRole::Tool`；属于某个主窗的工具窗设置 `parent`，在 Windows 上由父窗 own，点击父窗不会把工具窗压到后面。不需要 `DesktopShell` 才能使用边缘缩放。

`WindowService::create_window` 在完整就绪并发送 `WindowEvent::Ready` 后完成凭据；创建失败通过凭据返回错误；窗口若在处理 `Ready` 时被应用关闭，凭据返回 `WindowClosed`。服务从 `1 << 63` 起分配窗口 ID，并跳过仍存活的程序自选 ID（例如 Dock 浮动窗口的哈希 ID）。Vue 等宿主批次适配器另通过 `OpenFailed { id, error }` 通知失败，撤销创建中状态。`SetMousePassthrough { id, enabled }` 关闭整窗原生命中测试（Windows `WS_EX_TRANSPARENT`），overlay 收不到指针，也无法按命中自己收回。`SetMousePassthroughForward { id, enabled }` 是宿主持有的 Forward 模式：OS 穿透保持开启，宿主在窗口线程采样全局指针（Windows `GetCursorPos`、macOS `mouseLocationOutsideOfEventStream`、Linux X11 `XQueryPointer`；Wayland 当前采不到则无法自动收回），换算为 overlay 逻辑坐标后走现有 `pointer_target`。命中 `pointer-events` 非 `none` 的不透明/可交互内容时 `set_cursor_hittest(true)` 收回；离开该区域或窗口后再穿透。采样到的 Move 仍可喂 overlay hover/光标，按下必须在收回之后才由本窗接收，底层窗口在穿透期间可点。控件拿不到 HWND。`WindowHandle::set_mouse_passthrough_mode` 对应 `MousePassthroughMode::{Off, Passthrough, Forward}`。每次命中测试变化都回报 `MousePassthroughChanged { id, enabled, result }`（`enabled` 为当前 OS 穿透是否开启，未知窗口也回报失败）。应用收到成功确认后才显示锁定状态，并保留另一窗口的解除穿透入口。异形窗 / per-pixel alpha OS 形状不是这条合同。

`RuntimeProgram::window_material_mode_for(id)` 与 `appearance_backdrop_opacity_for(id)` 默认调用现有全局方法，允许主窗和透明工具窗分别配置。宿主在创建、外观变化、Surface 恢复时都按目标窗口调用；背景透明不改变前景文字的不透明度。纯透明窗口的内容背景由应用 Runtime 节点绘制。

`WindowDescriptor::constrain_to_work_area = true` 用于完整恢复工具窗：部分出屏的位置也会校正，尺寸超出屏幕时会缩小。Windows 使用扣除任务栏的原生工作区；其他平台当前回退显示器边界。默认 `false` 保留原有“与任意屏幕有交集即保留位置”的行为。

`WindowDescriptor::skip_taskbar = true` 让窗口不出现在任务栏，默认 `false`，主窗照常显示。运行中用 `WindowCommand::SetSkipTaskbar { id, skip_taskbar }` 或 `WindowHandle::set_skip_taskbar` 切换。描述符里的设置在窗口首次显示前应用，结果在 `Ready` 之后回报，失败不撤销创建。每次请求（包括描述符应用和未知窗口）都回报 `WindowEvent::SkipTaskbarChanged { id, skip_taskbar, result }`：`skip_taskbar` 是当前实际状态，失败时保持原值；描述符结果送达前的显式请求会取代它；`WindowHandle` 请求返回同一结果，平台不支持为 `Unsupported`，系统拒绝为 `OperationFailed`。

- Windows：宿主通过 `ITaskbarList` 删除或恢复任务栏按钮，每一步 HRESULT 失败都回报。每当 Explorer 放好按钮并发来 `TaskbarButtonCreated`（每次显示、Explorer 重启后）就再删除一次，不与系统创建按钮竞争。重复请求失败时保留已生效的隐藏；`skip_taskbar: false` 只恢复由这条合同隐藏过的按钮，不会给本来没有任务栏按钮的窗口（例如模态子窗）加按钮。不改 `WS_EX_TOOLWINDOW`，Alt+Tab 行为和标题栏样式不变。
- macOS：返回 `Unsupported`。Dock 按应用显示图标，普通窗口没有逐窗 Dock 项；但窗口最小化后仍会进入 Dock，应用图标也始终在 Dock 里。隐藏应用 Dock 图标是应用级激活策略，不属于这条逐窗合同。
- Linux：返回 `Unsupported`。

原生验收探针（会短暂移动鼠标到探针自身窗口）：

```powershell
cargo build -p nana-ui --example desktop-overlay-probe --features hosted,bundled-fonts --locked
python scripts/validate-desktop-overlay.py
```

探针验证主窗 Solid/Opaque 与工具窗 Transparent/PreMultiplied、首次不抢焦点、工具窗不在任务栏（UI Automation 比较任务栏按钮快照，不依赖窗口标题、按钮合并设置和系统语言，需要 `comtypes`；切换 `SetSkipTaskbar` 与隐藏后再显示都复核）、创建失败反馈、穿透开关反馈、实际鼠标 1→0→1 路由、Forward 在不透明命中区收回并收到后续 click、以及透明区域与关闭后的底层屏幕像素一致。结果写入 `target/desktop-overlay-native.json`；它不替代具体产品布局的视觉验收。Windows 是 Issue 必测平台；macOS 覆盖行为测试与文档。

## Runtime 中的原生网页内容

`runtime::BrowserView` 是保留树中的布局和可访问性节点；应用工具条、地址输入和
业务状态仍由 Runtime 承载。开启 `hosted` 后，`RuntimeProgram::native_browser_requests`
返回 `NativeBrowserRequest { id, node, policy, restore_url, visible, revision, command }`。宿主只为
当前窗口文档中、`browser_id` 与请求 `id` 一致的 `BrowserView` 创建原生内容。
同窗重复 `id`、跨文档节点和已销毁节点均不附着原生视图。

`revision` 是应用单调递增的命令身份；相同版本重复投影不会再次后退、刷新或截图。
已存在实例的 `command: None` 只同步状态。事件通过平台回调主动唤醒当前宿主，再经
`native_browser_event(window, NativeBrowserEvent { id, node, revision, event }, context)`
回流。事件保留产生它的命令版本；宿主再次核对窗口、当前请求和节点，迟到结果不会
被重新标记为后来的请求。截图通过 `Captured(Vec<u8>)` 返回 PNG，失败通过
`CaptureFailed`；截图过程中换页或隐藏会使原截图失效。

当前 macOS 后端使用主线程 `WKWebView` 子视图，绑定父窗口，支持 HTTP(S) 导航、
后退/前进、刷新/停止、聚焦和异步截图。`BrowserPolicy::allow_web` 默认关闭；
`about:blank` 始终允许，启用后仍拒绝文件、脚本和带凭据的 URL。重定向与新窗口链接
经过同一策略，新窗口链接留在同一浏览内容中。Windows/Linux 当前返回明确的不可用
状态，不创建占位浏览器；此边界不影响它们已有的窗口、文件对话框与 NativeContent
能力。

已挂载节点的隐藏保留浏览历史，原生视图隐藏并释放焦点；park 或销毁节点、撤回请求、
改变节点/策略或关闭父窗会移除原生视图，晚到回调被丢弃。节点再次挂载时创建新实例，
只导航至当前 Navigate 命令的目标或 `restore_url`，不会重放后退、停止或截图。
恢复页面后原生历史从新实例开始；应用应等待回流状态更新按钮。应用仍须在业务任务切换或
关闭时撤销自己的截图意图，不能仅在完成时比较当前任务（切走再切回也是新意图）。

原生子视图只支持有限平移和矩形裁剪，布局使用窗口逻辑坐标并跟随滚动。圆角裁剪、
非平移变换、透明度/滤镜组，或随后绘制的重叠 Runtime 内容，会暂时隐藏网页，避免
它遮住菜单和浮层。隐藏不缩小网页自身的布局尺寸；重复帧不会重新设置相同原生几何。
原生网页内容不会出现在 Runtime 离屏截图中；用户截图须使用 `BrowserCommand::Capture`。

可运行 `cargo run -p nana-ui --example native-browser --features hosted,bundled-fonts`，
通过示例页、说明页、后退、隐藏和截图按钮验证真实父窗。设置
`NANA_BROWSER_CAPTURE_OUTPUT=/tmp/nanaui-browser.png` 可保存网页 PNG。必须实际查看
原生窗口与 PNG 后才能宣称平台视觉和交互通过；编译及离屏树检查不能替代 WebKit 验收。

### Surface 故障恢复

Surface 创建、验证或材质 alpha 配置失败只暂停对应窗口，并以两秒间隔在现有共享 GPU 上重试；其他窗口继续绘制。`HostFailure::SurfaceRecovery` 报告窗口身份与首次错误。恢复成功后重置该窗口材质缓存并重绘，失败不关闭应用文档。恢复期间材质操作返回 `OperationFailed`。失败的材质操作不会保存新的覆盖值或透明偏好；若原生配置已部分改变，后续 Surface 恢复会重新应用上一次成功的设置。只有 Device 丢失才启动全局 GPU 恢复；Device 丢失导致的 Surface 失败不会逐窗口报告 `SurfaceRecovery`。standalone 恢复依次尝试各存活窗口作为新 Device 的基准，某扇窗口无法重建不会阻塞其他窗口；embedded 的局部 Surface 故障不会要求宿主更换 Device。

窗口拖动、边缘缩放与穿透控制保留底层错误分类：平台明确不支持时返回 `Unsupported`，操作被系统忽略或遇到其他 OS 错误时返回 `OperationFailed`。穿透事件与请求完成结果报告同一次操作的结果。

同一父窗口已有模态子窗口时，重复创建模态子窗口会失败；可以在现有模态子窗口内创建嵌套模态。请求聚焦父窗口会沿模态链转发到最深层活动子窗口。关闭父窗口会递归清理整条子窗口链，清理过程中不会重新启用或聚焦正在关闭的 Windows 父窗口。

`embedded-window-lifecycle --probe-device-loss` 在首帧前由宿主销毁外部 Device、通知管理器挂起并替换 GPU，然后执行相同的多窗口与焦点验收。它验证宿主通知/替换协议和恢复后的 present，不表示绘制中途恢复的性能数据。

创建描述符的尺寸必须有限且为正，位置必须有限，模态窗口必须指定父窗口。`WindowService::create_window` 对这些错误直接完成为 `InvalidParameter`，不入队、不唤醒宿主。standalone/embedded 的初始窗口同样在分配原生资源前校验，并且不能指定父窗口；有父窗口的窗口通过服务创建。

宿主直接销毁 `EmbeddedRuntime` 时，会先关闭请求队列并完成等待中的请求，再销毁应用状态。因此应用析构可以等待正在等待窗口请求的工作线程，未处理和后续请求均得到 `HostStopped`。`embedded-window-lifecycle --probe-host-stop` 覆盖这一退出顺序。

隔离窗口（`Nana.windows.create({ isolation: "isolated" })`，见 [Vue](vue.md#多窗口与-javascript-隔离)）的脚本在 `initialize_window` 里、窗口显示之前于它自己的 JavaScript 上下文中执行；执行失败按创建失败回滚，打开方收到 `window-open-failed`。关闭时 `window-closed` 同时送到它自己的上下文和打开方的上下文，卸载完成后，若这是该上下文里最后一扇窗口，上下文与其私有 `localStorage` 随即销毁。

Vue 主窗口也遵循独立关闭语义：原生关闭确认后释放对应文档、JS 定时器和监听器，其他 Vue 窗口及共享引擎继续存活。主窗口 DOM 操作在调用时解析存活文档，不会因引擎保留宿主操作而延长已关闭文档的生命周期。最后一个窗口的关闭通知仍会在引擎销毁前派发；材质和背景透明度按目标窗口读取。


`vue-hosted-acceptance --window-lifecycle-probe` 使用实际 Vue 组件验证主窗口独立关闭：两个 Surface 首先 present，主窗口关闭后检查 Vue 卸载钩子、文档和 JS 上下文释放，再确认附加窗口继续 present，最后检查其关闭通知与缓存释放。探针会将附加窗口置前，避免启动时被主窗口完全遮挡而等待不到首帧。加 `--isolated` 时附加窗口以隔离模式打开：两个 Surface 仍在同一 GPU generation 上 present，并在主窗口关闭前核对隔离窗口自己的上下文执行了应用、读不到主上下文写入的 `localStorage`、定时器照常触发。

Vue 窗口关闭时会卸载该窗口通过 `createApp().mount()` 或窗口 handle 挂载的应用，清理节点身份缓存、样式、动画、定时器及监听器。原生文档已先行销毁，因此同步卸载范围中的文档操作只完成 JS 侧清理；业务需要持久化数据时应在关闭请求阶段处理。普通存活窗口的卸载和宿主错误语义保持正常。
