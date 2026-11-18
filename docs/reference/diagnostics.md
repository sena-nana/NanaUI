# 诊断与应用路径

NanaUI 提供统一的诊断底座（采集、无锁缓冲、后台落盘、文件轮转、崩溃快照与离线导出），而诊断的业务语义（定义哪些应用事件和指标、什么情况算业务故障、如何脱敏与上报）由你的应用决定。核心设计原则是：**日志按设计允许有损丢弃，但画面渲染绝不允许因此卡顿或出错（Logging is lossy by design; rendering is not）**。

诊断实现位于叶子 crate `crates/nana-diagnostics`：不依赖其它 NanaUI crate，跨平台部分只用标准库，Windows 上另依赖 `windows-sys`（高精度计时、线程优先级与卡顿时的线程栈采样所需的 Win32 接口）（经 `nana_ui::diagnostics` 再导出）；跨平台应用目录解析位于 `nana-ui-platform` 的 `ApplicationPaths`。写界面时常见的运行期故障码速查见 [诊断码](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/errors.md)。

## 在应用中启用诊断

```rust
use nana_ui::{
    ApplicationIdentity, DiagnosticsConfig, NanaApplication, RuntimeApplication, WindowDescriptor,
};

NanaApplication::builder(ApplicationIdentity::new(
    "dev.nana.live",
    "NanaLive",
    env!("CARGO_PKG_VERSION"),
))
.diagnostics(DiagnosticsConfig::default())
.run::<RuntimeApplication<App>>(WindowDescriptor::new("NanaLive"))
```

- **默认关闭**：不调用 `.diagnostics(..)`（或直接使用裸 `run_runtime`）时诊断保持关闭，框架内所有埋点退化为一次 `0.5 ns` 的 `Relaxed` 原子读取。
- **生命周期管理**：`builder.run(...)` 返回时会自动执行最终排空（drain）与 `sync_data`。如果应用自管事件循环（如嵌入式宿主），请用 `let _session = builder.start();` 持有返回的 `ApplicationSession` 直到进程退出——切勿写成 `let _ = builder.start();`，否则会立即析构并关停诊断线程。
- **目录解析容错**：当容器或特殊环境缺少 `HOME` / `LOCALAPPDATA` 导致平台目录解析失败时，不会阻断应用启动——此时向 `stderr` 打印一行提示，`session.paths()` 返回 `None`，诊断数据仅保留在内存环形缓冲中。

## 定义应用自有事件与指标

域编号（`Domain`）中 `0x0000..=0x00FF` 归 NanaUI 框架保留，**应用自定义域从 `0x0100`（`Domain::APPLICATION_MIN`）起算**。事件与指标 ID 在各自域内唯一；一旦写入 `.nlog` 即成为持久化格式合同，**只能追加新号，严禁重排或复用旧号**。

```rust
use nana_ui::diagnostics::{
    Domain, EventDescriptor, FieldDescriptor as F, HistogramCells, Metric, Severity,
    event, fault, metric, span,
};

const LIVE: Domain = Domain(0x0100);

static MODEL_LOADED: EventDescriptor = EventDescriptor::new(
    LIVE,
    1,
    "live.model_loaded",
    Severity::Info,
    &[F::u64("bytes"), F::f64("ms")],
);
static TRACKER_LOST: EventDescriptor = EventDescriptor::new(
    LIVE,
    2,
    "live.tracker_lost",
    Severity::Error,
    &[F::u64("camera")],
);

static TRACK_CELLS: HistogramCells = HistogramCells::new();
static TRACK_NS: Metric = Metric::histogram(LIVE, 1, "live.track", "ns", &TRACK_CELLS);
static FRAMES_OUT: Metric = Metric::counter(LIVE, 2, "live.frames_out", "count");

event!(MODEL_LOADED, bytes = size, ms = elapsed_ms);
fault!(TRACKER_LOST, camera = id; "camera {id} stopped: {reason}");
metric!(FRAMES_OUT);                 // 计数器 +1
metric!(TRACK_NS, frame_time);       // Duration 自动记录纳秒数
let _span = span!(TRACK_NS);         // 作用域结束时自动记录耗时进直方图
```

四种原语的用途与实测单次调用开销（Apple M4，`--release`，通过 `nana-diagnostics-benchmark` 测得）：

| 宏 | 适用场景 | 单次开销 | 行为说明 |
| --- | --- | ---: | --- |
| `metric!`（Counter / Gauge） | 帧数、字节数、队列深度等高频计数 | `~1.6 ns` | 写静态原子量；worker 默认每 `10 s`（`metric_interval`）拍一次快照落盘 |
| `metric!`（Histogram） | 帧耗时、阶段耗时分布 | `~1.8 ns` | 同上（注意：多线程高频写同一个直方图会因缓存行争用升至 `~160 ns`，请按线程拆分指标） |
| `span!` | 代码块作用域计时 | `~39 ns` | 进出作用域各读一次单调时钟，自动写入直方图 |
| `event!` | 低频结构化事实事件（最多 4 个类型化字段） | `~24 ns` | 写入线程本地 SPSC 环（字段名仅在 debug 构建下核对描述符） |
| `fault!` | 关键错误与故障 | 仅带格式化消息时分配一次 | 额外写入独立应急环并立即唤醒 worker 尽快落盘 |

::: warning 帧路径铁律
高频数据必须使用 `metric!`，**严禁逐帧调用 `event!`**；帧路径上严禁使用外部日志 crate、`eprintln!`、字符串格式化或任何同步文件 I/O。
:::

## 运行时缓冲模型与落盘策略

```text
生产线程 ──event! / fault!──▶ 线程本地 SPSC 环形缓冲（满则丢弃最新事件并累加丢弃计数）
        ──metric! / span!──▶ 静态原子量
                                   │
                                   │ 后台 worker 轮询（默认 poll_interval = 50 ms），生产线程永不等待
                                   ▼
                     nana-diagnostics worker（降级线程优先级）
                       ├─ Flight Recorder：常驻保留最近 60 s（flight_window）/ 4 MiB（flight_bytes）
                       └─ 批量顺序写缓冲 ──▶ 攒满 64 KiB（batch_bytes）或每 2 s（batch_interval）一次顺序写盘
```

生产线程在首次记录事件时自动惰性注册（冷路径）；对延迟敏感的实时音频或渲染线程可在启动时调用 `nana_ui::diagnostics::register_thread()` 预先注册。若遇到慢盘或写入失败，仅后台 worker 受影响（丢弃当前批次并记入 `bytes_discarded`，按指数退避切换新文件重试），内存中的 Flight Recorder 照常保留最近事件。

会话日志的落盘级别由 `PersistMode` 控制（Flight Recorder 始终在内存中保留 `min_severity` 以上的全部事件，供崩溃或主动导出时生成完整快照）：

| `PersistMode` | 写入会话日志（`.nlog`）的内容 | 默认构建配置 |
| --- | --- | --- |
| `Off` | 不写常规会话日志；崩溃与显式快照照常写出 | — |
| `Essential` | `Warn` 及以上事件、所有 `fault!`、定期指标快照、会话信息与时钟同步标记 | `release` / `dist` |
| `All` | 全部级别的事件与指标 | `debug` |

## 日志路径、轮转与崩溃导出

日志与崩溃快照目录由 `ApplicationPaths` 按平台约定解析（若 `<exe 目录>/runtime/manifest/portable` 存在则视为便携版，macOS `.app` 包内除外；从 Cargo `target/` 直接运行时布局为 `Development`）：

| 目录用途 | Windows | macOS | Linux | 便携版（Portable） |
| --- | --- | --- | --- | --- |
| 会话日志（logs） | `%LOCALAPPDATA%\{id}\Logs` | `~/Library/Logs/{id}` | `$XDG_STATE_HOME/{id}/logs` | `<root>/data/logs` |
| 崩溃快照（crash） | `%LOCALAPPDATA%\{id}\Crash` | `~/Library/Logs/{id}/Crash` | `$XDG_STATE_HOME/{id}/crash` | `<root>/data/crash` |

- **命名与轮转**：会话日志名为 `{app}-{YYYYMMDDTHHMMSSZ}-{pid}.nlog`；单文件超过 `max_file_bytes`（默认 `16 MiB`）时自动轮转为 `.1.nlog`、`.2.nlog`，永不覆盖已有文件。
- **配额清理**：按修改时间从旧到新清理，直至满足 `max_files`（32 个）、`max_total_bytes`（128 MiB）与 `max_age`（14 天）；崩溃快照单独限制为 `max_crash_files`（16 个）与 `max_crash_bytes`（64 MiB）。其他进程在最近 `live_grace`（10 分钟）内修改过的文件受保护不删，避免误删并行运行实例的日志。
- **崩溃捕获与离线查看**：
  - **Rust Panic**：默认挂载在原 panic hook 之前（即使在 `panic = "abort"` 的 `dist` 配置下也会在 abort 前执行），记录一条 `diagnostics.panic` 故障并将 Flight Recorder 同步写出到 `crash` 目录（`-panic` 快照）。
  - **原生崩溃（Windows x64）**：`install` 默认（`DiagnosticsConfig::native_crash_handler`）挂上未处理异常过滤器（`SetUnhandledExceptionFilter`，访问违例、栈溢出等所有未处理的 SEH 异常）和 C `abort()` 的 `SIGABRT` 处理器，都串在已有处理器之前。崩溃线程上不分配、不加锁：它把寄存器和栈边界存进静态槽，唤醒安装时预先创建的写入线程并等它最多 30 s（栈溢出时崩溃线程只剩 std 用 `SetThreadStackGuarantee` 预留的几 KiB）。写入线程先把异常码和地址无分配地写进运行标记并同步落盘，再沿崩溃线程的栈回溯、符号化（与卡死报告同一套，帧格式为 `module+0xoffset` 加符号），记录 `diagnostics.native_crash` 故障（`code`、`address`、`os_thread`；C `abort()` 的 `code` 为 `0x40000015`）并同步写出 `-native-crash` 快照。崩溃若发生在持有堆锁或加载器锁时，后两步可能卡住，此时等待超时后进程照常结束，异常码留在运行标记里。`std::process::abort`、`handle_alloc_error` 的默认行为等 `__fastfail` 直接交给内核，进程内无法捕获，只能由运行标记兜底。
  - **异常退出（运行标记）**：每个会话启动时在 `crash` 目录（没有则用 `logs`）创建并锁住 `{app}-{时间}-{pid}.running`，每个指标周期写入一次最后存活时间，正常 `shutdown` 时删除（`DiagnosticsConfig::run_marker`）。进程被结束、断电或崩溃后，锁随进程释放而文件留下；下一个会话在 `Diagnostics::start` 里接手这些标记，为每个记录一条 `diagnostics.unclean_exit` 故障（`session_id`、`pid`、`start_unix_ns`、`last_write_unix_ns`；消息里有版本、时间、原生异常码和该会话的崩溃快照），并在返回前同步写出一个 `-unclean-exit` 快照（若那个会话已经留下了 panic 或原生崩溃快照则不再写）。仍在运行的实例持有锁，不会被误报；`Diagnostics::unclean_exits()` 返回这次启动发现的会话。
  - **上次崩溃查询**：`nana_diagnostics::last_crash()`（或 `Diagnostics::last_crash()`、`last_crash_in(dir, app_id)`）返回本应用 `crash` 目录里最新的 `-panic`、`-native-crash`、`-unclean-exit` 快照及其类型（`CrashKind`）和写入时间；`stall`、`device-lost`、`export` 等快照不算崩溃。
  - **事件循环卡死**：安装了诊断实例时，Scene 宿主在启动时调用 `nana_diagnostics::stall::watch_current_thread()` 监视事件循环线程，并用 `stall::busy(Callback)` 标记每个回调（`resumed`／`proxy_wake_up`／`window_event`／`about_to_wait`）。回调之间等待事件属于空闲，不计时；单个回调超过 `STALL_THRESHOLD`（5 s，Windows 判定「无响应」的时限）仍未返回时，后台看门狗线程记录 `diagnostics.main_thread_stalled` 故障并写出 `stall` 快照：字段 `elapsed_ms`、`report`（同一次卡死的第几次报告）与 `cpu_permille`（期间占用一个核心的千分比，忙等接近 1000，阻塞等待接近 0；平台取不到时为 `u64::MAX`），消息写明回调名，在 Windows x64 上附三次相隔 200 ms 的调用栈采样（相同的采样标注「same as」，即卡在等待；不同则在循环里）。每帧格式为 `模块+0x偏移`，本机能找到 PDB（模块旁）或 DLL 导出表时再附符号名；发行包没有 PDB 时用同版本的 PDB 离线解析偏移。卡死持续时在 30 s 与 120 s 各补报一次单次采样；回调最终返回时记录 `diagnostics.main_thread_recovered`（`stalled_ms`）。采样只在挂起线程的瞬间复制寄存器与栈，挂起期间不分配、不加锁、不查展开表，展开在副本上进行，跟不进副本的帧即停止。
  - **GPU 设备丢失**：`nana-gpu` 在第一次收到丢失时（WGPU 的设备丢失回调，或嵌入式宿主经 `EmbeddedRuntime::notify_device_lost` 报告）就在当前线程记录 `gpu.device_lost` 故障并同步写出 `device-lost` 快照（`nana_diagnostics::fault_with_snapshot`），之后 `GpuContext::is_lost()` 才变为真，不依赖后台线程。WGPU 把设备丢失类错误只交给丢失回调；其余未捕获错误（校验、内存不足、内部错误）走 WGPU 默认的处理器，即 panic，由 panic hook 同步记录。
  - **主动导出与命令行读取**：应用可调用 `Diagnostics::snapshot_blocking(reason, timeout)` 或 `export_package(dest, &PackageOptions)` 导出包含 `.nlog`、`.txt`、`.jsonl` 与 `manifest.json` 的诊断目录；命令行使用 `cargo run -p nana-diagnostics --bin nana-nlog -- text <file.nlog>`（或 `json`，加 `--redact-home` 可将用户主目录脱敏为 `~`）直接查看。

## `.nlog` 二进制格式与框架内置埋点

`.nlog`（schema v1）由文件头 `"NANALOG\0"` + 小端版本号 + 一系列带 `CRC32` 校验的块（chunk）组成：首块 `Header` 记录会话 ID、应用身份、操作系统、架构以及与 Rust `Instant` 同源的平台单调时钟起点（macOS `CLOCK_UPTIME_RAW`、Linux/Android `CLOCK_MONOTONIC`、Windows `QPC`）；`EventSchema` 与 `MetricSchema` 块在文件内自描述所有名称、字段与单位，因此读取工具无需依赖应用二进制；`ClockSync` 块每分钟记录一次单调时钟与墙钟的对照以跨越系统休眠。遇到文件截断或块损坏时，读取器会安全停止并返回此前已通过校验的全部内容（`NlogFile::truncated`）。

框架内置埋点分布在 `nana_diagnostics::framework` 的 9 个子域中（可能每帧重复发生的错误仅在 Runtime 层计数，由 Host 层按每秒至多一条限流记录故障，防止刷爆日志）：

| 框架域 | 核心监控指标与事件 |
| --- | --- |
| `Runtime` | 每次 `flush` 的 CPU 总耗时与 9 个阶段直方图（复用 `FrameProfiler`）、`flush` 次数与轮数、样式解析/命中更新/场景抽取计数（`runtime.style.nodes_resolved`、`runtime.hit_test.nodes_rebuilt`、`runtime.extract.nodes_{extracted,changed}`）、禁用 `snippet-transforms` 时的一次性 `runtime.snippet.transform_unavailable`、未收敛及样式/文本布局失败计数 |
| `Input` | 接收事件数、合并次数、载荷字节数、路由拒绝、裁剪拒绝、路由耗时、命中查询、焦点/捕获/hover 变化（`runtime.input.*`，ID 20–37） |
| `Layout` | 布局耗时、调用次数、整树布局次数、脏根数、测量与放置节点数（`layout.measure_nodes`、`layout.placement_nodes`）；动态布局（Dynamic Layout）的行求解数、包络重建数与预算回退数 |
| `Text` | 整形与布局缓存命中/未命中、字形解析数、字形图集（atlas）开页与总字节数、预算拒绝事件（每 10 s 限流一条）、图集压缩与驱逐计数 |
| `GPU` | 提交耗时、GPU 完成时间上界（每次重绘开头非阻塞 poll，空闲超过 50 ms 的样本自动丢弃）、上传字节、draw call、缓冲重分配、呈现/跳帧、Surface 异常、设备丢失与恢复、帧丢弃（`gpu.frames_discarded` / `gpu.retained_frame_discarded`）、预编译 MSL 解压次数与耗时/字节（`gpu.shader.msl_decompressions` / `gpu.shader.msl_decompress_ns` / `gpu.shader.msl_decompressed_bytes`）、管线回退/构建失败事件（`gpu.shader.precompiled_fallback` / `gpu.shader.build_failed`）与自定义材质拒收事件（`gpu.material.rejected`），并在会话信息中登记适配器与驱动信息 |
| `Window` | 窗口打开（含物理尺寸）、关闭、DPI 缩放系数变化、遮挡、resize 计数，以及跨平台屏幕取色失败/不支持事件（`window.color_sampler.failed`） |
| `Resource` | 资源包（`.nrpack`）挂载耗时与 TOC 字节数、每次打包资源读取/认证/解压耗时与字节数、未命中与完整性校验失败故障，以及关闭可选特性时的 `resource.fetch_unavailable`、`resource.fetch_failed`、`resource.image_codec_unavailable` 与 `resource.icu_unavailable` 事件 |
| `Package` | `package.json` 清单读取耗时、载入/缺失事件、无效清单与身份不匹配故障 |
| `Host` | 每次重绘的墙钟总耗时（含消息处理、flush、绘制、submit 与 present）、`Continuous` 窗口错过的帧周期数、消息队列排空深度、启动指标（`host.startup.*`）、`HostFailure` 限流故障与事件循环退出 |
