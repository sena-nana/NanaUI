# 原生窗口材质

原生材质由 `nana-window` 管理，NanaUI 控件不读取窗口句柄，也不直接调用平台 API。宿主先创建透明窗口，再把实现 `raw_window_handle::HasWindowHandle` 的窗口交给 `apply_system_material`。

自绘标题栏的布局、状态和动作语义是独立合同：`nana-ui::AppTitleBar` 发出窗口
动作，Scene host / Winit 宿主负责执行。macOS 必须通过原生句柄关闭系统标题区的隐式
控件拖拽：窗口默认不可由系统标题区隐式移动，只有空白父区域收到按下事件后才启动
AppKit 原生拖拽；这些无状态平台桥接函数位于 `nana-window`，不拥有按钮或窗口
控制状态。transparent titlebar / full-size content view 与 Windows/Linux 的
undecorated window 都由宿主创建窗口时设置，材质应用、清理和失败回退流程保持
不变。

| 平台 | 首选 | 回退 |
| --- | --- | --- |
| macOS 10.10+ | Vibrancy / `UnderWindowBackground` | 完全不透明的主题背景 |
| Windows 11 | Mica | Acrylic，再失败则使用不透明背景 |
| Windows 10 1809+ | Acrylic | 完全不透明的主题背景 |
| Linux / 其他 | 由合成器决定，不调用未支持的原生 API | 完全不透明的主题背景 |

`MaterialOutcome` 明确返回实际应用的效果和回退原因。宿主只有在获得原生效果时才使用半透明 UI 背景；不支持或调用失败时改用完全不透明背景，保证内容可读。主题深浅切换与 Appearance 中的「窗口材质」切换都会先清除旧效果，再按偏好重新应用材质。Appearance 设置段暴露实色/透明、透明区域、标题栏跟随与材质不透明度；控件只消费公开 outcome，不接触窗口句柄。

材质不透明度默认值为 `AppearanceSettings::DEFAULT_BACKDROP_OPACITY = 0.64`，与 Lilia /
`nanavue-components` 的 `BACKDROP_OPACITY_DEFAULT` 及 `--lilia-backdrop-opacity`
一致。侧边栏透明时，`titlebar_follows_sidebar` 决定标题栏是否共用该 alpha：关闭后
标题栏保持不透明，侧栏仍可半透明。`BackdropTarget` 在侧栏与主内容区之间切换透明区域。

`run_runtime` 会为主窗口和每个工具窗口分别应用、刷新与清理材质。业务通过
`AppearanceSettings` 声明窗口材质与 backdrop 偏好，不接触原生句柄。macOS Scene
GPU 路径固定走 `apply_hosted_system_material` 的实色回退（`NativeMaterialUnavailable`）：
CAMetalLayer 会盖住 AppKit visual-effect，在窗口于 WGPU layer 之后另有 content view
之前不会返回 Vibrancy。设备恢复会保留现有窗口，并在重建其 Surface 与 Scene painter 后
重新应用材质。主窗口默认使用 NanaUI 自绘标题栏。

当前本机 macOS 26.5.2 上，WGPU Surface 使用受支持的预乘或后乘 Alpha 模式。
Workspace 真机截图确认只有一层标题栏、交通灯未遮挡内容；空白区原生拖拽、交互控件
阻止拖拽与点击切换均已验证。Hosted GPU 首帧探针读取 host 实际 `MaterialOutcome`
（macOS Scene GPU 当前为 solid，不是 Vibrancy）；三次 Surface present 不能当作
Vibrancy 验收。系统截图只捕获到材质层时，也不能作为 Hosted GPU 最终像素表现证据。

Windows 与 Linux 当前只有条件编译结构和 GitHub Actions 目标，尚未获得真实 Windows 10、Windows 11 和 Linux 合成器运行证据。Acrylic 在部分 Windows 10/11 版本拖拽和 resize 时存在上游已知性能限制，因此不能把编译通过视为平台验收。
