# 原生窗口材质

原生材质由 `nana-window` 管理，NanaUI 控件不读取窗口句柄，也不直接调用平台 API。
宿主把实现 `raw_window_handle::HasWindowHandle` 的窗口交给
`apply_system_material` / `apply_hosted_system_material`，只应用
`RuntimeProgram::window_material_mode`（或 Appearance / Vue `backdrop`）请求的那一种
效果。`Translucent` 只开窗口透明；`Vibrancy` / `Mica` / `Acrylic` 必须显式申请。
失败则实色并带上原因，不改试另一种模糊。

自绘标题栏的布局、状态和动作语义是独立合同：`nana-ui::AppTitleBar` 发出窗口
动作，Scene host / Winit 宿主负责执行。macOS 必须通过原生句柄关闭系统标题区的隐式
控件拖拽：窗口默认不可由系统标题区隐式移动，只有空白父区域收到按下事件后才启动
AppKit 原生拖拽；这些无状态平台桥接函数位于 `nana-window`，不拥有按钮或窗口
控制状态。transparent titlebar / full-size content view 与 Windows/Linux 的
undecorated window 都由 Scene host 在创建窗口时设置。`WindowSettings::system_caption`
为 true 时保留系统标题栏，供没有自绘 `AppTitleBar` 的 hosted 示例使用。

| 平台 | 可申请 | 申请失败 |
| --- | --- | --- |
| macOS 10.10+ | 业务指定 `Vibrancy` / `UnderWindowBackground` | 完全不透明的主题背景 |
| Windows 11 | 业务指定 `Mica` 或 `Acrylic`（互不自动改用另一种） | 完全不透明的主题背景 |
| Windows 10 1809+ | 业务指定 `Acrylic` | 完全不透明的主题背景 |
| Linux / 其他 | 不调用未支持的原生模糊 API | 完全不透明的主题背景 |

`MaterialOutcome` 返回实际效果和回退原因。宿主只有在获得原生模糊时才使用半透明 UI
背景。主题或材质切换都会先清除旧效果再按当前请求重试。Appearance 设置段的「实色/透明」
只覆盖前两档；系统模糊由应用显式申请。控件只消费公开 outcome。

材质不透明度默认值为 `AppearanceSettings::DEFAULT_BACKDROP_OPACITY = 0.64`，与 Lilia /
`nanavue-components` 的 `BACKDROP_OPACITY_DEFAULT` 及 `--lilia-backdrop-opacity`
一致。侧边栏透明时，`titlebar_follows_sidebar` 决定标题栏是否共用该 alpha：关闭后
标题栏保持不透明，侧栏仍可半透明。`BackdropTarget` 在侧栏与主内容区之间切换透明区域。

`run_runtime` 会为主窗口和每个工具窗口分别应用、刷新与清理材质。窗口若 `WindowSettings.transparent` 为 true，则申请 `Transparent`（透明表面与 `[0,0,0,0]` 清屏），不以主窗口 Appearance 的 Solid 覆盖。Windows 上 Transparent 会调用 `DwmExtendFrameIntoClientArea`（负边距）并设置 `WS_EX_NOREDIRECTIONBITMAP`，Scene host 在创建 WGPU Surface 之前应用该 HWND/DWM 效果。macOS Scene GPU 申请
Vibrancy 时仍报告 `NativeMaterialUnavailable`（CAMetalLayer 盖住 visual-effect）。
设备恢复会保留现有窗口，并在重建其 Surface 与 Scene painter 后按当前请求重新应用。
主窗口默认使用 NanaUI 自绘标题栏。

当前本机 macOS 26.5.2 上，WGPU Surface 使用受支持的预乘或后乘 Alpha 模式。
Workspace 真机截图确认只有一层标题栏、交通灯未遮挡内容；空白区原生拖拽、交互控件
阻止拖拽与点击切换均已验证。Hosted GPU 首帧探针读取 host 实际 `MaterialOutcome`
（macOS Scene GPU 当前为 solid，不是 Vibrancy）；三次 Surface present 不能当作
Vibrancy 验收。系统截图只捕获到材质层时，也不能作为 Hosted GPU 最终像素表现证据。

Windows 11 已在 2026-08-23（26300）上真机运行；同日稍后 `--hybrid --windows` 辅助窗
报告 `CompositeAlphaMode::PreMultiplied`，但 DWM 截图客户区仍为不透明白底，desktop Alpha
仍未被验收。Mica / Acrylic 亦未验收。Linux 当前只有条件编译结构和 GitHub Actions 目标。Acrylic 在部分 Windows 10/11 版本拖拽和 resize 时存在上游已知性能限制，因此不能把编译通过视为平台验收。
