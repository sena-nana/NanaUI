# 两阶段启动

Issue #225。启动分两个呈现阶段，但只有一套 UI 引擎：

1. **Early Splash**：GPU 设备和程序都还不存在时，宿主显示一个独立的小型原生 Splash 窗口（编进二进制，或从包的 `early-splash` pack 读一次），可选一种由系统合成器自己推进的动画。
2. **应用自己的界面**：宿主已经能挂载、布局、派发事件并绘制普通文档时（`UiReady`），调用 `RuntimeProgram::initialize`。应用在这里挂加载页，或直接挂主界面，然后说明“这份内容可以接管”。宿主确认这份内容的首帧已经交给合成器，才撤下 Logo。

**能力交接点和画面交接点是分开的。** `UiReady` 只表示可以画普通界面，不表示业务初始化完成；Logo 在接管内容的首帧确认之后才撤。

两者都是可选的。不配置 splash 时，宿主不创建任何图层、窗口、设备或 timer，窗口仍在 `initialize` 之后显示，和以前一样。

## 用法

```rust
static LOGO: &[u8] = include_bytes!("../assets/logo.png");

NanaApplication::builder(identity)
    .early_splash(
        SplashSpec::new(SplashLogo::png(LOGO))
            .with_logo_size(128.0, 128.0)
            .with_animation(SplashAnimation::Pulse),
    )
    .run::<RuntimeApplication<App>>(WindowDescriptor::new("App"))
```

不走 builder 的宿主，以及包装 `run_runtime` 的前端（Vue），用 `nana_ui::with_startup(StartupOptions { splash: Some(spec) }, || run_runtime::<P>(settings))`。

完整例子见 `crates/nana-ui/examples/startup-splash.rs`：加载页 → 后台线程模拟业务 → 切主界面。`--probe` 让它变成验收探针，交接后空闲一秒自行退出，打印一行 JSON 记录，记录违反合同时返回非零。

## 第一阶段：Early Splash

`SplashSpec` 就是全部配置：

| 字段 | 含义 |
| --- | --- |
| `logo` | `SplashLogo::png(&'static [u8])`：编进二进制的 PNG。`SplashLogo::packaged("nana://res/…")`：包里 `early-splash` pack 的一个条目（见下文）。都不发网络请求，不扫描文件，不走资源管理器 |
| `logo_size` | 逻辑客户区尺寸，同时也是独立 Splash 窗口和 Logo 内容框的尺寸；Logo 按比例缩放后居中 |
| `background` | `System`（默认主题调色板在当前**系统**明暗下的背景色）、`Color(..)`、`Transparent`（只画 Logo，给透明窗口用）。splash 出现时程序还不存在，读不到它的主题：主题不跟随系统明暗的应用（例如 Vue 宿主默认 `Light`）应传 `Color(..)` 为自己的背景色，否则交接时底色会变 |
| `animation` | `None`、`FadeIn`（默认，淡入一次后保持）、`Pulse`（呼吸）、`Rotate`（旋转） |

Logo 的上限：编码后 ≤ 1 MiB（与打包器 `early-splash` pack 的上限一致），最长边 ≤ 1024 像素，解码后 ≤ 4 MiB。只读 PNG 头做检查，不解码像素。两种来源用同一套上限。

这一阶段不提供文字、进度条、布局、控件、脚本、shader 或动画回调。产品名要出现在画面里，就合进 Logo 图片。动画交给平台合成器推进，本进程没有任何逐帧回调或 timer；动画帧再多，NanaUI 的工作量也不增加（`SplashWork::animation_submissions` 恒为 1）。

### 平台

| 平台 | 实现 | 动画 | 交接 | 验证 |
| --- | --- | --- | --- | --- |
| macOS | 独立无标题栏、透明背景的 `NSWindow`，内容为 `CALayer`；窗口忽略鼠标事件，交接期间输入仍归主窗口 | `CABasicAnimation`，由 render server 推进 | 目标帧以 `presentsWithTransaction` present，同一轮里移除独立 Splash；drawable 与移除落在同一个 Core Animation 提交里 | 本机真窗口，60 fps 录屏逐帧检查 |
| Windows，普通 HWND | topmost `CreateTargetForHwnd(hwnd, TRUE)` 上的 DirectComposition 视觉树；Logo 与背景由一个短生命周期 D3D11 设备上传一次；独立 HWND 对 `WM_NCHITTEST` 返回 `HTTRANSPARENT`，交接期间输入仍归主窗口；子类跟随 `WM_SIZE` / `WM_DPICHANGED` 重新居中 | `IDCompositionAnimation`（透明度、旋转），由 DWM 推进 | 目标帧 present → 等它的 GPU 工作完成（`on_submitted_work_done`）→ `DwmFlush()` 一次 → 移除视觉并提交 → 释放 D3D11 / DComp | **只交叉编译检查过，未经 Windows 真机验证** |
| Windows，合成路径（`WS_EX_NOREDIRECTIONBITMAP`） | 不显示，`Skipped(CompositionTarget)`：这扇窗口的 topmost 槽已经被 NanaUI 自己的合成树占用 | — | — | — |
| Linux 及其他 | 不显示，`Skipped(PlatformUnsupported)` | — | — | — |

D3D11 设备只用来上传两张小图，与 wgpu 的 adapter / backend 选择无关，交接时和合成树一起释放。

平台不支持、合成路径、隐藏启动（`WindowDescriptor::visible = false`，例如托盘启动）或嵌入宿主时，splash 一律跳过，不分配任何资源，`UiReady` 与接管合同照常工作。

### 降级

实际结果在 `StartupStatus::splash`（`SplashOutcome`）里，不会把请求当成结果：

- `Shown { animation: Applied(..) }`：动画在合成器里跑；
- `Shown { animation: Static { requested, reason } }`：静态 Logo。`reason` 是 `ReducedMotion`（系统要求减少动态效果）或 `NativeAnimationFailed`（合成器拒绝了动画）；`requested == None` 时 `reason` 为 `None`；
- `Skipped(..)`：没有配置，或上面的跳过条件；
- `Failed(Logo(..) | Package(..) | Native(..))`：坏 Logo、超限、包里的 Logo 读不到或平台调用失败。应用照常启动，只是没有 splash。

### 从 `early-splash` pack 读取 Logo

需要 `nana-ui` 的 `packaged-resources` feature 和 `NanaApplicationBuilder::resource_packs(..)`（Issue #226）：

```rust
NanaApplication::builder(identity)
    .resource_packs(ResourcePackOptions::new().trust(trust).loose_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")))
    .early_splash(SplashSpec::new(SplashLogo::packaged("nana://res/splash/logo.png")))
    .run::<RuntimeApplication<App>>(WindowDescriptor::new("App"))
```

```toml
[[resources.packs]]
name = "splash"
class = "early-splash"
include = ["splash/**"]
```

这次读取只碰一个 pack：

- manifest 在 `start()` 时已经读过，并按 `trust` 校验过签名；这里不再读 manifest，也不列目录。
- 路径按 manifest 的前缀路由到唯一一个 pack（与 `nana://res/` 同一条最长前缀规则）。这个 pack 必须是 `early-splash` 类，否则报 `NotEarlySplash`，那个 pack 根本不会被打开。
- `early-splash` pack 不加密，所以不调用 `KeyProvider`，也就不会有取钥匙的网络请求。
- pack 按 manifest 钉住（pack id、TOC hash、类别、代次），发布者签名按 `trust` 校验。
- reader 读完 4 KiB header 就检查文件大小：超过 4 MiB（`EARLY_SPLASH_MAX_PACK_BYTES`，打包器也拒绝造出更大的 early-splash pack）就报 `Pack { code: 21 }`，TOC 不读。
- 读取走 `nana://res/` 挂载的同一个缓存 reader 和诊断，之后再从这个 pack 读资源不会重开。
- 先查 TOC 里的条目长度，超过 1 MiB 就报 `Logo(TooLarge)`，条目数据一个字节都不读。数据读出来后逐块校验 hash，全部通过才交给 PNG 头检查。

读取发生在事件线程上，位置在建窗和发起设备请求之后、窗口显示之前，每次启动最多一次。隐藏启动和不支持原生 splash 的平台会跳过读取。Windows 与 macOS 使用无激活的独立原生 Splash 窗口；它只跟随主窗口的中心点和 DPI，不读取或跟随主窗口大小。开发布局（从 `target/` 运行、没有 manifest）改读 `loose_root` 下的同一个逻辑路径，只读这一个文件；那里没有 pack 类别可查，路径是否真在 `early-splash` pack 里，要到打包后才能确认（见下面的自检）。

读不到时，结果是 `Failed(Package(SplashPackageError))`：

| 变体 | 含义 |
| --- | --- |
| `Unsupported` | 没开 `packaged-resources` |
| `NotMounted` | 没配 `resource_packs`，或 manifest 缺失、无效（manifest 自己的故障另有记录） |
| `InvalidUrl` | 不是 `nana://res/` 的合法逻辑路径 |
| `NotFound` | 没有 pack 认领这个路径，或 pack 里没有这个条目 |
| `NotEarlySplash { pack, class }` | 路径属于别的类别的 pack，没有打开 |
| `Pack { pack, code, reason }` | pack 打不开或条目校验失败，`code` 即 `PackError::code` |
| `Io` | 开发布局下读 `loose_root` 失败 |

`StartupWork::splash_logo_read` 记录这次读取的耗时（内嵌 Logo 时为 `None`）。诊断：gauge `host.startup.splash_logo_read`；读取失败时记一条 `host.splash_logo_failed { code }` 故障。

打包自检（`NANA_PACKAGE_VALIDATE=1`）会用宿主同一套逻辑读这个 Logo 并检查 PNG 头，结果写进 `startup.early-splash-logo`。`nana-packager validate --run` 会把它单独报出来。

减少动态效果：Windows 读 `SPI_GETCLIENTAREAANIMATION`；macOS 读 `NSWorkspace.accessibilityDisplayShouldReduceMotion`（只读启动时的值）。

## 第二阶段：UiReady

`RuntimeProgram::initialize`（`ApplicationState::initialize` + `build`）被调用的时刻就是 `UiReady`：窗口、surface、设备、scene painter 和文本系统都已就绪，可以挂载、布局、派发、绘制普通文档。它不要求加载全部组件、预热全部字体或创建全部 pipeline。

`initialize` 只做第一屏需要的事：建加载页（或主界面），把业务工作交给 `run_task` 或自己的线程，然后返回。业务状态不必在这之前存在，`initialize` 本身就是这个信号的接收者。后台工作只回传数据和进度，由 `update` 在窗口线程改文档。

`initialize` 返回的 startup 消息：有 splash 时在之后几轮事件循环里按 2 ms / 64 条的批次处理，不会在一轮里同步清空；`Immediate` 接管等它们处理完才发出，所以撤下 Logo 的那一帧已经包含它们的效果。没有 splash 时仍在窗口首次显示前同步处理，和以前一样。

框架不提供业务启动 DAG、服务容器或“业务完成百分比”。加载页显示什么、何时切到主界面，都是应用的普通行为。

## 接管

`RuntimeProgram::startup_takeover()`（`ApplicationState` 同名）在 `initialize` 返回后读一次：

- `Immediate`（默认）：`initialize` 建的主窗口文档就是接管内容；
- `Deferred`：Logo 保持，直到应用调用 `context.startup().take_over(ticket)`。没有 splash 的平台（Linux、隐藏启动、坏 Logo）也一样等这次请求：窗口照常绘制，ticket 一直有效，同一份应用代码在各平台行为一致。

```rust
let startup = context.startup();
let ticket = startup.status().ticket.expect("UiReady 之后才有 ticket");
startup.take_over(ticket)?;          // 任意线程都可以调用
startup.cancel_takeover(ticket)?;    // 撤回；这张 ticket 作废
```

- **代次**：`cancel_takeover` 让当前 ticket 作废；接管帧已经 present 之后（Windows 上正在等合成器取走它）再撤回会被拒绝（`AlreadyHandedOff`）。取消之前发出的任务稍后带着旧 ticket 回来，会被拒绝（`StartupError::StaleTicket`），不会用没人要求的内容接管。新请求要用 `status()` 里的新 ticket。
- **目标帧**：请求记录主窗口此刻的 flush 序号。只有在这之后 flush、并且 **present 成功** 的主窗口帧才算数。旧帧、跳过的帧（`Skipped` / `Retry`）和失败的帧都到不了这个判断；设备或 surface 重建期间窗口不 present，重建后的第一帧才算。
- **接管之前**：主窗口已创建并用于 Surface/GPU 初始化，但保持隐藏；宿主照常为它布局、排版、settle 文档，但不 present。独立小 Splash 保持可见，有了请求，下一帧直接 present；交接时先显示主窗口，再按平台合成器条件移除 Splash，不会闪出大尺寸空窗口。
- **其他窗口**：不受影响，照常创建和绘制。

阶段变化（请求、撤回、交接完成）通过 `RuntimeProgram::startup_changed(status, ctx)` 送达，也随时可以从 `RuntimeProgramContext::startup().status()` 读到。`WindowEvent::Ready` 仍是每扇窗口一次，与启动阶段无关。

## 失败、取消与清理

- **GPU 或最小引擎初始化失败**：不发 `UiReady`，splash 撤下，窗口关闭，`run` 返回 `HostedRunError::Startup`，并记录 `host.startup_failed`。不会一直转圈。
- **启动期间关窗**：`UiReady` 之前关闭窗口会取消启动，窗口立即隐藏。事件循环在设备请求期间一直在转，所以关窗随时有效。平台设备请求一旦开始就无法中途取消，结果到达后直接丢弃；设备线程持有的 surface 在宿主已退出时留到进程结束，不在窗口线程之外释放窗口。
- **`UiReady` 之后、交接之前退出或关主窗**：splash 在窗口销毁前撤下并释放（不等合成器）。
- **设备 / surface 重建**：交接前发生时，等新 surface generation 上的帧；不会重新调用 `initialize`，也不会重跑业务初始化。
- **交接之后的业务错误**：由应用的普通界面处理，不会退回第一阶段。

splash 的图层、视觉、位图、D3D11 设备、子类和动画都由同一个 `NativeSplash` 持有，成功交接和所有失败路径都经它释放；`SplashWork::live_resources` 在交接后为 0。

## 启动线程

独立宿主（`run_runtime`）在窗口线程上创建窗口、实例和 surface，先把 adapter 选择、设备请求和 scene painter 的 pipeline 编译交给 `nana-startup-gpu` 线程，再挂 splash、显示窗口，所以 splash 与设备请求重叠，不排在它前面；设备就绪后唤醒事件循环。macOS 的 Dock 图标（需要重新填充、编码 PNG，是唯一昂贵的图标）在 `nana-startup-icons` 线程渲染，到达时再应用，不阻塞 `UiReady`；程序在此之前自己设置的图标不会被覆盖。其他平台的窗口与任务栏图标在建窗时设好，不起线程。

没有嵌套事件循环，也没有第二个 `run_app`。嵌入宿主（`EmbeddedRuntime`）的设备已由宿主持有，仍同步启动，不显示 splash（`Skipped(Embedded)`）。

## 测量

`StartupStatus::timeline` 的时间都从宿主入口（`run_runtime`）算起，**不是**进程创建时间：

| 字段 | 含义 |
| --- | --- |
| `splash_committed` | splash 已提交给合成器、窗口已请求显示。CPU 侧的请求时刻；这里用到的平台都不报告图层真正上屏的时间，所以不写“可见时间” |
| `ui_ready` | 调用 `initialize` |
| `takeover_requested` | 接管请求被宿主接受 |
| `first_frame_submitted` | 完成接管的那一帧已 present |
| `handoff_completed` | splash 已移除（macOS 与该帧同一次提交；Windows 在合成器取走该帧之后）。没有 splash 时等于上一项 |
| `splash_released` | splash 创建的原生对象全部释放 |

`StartupStatus::work`：事件线程最长单次占用（从入口到交接，含完成交接的那次回调）、设备请求次数（每个尝试的呈现目标一次，只有合成目标失败回退时为 2）、交接前创建的 painter 数（每种 surface 格式一个）、从包里读 Logo 的耗时、`SplashWork`（Logo 解码 / 上传次数、动画提交次数、合成器提交次数、存活资源数）。

诊断事件（`nana_diagnostics::framework::host`）：`STARTUP_PHASE { phase, elapsed_ns }`（0 入口 … 6 splash 释放）、`SPLASH_OUTCOME { outcome }`、`STARTUP_FAILED`、`SPLASH_LOGO_FAILED { code }`，以及 gauge `host.startup.longest_block`、`host.startup.splash_logo_read`。

本机（macOS，debug 构建，`startup-splash --probe --app-ms=200`，有 / 无 splash 交替各 8 轮，负载 4–7，取最小值，括号内为中位数）：

| 场景 | 最早可见 | UiReady | 交接完成 | 事件线程最长占用 |
| --- | --- | --- | --- | --- |
| 有 splash | 0.40 s（0.40）：splash 提交 | 0.65 s（0.66） | 0.68 s（0.69） | 42 ms（44） |
| 无 splash | 0.70 s（0.71）：首帧，窗口此前隐藏 | 0.61 s（0.61） | 0.70 s（0.71） | 29 ms（32） |

有 splash 时 Logo 比首帧早约 0.3 s 出现，实际就绪（交接）也没有变晚；`UiReady` 晚约 45 ms，是提前显示窗口与建 splash 在事件线程上的代价（上表的最长占用即这一次回调）。

入口到第一次窗口回调约 0.3 s 花在 winit / AppKit 启动上，在宿主能做任何事之前。同一构建在去掉两处同步图标工作之前，`UiReady` 在 1.3–1.4 s，事件线程单次被占用约 0.8 s：默认图标在建窗前同步栅格化（macOS 上 winit 根本不用窗口图标），Dock 图标在 `initialize` 前同步生成。数字只说明这台机器 debug 构建的量级，不是预算。

从 `early-splash` pack 读 Logo。测量对象：用 `nana-packager` 打成 `.app` 的 `startup-splash`（debug 构建，Logo 71,584 字节，存原文，未签名 pack），在同一个 `.app` 里交替跑内嵌与 `--packaged-logo` 各 8 轮，两种先后顺序都有，负载 8–11：

| | `splash_logo_read` | `splash_committed` | 事件线程最长占用 |
| --- | --- | --- | --- |
| 内嵌 | — | 104.6 ms（108） | 39.9 ms（42.1） |
| 从 pack 读 | 0.24 ms（0.26） | 102.0 ms（105.9） | 40.6 ms（42.3） |

读一次 pack 的代价是亚毫秒，淹没在 `splash_committed` 的轮间波动里。换成 720,683 字节、不可压缩的 600×400 PNG 后，5 轮读取为 1.42–1.57 ms。包括打开 pack、校验 header 与 TOC hash、读条目、逐块 hash 和整条目 BLAKE3。

## Vue / JS

`Nana.startup` 是宿主记录的投影，不是另一套状态机：

```js
Nana.startup.deferTakeover();         // 只在首次求值 bundle 时有效，返回是否生效
Nana.startup.state;                    // { phase, splash, ticket, timeline }，读的时候就是最新的
Nana.startup.takeOver();               // 不传 ticket 就用当前的
Nana.startup.onChange(status => {});   // 宿主的 "startup" 事件
```

晚加载的前端读 `state`，不必担心错过一次性事件。JS 未就绪不会推迟原生 Logo：bundle 在 `initialize` 里求值，那时 Logo 早已在屏幕上。

## 已知边界

- Windows 路径只经过交叉编译检查（`x86_64-pc-windows-gnu`），没有真机首帧交接和动画证据；Linux 没有 splash。
- Windows 使用独立的无激活 Splash 窗口；它与主窗口共用启动线程，但使用自己的 DirectComposition target。
- macOS 只读取启动时的减少动态效果设置，不跟随运行中的切换。窗口移到缩放不同的显示器时，macOS 重新渲染 Logo 图层，Windows 由子类跟随 `WM_DPICHANGED`。
- 从 `early-splash` pack 读 Logo 在事件线程上同步进行，位置在窗口显示之前，所以这次读取的耗时会直接推迟首次可见。本机 debug 构建下，71 KB 的 Logo 约 0.25 ms，720 KB 的约 1.4 ms（见“测量”）。它没有提前到后台线程预取。
- 包里的 Logo 读不到时，不会退回内嵌 Logo，结果就是 `Failed(Package(..))`。需要兜底的应用只能自己选来源。
- 打包器不检查 `early-splash` pack 里的 PNG 是否满足 Logo 上限（它只限制整个 pack ≤ 1 MiB），也不知道应用会请求哪个路径。这两件事由应用的打包自检（`startup.early-splash-logo`）在 `validate --run` 时检查，前提是应用在 builder 上声明了 packaged Logo。
- 开发布局从 `loose_root` 读，不检查类别；类别规则只在打包后的包里生效。
- 从 pack 读 Logo 只在 macOS 真窗口上验证过；Windows 只经过交叉编译检查。
- Windows 与 macOS 都使用独立的无边框 Splash 原生窗口；客户区严格等于 `logo_size` 按当前 DPI 换算后的物理尺寸，不进入任务栏、不抢焦点，并在 handoff 或失败路径中销毁。主窗口只作为恢复后中心点和 DPI 的参考，主窗口大小不会改变 Splash 大小。
