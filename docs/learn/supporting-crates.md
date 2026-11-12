---
title: 外围支撑 crate 一览
---

# 外围支撑 Crate 一览

在核心渲染主线（`core → runtime → scene → nana-ui`）之外，工作区还包含一组边界清晰的外围支撑 crate：图表模型、零 I/O 诊断底座、SVG 栅格化、应用图标、矢量图标目录、资源打包（运行时读侧与构建期写侧）、开发期热重载以及无头诊断会话（devtools）。它们不持有保留树、也不另起渲染管线，而是各自守住一项正交职责。本章逐一拆解这 9 个支撑 crate 的内部结构与源码锚点，并在文末给出整个仓库周边生态（`examples/`、`platform/android`、`tools/css-parity`、`perf/`）的全景地图。

## 职责与边界总表

| Crate | 一句话职责 | 关键架构边界 |
| --- | --- | --- |
| `nana-ui-charts` | 图表模型：ECharts 形状的 `ChartOption`、刻度、布局、命中与 GPU 标记数组 | 仅依赖 `nana-ui-core` + `bytemuck`；`Chart` 组件在 `runtime`，WGSL 管线在 `nana-ui` |
| `nana-ui-diagram` | Mermaid 图表：解析、分层布局与正交连线、主题无关的矢量显示列表 | 依赖 `nana-ui-core` 与 `nana-ui-charts` 的刻度；纯 CPU，测宽由调用方传入；Markdown 绘制入口在 `runtime`（`rich-text`），细分走 `scene` 的自绘路径 |
| `nana-diagnostics` | `event!` / `metric!` / `fault!` / `span!` 宏与静态描述符 | 生产线程零文件 I/O、零字符串格式化、零全局锁、首次注册后零堆分配 |
| `nana-svg-raster` | CPU 侧 SVG 栅格化（`resvg → pixmap`） | 纯位图光栅化工具，不是绘制器，也不维护 SVG DOM |
| `nana-app-icon` | 默认应用图标、PNG/ICO/ICNS 编码与 Windows PE 资源嵌入 | 只管图标编码与嵌入；目录打包布局归 `nana-packager` |
| `nana-icons-tabler` | 自动生成的 Tabler 矢量图标常量表（约 32.3 万行） | 脚本生成物，非 workspace member；未引用图标由链接器自动剔除 |
| `nana-package` | 打包契约的**运行时只读侧**（解析 `package.json` 与 `.nrpack`） | 仅含只读逻辑与纯 Rust 解码器，不含打包或密钥生成代码 |
| `nana-packager` | 打包契约的**构建期写侧** CLI | 将已编译可执行文件、资源树与 `nana-package.toml` 打成交付物，绝不随应用发布 |
| `nana-ui-dev` | 开发期热重载公共底座（文件监视、防抖与安全读取） | 顶部带 `#[cfg(not(debug_assertions))] compile_error!`，绝不进入 release/dist 构建 |
| `nana-ui-devtools` | 开发期无头 Agent 自动化会话与离屏快照 | 不含任何面向最终用户的产品 UI；CPU 回读仅限于此处的离屏快照 |

## 内部结构与源码锚点

### 1. `nana-ui-charts`：CPU 求解几何，GPU 顶点插值

`crates/nana-ui-charts/src/lib.rs`（27 行）将图表拆为「CPU 侧布局与命中」和「GPU 侧 WGSL 标记数组」两半：

- **ECharts 形状的纯 Rust 配置（`option.rs`，1,424 行）**：涵盖 `grid`、`xAxis` / `yAxis`、`series`（折线、柱状、饼环、散点、雷达、仪表盘）、`tooltip`、`legend`、`dataZoom`。尺寸支持 `Length::{Px, Percent}`，颜色支持 `ChartColor::{Role(SemanticColorRole), Palette, Rgba}`（`:35`），数值与时间标签通过 `ValueFormatter`（`:45`）与 `TimeFormatter`（`:71`）由应用提供。
- **与 WGSL 严格对齐的 GPU 标记数组（`marks.rs`，307 行）**：头部注释明确声明这里的内存布局就是 `nana-ui` 中 `scene_paint/shader/chart.wgsl` 的二进制合同。每个标记携带目标态 `to` 与起始态 `from`（如 `GpuPoint { to, from, dist: [f32; 4] }`，编译期断言 `size_of == 48`，`:27`；形状类别 `ShapeKind { Rect = 0, Sector = 1, Symbol = 2, Needle = 3 }`，`:38`），使过渡动画直接在顶点着色器中按 `MarkTransition` 进度插值，稳态动画零 CPU 重算开销。
- **Hover 不触发重排版（`layout/mod.rs`，276 行）**：图表布局只依赖 `ChartOption`、容器尺寸、主题、`LabelMeasure`（`:29`，由宿主文本引擎注入）与 `ChartViewState`（`:51`，图例显隐与缩放范围）；指针移动悬停直接查询已完成布局的 `ChartLayout::hover_at`，绝不因鼠标移动重排图表。

### 2. `nana-diagnostics`：四个宏、静态描述符与后台 Flight Recorder

`crates/nana-diagnostics/src/lib.rs`（135 行）提供四个诊断原语：`event!`（`:83`）、`fault!`（`:100`）、`metric!`（`:118`）与 `span!`（`:131`）。

| 宏名称 | 写入通道 | 核心语义与约束 |
| --- | --- | --- |
| `event!` | 线程本地 SPSC 环形缓冲（满则丢弃最新事件） | 低频结构化事实事件（最多 4 个类型化字段） |
| `fault!` | 同上，并写入独立应急环唤醒后台 worker 立即落盘 | 设备丢失、签名失败等关键故障；消息字符串仅在触发时格式化一次 |
| `metric!` | 静态原子变量（Counter / Gauge / Histogram） | **所有高频每帧数据必须走 `metric!`**，由 worker 定期快照，严禁发逐帧 `event!` |
| `span!` | 静态原子直方图 + 作用域守卫 `SpanGuard` | 析构时自动计算单调时钟耗时并写入直方图 |

框架自身的描述符集中在 `crates/nana-diagnostics/src/framework.rs`（635 行，域 `0x0001..=0x0009`），第 2 行即规定 **"IDs are part of the `.nlog` contract: never renumber, only append"**（已退役的旧 ID 保留注释占位，永不挪作他用）。完整用法与 `.nlog` 格式见 [诊断与应用路径（参考）](/reference/diagnostics)。

### 3. `nana-svg-raster`、`nana-app-icon` 与 `nana-icons-tabler`

- **`nana-svg-raster`（`crates/nana-svg-raster/src/lib.rs`，243 行）**：基于 `resvg 0.48.1` 将 SVG 栅格化为 `RasterizedSvg { width, height, rgba: Arc<[u8]> }`（`:7`），按消费场景提供四个入口：`rasterize_stretch`（`:21`，独立拉伸到目标宽高）、`rasterize_white_mask`（`:42`，图标图集白色遮罩，自动将 `currentColor` 替换为纯白）、`rasterize_document`（`:58`，按文档尺寸输出直通 alpha）与 `rasterize_document_capped`（`:71`，带最大边长限制的 URL SVG 解码）。它只在 `nana-ui` 的 `svg` feature 打开时链接：SVG 图片要用它，图标不用。它自己不找字体：`<text>` 用哪些字体文件、通用字体族叫什么，由调用方通过 `SvgFonts` 给出（`nana-ui` 给的是字体集 `resolve_font_faces` 的结果，字节共享不复制），不给就不画文字。`rasterize_white_mask` 只剩一个用处：宿主包了一个只有 SVG、没有 shapes 的图标。
- **`nana-app-icon`（`crates/nana-app-icon/src/lib.rs`，140 行）**：提供 PNG / ICO / ICNS 编码、Windows PE 资源嵌入（`embed` feature）以及运行时默认图标与 macOS Dock `828 / 1024` 网格内缩适配（`runtime` feature，`with_system_grid:32`）。
- **`nana-icons-tabler`（`crates/nana-icons-tabler/src/lib.rs`）**：由 `scripts/generate_tabler_catalog.py` 从 `@tabler/icons 3.46.0` 生成的静态常量表。每个图标导出为 `pub static *_ICON: IconData { name, shapes, svg: "" }`；图集画的是 `shapes`（`nana-ui/src/scene_paint/icon_raster.rs`）：每个形状是 2 个网格单位宽、圆头圆角的轮廓，`IconShape::Fill` 是填充，用的是给字形做栅格的同一个栅格器（`nana_text::raster::path_coverage`），已生成 `shapes` 的图标不再保留原始 `svg` 字符串；由于声明为独立静态常量，链接器会自动剔除所有未被应用引用的图标，不会膨胀最终二进制体积。

### 4. `nana-package` 与 `nana-packager`：打包契约的读侧与写侧

打包系统（Issue #226，详见 [打包与分发合同](/reference/packaging)）严格拆分为运行时读侧与构建期写侧：

- **运行时只读侧 `nana-package`（`crates/nana-package/src/lib.rs`）**：仅使用纯 Rust 依赖（`blake3` pure 后端、纯 Rust `ruzstd` 解码与 `XChaCha20-Poly1305` / `Ed25519`）。定义三级启动资源类别 `ResourceClass { EarlySplash = 0, BootstrapUi = 1, Protected = 2 }`（`pack/mod.rs:30`）、首屏包上限 `EARLY_SPLASH_MAX_PACK_BYTES = 4 MiB`（`:25`）以及开窗前自检协议 `SELF_CHECK_ENV = "NANA_PACKAGE_VALIDATE"`（`lib.rs:42`）。
- **构建期写侧 `nana-packager`（`crates/nana-packager/src/main.rs`）**：提供 `package`、`validate`（含 `--run` 与 `--tamper-suite` 六项破坏性篡改测试）、`keygen`、`inspect`、`delta`、`macos-app` 六个子命令（`:32-38`），支持基于上一版产物的 SteamPipe 增量块复用（`--baseline` / `--cache`）。

### 5. `nana-ui-dev` 与 `nana-ui-devtools`：开发期热重载与无头验证

- **`nana-ui-dev`（`crates/nana-ui-dev/src/lib.rs`，87 行）**：封装路径级静默防抖器 `Debouncer`（`watch.rs:25`）以及两档重载请求 `ReloadRequest { Full, Template }`（`lib.rs:46`）。第 `:22` 行的 `#[cfg(not(debug_assertions))] compile_error!(...)` 从编译期杜绝它进入发布包（详见 [开发期热重载合同](/reference/hot-reload)）。
- **`nana-ui-devtools`（`crates/nana-ui-devtools/src/lib.rs`）**：提供按 feature 逐层递进的无头自动化会话（`offscreen` ⊂ `runtime-agent`，见 `lib.rs` 头部与 `agent/mod.rs`）：纯 Rust 的 `RuntimeAgentSession` 通过标准输入输出 JSON 协议执行指针点击、键盘输入、无障碍树导出与离屏 PNG 截图比对。`headless-host` 在 `runtime-agent` 之上加 `HeadlessHost<P>`：用 `nana_ui::headless::HeadlessRuntime` 按原生宿主的钩子顺序跑整个 `RuntimeProgram`（含它开的辅助窗口），再画出、回读各窗口。**全仓唯一的 GPU → CPU 像素回读严格限制在 `offscreen` 模块内**，产品 `run_runtime` 呈现路径绝不调用它。

## 仓库周边生态全景

除 `crates/` 下的 24 个核心 crate 外，工作区还包含以下目录：

| 目录路径 | 包含内容与定位说明 |
| --- | --- |
| `examples/`（5 个工作区成员） | `component-gallery`（全控件画廊，附带 `ui-benchmark` / `ui-snapshots` / `ui-migration-ab` / `ui-host-ab` 四个二进制）、`reactive-sfc`（`.vue` Rust 脚本示例）、`native-rhi-probe`、`runtime-host-fixture`、`package-fixture`（打包与防篡改校验被试程序） |
| `platform/android/` | 实验性 Android ARM64 `GameActivity` 宿主（共享 `UiWorld` → `UiScene` → `SceneWgpuPainter`，详见 [Android 实验宿主](/reference/android)） |
| `tools/css-parity/` | 149 组 CSS 盒模型一致性夹具与比对工具（另有 workspace 外的 `tools/css-parity-webview` 用于本机 WKWebView 对照） |
| `perf/` | 性能回归基准套件（涵盖 `compositor-steady`、`windows-composition-steady`、`nana_text_ids` 与 `nana_gpu_scale_ids`） |

## 延伸阅读

- [GPU 与帧生命周期](/learn/gpu-frame)：`nana-ui-charts` 生成的 GPU 标记数组如何被 `chart.wgsl` 消费
- [作者工具链](/learn/toolchain)：`nana-ui-dev` 的 `watch_templates` 如何驱动 `nana-ui-sfc` 的静态文本热替换
- [24 crate 覆盖矩阵](/learn/coverage-matrix) · [证据清单](/learn/evidence-checklist)
