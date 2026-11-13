---
title: 窗口与平台
---

# 窗口与平台：中立合同、原生边界与回退语义

本章走完「谁拥有窗口」这条链路的最后一段：**`nana-ui-platform` 的平台中立窗口合同长什么样**；**`nana-window` 如何在操作系统原生层管理句柄、系统材质、无框 chrome、高 DPI 缩放与全屏**；**为什么材质、合成与阴影失败时一律「如实上报实际生效结果」而非静默重试**；**为什么普通控件永远拿不到原生窗口句柄**；以及**无头 `WebSurface` 页面到底是什么**。本章承接 [GPU 与帧生命周期](/learn/gpu-frame) 的 Surface 所有权模型。

## 职责与边界

- **`nana-ui-platform`（平台中立合同层）**：定义 `WindowId`、`WindowDescriptor`、`WindowEvent`、`WindowCommand`、`DisplayInfo` 等纯数据窗口合同（`crates/nana-ui-platform/src/lib.rs:42-49`），并原样再导出 `nana-ui-input` 的规范输入合同（`:156`）以及跨平台路径/剪贴板/网络宿主接口。**没有任何平台特定句柄穿过这一层边界**（`window.rs:494`）。
- **`nana-window`（平台原生执行层）**：封装原始窗口句柄（`raw-window-handle`）、Windows DWM / DirectComposition、macOS AppKit / Core Animation、Mica / Acrylic / Vibrancy 系统材质、自绘无框窗口边缘缩放与样式守卫、贴底伴生窗口（`Underlay`）、原生启动画面（`NativeSplash`）以及无头网页捕获（`WebSurface`）（`crates/nana-window/src/lib.rs:1`）。
- **句柄隔离铁律**：应用/宿主拥有 `Window` 与 `Surface`；`nana-window` 的函数仅在调用期间通过 `impl HasWindowHandle` 临时借用窗口。普通控件在编译期和运行期都拿不到原生句柄；需要父窗口句柄的操作（安装系统菜单栏、弹出系统文件对话框）统一由宿主代为执行，结果通过事件异步回流。

## 依赖与邻居

在依赖关系上：`nana-ui-core`、`nana-ui-input` → `nana-ui-platform` → `nana-window` → `nana-ui`。`nana-window` 依赖 `nana-ui-platform`，只用到它的 `path::normalize_path`（文件对话框与「在访达 / 资源管理器中显示」把路径交给系统前先规范化）。`nana-ui-runtime` 与 `nana-ui-scene` 两者都不依赖——Runtime 只依赖 `nana-ui-input` 的规范输入合同，普通控件在编译期就无法触及窗口合同或原生窗口 API，只有宿主适配器 `nana-ui` 同时连接两边：

| 邻居 Crate | 协作关系 |
| --- | --- |
| `nana-ui-input` | 规范输入合同由 `nana-ui-platform` 再导出（`lib.rs:156`），宿主将原生窗口事件降维送入其中 |
| `nana-ui` | `WindowService` / `WindowHandle` 将跨线程窗口命令排队回窗口线程执行；`ResolvedWindowPresentation` 在此结算每扇窗口的最终呈现状态与 `SurfacePresentationPolicy` 色彩协商 |
| `nana-gpu` | 设备、帧与纹理后端合同（见 [GPU 与帧生命周期](/learn/gpu-frame)） |
| `winit` | `nana-window` 在原生 OS API 与 winit 之间补齐桌面级能力（Win32 无框样式守卫、Layered 窗口合成整平、macOS 无激活显示与红绿灯精确定位） |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `WindowId` | `crates/nana-ui-platform/src/window.rs:5` | 平台中立窗口 ID（`pub u64`，`PRIMARY = 0`；底层平台窗口 ID 保持私有映射） |
| `WindowDescriptor` | `crates/nana-ui-platform/src/window.rs:429` | 开窗申请单：标题、初始/最小尺寸、位置、透明、阴影、角色、`persist_key`、`tag` 等 |
| `WindowEvent` | `crates/nana-ui-platform/src/window.rs:114` | 窗口结果事件：`Ready` / `Resized` / `ModeChanged` / `CloseRequested` / `FileDialogCompleted` 等 |
| `WindowCommand` | `crates/nana-ui-platform/src/window.rs:684` | 宿主可执行窗口命令：`Move` / `Resize` / `SetFullscreen` / `SetShadow` / `OpenFileDialog` 等 |
| `WindowGeometry` | `crates/nana-ui-platform/src/window.rs:12` | 同时携带物理像素与逻辑像素的位置/尺寸、`scale_factor` 与 `maximized` 状态 |
| `DisplayInfo` / `DisplayId` | `crates/nana-ui-platform/src/window.rs:214` / `:210` | 显示器枚举信息与单进程跨枚举稳定标识 |
| `WindowSurfacePreference` | `crates/nana-ui-platform/src/window.rs:496` | 每窗口呈现目标偏好：`Auto` / `NativeWindow` / `Composition` / `RequireComposition` |
| `MousePassthroughMode` | `crates/nana-ui-platform/src/window.rs:653` | 鼠标穿透模式：`Off` / `Passthrough`（整窗穿透）/ `Forward`（透明空白处穿透、控件处恢复） |
| `MaterialEffect` / `MaterialOutcome` | `crates/nana-window/src/material.rs:36` / `:121` | 系统材质申请枚举与实际生效结果（含显式回退原因 `MaterialFallback`） |
| `NonClientRenderingStrategy` | `crates/nana-window/src/chrome.rs:71` | Windows 无框透明窗口策略：`StripFrameStyles`（默认）/ `SuppressNonClientRendering` |
| `ResolvedWindowPresentation` | `crates/nana-ui/src/presentation.rs:343` | 单扇窗口的最终结算呈现状态：有效材质 + alpha 模式 + 呈现目标 + chrome + 阴影 |
| `WindowHandle` | `crates/nana-ui/src/window_service.rs:271` | 可跨线程克隆发送的窗口控制句柄（内部通过有界通道排队回窗口线程） |
| `Underlay` | `crates/nana-window/src/underlay.rs:42` | `WindowRole::Underlay` 贴底伴生原生窗口绑定 |
| `WebSurface` / `WebFrame` | `crates/nana-window/src/web_surface.rs` | 无头网页引擎实例与其后台捕获的 RGBA8 帧 |
| `NativeSplash` | `crates/nana-window/src/splash.rs` | 两阶段启动的独立原生 Splash 窗口与系统合成器动画 |

## 内部结构

### 1. 申请与结果分离：`WindowDescriptor` / `WindowEvent` / `WindowCommand`

`crates/nana-ui-platform/src/window.rs` 将「业务申请」与「平台实际生效结果」严格拆开：

- **`WindowDescriptor`（`:429`）**：默认尺寸 `1200×800`、最小尺寸 `760×520`、阴影 `WindowShadow::Auto`（`:535-541`）。`visible` 字段仅在文档初始化成功后才真正生效（`:442`，防止初始化失败时闪现空窗）；`persist_key` 提供跨进程重启稳定的尺寸位置恢复键（恢复时经 `clamp_position_to_displays:64` 校验，若上次保存的副屏已拔掉则自动夹回最近的在线显示器）；`tag` 允许应用按业务角色标识窗口，无需靠创建顺序去猜 `WindowId`（`:464`）。
- **`WindowEvent`（`:114`）**：全屏与置顶层级请求的最终结果以 `WindowEvent::ModeChanged`（`:201`）为准，仅在状态与上次投递实际不同时才触发。

### 2. 原生材质、Win32 无框 Chrome 守卫与贴底伴生窗口（`Underlay`）

- **系统材质（`crates/nana-window/src/material.rs`）**：`MaterialEffect` 涵盖 `Solid`、`Transparent`、`Vibrancy`、`Mica`、`Acrylic`（`:36`）。调用 `apply_system_material`（`:176`）在切换材质前必定先清除旧效果，失败时立即回退至不透明实色并返回 `MaterialFallback::{PlatformDoesNotProvideNativeMaterial, NativeMaterialUnavailable}`（`:61`），绝不静默改试另一种材质。
- **Windows 无框透明窗口与样式守卫（`crates/nana-window/src/chrome.rs`）**：在 Windows 上，只要 `HWND` 保留了 `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU` 样式位，DWM 就会在客户区下方绘制原生标题栏与边框——不透明窗口能盖住它，但透明或圆角窗口就会透出底下的原生白边（`chrome.rs:5-13`）。因此默认采用 `NonClientRenderingStrategy::StripFrameStyles`（`:71`）剥离这三个框架样式位，并通过 Win32 窗口子类在 `DefSubclassProc` 之后拦截 `WM_STYLECHANGING`（`:1364`）维持掩码不被系统操作冲掉；客户区外沿 `8 px` 缩放与标题栏拖拽则由 `LiveFrameResize` / `LiveFrameMove`（`:313 / :471`）接管。macOS 侧则提供 `set_present_transaction`（`:256`，将 Metal 呈现锁入 Core Animation 事务防抖）与 `place_native_window_controls`（`:726`，将原生红绿灯按钮精确对齐到自绘标题栏矩形）。
- **贴底伴生窗口（`Underlay`，`crates/nana-window/src/underlay.rs`）**：`WindowRole::Underlay` 创建一个紧贴在父窗口正下方的普通顶层原生窗口，拥有独立的 Surface 与窗口标题，不接收输入也不抢焦点（`window.rs:302`），使 OBS 等录屏工具可以单独捕获底层实时画面而不录进上层透明 UI 面板。父窗口是唯一的几何权威，宿主在父窗口几何、模式、焦点或显隐变化时调用 `Underlay::sync`（不在每帧渲染时调用，避免 macOS Window Server 往返开销，`underlay.rs:13`）。

### 3. 单一呈现结算真相：`ResolvedWindowPresentation`

`ResolvedWindowPresentation`（`crates/nana-ui/src/presentation.rs:343`）将材质协商结果、Surface alpha 模式、呈现目标（普通交换链 vs DirectComposition）与客户区 chrome **一次性原子结算**（`:363`）。若透明材质降级为实色，客户区 chrome 同步按实色衍生，绝不会出现「画面渲染以为是不透明实色，原生边框却处于透明无框状态」的撕裂。

### 4. 为什么原始窗口句柄永远不出边界（三道防线）

| 防线层级 | 机制说明 |
| --- | --- |
| **1. 编译期依赖隔离** | `nana-ui-runtime` 不依赖 `nana-window`，普通控件在编译期根本看不见任何平台句柄类型 |
| **2. 运行期宿主代执行** | 菜单栏安装（`SetMenuBar`）、文件对话框（`OpenFileDialog`）等需要父窗口句柄的操作，控件只发出意图或数据模型，统一由宿主在窗口线程代为执行 |
| **3. 跨线程排队与瞬时借用** | 应用拿到的 `WindowHandle`（`window_service.rs:271`）只是 `(WindowId, generation, sender)` 值对象；唯一接触原生句柄的 `with_native_handle`（`:279`）仅在窗口线程回调内提供临时借用引用 |

### 5. 菜单栏双通道、文件对话框与原生 Splash

- **菜单栏双通道**：应用声明一份 `nana_ui_core::MenuBar` 模型。在 macOS 上可通过 `WindowHandle::set_menu_bar` 安装为系统顶部原生菜单栏；在自绘标题栏或 Linux 等无原生菜单的平台上，则使用 Runtime 的 [`AppMenuBar`](/components/app-menu-bar) 组件在界面内绘制。两条通道的选中项 ID 统一汇入 `nana-ui-core` 的 `push_menu_activation` / `take_menu_activations` 队列。
- **系统文件对话框**：`PathField` 等控件仅发出 `BrowseRequested`，宿主调用原生对话框并在完成后投递 `WindowEvent::FileDialogCompleted`（用户取消时返回空路径列表而非错误，`window.rs:775`）。
- **原生启动窗（`NativeSplash`）**：位于 `crates/nana-window/src/splash.rs`，在 GPU 设备就绪前展示独立无激活窗口并由系统合成器驱动动画，待主窗口接管帧 `present` 确认后无缝撤下（详见 [两阶段启动](/reference/startup)）。

### 6. 无头网页生产者：`WebSurface`

`WebSurface`（`crates/nana-window/src/web_surface.rs`）是运行在窗口视图树之外的平台网页引擎实例（macOS 为屏外 `WKWebView`，Windows 为独立 STA 线程上的 WebView2 Composition Controller + `Windows.Graphics.Capture`）：页面按 `WebSurfaceDesc::size` 排版，以至多 `max_fps`（上限 `60 fps`，边长上限 `4096 px`，`:29-31`）在后台线程将捕获到的预乘 RGBA8 帧送入应用的 `WebFrameSink`，UI 线程零转换、零阻塞（慢消费方直接丢弃后续帧而不积压队列，`:21`）。需要用户登录网页或修改页面内设置时，发送 `ShowWindow` 即可将同一个页面实例临时放进普通原生窗口交互，Cookie 与页面状态全程保留（`:11`）。

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-platform/src/lib.rs` | 平台中立合同总导出与输入合同转发 |
| `crates/nana-ui-platform/src/window.rs` | `WindowDescriptor`、`WindowEvent`、`WindowCommand`、`WindowGeometry`、`DisplayInfo`、`MousePassthroughMode` |
| `crates/nana-ui-platform/src/window_shadow.rs` | `WindowShadow` 窗口级阴影合同 |
| `crates/nana-window/src/lib.rs` | 原生窗口支持导出、`skip_taskbar`、`show_without_activation`、`settle_layered_window` |
| `crates/nana-window/src/material.rs` | `MaterialEffect`、`MaterialOutcome`、`MaterialFallback` 与平台材质能力探测 |
| `crates/nana-window/src/chrome.rs` | 无框窗口样式剥离、`WM_STYLECHANGING` 守卫子类、红绿灯定位与边缘拖拽缩放 |
| `crates/nana-window/src/{underlay,pointer,splash,menu,file_dialog,size_move}.rs` | 贴底伴生窗、指针安全换算、原生 Splash、菜单安装、文件对话框与实尺寸移动 |
| `crates/nana-window/src/web_surface.rs` | 无头 `WebSurface` 描述符、命令、事件与后台帧接收器 |
| `crates/nana-ui/src/{window_service,presentation}.rs` | `WindowService` / `WindowHandle` 跨线程队列与 `ResolvedWindowPresentation` |

## 延伸阅读

- [GPU 与帧生命周期](/learn/gpu-frame)：Surface 所有权、帧槽预留与 35 步重绘序列
- [原生窗口、标题栏与系统材质（架构）](/architecture/window) · [窗口参考合同](/reference/window)
