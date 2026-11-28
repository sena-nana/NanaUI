# 两阶段启动与原生 Splash 合同

本文规定 NanaUI 的两阶段启动（Issue #225）：在不引入第二套 UI 引擎的前提下，将**原生首屏反馈（Early Splash）**与**完整界面就绪（UiReady / 画面接管）**解耦，消除应用初始化与着色器编译期间的空白窗口等待。

## 两个阶段与核心边界

1. **阶段一：Early Splash（原生启动窗）**。当 GPU 设备、管线与应用业务状态都尚未就绪时，宿主率先显示一个独立、无激活的小型原生 Splash 窗口。Logo 可直接编译进二进制，也可从 `early-splash` 资源包读取一次；呼吸或旋转等动画直接交由操作系统合成器（macOS Render Server / Windows DWM）推进，本进程零定时器、零逐帧回调（`SplashWork::animation_submissions` 恒为 `1`）。
2. **阶段二：`UiReady` 与画面接管（Takeover）**。当窗口、Surface、GPU 设备、`SceneWgpuPainter` 与文本系统全部就绪，能够挂载、布局并绘制普通文档时，宿主进入 `UiReady` 并调用 `RuntimeProgram::initialize`。应用在此挂载加载页或主界面并声明接管策略；**宿主必须确认接管内容的首帧已经成功提交给系统合成器后，才会撤下原生 Splash**。

::: tip 能力交接点 vs 画面交接点
`UiReady` 只代表「UI 渲染能力已就绪」，不代表漫长的业务加载已完成。如果不配置 `early_splash`，宿主不会创建任何额外图层或原生窗口，行为与传统单阶段启动完全一致。
:::

## 快速接入

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

不经过 `NanaApplication::builder` 的宿主可使用 `nana_ui::with_startup(StartupOptions::default().with_splash(spec), || run_runtime::<P>(settings))`。完整可运行演示与验收探针见 `crates/nana-ui/examples/startup-splash.rs`（传入 `--probe` 可在交接完成后输出结构化 JSON 计时记录并校验合同）。

## 第一阶段：Early Splash 配置与平台实现

`SplashSpec` 包含以下四个配置项（此阶段不提供文字排版、进度条、脚本或自定义着色器；如需展示产品名称请直接合入 Logo 图片）：

| 字段 | 说明 |
| --- | --- |
| `logo` | `SplashLogo::png(&'static [u8])`（内嵌静态 PNG）或 `SplashLogo::packaged("nana://res/…")`（从 `early-splash` 资源包读取）。编码大小上限 `1 MiB`，最长边上限 `1024 px`，解码后像素上限 `4 MiB`（启动前先校验 PNG 头，超限直接拒绝而不解码）。 |
| `logo_size` | 独立 Splash 窗口与 Logo 内容框的逻辑像素尺寸（Logo 按比例缩放居中；独立 Splash 窗口不进任务栏、不抢焦点，仅跟随主窗口中心点与 DPI）。 |
| `background` | `System`（取默认主题调色板在当前系统明暗下的背景色）、`Color(..)` 或 `Transparent`（仅画 Logo）。若应用自身主题不跟随系统明暗，请显式指定 `Color(..)` 以避免交接时底色跳变。 |
| `animation` | `None`、`FadeIn`（默认，淡入一次后保持）、`Pulse`（呼吸脉动）、`Rotate`（持续旋转）。 |

### 跨平台原生实现与交接方式

| 平台 | 原生 Splash 实现 | 系统合成器动画 | 首帧无闪烁交接机制 |
| --- | --- | --- | --- |
| **macOS** | 独立无标题栏、透明背景、忽略鼠标事件的 `NSWindow` + `CALayer` | `CABasicAnimation`（由 Render Server 推进） | 主窗口在 Splash 期间以 `alphaValue = 0` 保持（可排版但不可见、不收指针）；接管目标帧 `present` 后将主窗口 `alphaValue` 恢复为 `1` 并 `orderOut` 移除 Splash |
| **Windows** | 独立无激活 `WS_POPUP` 窗口上的 DirectComposition 视觉树（由短生命周期 D3D11 设备上传一次位图，交接后立即释放） | `IDCompositionAnimation`（由 DWM 推进） | 主窗口在 Splash 期间通过 `DWMWA_CLOAK` 隐身；接管目标帧 `present` 并等待 `on_submitted_work_done` 完成后，取消 Cloak → `DwmFlush()` → 移除 Splash 视觉并释放 D3D11/DComp |
| **Linux 及其他** | 不显示原生 Splash，直接上报 `Skipped(PlatformUnsupported)` | — | `UiReady` 与接管 ticket 合同照常工作，应用代码无需分支判断 |

实际生效结果记录在 `StartupStatus::splash`（`SplashOutcome`）中，绝不把申请值当作结果：`Shown { animation: Applied(..) | Static { requested, reason } }`（当系统开启“减少动态效果”或合成器拒绝动画时自动降级为静态 Logo 并注明 `ReducedMotion` / `NativeAnimationFailed`）、`Skipped(..)`（未配置、隐藏启动 `visible = false`、嵌入式宿主、平台不支持，或 Splash 的渲染资源还没建好 UI 已就绪的 `UiReadyFirst`）或 `Failed(Logo(..) | Package(..) | Native(..))`（Logo 损坏或超限时不阻断应用启动，仅跳过 Splash）。

### Windows 实机交接验收（Issue #273）

`startup-splash --probe` 校验启动记录的合同（只请求一次设备、只建一个 painter、里程碑有序、交接后 Splash 资源为 0、动画只提交一次）；`scripts/validate-startup-handoff.ps1` 同时从桌面合成结果截屏（约 45 帧/秒，DWM 的 Cloak 与 DirectComposition 都在画面里），以启动前的桌面为基准，要求 Splash 出现之后、进程退出之前没有一帧回到“什么都没有”。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-startup-handoff.ps1 -ExePath target/release/examples/startup-splash.exe -Arguments '--probe'
```

2026-11-23 实测（Windows 11 企业版 25H2 build 26200.9974，RTX 5060 Ti / DX12，release，60 Hz）：

| 启动方式 | Splash 结果 | 截屏 |
| --- | --- | --- |
| 开发运行，fade / pulse / rotate、`--defer=400`、`--composition` | `Shown { Applied(..) }`；提交于 243–422 ms，`UiReady` 915–1265 ms，交接完成 1005–1513 ms；合同各项通过 | Splash 直接切到带内容的主窗口，无空帧 |
| 开发运行，`--packaged-logo`（开 `packaged-resources`，从 `examples/assets` 读） | `Shown`，读 Logo 0.35 ms | 无空帧 |
| `nana-packager` 打出的 portable 包，`--packaged-logo`（Logo 在 `early-splash` 的 `.nrpack` 里） | `Shown`，读 Logo 1.1 ms | 无空帧 |
| `--packaged-logo` 但未开 `packaged-resources` | `Failed(Package(Unsupported))`，应用照常启动 | — |
| `--bad-logo` | `Failed(Logo(NotPng))`，应用照常启动 | — |
| `--no-splash` | `Skipped(NotConfigured)` | 修复前有一帧（约 30 ms）是只有边框的空窗口；修复后窗口第一次出现时已带内容，无空帧 |

观察到的两点：没有 Splash 时主窗口在 `UiReady` 显示、约 150 ms 后才有第一帧。现在宿主在显示前 Cloak 它（照常显示与取得焦点，只是不合成到屏幕），等第一帧的 GPU 工作完成再解除；1 s 内仍没有第一帧时照样解除，画不出来的窗口也会出现。不能 Cloak 的平台照旧直接显示。有 Splash 时事件线程曾最长阻塞 260–445 ms，几乎都是建 Splash 的 D3D11 设备（加载驱动）；现在这个设备在 `nana-splash-device` 线程上与窗口、设备请求同时创建，建好后唤醒事件循环再显示 Splash，最长阻塞降到 36–65 ms（无 Splash 约 50 ms），Splash 提交于 214–326 ms。`--composition`（窗口走 DirectComposition）曾因建窗前的 DX12 适配器探测阻塞 320–460 ms；现在建窗前只看 `WGPU_BACKEND` 是否允许 DX12，直接按合成目标建窗，枚举放到 `nana-startup-gpu` 线程上，最长阻塞 35–81 ms，设备只请求一次。合成目标建不成时照旧回退普通窗口（`NANA_FORCE_COMPOSITION_FAILURE=1` 下请求两次设备，探针通过）。截屏频率分辨不出单帧（16 ms）的重复或空帧。macOS 未执行（没有机器）；Linux 按上表报 `Skipped(PlatformUnsupported)`，本次没有在真实 Linux 上运行。

### 从 `early-splash` 资源包读取 Logo

启用 `nana-ui` 的 `packaged-resources` feature 并配置 `resource_packs` 后，可使用 `SplashLogo::packaged("nana://res/splash/logo.png")` 从 `early-splash` 类别的资源包读取 Logo（耗时记录在 `StartupWork::splash_logo_read`，本机实测 71 KB Logo 仅需约 `0.25 ms`）：

- 路径通过已验签的 `package.json` 最长前缀路由到唯一 pack；该 pack 必须属于不加密的 `early-splash` 类别（否则拒绝打开并报 `NotEarlySplash`，全程不调用 `KeyProvider`）。
- 读取器读完 4 KiB 头部即检查文件总大小（超过 `4 MiB` 报 `EarlySplashTooLarge`），查 TOC 确认条目不超过 `1 MiB` 后逐块校验哈希并检查 PNG 头（开发期从 `target/` 运行时自动回退读取 `loose_root` 下的同名文件）。

## 第二阶段：`UiReady` 与画面接管（Takeover）

当 `RuntimeProgram::initialize`（或 `ApplicationState::initialize` + `build`）被调用时，即到达 `UiReady`。此时 `initialize` 只需挂载首屏视图（加载页或主界面）、将耗时业务任务派发给后台线程并立即返回。

`RuntimeProgram::startup_takeover()` 决定何时撤下原生 Splash：

- **`Immediate`（默认）**：`initialize` 构建的主窗口文档即为接管画面（当配置了 Splash 时，`initialize` 返回的初始消息会按 `2 ms / 64 条` 分批处理完毕后再发起接管，确保撤下 Logo 的那一帧已包含初始消息的效果）。
- **`Deferred`（延迟接管）**：保持显示 Splash（或保持加载态），直到应用显式调用 `context.startup().take_over(ticket)`：

```rust
let startup = context.startup();
let ticket = startup.status().ticket.expect("UiReady 之后即分配 ticket");
startup.take_over(ticket)?;          // 可从任意线程调用
startup.cancel_takeover(ticket)?;    // 撤回接管请求并使旧 ticket 作废
```

- **代际票据与目标帧确认**：调用 `cancel_takeover` 会使旧 `ticket` 作废（若接管帧已 present 则返回 `AlreadyHandedOff`；迟到的后台任务持旧票据调用会收到 `StaleTicket`）。`take_over` 会记录主窗口当前的 `flush` 序号——**只有在此之后完成 `flush` 且真正 `present` 成功的主窗口帧才算完成交接**（跳过的帧、失败的帧或设备重建前的旧帧均不算数）。

## 启动线程并行化与着色器编译加速

独立宿主（`run_runtime`）在窗口线程创建窗口与 Surface 后，立即将 GPU 适配器选择、设备请求与 `SceneWgpuPainter` 的管线编译放到后台的 `nana-startup-gpu` 线程执行，同时在主线程挂载原生 Splash——因此 **Splash 的显示与 GPU 初始化完全并行重叠**。

在 `nana-startup-gpu` 线程内部：
1. 枚举一次适配器（合成目标只枚举 DX12），建窗前不再为合成单独探测、也不重复枚举。在这份列表上按 `StartupOptions::adapter` 选卡，规则见 [GPU · 适配器选择](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#适配器选择)；
2. `SceneWgpuPainter` 的各大管线家族（quad、mesh/path、icon、text、HostTexture、backdrop）在 scoped 线程上并行编译，总构建耗时取决于最慢的单个家族（quad）而非全部累加；仅用于测试回读的 motion 求值管线不在产品启动路径中编译。
3. macOS 上唯一较慢的 Dock 图标重采样与 PNG 编码放在独立的 `nana-startup-icons` 线程异步完成，不阻塞 `UiReady`。

### DX12 着色器编译器

在 Windows DX12 下，`SceneWgpuPainter` 的启动构建耗时几乎全部花在着色器编译上。**DXC 比系统自带的 FXC 快约一倍**（实测 RTX 5060 Ti 上 release 构建的 painter 编译从 FXC 的 `2.24–2.39 s` 降至 DXC 的 `1.18–1.79 s`，debug 构建从 `4.73–7.77 s` 降至 `1.61–1.98 s`）：

- 当可执行文件（`.exe`）同目录或其嵌套的 `runtime/` 目录下存在与目标架构匹配的 `dxcompiler.dll`（建议 DXC `v1.8.2502` 或更新版本，来自 [DirectXShaderCompiler releases](https://github.com/microsoft/DirectXShaderCompiler/releases) 或 Windows SDK `10.0.26100`，并同目录附带匹配的 `dxil.dll` 以在未开启开发者模式的零售 Windows 上完成 DXIL 签名）时，宿主会按完整绝对路径优先加载它（同目录优先于 `runtime/`，不受 `PATH` 或当前工作目录污染）；
- 未携带 `dxcompiler.dll` 时自动回退到系统内置的 FXC，也可通过环境变量 `WGPU_DX12_COMPILER`（`fxc` / `dxc` / `staticdxc` / `auto`）显式指定。

### 预编译的着色器产物（DXIL / Metal MSL 3.1 / Naga IR）

- **DXIL（Windows DX12，`scene_paint::precompiled`）**：quad、mesh/path、text（含双源混合的子像素程序）、group、group blend、icon 与 host texture 家族的着色器阶段在框架内预先编译成 DXIL（shader model 6.0），随二进制发布。宿主在 DX12 适配器支持时请求 `PASSTHROUGH_SHADERS`，这些家族就直接加载 DXIL，不再经过 naga → HLSL → DXC；驱动只把 DXIL 降为自己的代码并跨启动缓存。实测（RTX 5060 Ti，release，DXC）：quad 家族 `1.26–1.30 s` → `1.4–1.6 ms`，mesh `199–248 ms` → `2 ms`，text `126–160 ms` → `2 ms`；painter 整体从全部编译 WGSL 的 `1.40–1.74 s`、只预编译 quad 时的约 `250 ms` 降到 `75–84 ms`，此时最慢的是 backdrop 与 dest 家族（约 `90–105 ms`）。group 合成家族（以及读取背景的混合模式首次用到时才建的 group blend）、icon 与 host texture 家族也以同样方式预编译。它们在宿主用 FXC 编译时（可执行文件旁没有 `dxcompiler.dll`）最要紧：FXC 编这些家族每个阶段要 5–30 s，`window-input-region-probe`（release，Windows 11，D3D12）在 12 s 上限内一帧都没呈现；宿主启动时建的家族全部预编译后，管线约在启动 1.7 s 后就绪，探针 3.6 s 跑完。家族的程序或布局变了而没重新生成 DXIL 时，`precompiled_stages_are_current_with_their_programs_and_layouts` 会失败。
- **Metal MSL 3.1（Apple，`scene_paint::msl_precompiled`）**：最低支持版本为 macOS 14.0+ / iOS 17.0+（`MSL 3.1`，与 `nana-packager` 默认的 `minimum_system_version = "14.0"` 一致），通过内部 `wgpu_hal::metal::Device` 对齐并保留前置 `// language: metal3.1` 声明（且按适配器能力上限封顶，同时拒绝低于 `(3, 1)` 的设备），不随宿主 macOS 版本漂移；所有内置程序汇总成单份压缩存档 `stages.msl.zz`。构建 painter 时使用惰性 `ArchiveScope` + `ArchiveLease`（`archive_scope()` / `lease_archive()`）：作用域创建零开销，热管线缓存命中时 `0` 次解压；冷构建发生未命中时多线程共享单次解压并上报 `gpu::MSL_ARCHIVE_DECOMPRESSIONS`、`gpu::MSL_ARCHIVE_DECOMPRESS_NS` 与 `gpu::MSL_ARCHIVE_DECOMPRESSED_BYTES`，最外层 `ArchiveScope` 释放后强引用归零，使稳态常驻未压缩 MSL 保持 `0 B`。
- **二进制 Naga IR（非 Apple / 其余后端回退，`scene_paint::naga_ir`）**：按入口点裁剪后的 IR 使用自包含确定性二进制编解码器 `naga_ir::codec`（`NAGA1` 魔数 + LEB128 变长整数 + 小端定长基元，不依赖 `bincode`，无运行时 panic）写入 `*.naga`；`builtin_programs_cover_every_production_shader` 测试扫描 `crates/nana-ui/src/` 生产着色器调用确保 manifest 完整。
- **非 panic 降级与恢复**：只加载编入框架二进制的产物；所有预编译加载均包裹 `OutOfMemory` / `Internal` / `Validation` 错误作用域。阶段缺失、版本不匹配或设备拒绝时，`Program::try_pipeline` 在开启 `runtime-wgsl`（或测试）时记录 `gpu::SHADER_PRECOMPILED_FALLBACK` 并编译 WGSL，否则记录 `gpu::SHADER_BUILD_FAILED` 并返回 `Err(ScenePaintError::ShaderUnavailable)`（自定义材质被拒记录 `gpu.material.rejected`）；`SceneWgpuPainter::try_new` / `try_new_with_presentation` 将该错误向上交给 `scene_host` 启动/呈现流程而不是 panic。
- **重新生成与新鲜度校验**：
  - Metal MSL（macOS）：`cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1`（`precompiled_msl_stages_are_fresh` 校验 `WGPU`、`FORK_MSL_HASH`、`MSL_LANG_VERSION == (3, 1)`、`UNCOMPRESSED_LEN` 及 `builtin_programs()` 全覆盖；`precompiled_msl_stages_pass_metal_syntax_check` 在存在 `xcrun` 时对 macOS 3.1 与可用的 iOS Simulator 3.1 进行 `-fsyntax-only` 语法校验）。
  - Naga IR（任意宿主）：`cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1`。
  - DXIL（Windows + DXC）：`NANA_DXC_PATH=<dxcompiler.dll> cargo test -p nana-ui --lib --features gpu,hosted,full -- --ignored regenerate_precompiled_dxil --test-threads=1`（需要 DX12 时可用 `precompiled_families_paint_what_their_programs_paint` 逐像素核对）。

### 跨启动的着色器与管线缓存

painter（以及设备上任何别的东西）建的每条管线都经过设备的默认管线缓存（[GPU · 跨启动的着色器与管线缓存](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#5-跨启动的着色器与管线缓存)）。用 `NanaApplicationBuilder::pipeline_cache(store)` 交出存储后，`nana-startup-gpu` 线程在 painter 编译前恢复缓存、建完后保存；此后编译的管线由后台的 `PipelineCacheSaver` 在缓存增长时保存（编译停下后、最多每 10 s 一次，退出时再一次），设备丢失后的新设备在重建前从同一存储恢复；DX12 上热启动时缓存里有的阶段都不再经过 DXC/FXC。用 `startup-splash --probe --no-splash --app-ms=100 [--pipeline-cache=DIR]` 实测（RTX 5060 Ti，release，可执行文件旁放 DXC `10.0.26100`，`painter_build_ms`；构建后第一次运行偏慢，未计入）：

| 后端与路径 | 不用缓存 | 冷（空缓存，随后保存） | 热 |
| --- | --- | --- | --- |
| DX12，quad/mesh/text 预编译 | `69–70 ms` | `70 ms` | `23–38 ms`（缓存文件 `93 KB`） |
| DX12，全部 WGSL（为测量关掉预编译路径） | `1.56–1.69 s` | `1.80 s` | `31–33 ms`（`264 KB`） |
| Vulkan | `43–46 ms` | `46 ms` | `31–35 ms`（`617 KB`） |

热启动时间包含恢复与保存：读写文件都在计时的构建之内。DXC 保存的字节在 FXC 下（`WGPU_DX12_COMPILER=fxc`）载入时被校验键丢弃：那次启动全部重新编译（FXC、无预编译 DXIL 时 `3.1 s`），并以 FXC 字节码覆盖保存。

**随包附带的种子缓存能否取代 `scene_paint::precompiled`？** 性能上可以：全部 WGSL 的 painter 在热缓存下（`31–33 ms`）与预编译 DXIL 加缓存（`23–38 ms`）只差几毫秒，剩下的是 naga 为各阶段写 HLSL，缓存键要用它。覆盖面上不行：条目只对生成它的编译器构建与 shader model 命中。固定 shader model、随包附带 `dxcompiler.dll`（约 17.2 MiB）与 FXC 回退的取舍见 [GPU · 种子缓存取代预编译 DXIL 的设计说明](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#种子缓存取代预编译-dxil-的设计说明)；结论是保留预编译路径。

### 按入口点裁剪的着色器 IR

所有平台上，框架的场景程序（quad、mesh、text、icon、chart、backdrop、dest、HostTexture、运动求值，以及表面/文字材质程序）都由 `scene_paint::wgsl::Program` 交给设备：程序在第一次要用时解析、校验一次，每个入口点各自成为一个只含它能到达的函数、类型与常量的 naga 模块（`naga::compact`），以 `ShaderSource::Naga` 交给 wgpu。共用入口点的管线共用模块；设备只校验裁剪后的 IR；wgpu-hal 不再为每个管线阶段克隆、裁剪并重新校验整个模块；每条管线只留着自己阶段的代码（Metal 按管线生成 MSL，模块要一直留着）。例如 blit 程序有 39 个入口点、658 个表达式，一个阶段只带 5–424 个；mesh 一个阶段 53–737 个（整个程序 1 518 个），chart 411–1 282 个（2 593 个）。切下的模块不再声明它用不到的覆盖常量，管线只设置该阶段仍声明的常量（`Stage::constants`）。程序在这里解析或校验失败时（框架自己的不会，测试逐个编译）整体以 WGSL 交给设备，由设备照旧报错。窗口输出、颜色转换、默认 GpuView 与原生内容占位这几处小程序也经由 `Program` 交给设备。

框架程序编译时不加循环保护（wgpu 给每个循环加的计数器）：框架着色器里的每个循环都与数据无关地必然结束——计数器从常量起步、每次加一、与界严格比较，其余三处各有理由，`loop_bounds_hold` 逐个检查。越界检查和整数除法检查保留，因为下标与除数来自 CPU 写的缓冲，而 Metal 背后没有硬件兜底。应用与皮肤的材质程序（`Program::material`）保留全部检查。去掉循环计数器后，quad、mesh、text 的 DXIL 小了 1–2.5%。

## 时间线观测指标

`StartupStatus::timeline` 记录从宿主入口（`run_runtime`）起算的六个关键里程碑：`splash_committed` → `ui_ready` → `takeover_requested` → `first_frame_submitted` → `handoff_completed` → `splash_released`；`StartupStatus::work` 与 `nana_diagnostics::framework::host`（`STARTUP_PHASE`、`SPLASH_OUTCOME`、`host.startup.device_request`、`host.startup.painter_build` 等）同步记录事件线程最长单次阻塞、设备请求耗时、painter 编译耗时与存活原生资源数（交接完成后 `SplashWork::live_resources` 严格归零）。
