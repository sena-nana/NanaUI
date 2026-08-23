# 窗口

NanaUI 画的是桌面窗口，不是网页标签页。标题栏、图标、系统材质和多窗口都按桌面软件来，不按浏览器来。

## 标题栏

默认是自绘标题栏：左侧内容、中间标题、右侧窗口按钮。空白处拖动窗口，按钮不会被拖走。macOS 给系统红黄绿让出位置；Windows 和 Linux 用自绘的最小化、最大化、关闭。

关闭、最小化、最大化是窗口动作。控件只发出「请关这个窗口」，真正执行的是宿主。普通控件拿不到窗口句柄，也不能自己去调系统 API。

## 图标

任务栏、exe、Dock 上的图标是应用身份，不是界面里的小图标字形。可以在创建窗口时设置，也可以之后再改。

- JavaScript：`Nana.windows.setApplicationIcon`，或在创建窗口时传入 `icon`
- Windows 可执行文件还可以在构建时打进图标
- macOS Dock 由窗口层设置

不设的话会用默认标记，不至于空白，但也不要拿它当产品品牌。

## 材质

可以申请系统模糊或透明，让侧栏、标题栏和桌面长在一起。只能申请你点名的那一种：macOS 的 Vibrancy、Windows 11 的 Mica 或 Acrylic、Windows 10 的 Acrylic。失败就回到实色，并告诉你原因，不会自动改试另一种。

Linux 没有这类系统模糊。透明窗口和系统模糊是两件事：透明只让窗口透过去，模糊必须显式要。

当前 macOS 在 GPU 窗口上申请 Vibrancy 可能拿不到系统效果（金属层会盖住系统材质）。Windows 的透明客户区和 Mica / Acrylic 要以真机为准。编译通过不等于已经在那台机器上看起来对。

## 多窗口

可以再开工具窗口、预览窗口。它们共用同一份图形设备和同一套 Vue。关主窗口即退出；关辅助窗口只拆那一扇。

窗口位置、最大化、上次开在哪块屏幕，由应用自己记。框架不会替你选配置目录，也不会悄悄写盘。

## 内部如何工作

原生材质由 `nana-window` 管理。宿主把带窗口句柄的窗口交给 `apply_system_material` / `apply_hosted_system_material`，只应用业务点名的那一种：`RuntimeProgram::window_material_mode`，或外观设置 / Vue `backdrop`。`Translucent` 只开窗口透明；`Vibrancy` / `Mica` / `Acrylic` 必须显式申请。`MaterialOutcome` 返回实际效果和失败原因。主题或材质切换会先清掉旧效果再按当前请求重试。

| 平台 | 可申请 | 申请失败 |
| --- | --- | --- |
| macOS 10.10+ | 指定的 Vibrancy / UnderWindowBackground | 完全不透明的主题背景 |
| Windows 11 | 指定的 Mica **或** Acrylic（互不改用另一种） | 完全不透明的主题背景 |
| Windows 10 1809+ | 指定的 Acrylic | 完全不透明的主题背景 |
| Linux / 其他 | 不调用未支持的模糊 API | 完全不透明的主题背景 |

标题栏是另一份合同。`AppTitleBar` 发出 drag / minimize / toggle-maximize / close；Scene host 去执行。macOS 默认禁止系统标题区抢鼠标，只有空白父区域按下后才通过 `nana-window` 启动原生拖拽，按钮会先吃到事件。Windows / Linux 关系统 decorations，由标题栏自己画三枚按钮。

`WindowChromeState` 绑在明确的 `WindowId` 上。单窗口可以用默认入口（只认收到的第一扇窗）；多窗口要在 `WindowCommand::Open` 拿到 ID 后用 `for_window` 各建一份。关窗后不会自动接管别的窗口。

`run_runtime` 会给主窗口和每个工具窗口分别应用、刷新和清理材质。`WindowSettings.transparent` 为 true 时申请透明表面，不以主窗口外观的实色去盖。设备恢复后按当前请求重新应用材质。

默认不透明度 0.64。侧栏透明时，`titlebar_follows_sidebar` 决定标题栏是否跟着透明。`BackdropTarget` 在侧栏和主内容之间切换透明区域。
