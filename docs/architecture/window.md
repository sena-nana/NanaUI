# 窗口

NanaUI 画的是桌面窗口。标题栏、系统材质和缩放按桌面软件来，不按浏览器来。

你的应用拥有 Window 和 Surface。`run_runtime(WindowDescriptor::new("标题"))` 创建主窗口、这一进程里的那一份 `GpuContext`，并开始事件循环。普通控件拿不到 `HWND` / `NSWindow`。系统材质、标题栏拖拽、客户区 chrome 和缩放属于 `nana-window`。

界面仍走 `UiWorld` → `UiScene` → `SceneWgpuPainter` → 这扇窗口的 Surface。附加窗口共用注册表和这一份 Device / Queue。每扇窗口自己持有 Surface、输入、输入法、文档和渲染目标。全文在 [窗口](../reference/window.md)。

## 标题栏属于哪一层

默认是自绘标题栏：左侧内容、中间标题、右侧窗口按钮。空白处拖动窗口。按钮先吃到指针，不会被拖走。

栏上的文字和操作放在 `leading` / `center` / `trailing` / `controls`。`transparent(true)` 只去掉栏的背景，内容和命中还在。不必为此把整窗做成透明。`drag_enabled(false)` 禁止从栏内空白和文字启动拖动。

关闭、最小化和最大化是窗口动作。控件发出 `WindowChromeAction`。持有窗口的宿主去执行。关闭和系统关闭一样，只向程序发 `CloseRequested`，由你决定关不关。

没有自绘标题栏时，设 `WindowDescriptor::system_caption(true)`。这样无框窗口不会失去关闭按钮。系统标题栏的窗口把缩放交给平台，不叠第二套命中。

自绘 chrome、可缩放、未最大化、也不是全屏时，客户区最外 8px 可以缩放。

L1 的 `app-region` 不是拖拽合同。一个盒子写上 `drag`，不会变成标题栏。

指针、拖拽和缩放用逻辑坐标。物理像素只用于 Surface。拖动边框时事件循环继续跑。物理尺寸或 present 策略变了，才配置 Surface。同一尺寸会跳过。

焦点进入可编辑字段时，窗口更新一次输入法：用途、光标盒，以及非密码字段周围的文本。之后光标或周围文本变了，走更新。失焦则关闭。候选框相对光标，不相对系统非客户区。

## 材质失败时你拿到什么

通过 `RuntimeProgram::window_material_mode` 申请一种系统效果。Appearance 在宿主提供时，可以指定 Mica、Acrylic 或 Vibrancy。失败回到实色，并给出原因。不会改试另一种。

| 平台 | 可申请 | 失败时 |
| --- | --- | --- |
| macOS 10.10+ | 指定的 Vibrancy / UnderWindowBackground | 不透明主题背景 |
| Windows 11 | 指定的 Mica 或 Acrylic | 不透明主题背景 |
| Windows 10 1809+ | 指定的 Acrylic | 不透明主题背景 |
| Linux | 无系统模糊 | 不透明主题背景 |

返回的是实际应用的结果，或一次明确的回退。`Translucent` 只打开窗口透明，不等于模糊。透明和系统模糊是两件事。

透明若协商不到能透出的表面，只能回到不透明实色，并带 `MaterialFallback`。它不会改去试 Mica 或 Acrylic。

`WindowDescriptor::transparent` 是创建时的终身合同。它为真之后，材质不能再切回实色。想给用户留实色开关，让描述符保持 `false`，用 `window_material_mode_for` 报当前这扇窗口要的材质。

每扇窗口「现在呈现什么」只记在 `ResolvedWindowPresentation`。业务请求的材质，和 surface 协商之后真正拿到的材质，是两列。chrome 跟实际拿到的材质一起定下来。不会出现画面认为实色、原生 chrome 却认为透明。你从 `RuntimeProgramContext::presentation()` 读这份结果。

原生材质由 `nana-window` 执行。主题或材质切换会先清掉旧效果，再按当前请求重试。设备恢复后按当前请求重新应用。编译通过不等于那台机器上的材质看起来对。

窗口阴影和界面里的 `DropShadow` 是两套权威。前者由窗口层决定，后者画在帧缓冲里。打开之后换阴影，用 `WindowCommand::SetShadow`。你读 `presentation().shadow()` 得到实际结果。窗口还没显示时，这个结果是 `Pending`。透明窗口如果自己又画了卡片阴影，应设 `WindowShadow::None`，否则会叠成两层。

## 多扇窗口

普通应用用 `context.windows()` 创建窗口。返回的 `WindowHandle` 可以发给工作线程。它不持有原生窗口。操作排回窗口线程。尺寸和位置是逻辑坐标。

```rust
// 工作线程；在异步代码中也可以用 .await。
let window = service.create_window(WindowDescriptor {
    title: "Notes".into(),
    initial_size: (480.0, 320.0),
    minimum_size: (240.0, 120.0),
    system_caption: true,
    ..Default::default()
}).wait()?;
```

描述符的字段以后还会增加。构造时用 `WindowDescriptor::new(...)` 或 `..Default::default()` 收尾。不要逐字段写满。

这扇窗口是哪种文档，用 `tag` 声明。不要靠创建顺序去对 id。构建文档时从 `window_tag()` 读回来。窗口关掉之后是 `None`。

创建要等隐藏的原生窗口、Surface、输入和应用文档都就绪才完成。失败会回滚，不发送 `Ready`。程序自己选 `WindowId` 再 `WindowCommand::Open`，只留给框架适配器。普通应用不用它来区分窗口。

`WindowRole::Underlay` 是贴在父窗正下方的伴随窗口：同一块客户区，跟着父窗动和藏，不收输入，被父窗盖着也照常出帧，录屏软件能单独捕获它。用来把画面和界面拆成两扇原生窗口，看起来仍是一扇。细节见参考文档的「贴底窗口」。

关掉一扇窗口，只释放这扇和它的原生子窗口。standalone 的最后一扇关掉后退出。需要关主窗口就退出整个应用时，你自己返回退出。

已有事件循环时用 `EmbeddedRuntime`。它不创建、也不退出宿主的事件循环。已有的设备用 `HostedGpuShared::from_device` 注入。不要再申请第二个 Device。`with_native_handle` 只在回调期间借用原始句柄。不能把指针留下来，也拿不到窗口的所有权。

全屏和置顶的有效状态经 `WindowEvent::ModeChanged` 上报。不要自己记一份请求镜像。全屏是无边框盖住当前视频模式，不改显示器分辨率。

位置和尺寸可以交给 `WindowDescriptor::persist_key`。全屏和最小化不写入这份恢复。

系统若要求减少动态效果，由你缩短过渡或关掉位移。框架不改写已经声明的动画。无论是否减少动态效果，合成采样都不写回基础样式，呈现停在 overlay 上。

## 菜单、对话框和网页内容

原生菜单栏是唯一画不进界面树的桌面 chrome。macOS 上它属于应用，住在系统菜单条里。模型在 `nana-ui-core`。应用声明，持有窗口的宿主安装。框架只告诉你选中了哪一个 id。这个 id 是什么意思，由你决定。平台做不到时如实报 `Unavailable`，不假装装上了。

系统文件对话框也要父窗口句柄，所以同样由宿主打开。控件只发出浏览请求。结果经 `WindowEvent::FileDialogCompleted` 异步回来。取消不是错误：没有 error，路径为空。每个窗口同时只能有一个对话框。

`runtime::BrowserView` 是保留树上的布局和可访问性节点。工具条、地址和业务状态仍由 Runtime 承载。宿主只为当前窗口文档里、id 对得上的节点创建原生内容。当前 macOS 后端是主线程上的 `WKWebView` 子视图，绑定父窗口。Windows 和 Linux 返回明确的不可用，不创建占位浏览器。

原生子视图只做有限的平移和矩形裁剪，跟随着窗口的逻辑坐标和滚动。圆角、非平移变换、透明度或滤镜组，以及随后画上去的 Runtime 内容，会暂时把网页隐藏，免得它挡住菜单和浮层。原生网页不进 Runtime 的离屏截图。它不是 `GpuTextureView`，也不是第二套 Device。

## 不要做的

不要让普通控件拿着窗口句柄去调系统 API。不要在界面画完之后把原生网页盖在窗口上。材质失败时读实际结果或那一次明确回退。框架不会改去试另一种效果。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/reference/window">
    <p class="next-step-link">窗口合同</p>
    <p class="next-step-caption">阴影、标题栏、材质和多窗口的全文。</p>
  </a>
  <a class="next-step" href="/architecture/">
    <p class="next-step-link">架构</p>
    <p class="next-step-caption">回到这条路径的总览。</p>
  </a>
  <a class="next-step" href="/architecture/gpu">
    <p class="next-step-link">实时画面</p>
    <p class="next-step-caption">同一份设备上，纹理怎样留在树上。</p>
  </a>
</div>
