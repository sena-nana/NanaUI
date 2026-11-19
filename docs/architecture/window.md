# 原生窗口、标题栏与系统材质

NanaUI 面向原生桌面软件：标题栏、系统毛玻璃材质与高 DPI 缩放遵循桌面操作系统语义，而非浏览器规则。

你的应用拥有 `Window` 与 `Surface`。调用 `run_runtime(WindowDescriptor::new("标题"))` 会创建主窗口、初始化进程唯一的 `GpuContext` 并启动事件循环；后续附加窗口共享同一份 `GpuContext` 与组件注册表，但每扇窗口各自独立持有 Surface、输入端点、IME 会话、文档与渲染目标。原始窗口句柄（`HWND` / `NSWindow`）严格封闭在 `nana-window` 内，普通控件永远拿不到句柄。完整技术合同见 [窗口参考](../reference/window.md)。

## 自绘标题栏与客户区 Chrome

默认配置下窗口使用自绘标题栏（`AppTitleBar`，标准高度 `36 px`）：由左侧（`leading`）、中间标题（`center`）、右侧业务槽位（`trailing`）与窗口控制按钮（`controls`）组成：

- **拖拽与按钮命中**：在标题栏空白处按下可直接拖动窗口，而栏内的按钮与交互控件优先捕获指针，不会被误拖走（注意：CSS 的 `app-region: drag` 不是 NanaUI 的拖拽合同，给普通盒子写 `drag` 不会把它变成标题栏）。设置 `.transparent(true)` 可仅去掉标题栏背景而保留内容与拖拽命中；设置 `.drag_enabled(false)`（如全屏状态）可禁用拖拽。
- **窗口控制语义**：最小化、最大化与关闭按钮只发出类型化的 `WindowChromeAction` 请求，由持有窗口的宿主执行。点击关闭按钮与系统级关闭完全一致，仅向程序派发 `WindowEvent::CloseRequested`，由应用决定立即关闭、弹窗确认还是最小化到托盘。
- **系统标题栏与边框缩放**：若不使用自绘标题栏，请设置 `WindowDescriptor::system_caption(true)` 以保留系统原生标题栏与关闭按钮（此时窗口缩放交由操作系统处理，框架不再叠加第二套边缘命中）。使用自绘无框 chrome 且窗口处于可缩放、未最大化、非全屏状态时，客户区最外侧 `8 px` 边缘自动承接窗口缩放。
- **坐标与输入法（IME）**：指针、拖拽与窗口尺寸在 API 层统一使用**逻辑像素**，物理像素仅用于配置 GPU Surface（仅当物理尺寸或呈现策略变化时才重新 configure）。焦点进入可编辑字段时窗口开启一次 IME 会话（同字段内光标移动或上下文变化走增量更新，不重开会话；候选框始终锚定在文本光标处），失焦时自动关闭。

## 系统材质与回退语义

应用通过 `RuntimeProgram::window_material_mode`（或按窗口的 `window_material_mode_for`）申请系统背景材质。框架严格区分**业务请求的材质**与**窗口实际生效的材质**——申请失败时立即回退到可读的不透明主题实色背景并返回明确原因，绝不静默替换成另一种材质家族：

| 平台 | 可申请的系统材质 | 不可用或失败时的回退结果 |
| --- | --- | --- |
| macOS 10.10+ | 指定的 `Vibrancy` / `UnderWindowBackground` | 不透明主题实色背景（附带 `MaterialFallback`） |
| Windows 11 | 指定的 `Mica` 或 `Acrylic` | 不透明主题实色背景（附带 `MaterialFallback`） |
| Windows 10 1809+ | 指定的 `Acrylic` | 不透明主题实色背景（附带 `MaterialFallback`） |
| Linux | 无系统级背景模糊 | 不透明主题实色背景（`PlatformDoesNotProvideNativeMaterial`） |

使用材质与阴影时需注意四条边界：

1. **透明（`Translucent`）不等于系统模糊**：`Translucent` 仅开启透明合成表面；若协商不到透明表面则回退为不透明实色，不会擅自改试 Mica 或 Acrylic。
2. **`WindowDescriptor::transparent` 是创建期的终身合同**：若创建窗口时设为 `true`，该窗口此后不能再切回不透明实色。若希望让用户在设置里自由切换「实色 / 透明材质」，请让描述符的 `transparent` 保持 `false`，改由 `window_material_mode_for` 动态返回目标材质。
3. **单一呈现真相 `ResolvedWindowPresentation`**：每扇窗口实际拿到的材质、Surface alpha 模式、客户区 chrome 与窗口阴影统一结算在 `ResolvedWindowPresentation` 中（通过 `RuntimeProgramContext::presentation()` 读取），确保画面渲染与原生边框对「当前是否透明」的认知永远一致。
4. **原生窗口阴影 vs 帧内 `DropShadow`**：窗口级阴影由平台窗口层绘制，运行期可通过 `WindowCommand::SetShadow` 切换（从 `presentation().shadow()` 读取生效状态，窗口显示前为 `Pending`）；若透明窗口已经在帧缓冲内自行绘制了根卡片阴影，应将窗口阴影设为 `WindowShadow::None` 以免叠出双层阴影。

## 多窗口、伴生窗口与嵌入式宿主

应用通过 `context.windows()` 获取窗口服务，返回的 `WindowHandle` 可安全发送到工作线程（内部将命令排回窗口线程执行）：

```rust
// 在工作线程中创建辅助窗口；异步上下文中也可直接 .await
let window = service.create_window(WindowDescriptor {
    title: "Notes".into(),
    tag: Some("notes".into()),
    initial_size: (480.0, 320.0),
    minimum_size: (240.0, 120.0),
    system_caption: true,
    ..Default::default()
}).wait()?;
```

- **用 `tag` 标识窗口角色**：不要依赖窗口创建顺序去猜测 `WindowId`；在描述符中设置 `tag`，构建文档时通过 `window_tag()` 读取即可。新窗口需等隐藏的原生窗口、Surface、输入端点与应用文档全部就绪后才完成创建（任一步失败即自动回滚，不会发出 `Ready`）；窗口位置与尺寸可通过 `WindowDescriptor::persist_key` 自动持久化恢复（全屏与最小化状态不写入恢复记录）。
- **父属伴生子窗口（`WindowDescriptor::parent`）与贴底窗口（`WindowRole::Underlay`）**：普通辅助顶层窗口通过 `WindowDescriptor::parent` 维持在属主窗口之上并继承层级，与共享客户区且排在属主正下方的 `Underlay` 语义严格分离；`Underlay` 在被父窗口盖住时仍独立出帧，专为 OBS 等录屏软件「单独捕获底层画面而不录进上层 UI 面板」的场景设计（详见 [窗口参考](../reference/window.md)）。
- **原生屏幕吸色（`ColorSampler`）**：`ColorField` 等控件所需的桌面吸色统一通过原生边界实现（macOS `NSColorSampler`、Linux Wayland 严格走 XDG Portal v2+ 而不回退 X11 / X11 根窗口读取、Windows 逐显示器 DPI 放大镜），宿主不支持时由 `set_eyedropper_supported(false)` 隐藏入口并记入 `window.color_sampler.failed` 诊断事件。
- **嵌入式宿主（`EmbeddedRuntime`）**：当外部宿主已拥有事件循环与 GPU 设备时，使用 `EmbeddedRuntime` 并通过 `HostedGpuShared::from_device` 注入现有设备；如需在回调中临时访问原生窗口句柄，使用 `with_native_handle`（借用仅在回调期间有效，切勿保留裸指针）。

## 菜单栏、系统文件对话框与网页内容

1. **菜单栏（原生菜单与 `AppMenuBar`）**：菜单模型 `MenuBar` 定义在 `nana-ui-core`。macOS 上可交由宿主安装到系统顶部菜单栏；在自绘标题栏或 Linux 等无原生菜单的平台上，可使用 [`AppMenuBar`](/components/app-menu-bar) 组件将同一份 `MenuBar` 画在界面内。两条路径共用 `take_menu_activations()` 激活队列，应用只需按菜单项 ID 分发业务动作。
2. **系统文件对话框**：因需要父窗口原生句柄，文件对话框统一由宿主打开（如 `PathField` 发出浏览请求后，结果经 `WindowEvent::FileDialogCompleted` 异步回流；用户取消时路径列表为空而非报错，每扇窗口同时最多打开一个对话框）。
3. **应用内网页（`BrowserView`）**：作为保留树上的布局与无障碍锚点，由宿主在 macOS 上挂载原生 `WKWebView` 子视图（Windows 与 Linux 返回明确的 `Unsupported`）。遇到圆角、非平移变换、透明度/滤镜组或被后续 Runtime 内容覆盖时，宿主会暂时隐藏原生子视图以免遮挡浮层与菜单。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/reference/window">
    <p class="next-step-link">窗口参考合同</p>
    <p class="next-step-caption">阴影、自绘 chrome、材质协商、多窗口与贴底窗口的完整规范。</p>
  </a>
  <a class="next-step" href="/architecture/">
    <p class="next-step-link">架构总览</p>
    <p class="next-step-caption">回到保留树到宿主 Surface 的全局架构图。</p>
  </a>
  <a class="next-step" href="/architecture/gpu">
    <p class="next-step-link">实时画面</p>
    <p class="next-step-caption">在同一份 GPU 设备上将实时纹理挂入窗口树中。</p>
  </a>
</div>
