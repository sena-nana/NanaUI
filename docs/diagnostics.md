# 诊断与应用路径

NanaUI 负责诊断的**机制**：采集、缓冲、落盘、轮转、崩溃快照、导出。应用负责**语义**：有哪些事件和指标、什么算故障、隐私策略。NanaUI 不认识 Live2D、Tracking、Spout / NDI 这些产品概念。

原则：日志允许丢，画面不允许错（Logging is lossy by design; rendering is not）。

代码在 `crates/nana-diagnostics`（纯 std 叶子 crate），经 `nana_ui::diagnostics` 再导出；应用身份与路径在 `nana-ui-platform` 的 `ApplicationPaths`。

## 接入

```rust
use nana_ui::{ApplicationIdentity, DiagnosticsConfig, NanaApplication, RuntimeApplication, WindowDescriptor};

NanaApplication::builder(ApplicationIdentity::new("dev.nana.live", "NanaLive", env!("CARGO_PKG_VERSION")))
    .diagnostics(DiagnosticsConfig::default())
    .run::<RuntimeApplication<App>>(WindowDescriptor::new("NanaLive"))
```

- 不调 `.diagnostics(..)` 就不开：框架里每个埋点只剩一次 Relaxed 原子读。
- `run` 返回时做最终排空和 `sync_data`。自己跑事件循环的宿主（Vue、嵌入式）用 `let _session = builder.start();` 持有 `ApplicationSession`，退出时 drop 它（`let _ = …` 会立刻关掉诊断）。
- 平台目录解析失败（容器里没有 `HOME`、缺 `LOCALAPPDATA` 等）不会拦住应用启动：打一行 stderr，`session.paths()` 为 `None`，诊断只留在内存里。要把它当错误处理，用 `try_start()`。
- 不经 builder 的旧入口 `run_runtime` 行为不变，诊断保持关闭。

最小例子：`crates/nana-ui/examples/application-counter.rs`。

## 定义应用自己的事件与指标

域（`Domain`）`0x0000..=0x00FF` 归 NanaUI，应用从 `0x0100` 起。ID 只在域内唯一，写进 `.nlog` 后就是合同：只追加，不改号。

```rust
use nana_ui::diagnostics::{
    Domain, EventDescriptor, FieldDescriptor as F, HistogramCells, Metric, Severity,
    event, fault, metric, span,
};

const LIVE: Domain = Domain(0x0100);

static MODEL_LOADED: EventDescriptor =
    EventDescriptor::new(LIVE, 1, "live.model_loaded", Severity::Info, &[F::u64("bytes"), F::f64("ms")]);
static TRACKER_LOST: EventDescriptor =
    EventDescriptor::new(LIVE, 2, "live.tracker_lost", Severity::Error, &[F::u64("camera")]);

static TRACK_CELLS: HistogramCells = HistogramCells::new();
static TRACK_NS: Metric = Metric::histogram(LIVE, 1, "live.track", "ns", &TRACK_CELLS);
static FRAMES_OUT: Metric = Metric::counter(LIVE, 2, "live.frames_out", "count");

event!(MODEL_LOADED, bytes = size, ms = elapsed_ms);
fault!(TRACKER_LOST, camera = id; "camera {id} stopped: {reason}");
metric!(FRAMES_OUT);                 // counter +1
metric!(TRACK_NS, frame_time);       // Duration 记纳秒
let _span = span!(TRACK_NS);         // 作用域耗时进直方图
```

| 种类 | 用途 | 成本（M4，release，本机负载 ~3） |
| --- | --- | --- |
| `metric!` 计数 / 仪表 | 帧数、字节、队列深度 | ~1.6 ns |
| `metric!` 直方图 | 帧时间、GPU 时间 | ~4.5 ns |
| `span!` | 作用域耗时 | ~39 ns（两次读时钟） |
| `event!` | 低频结构化事件，最多 4 个 typed 字段 | ~24 ns |
| `fault!` | 错误；独立应急环，唤醒 worker 尽快落盘 | 有消息时分配一次 |
| 关闭时任意埋点 | | ~0.5 ns |

高频数据用指标，不要逐帧发事件：worker 每 `metric_interval`（默认 10 s）取一次快照写入。字段名只在 debug 构建里和描述符比对。

数字来自 `cargo run --release -p nana-diagnostics --features benchmark --bin nana-diagnostics-benchmark`；同一个直方图被 4 个线程同时写时约 270 ns/次（缓存行争用），高频多线程指标请各线程用各自的指标。

## 运行时模型

```text
生产线程 ──event!/fault!──▶ 线程本地 SPSC 环（满了丢最新并计数）
        ──metric!/span!──▶ 静态原子量
                               │ worker 轮询（poll_interval 50 ms），生产者从不等待
                               ▼
                  nana-diagnostics worker（降优先级）
                    ├─ Flight Recorder：最近 flight_window（60 s）/ flight_bytes（4 MiB）
                    └─ 批量缓冲 ──▶ 攒够 batch_bytes（64 KiB）或 batch_interval（2 s）──▶ 一次顺序写
```

生产者路径保证：不做文件 I/O、不 `fsync`、不等 worker、队列满不反压、无全局锁、不格式化；线程第一次记录时注册（冷路径），实时线程应在启动时调用 `nana_ui::diagnostics::register_thread()` 预注册。慢盘或写失败只影响 worker：批次丢弃并计入 `bytes_discarded`，指数退避后换新文件重试，Flight Recorder 照常保留。

`PersistMode`：

| 模式 | 写进会话日志 | 默认 |
| --- | --- | --- |
| `Off` | 不写；快照照写 | |
| `Essential` | Warn 以上事件、故障、指标快照、会话信息、标记 | release |
| `All` | 所有事件 | debug |

Flight Recorder 始终保留 `min_severity` 以上的全部事件，崩溃或显式导出时写成快照。

配置在 `Diagnostics::start` 时经 `DiagnosticsConfig::sanitized` 钳制：`flight_window` 限 30–120 s，`poll_interval` 1 ms–60 s，其余间隔最长 1 h（传 `Duration::MAX` 表示“很少”，不会溢出），环容量有上下限。

## 文件与保留

路径来自 `ApplicationPaths`：

| 位置 | Windows | macOS | Linux | 便携版 |
| --- | --- | --- | --- | --- |
| logs | `%LOCALAPPDATA%\{id}\Logs` | `~/Library/Logs/{id}` | `$XDG_STATE_HOME/{id}/logs` | `<root>/data/logs` |
| crash | `%LOCALAPPDATA%\{id}\Crash` | `~/Library/Logs/{id}/Crash` | `$XDG_STATE_HOME/{id}/crash` | `<root>/data/crash` |

完整映射（runtime 目录、data、config、cache、Android）见 `nana-ui-platform/src/paths.rs` 模块文档。`<exe 目录>/runtime/manifest/portable` 存在时为便携版（macOS `.app` 内不支持）；从 Cargo `target/` 运行时 `layout()` 为 `Development`。

- 会话日志：`{app}-{YYYYMMDDTHHMMSSZ}-{pid}.nlog`，超过 `max_file_bytes`（16 MiB）轮转为 `.1.nlog`、`.2.nlog`，从不覆盖已有文件。
- 保留：按修改时间删最旧的，直到满足 `max_files`（32）、`max_total_bytes`（128 MiB）、`max_age`（14 天）；快照另限 `max_crash_files`（16）和 `max_crash_bytes`（64 MiB）。只匹配本应用精确文件名模式；其它会话最近 `live_grace`（10 分钟）内改过的文件不删（可能属于另一个运行中的实例），本会话自己轮转出的文件和快照不受这条保护，所以一次失控的会话也不会突破预算。

## 崩溃与导出

- Panic：`install` 默认挂 panic hook（链到原 hook 之前），记一条 `diagnostics.panic` 故障并把 Flight Recorder 同步写到 crash 目录。`dist` profile 的 `panic = "abort"` 下 hook 仍在 abort 前运行。
- 设备丢失：宿主记 `gpu.device_lost` 故障并写 `device-lost` 快照；嵌入式宿主在 `EmbeddedRuntime::notify_device_lost` 时同样记录。
- 显式：`Diagnostics::snapshot_blocking(reason, timeout)`；`export_package(dest, &PackageOptions)` 生成一个目录：最近几次会话日志与快照、每份的 `.txt` / `.jsonl` 导出、`manifest.json`。打包成 zip、上传属于应用策略。
- 查看：`cargo run -p nana-diagnostics --bin nana-nlog -- text <file.nlog>`（或 `json`；`--redact-home` 把家目录换成 `~`）。

**不覆盖**：原生崩溃（SIGSEGV、访问违例、C 代码 abort）。信号处理器里做文件 I/O 不是 async-signal-safe，需要进程外 crash handler，不在本 crate 范围。进程被 SIGKILL / 强制结束时，最后一个未写出的批次（至多 `batch_interval`）会丢；已写出的文件可读。

## `.nlog` 格式（schema v1）

```text
file  := "NANALOG\0" format_version:u16le chunk*
chunk := kind:u8 len:u32le payload[len] crc32:u32le
```

第一个块是 Header（schema 版本、会话 id、app id / 名称 / 版本 / build id、框架版本、OS / 架构、pid、墙钟起点、单调时钟起点）。单调时钟起点取自 Rust `Instant` 背后的同一个平台时钟（macOS `CLOCK_UPTIME_RAW`、Linux / Android `CLOCK_MONOTONIC`、Windows QPC），可以和同一台机器同一次开机的其它日志对齐。事件只存单调时间增量、域 + ID、线程号和 typed 值；名字、字段名、单位由 EventSchema / MetricSchema 块在同一文件中先行给出，读取端不需要应用二进制。GPU 适配器这类启动后才知道的信息走 SessionInfo 块；ClockSync 块每分钟记一次单调时间与墙钟的对照，覆盖系统睡眠。

读取端逐块校验 CRC，遇到截断或损坏就停，报告此前全部内容（`NlogFile::truncated`）。未知块类型跳过。

## 框架埋点

| 域 | 内容 |
| --- | --- |
| Runtime | 每次 flush 的 CPU 总耗时与 9 个 Runtime 阶段直方图（复用 `FrameProfiler`，不重复计时）、flush 次数与轮数、不收敛 / 样式与文本布局失败计数 |
| Layout | 每次布局耗时、调用数、整树布局数、dirty 根数、参与布局的盒子数 |
| Text | shape / layout 缓存命中与未命中、字形解析数、字形图集开页（含图集总字节）/ 预算耗尽（每个图集只报一次）/ 压缩、驱逐数 |
| GPU | submit 耗时、GPU 完成时间上界（submit 到宿主观察到完成；每次 redraw 开头非阻塞 poll；两次 poll 相隔超过 50 ms（窗口空闲过）时丢弃该样本，所以误差不超过一个活跃 redraw 间隔。精确 GPU 时间需要 timestamp query，Metal 不能在 encoder 内写时间戳，未做）、上传字节、draw call、缓冲重分配、呈现 / 跳过帧、surface Outdated / Lost / Timeout、设备丢失（含嵌入式宿主上报的）/ 恢复 / 恢复失败、surface 挂起；适配器名、后端、类型、驱动写进会话信息 |
| Window | 打开（物理尺寸）、关闭、缩放系数变化、遮挡、resize 次数 |
| Host | 每次 redraw 的墙钟耗时（消息处理 + flush + 绘制 + submit + present，可能含 vsync 等待，不是 GPU 时间）、`Continuous` 窗口错过的帧周期数（丢帧）、每次排空时的程序消息队列深度、`HostFailure` 计数与故障（变体码 + 窗口 + 错误文本；同一变体每秒至多记一条，计数不漏）、运行失败、事件循环退出 |

每帧都可能重复的失败（帧不收敛、输入处理器报错）只在宿主层记故障并限流，Runtime 层只计数，避免同一次失败记两遍、也避免逐帧刷日志。

ID 表在 `nana_diagnostics::framework`。
