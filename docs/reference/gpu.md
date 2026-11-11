# 实时画面与 GPU 集成合同

本文规定 NanaUI 的宿主 GPU 上下文（`GpuContext` / `FrameContext` / `GpuTexture`）、实时纹理挂载（`HostTexture` / `GpuTextureView`）、同 pass 直写（`GpuView`）、跨线程最新帧交换（`FrameExchange`）以及按图离屏生产（`SceneResourceProducer`）的完整技术合同。

关于无窗口的保留输出、目标规划与多消费方（如 Spout / 录屏器）路径，请参见 [窗口外输出（ExternalSurface）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/output.md)；本文所述的 `GpuContext`、`FrameContext` 与 `GpuTexture` 同样是该输出边界的唯一设备、提交与资源所有者。

## 后端选择、GPU 策略与 HDR 呈现

### 1. Native RHI 决策门（Issue #186）与 WGPU 后端自选

Issue #186 的评估结论为 **NO-GO**：WGPU 仍是 NanaUI 的唯一正式图形后端，正式路径保持 `Logical GPU ABI -> WgpuBackend -> wgpu`（详见 [Issue #186 交付记录](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/consumer-upgrade-2026-09-25-issue186.md)）。

框架 crate 不会越俎代庖地替应用锁死图形后端：workspace 的 `wgpu` 默认只开启 `std`、`parking_lot` 与 `wgsl`；`nana-ui` 的默认特性 `wgpu-backends` 在启用 `gpu` 时会打开当前平台的全部标准后端（Windows 为 DX12 / Vulkan / GLES，Apple 为 Metal，Linux 为 Vulkan / GLES）。如果需要缩减体积、只发行特定图形后端，请在依赖中设置 `default-features = false`，并在应用自己的 `wgpu` 依赖上按平台显式列出所需后端——`hosted_context` 只会在实际编入的后端中选择适配器。DX12 下着色器编译器（`dxcompiler.dll` vs FXC）的选择规则见 [两阶段启动 · DX12 着色器编译器](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/startup.md#dx12-着色器编译器)。

### 适配器选择

宿主只在能呈现主窗口的适配器中选择，选择在启动线程完成，设备丢失后的替换设备按同一规则重选：

- 应用经 `StartupOptions::with_adapter(GpuAdapterChoice)` 指定。`Matching(GpuAdapterId)` 按 vendor／device id 匹配，同型号多卡再比 PCI 总线地址，任一方没有总线地址时比名称；该卡不在、不能呈现主窗口或建不出设备时退回自动选择，不算启动失败。
- `Automatic`（默认）沿用 wgpu `request_adapter` 的规则：设置了 `WGPU_ADAPTER_NAME` 时取名称包含它的第一块，否则按 `WGPU_POWER_PREF` 稳定排序后取第一块；都未设置时就是枚举顺序的第一块。应用的 `Matching` 优先于这两个环境变量。
- 程序在 `RuntimeProgramContext::gpu_adapters()` 读到本次设备创建时的 `GpuAdapterSelection`：可呈现主窗口的适配器（枚举顺序）、实际所用的一块，以及所指定的卡是否被跳过（`requested_missing`）。没有替宿主选择设备的上下文（headless、staged、嵌入式注入）返回空列表或只含注入设备一项。设置界面应读这份值，不再自行枚举适配器。
- 运行中不切换适配器：应用保存新的选择，下次启动生效。

### 2. 统一 GPU 策略（Issue #184）

`GpuContext::policy()` 按设备代际（`DeviceGeneration`）维护共享的管线注册表（pipeline registry）与工作计数。`SceneWgpuPainter` 的静态管线通过它跨窗口复用底层 WGPU 对象；`GpuWorkSink` 将缓冲上传与重分配统一记入设备统计（`GpuPolicyStats`：含 `upload_bytes`、`upload_writes`、`upload_copies`、`upload_flushes`、`upload_ring_allocations`、`upload_ring_waits`、`frame_slot_waits`、`frame_slot_stalls` 等），公开边界始终保持为纯 `nana-gpu` 类型。

### 3. HDR 显示信息与色彩空间合同（Issue #283）

Surface 的 HDR 能力由 `SurfaceCapabilities::format_capabilities` 决定，而 `wgpu::Surface::display_hdr_info` 返回的是当前显示器的运行时建议值（仅用于调节最终的色调映射 tone-map：Apple EDR 提供相对 headroom，Windows 提供峰值亮度与 SDR 参考白 nit 值，缺失仅代表未知而非 SDR，不能单凭它决定是否开启 HDR Surface）：

- **事件驱动刷新**：宿主在创建 Surface、设备恢复以及窗口移动、缩放、resize 等显示事件中缓存并更新 `DisplayHdrInfo`（稳态帧不轮询操作系统，Metal 查询始终在事件线程执行）。数值未变时不触发重绘；数值变化时仅更新呈现 uniform，不重建 Runtime 或 Scene 状态。安全 headroom 取值为 `DisplayHdrInfo::tone_map_headroom().unwrap_or(1.0)`，SDR 参考白缺省按 `80 nit` scRGB 约定处理。嵌入式宿主收到系统显示通知时，可在窗口线程调用 `EmbeddedRuntime::refresh_display_hdr_info()`（或 `HostedGpuContext` 的同名方法）。
- **线性 scRGB 工作空间**：`SceneWgpuPainter` 的内部合成工作空间统一为**预乘线性 scRGB（premultiplied linear scRGB）**。图表标记（`ScenePrimitiveKind::Chart`）由折线、面积、形状三条解析式抗锯齿管线绘制（边缘为 1 设备像素过渡，不依赖 MSAA），顶点与样式数组按 revision 常驻于目标 GPU 缓冲，首次遇到图表时才惰性编译管线。
- **跨线程与离屏导出的色彩元数据**：`FrameExchange` 仅按字节复制纹素而不做色彩空间转换。生产端需要跨线程传递色彩空间时，应调用 `copy_from_with_metadata` 附带 `FrameColorMetadata`（声明原色 primaries、传递函数 transfer 与 alpha 语义），消费端从 `FrameLease::color_metadata()` 或 `FrameBinding::color_metadata()` 读取（旧 `copy_from` 标记为 `Unknown`，消费端严禁将其静默假定为 sRGB）。`FrameBinding` 默认只直接接收全范围 sRGB 或 80-nit 线性 scRGB；已声明的 P3、PQ、HLG、有限范围或非预乘 alpha 帧会留在收件箱中不被误采样，需由生产端先行转换或由自定义消费端处理。
- **`nana-ui-devtools::offscreen` 快照边界**：固定使用 `BGRA8_UNORM_SRGB`。导出 PNG 应使用 `readback_image` / `paint_image` 返回的 `SnapshotImage` 并调用 `into_png_srgb` 或 `write_png_with_metadata`（自动完成预乘 sRGB 到直通 alpha sRGB 的转换并写入 PNG sRGB 块）。

### 4. 帧上传环（UploadRing）、帧槽（Frame Slots）与缓存

- **批量合并上传（`UploadRing`）**：渲染器不散乱调用 `queue.write_buffer` / `write_texture`，而是通过 `GpuWorkSink` 将写入追加到当前 `FrameContext` 的上传批。调用 `FrameContext::submit` 时，设备待落地批（含帧外写入与此前被丢弃帧的写入）与本帧上传批通过一次 `memcpy` 拷入 `UploadRing`（上限 `32 MiB`）的一个映射块，合并为单个上传命令缓冲并在同一次 `queue.submit` 中排在渲染命令之前落地，从而消除每次小写入在 WGPU 内部的 staging 分配（文本字形块仍走 encoder 内按录制位置的 `copy_buffer_to_buffer`，以保证单次提交前同一目标多次绘制各自看到正确的字形，见 #224；同一帧内第二个及之后的 paint 写入的字形图集区域同样按录制位置拷贝，见下文“同帧多目标的图集顺序”，计入 `upload_bytes` 但不计入 `upload_writes` / `upload_copies`）。
- **有界帧槽（Frame Slots）**：每个宿主帧（`begin_frame` / `try_begin_frame`）在录制期占用一个帧槽（discard 立即归还，submit 在 GPU 完成回调中归还；生产者帧不占帧槽，见 §9）。槽满时 `begin_frame()` 对最早已提交的帧做一次有界等待，等到即释放该槽；`try_begin_frame()` 则在槽满时立即返回 `None`。窗口呈现路径使用 `try_begin_frame()`，且**必须在拿到帧槽之后才调用 `get_current_texture`**（拿不到槽时先非阻塞 `poll` 一次，仍满则直接跳过本帧，让已上屏的缓冲继续留在屏幕上，避免透明交换链取走缓冲后等待导致整窗空白）。
- **代际化缓存与 URL 纹理共享**：管线与资源布局注册表采用按 `DeviceGeneration` 隔离的 `StampedCache`（满时线性淘汰最久未用的 1/8，并在锁外并行编译管线）。`SceneWgpuPainter` 持有唯一的 `UrlTextureCache`，供 Quad 背景、`border-image` 与 HostTexture 遮罩共享（最多并发拉取 4 个 HTTP 资源，闲置上限 256 项 / `64 MiB`，连续 120 个绘制帧未引用后自动释放；栅格图解码上限 `4096 px` / `64 MiB`，SVG 上限 `2048 px`）。
- **跨 profile 共享 painter 核心**：headless `PresentationDriver` 的不兼容输出组共用一个 `SceneWgpuPainter`。与 profile 无关的部分只存一份：文本引擎、shape / raster 缓存与字形图集、`UrlTextureCache` 与 motion 资源；图标图集属于目标状态，每个绘制过的 `RenderTargetId` 各一份（另有 painter 自身默认目标的一份）。每个工作格式（及其 URL 源编码）保留一份管线集（quad、mesh、图标与文本程序、HostTexture、backdrop、chart），切换 profile 时换入，首次使用时构建。URL 纹理按 egress 与源编码（sRGB 或 legacy 线性直写）分桶，两种编码互不覆盖。每个目标终身只用一个 profile；共享图集为任一 profile 淘汰或搬移字形都会推进 placement epoch，使其他目标保留的 `PreparedBatch` 重建。窗口宿主仍按 profile 各持有 painter。
- **同帧多目标的图集顺序**：一个 `FrameContext` 的上传批在提交时整体排在该帧所有 pass 之前，而字形图集被该 painter 绘制的所有目标共享。因此同一帧内第二个及之后的 paint 把字形区域改为在该帧 encoder 的录制位置上 `copy_buffer_to_texture`，排在此前已录制的 pass 之后，后一组淘汰、复用或 compact 前一组的字形矩形时，前一组仍采样自己的字形。首个 paint 仍走上传批，单目标帧成本不变。窗口与它的同 profile 输出在一帧内共用 painter 时同样适用。这些拷贝随帧的 encoder 存在：帧被丢弃时它们没有落地，painter 在下一次绘制前让图集忘掉全部字形位置，各目标重新放置并重建批次。一个 painter 同一时刻只向一个帧录制：宿主提交或丢弃当前帧之后再向下一帧绘制，乱序提交的两个帧会让一帧的图集写入落在另一帧的 pass 之下。
- **同帧多场景的 motion 顺序**：motion 的描述符、keyframe 和时钟缓冲也由 painter 共享。同帧第二个及之后的 paint 只将变化的数据按 encoder 录制位置上传，避免后一个场景覆盖前一个场景的动画；首个 paint 继续使用帧上传批，相同 motion 数据不重复上传。按序拷贝记入上传字节，不增加提交或 CPU 等待；丢帧通过同一共享资源账本使 motion 的 CPU 上传缓存失效，下一帧重新上传。此规则同时覆盖 `paint` 和 `paint_target`。
- **painter 资源计数**：`SceneWgpuPainter::text_glyph_counters()` 报告字形图集、raster 缓存与字形上传；`resource_counters()`（`ScenePainterResourceCounters`）报告图标图集个数与字节、URL 纹理个数、已上传 GPU 字节（含全部 mip，不含驱动填充）、待上传 CPU 字节与进行中的加载。headless 宿主经 `PresentationDriver::text_glyph_counters()` / `painter_resource_counters()` 读取共享 painter 的同一组数字。均为 painter 分配的纹素字节，不是驱动显存。
- **目标附件与管线复用**：UI-only 与 GPU 节点混排之间切换时，只增加或释放 4x MSAA 附件，保留同尺寸、同格式及同 presentation profile 的线性颜色目标、绑定和组缓存。缩放必须重建尺寸相关纹理，但设备管线缓存命中时不再创建着色器模块或管线布局，避免多目标首帧重复解析 WGSL。改变尺寸、输出格式或 presentation profile 仍按原合同重建目标。


### 5. 跨启动的着色器与管线缓存

设备带 `PIPELINE_CACHE`（宿主设备在适配器提供时都会请求）的每个 `GpuContext` 自有一个管线缓存，并通过 fork 的 `wgpu::Device::set_default_pipeline_cache` 设为设备默认缓存。所有不指名缓存创建的管线都经过它——各场景家族、表面与文字材质、图表程序，以及在原始设备上自建管线的渲染器。DX12 上它存放编译好的着色器字节码，缓存里有的阶段不再交给 DXC/FXC 编译 naga 写出的 HLSL；Vulkan 上它就是驱动的 `VkPipelineCache`。Metal 与 GL 没有（`pipeline_cache_key()` 为 `None`）。

框架只交出、收回字节，放在哪里由宿主决定：

- `GpuContext::pipeline_cache_key()` 说明字节属于谁；`file_stem()` 为 `dx12`（字节码与适配器无关）或 `vulkan-<vendor>-<device>`。
- `load_pipeline_cache(&bytes)` 在建 painter 之前替换缓存；`pipeline_cache_data()` 返回要保存的字节。`restore_pipeline_cache` / `persist_pipeline_cache` 经由 `PipelineCacheStore`（`load` / `save`）做同样的事。
- 宿主应用用 `NanaApplicationBuilder::pipeline_cache(store)`（或 `StartupOptions::pipeline_cache`）交出存储：`nana-startup-gpu` 线程在 painter 编译前恢复、建完后保存。
- 之后编译的管线（材质、图表、按需建的家族）由 `PipelineCacheSaver` 保存：它在自己的线程上每 `PipelineCacheSaver::DEFAULT_INTERVAL`（10 s）看一次设备缓存经手的管线数（fork 的 `wgpu::PipelineCache::pipelines_built`），一批编译停下来之后（相邻两次计数相同）才保存，且只在缓存确实变长时保存（`GpuContext::persist_pipeline_cache_if_grown`）：命中缓存的管线和空闲都不写；空闲时一次检查只读一个计数。宿主正常退出时（`WindowManager` 析构，与存储的最终刷写同一处）再保存一次增长的部分。丢失的设备不保存。
- 设备丢失后，宿主在换上新设备、painter 与程序渲染器重建之前从同一存储恢复缓存（`switch_gpu`），所以 TDR 之后的重建与热启动一样跳过编译器；这次读取在事件线程上，和恢复本身阻塞等待新设备是同一段，不在帧路径上。没有用 `pipeline_cache(store)` 的宿主自己调用上面几个接口即可，`PipelineCacheSaver::spawn(&gpu, store, interval)` 也可直接使用。
- 保存的字节在 wgpu 自己的头部与校验键之外再包一层信封（魔数、键、长度、校验和），DX12 载荷逐条校验。不是缓存、属于别的键、被截断或损坏、或被后端拒绝的字节一律拒收（`PipelineCacheLoad::Refused`），设备照常编译，就像什么都没载入；不会产生后端校验错误。驱动更新会在 wgpu/驱动内部使 Vulkan 数据失效，DXC 更新经校验键使 DX12 数据失效，两者都静默重编。

#### 种子缓存取代预编译 DXIL 的设计说明

未实现，只是取舍。随包附带一份种子缓存，让首次启动也是热的，从而去掉 `crates/nana-ui/src/scene_paint/precompiled/` 里 passthrough 的 DXIL？DX12 缓存条目的键是 naga 写出的 HLSL、入口点、目标 profile（阶段与 shader model）和编译选项（`InstanceFlags::DEBUG`、去调试信息的 workaround、设备是否开了 `SHADER_F16`/`SHADER_I16`）的哈希，整份字节又由校验键绑定到编译器（种类、版本、DXC 提交）。种子要在用户机器上命中，这些量就都得在构建时定下来：

- **固定 shader model。** naga 本身支持 SM 6.0（`naga::back::hlsl::ShaderModel::V6_0`，预编译 DXIL 就是 `*_6_0`），不需要改 fork：wgpu 已有 `Dx12BackendOptions::force_shader_model`（`unsafe ForceShaderModelToken::with_shader_model`），wgpu-hal 取 `min(适配器上限, 指定值)`，指定 `V6_0` 后在每个能跑 DXIL 的适配器上都写 SM 6.0。代价在整台设备：需要更高 SM 的特性（`SHADER_F16` 要 6.2，网格着色器与光线查询要 6.5，64 位原子要 6.6）随之消失，应用经由 `shared_device_features` 请求的也一样；框架自己的管线不需要它们。这个指定只能在用 DXC 时设：FXC 没有上限，指定值会让它去编 SM 6.0，而 FXC 只到 5.1。另外调试与发布的选项不同，种子得按发布构建的标志生成。
- **随包附带 `dxcompiler.dll`。** wgpu 只加载这一个文件（`Dx12Compiler::DynamicDxc { dxc_path }`）。DXC v1.8.2502 的 x64 `dxcompiler.dll` 是 17 986 592 字节（约 17.2 MiB），而预编译的 11 个 DXIL 阶段加生成代码一共约 195 KB。种子只对生成它的那个 DLL 构建命中，应用自带别的 DXC 版本时整份被校验键丢弃。
- **FXC 回退。** FXC 的身份是固定的 `fxc d3dcompiler_47`，shader model 恒为 5.1，所以 FXC 种子反而在任何 Windows 上都能命中，什么都不用附带。但缓存键的文件名 `dx12` 对两种编译器相同，一份存储只能放一种字节，另一种被校验键拒收后整份重编；要覆盖两者就得随包附带两份种子，按设备实际拿到的编译器挑一份。预编译 DXIL 则在两种编译器下都以 passthrough 载入。
- **建议：保留预编译 DXIL，不做种子缓存，也不在框架层固定 shader model。** 预编译路径不依赖编译器、不需要 17 MB 的 DLL、不削减设备特性；运行期缓存已让第二次起的启动对其余管线（材质、图表、按需建的家族）也是热的。若以后确有首启需求，种子应作为应用层选项：已自带 DXC 的应用在自己的构建里用同一个 DLL、发布标志和 `force_shader_model = V6_0` 生成种子；或为不带 DXC 的应用只做一份 FXC 种子，覆盖预编译之外的框架家族——两者都先实测再定。

### 6. 实测：wgpu-core 校验占比与输入到呈现延迟（2026-11-11）

`third_party/wgpu` 有两项 fork 改动等实测后再定：(a) 让可信的框架 pass 跳过 wgpu-core 的逐 draw 校验与资源跟踪；(b) 基于 frame-latency waitable 的帧节奏（frame pacing）。测量机器为 Windows 11、RTX 5060 Ti、60 Hz 显示器，release 构建并设 `CARGO_PROFILE_RELEASE_DEBUG=limited`。探针只是测量工具（示例程序和脚本），产品帧路径上没有新增任何代码。

**Painter CPU。** `stack-sampler` 每 2 ms 挂起 bench 最忙的线程，持续 8 s；每次只读一次寄存器、拷一次栈（每样本 25–37 µs），再用 dbghelp 遍历这份拷贝（含内联帧）。`perf/stack_attribution.py` 把每个样本记到最靠叶端、有明确归属的帧上（`alloc`/`core`/堆/`memcpy` 帧记给它们的调用者）。“阻塞”指样本落在内核等待里。每帧 CPU = 同一窗口内线程 CPU 时间 ÷ 帧数。“回放内 core”指 `encode_render_pass` 下 wgpu-core 自身的帧：每条录下的 draw 和 `set_*` 都在这里校验、跟踪并回放。它是跳过校验最多能省下的部分，因为其中还有必须保留的簿记。

| 场景 | 后端 | 每帧 draw | 每帧 CPU µs | 阻塞 | NanaUI | wgpu-core | hal + 驱动 | 回放内 core |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `ui-dense-2k` | DX12 | 4 | 275 | 91% | 10.0% | 13.3% | 70.5% | 11.9 µs |
| `host-textures-64` | DX12 | 68 | 1405 | 32% | 6.4% | 12.2% | 76.5% | 110 µs |
| `shader-nodes-256` | DX12 | 14 | 247 | 86% | 33.4% | 11.6% | 50.5% | 13.4 µs |
| `text-paint-color` | DX12 | 3 | 3584 | 0% | 93.4% | 0.8% | 5.3% | 10.9 µs |
| `text-constraint-resize` | DX12 | 2 | 19584 | 0% | 96.1% | 0.5% | 2.1% | 35 µs |
| `ui-dense-2k` | Vulkan | 4 | 104 | 90% | 14.4% | 19.0% | 62.5% | 8.7 µs |
| `host-textures-64` | Vulkan | 68 | 1322 | 32% | 8.3% | 14.2% | 74.8% | 117 µs |
| `shader-nodes-256` | Vulkan | 14 | 182 | 84% | 36.1% | 11.1% | 50.0% | 9.5 µs |
| `text-paint-color` | Vulkan | 3 | 3918 | 0% | 93.5% | 1.2% | 4.9% | 19.9 µs |
| `text-constraint-resize` | Vulkan | 2 | 21690 | 0% | 97.2% | 0.5% | 1.4% | 22.1 µs |

占比以非阻塞样本为分母。其余为 `wgpu` 前端（≤1.3%）和 OS/其他帧。

- Painter 把 2,000 个节点合成 4 次 draw，逐 draw 校验几乎没有可放大的对象。UI 场景受 GPU 限制：主线程在 `GpuContext::begin_frame` 里等帧槽（`Device::maintain`）。
- `host-textures-64` 是“大量小 pass”的情形（bench 自己的 64 个 producer pass 加上 UI pass），开销在驱动。其中 25% 是 bench 自己的 `queue.write_buffer`。在 `command_encoder_finish` 内部，42% 是 wgpu-core 为每个 pass 新开一条 hal 命令列表（`InnerCommandEncoder::open_pass` → `begin_encoding`，DX12 每帧 290 µs，Vulkan 528 µs）。`finish` 里只有 19% 是 wgpu-core 自身代码。
- NanaUI 自身的开销按 CPU 的包含式占比统计：文字在 `ui-dense-2k` 占 3.5%，在 `text-paint-color` 占 57%（主要是 `RuntimeDocument::flush`，painter 占 9.6%），在 `text-constraint-resize` 占 84%。图标在 `ui-dense-2k` 占 0.5%。`gpu-scene-*` 场景都不画矢量路径，所以这次测量没有覆盖路径的开销；路径另见 §10。

**(a) 结论：不做。** 跳过校验，普通场景每帧最多省 9–35 µs，65 个 pass 的合成场景约省 110–117 µs。在文字为主的帧里不到 CPU 的 1%，这类帧 93–97% 是 NanaUI 自身的工作。只有当真实场景每帧出现数百个 draw 或 pass 时才值得重看；即便那样，逐 pass 开命令列表才是更大的目标。

**输入到呈现延迟。** `input-latency-probe` 打开一个普通的 hosted 窗口（Mailbox，`desired_maximum_frame_latency` 为 1，wgpu 默认的 `Dx12UseFrameLatencyWaitableObject::Wait`）。驱动线程每隔随机 120–250 ms 投递一次 `WM_LBUTTONDOWN`，每次按下让窗口换色。时间戳全部取 QPC：`post` 在 `PostMessageW` 之前；`input` 在 `RuntimeProgram::input_event` 里（降级和路由之后）；`prepare` 和 `presented` 在帧钩子里（`present` 返回之后，并附上宿主各阶段 gauge）；`screen` 是 Desktop Duplication 第一次出现新颜色的合成桌面帧的 `LastPresentTime`。wgpu 没有为 swapchain 暴露 `GetFrameStatistics`，所以用 DWM 的呈现时间代替 DXGI 帧统计。DWM 把多次更新折进同一帧的样本已剔除。下表单位都是 ms，每行 n = 161–297。

| 窗口 | 后端 | post→input | input→prepare | prepare→presented | presented→screen | **post→screen p50 / p90 / p99** | `acquire` p50 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 空闲（按需） | DX12 | 0.37 | 0.19 | 0.88 | 7.01 | **8.5 / 12.8 / 15.9** | 0.024 |
| 空闲（按需） | Vulkan | 0.37 | 0.20 | 0.75 | 5.55 | **7.0 / 11.0 / 15.2** | 0.039 |
| 持续 60 fps | DX12 | 0.46 | 0.12 | 15.44 | 16.32 | **32.4 / 38.7 / 40.3** | 14.64 |
| 持续 60 fps | Vulkan | 0.34 | 0.14 | 0.56 | 5.48 | **6.6 / 10.1 / 13.4** | 0.029 |
| 持续 60，waitable `DontWait`（对照） | DX12 | 0.33 | 0.12 | 0.69 | 55.66 | **59.3 / 68.1 / 74.1** | 0.019 |
| 持续 60，waitable `None`（对照） | DX12 | 0.34 | 0.12 | 0.64 | 53.55 | **56.5 / 66.7 / 72.2** | 0.019 |

除 post→screen 外，各格都是 p50。对照两行来自一个临时、未提交的补丁：在 `hosted_context::instance_descriptor` 里按 `WGPU_DX12_USE_FRAME_LATENCY_WAITABLE_OBJECT` 设置 `backend_options.dx12.latency_waitable_object`。

- 空闲 UI 是 NanaUI 的大多数帧：输入 p50 约 8.5 ms（DX12）或 7 ms（Vulkan）上屏。其中从投递到呈现的 NanaUI 部分约 1.4 ms，约 82% 是 DWM 等下一个 60 Hz vblank 再合成。帧槽（`reserve` 0.004 ms）、swapchain 和 waitable 都没有增加延迟。
- 会排队的是持续重绘的 DX12 窗口。帧已经应用了输入之后，`acquire` 还要在 frame-latency waitable 上阻塞 14.6 ms，呈现出的帧随后又要再等一个刷新周期才被 DWM 合成。waitable 本身不能去掉：关掉它，DXGI 会排约三帧，p50 升到 56–59 ms。同一台机器上 Vulkan 不阻塞（6.6 ms）。

**(b) 结论：DX12 上做，范围限于持续重绘的窗口。** 不是去掉等待，而是挪位置：在帧开头、取输入和准备帧之前等 waitable，而不是在 `get_current_texture` 里等。wgpu 已经创建了这个句柄（`Dx12UseFrameLatencyWaitableObject::DontWait`，hal 的 `dx12::Surface::waitable_handle()`），fork 只需加一个让宿主能等它的增量接口。下面是估算，不是测量值：在等待期间到达的输入仍要等完剩下的部分，所以 p50 预计能省下 14.6 ms 阻塞的一半左右，约 32 → 25 ms。要接近 Vulkan 的约 7 ms，还需要对齐相位（刚好在 DWM 合成前呈现）。空闲的按需 UI 没有收益。Metal drawable 时序没有测（没有 macOS 主机），继续搁置。

命令（在仓库根目录执行，S 为临时目录）：

```sh
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 12 --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu
cargo build -j 12 --release -p nana-ui --example stack-sampler --example input-latency-probe --features hosted,bundled-fonts
# 每个后端（dx12 | vulkan）和场景各跑一次
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 3000 --duration-ms 8000 --interval-us 2000 \
  --out $S/dx12-ui-dense-2k.folded -- target/release/nana-gpu-scene-benchmark.exe \
  --scenario perf/scenarios/gpu-scene-ui-dense-2k.json --sample-seconds 12 --output $S/dx12-ui-dense-2k.bench.json
python perf/stack_attribution.py $S/dx12-ui-dense-2k.folded            # 输出表格；--json 输出汇总
WGPU_BACKEND=dx12 target/release/examples/input-latency-probe.exe --clicks 300 --output $S/lat-dx12.json
WGPU_BACKEND=dx12 target/release/examples/input-latency-probe.exe --clicks 300 --continuous 60 --output $S/lat-dx12-c60.json
```

### 7. 实测：持续映射的上传路径（2026-11-11）

设想的 fork 改动：新增一种创建即映射、永不 unmap 的缓冲，NanaUI 用帧槽 fence 自证 GPU 不在读某段，再直接写进去，省掉 `UploadRing` 的映射块与拷贝命令、文本 staging ring 的 `write_buffer_with` 与 `copy_buffer_to_buffer`。统一内存设备上 GPU 直接读这块内存；独显上用 upload heap / ReBAR。**结论：不做**，理由是下面两组测量。测量机器同 §6（RTX 5060 Ti 独显，另加 Core Ultra 7 265K 的 Intel 核显），release 构建。Metal 没有测（没有 macOS 主机）。文本引擎文档里「`write_buffer` 每次约五万条指令」的数字来自 Metal，那里是统一内存，也是这项改动收益最大的地方，有 Mac 时值得重测。

**单测上传路径（`crates/nana-gpu/examples/upload-cost.rs`）。** 每帧在一个 render pass 里清一张 256×256 目标，再把 `writes` 段、每段 `bytes` 字节写进各自的目标缓冲，然后 `FrameContext::submit` 并等这一帧完成（任何时候只有一个提交在飞）。计时用 `QueryThreadCycleTime`，阻塞等待不计入。下表是「写入到提交返回」与「等待这一帧」两段线程周期中位数之和（2000 帧），减去不上传的同一帧，单位千周期（约 3.8 千周期 = 1 µs）。`ring` 是 `UploadRing`；`text` 是文本路径（一次 `write_buffer_with` 加每段一条拷贝）；`copy` 只录拷贝命令，源已在 GPU 上；`memcpy` 是同样的字节拷进一块一直映射着的 upload 内存，即持续映射路径的下限。示例另有 `queue`（每段一次 `queue.write_buffer`），表里略去。

| 适配器 | 后端 | 空帧 | 4×256 B：ring / text / copy / memcpy | 32×1 KiB：ring / text / copy / memcpy | 8×64 KiB：ring / text / copy / memcpy |
| --- | --- | ---: | --- | --- | --- |
| RTX 5060 Ti | DX12 | 370–416 | +230 / +282 / +101 / +12 | +199 / +277 / +140 / −5 | +759 / +567 / −122 / +157 |
| RTX 5060 Ti | Vulkan | 310–394 | −52 / −71 / −71 / −85 | +131 / +138 / +114 / +15 | +741 / +225 / −16 / +246 |
| Intel 核显 | DX12 | 99–134 | +59 / +102 / +37 / +4 | +143 / +277 / +155 / −10 | +165 / +271 / +100 / +31 |
| Intel 核显 | Vulkan | 67–71 | +22 / +30 / +11 / −5 | +108 / +114 / +130 / +32 | +154 / +109 / +75 / +65 |

NVIDIA 两个后端的空帧本身在 ±50 千周期内波动，表里的负数就是这个噪声。

- 小上传每帧多花 6–75 µs（`ring` 减 `memcpy`），这是持续映射最多能省下的。其中大半是驱动处理拷贝命令和多一条命令缓冲的成本（`copy` 一列），staging 本身只占一部分。
- `write_buffer_with` / `write_buffer` 每次调用都要让 wgpu-core 建一块 staging 缓冲：NVIDIA DX12 上一次约 160–180 千周期（约 45 µs）。文本路径在每个写 instance 的帧付一次。
- `ring` 在 NVIDIA 上写 512 KiB 时多出约 750 千周期，比把同样字节拷进映射内存贵得多，Intel 上没有这个现象。原因没有查明（这一格没有采样），是 `UploadRing` 自身的问题，与 fork 无关。

**真实场景里的占比。** 用 §6 的 `stack-sampler` 采 `gpu-scene-ui-dense-2k`、`text-paint-color`、`host-textures-64`，以及 `nana-text-paint-benchmark` 一万标签的 `static` 与 `mutate-1pct`，按包含式计：`nana_gpu::upload::record`（取映射块、拷贝、unmap、录拷贝、`map_buffer_on_submit`）在每个场景都不超过非阻塞 CPU 的 0.5%；`ui-dense-2k` 的稳态帧根本不上传。文本 staging（`write_buffer_with` 加 `copy_writes`）在 DX12 的 `static` 和 `mutate-1pct` 里不超过 0.5%。上传命令缓冲在 `queue_submit` 里多出的部分采样分不出来，按上表 `copy` 一列估计每帧不超过约 40 µs，对 `text-paint-color` 约 3.6 ms 的帧是 1% 左右。NVIDIA 的 Vulkan 驱动在等待里自旋，那几组的百分比被稀释，所以结论只用 DX12 的数。`host-textures-64` 和 `ui-dense-2k` 里显眼的 `queue.write_buffer`（18% 与 4–7%）是 bench 自己的 GpuView 预览每帧写 uniform，不是框架上传。

**不做的理由。** 能省的上限是有上传的帧 1% 左右的 CPU，静止帧为零。代价是新增一类错误：CPU 写进 GPU 可能正在读的内存，正确性完全押在帧槽 fence 上，wgpu-core 的校验也管不到它。实现还要改 wgpu-core 的映射状态、提交时对已映射缓冲的拒绝、初始化跟踪，以及 `MAP_WRITE` 不能与 `STORAGE`/`VERTEX` 共用一个缓冲的规则（今天只有原生特性 `MAPPABLE_PRIMARY_BUFFERS` 放开它）。独显没有 ReBAR 时，GPU 每次 draw 都要隔着 PCIe 读 instance 数据，CPU 省下的会变成 GPU 时间。如果以后上传真的成了热点，先在 NanaUI 这一侧做：让文本 staging 以帧的 `UploadRing` 块为拷贝源（块在提交前已 unmap，帧 encoder 里的拷贝可以读它），省掉每帧一次 staging 分配；再查明 512 KiB 那一格。

命令（仓库根目录；S 为临时目录）：

```sh
cargo build -j 12 --release -p nana-gpu --example upload-cost
WGPU_BACKEND=dx12 WGPU_ADAPTER_NAME=nvidia target/release/examples/upload-cost.exe   # 另跑 vulkan、intel
UPLOAD_COST_ONLY=ring,8,65536 target/release/examples/upload-cost.exe                # 只跑一格，时长十倍
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 12 --release -p nana-ui --bin nana-text-paint-benchmark --features gpu
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 4000 --duration-ms 8000 --interval-us 2000 \
  --out $S/dx12-text-mutate-1pct.folded -- target/release/nana-text-paint-benchmark.exe \
  --labels 10000 --workload mutate-1pct --frames 2000
python perf/stack_attribution.py --scope "upload::record" $S/dx12-text-mutate-1pct.folded
```

`gpu-scene-*` 场景的采样命令同 §6。`nana-text-paint-benchmark` 现在按 `WGPU_BACKEND` 选后端。

### 8. 实测：局部呈现（2026-11-11）

光标闪烁、悬停这类小改动的帧在 DX12 上的开销，以及带脏矩形呈现与 damage 跟踪的设计和分阶段计划，见 [局部呈现与 damage 跟踪](./partial-present.md)。结论：现在不做。

### 9. 提交预算：不让应用拖垮桌面

设备只有一条队列，桌面合成器（Windows 上的 DWM）也排在同一块 GPU 后面。一个进程排进几秒的工作，整个桌面都会卡住；早期一个不限速的探针就这样冻住过整台机器。`GpuContext` 因此按**通道**（`FrameLane`）限制尚未完成的提交数：

| 通道 | 入口 | 上限 | 谁会等 |
| --- | --- | --- | --- |
| `Presentation` | `begin_frame` / `try_begin_frame` | 帧槽数（默认 3） | 只等自己通道最早的提交 |
| `Producer` | `begin_producer_frame` / `try_begin_producer_frame` | `GpuDeviceState::set_producer_submission_budget`，默认 **2** | 只等生产者通道最早的提交 |

- **生产者**（视频、模型、`CustomRenderNode` 背后的离屏工作、`FrameExchange` 的拷贝）使用 `begin_producer_frame`。它不占帧槽，所以生产者再忙也不会让窗口帧拿不到槽；预算满时 `begin_producer_frame` 阻塞到本通道最早的提交完成，`try_begin_producer_frame` 非阻塞 `poll` 一次后返回 `None`。`FrameContext::submit` 在取 `submission_order` 锁**之前**再按预算检查一次（并发录制的多个生产者也不会超额），所以等待永远不会挡住另一条通道的提交。`FrameExchange::copy_from` 已走生产者通道，预算满时返回 `CopyOutcome::PoolFull`。
- **宿主帧**由帧槽定节奏。帧槽全被未提交的录制占住时，帧会无槽运行（`gpu.frame_slots_exhausted`）；这些帧仍受同一上限约束，提交前等待本通道最早的提交，而不是无限堆积。
- 计数是保守的：一条提交只有在完成回调或一次等待证明它已完成后才离开，所以真实的队列深度不会超过统计值。`GpuPolicyStats` 报告 `producer_submission_waits`、`producer_submissions_peak` 与 `presentation_submission_waits`；指标 `gpu.submission_budget_waits` 计等待次数。
- 默认值 2：一条在执行、一条已备好，足以让生产者保持忙碌，又不会让它在合成器前排出多帧工作。每帧工作很轻、需要更多并行的生产者可以调高；单帧很重的生产者应保持 1–2。

**预算管不到的地方**（如实说明）：

- `wgpu-interop` 的裸 `queue.submit` 与 `GpuContext::flush_uploads` 不计入预算。逃生口按原样交出队列，调用方自己负责限速（最多 1–2 条在途，见 `AGENTS.md` 的 GPU 负载规则）。
- 队列深度管不住**单条**跑几秒的绘制或计算。Windows 在约 2 秒后触发 TDR 重置设备；单次等待超过 5 秒的提交会照常放行并上报 `gpu.submission_budget_overrun`，不会把提交线程永远挂住。设备丢失后等待立即结束，宿主按 `is_lost()` 换上新的 `GpuContext`。
- 设备丢失由 `poll` 报告，不再 panic：fork 的 `wgpu::Device::poll` 在后端报设备丢失时（Vulkan 的 `VK_ERROR_DEVICE_LOST`；DX12 上围栏读出 `u64::MAX` 且 `GetDeviceRemovedReason` 报设备已移除）返回 `PollError::DeviceLost`，同时把设备标为无效、只调用一次设备丢失回调，并把所有提交当作已完成，挂起的完成回调都会执行；之后每次 `poll` 都再报这个错误（见 `third_party/wgpu/README.zh-CN.md`）。框架内的 `poll` 与等待（帧槽等待、提交预算、`GpuContext::poll`、场景宿主与 `FrameExchange` 的 poll）都经 `GpuContext` 进行，遇到这个错误就把上下文标为丢失，即使上下文由 `from_wgpu` 接管、没有安装丢失回调；上传环自己的 poll 忽略它，交给上下文的下一次 poll。测试用 fork 的 Vulkan 测试钩子与 D3D12 的 `RemoveDevice`，没有真实的 TDR 验证。

实测（2026-11-11，RTX 5060 Ti，DX12，Mailbox，`surface-resize-probe --frame-context --resizes 200`，生产者连续提交、不自行等待，仅靠预算 2 限速；每档 20 秒上限，逐档加载，桌面始终可用）：

| 负载 | 窗口帧 p50 / p95 / max | 生产者峰值在途 | 生产者等待 | 窗口通道等待 |
| --- | --- | --- | --- | --- |
| 无生产者 | 4.0 / 6.5 ms（一次 522 ms 离群值，与生产者无关） | 0 | 0 | 0 |
| `--load 8` | 5.3 / 8.2 / 10.1 ms | 2 | 1043 / 1047 次提交 | 0 |
| `--load 16` | 6.3–6.4 / 9.2–9.6 / 15.9–33.7 ms | 2 | 618–626 / 620–628 次提交 | 0 |
| `--load 16`，裸 wgpu 生产者（每条提交后自行等待） | 8.9–9.3 / 50.6–55.2 / 108–109 ms | 1 | — | — |

```bash
cargo run -p nana-ui --example surface-resize-probe --features gpu,wgpu-interop --release -- --frame-context --load 4
```

`--load` 上限 16，`--resizes` 上限 200；不要同时运行两个 GPU 探针。

### 10. 实测：矢量路径的 CPU 网格化（2026-11-11）

问题：矢量路径能不能直接在 GPU 上画，省掉 CPU 上的转换？现在的路线（`nana-ui-scene` 的 `custom_paint.rs`）是：展平曲线 → `i_overlay` 求填充规则、描边轮廓与裁剪交集 → `lyon` 三角化并加抗锯齿边带 → painter 的 `MeshPipeline::push_path` 把顶点变换到场景坐标、`PathBuffers::upload` 上传。§6 的场景都不画路径，这次补上。测量时产品帧路径上没有改任何代码；之后按结论第 1 项做的改动及其前后数字见本节末尾。

**场景。** 两个新场景，用 `nana-gpu-scene-benchmark` 跑（`--features gpu,rich-text`）：

- `gpu-scene-paths-canvas`：一个 1200×720 的自绘画布，96 格，每格一个三次曲线闭合填充、一条圆头描边曲线（每四格一条虚线）、一个斜接三角描边；每五格一个 even-odd 圆环，每八格一个带阴影的星形；每三格的内容在圆形路径裁剪里。共 332 条路径，每条一个网格，约 13.5 万顶点。
- `gpu-scene-paths-mermaid`：一篇 Markdown，含一个中等流程图（23 个节点、29 条边、两个子图）和一个时序图（5 个参与者、16 条消息、一个循环、注释）。156 个网格，约 12 万顶点。

每个场景跑四种情形，由 `--path-animation` 选：静止（`none`）；滚动（`scroll`，列表每帧上移 1.7 px）；合成缩放（`scale`，列表在 0.9–1.1 倍之间缩放）；重录（`rerecord`，画布每帧换一个缩放重画，`paint_key` 随之改变；Mermaid 每帧改一行注释，相当于实时编辑图表）。bench 每帧在 `path_counters` 里报告场景持有的网格数，以及其中不是上一帧同一个 `Arc` 的网格数（即重新网格化的）。

**方法。** 测量机器同 §6（RTX 5060 Ti，Core Ultra 7 265K），DX12，release 构建并设 `CARGO_PROFILE_RELEASE_DEBUG=limited`。`--serialize` 让每帧提交后等 GPU 完成，任何时候只有一条提交在途。每种情形跑 3 轮，每轮先不采样跑 5 秒（取 `framework_cpu_prepare_ms`，即 Runtime flush 加 painter 录制），再用 §6 的 `stack-sampler` 每 2 ms 采样 6 秒。每帧 CPU = 采样窗口内主线程 CPU 时间 ÷ 窗口内帧数。`perf/stack_attribution.py --path-stages` 把每个非阻塞样本按调用栈只记到一个阶段（构建内依次判断 `i_overlay`、`lyon`、展平、边带）。下表都是 3 轮的中位数，方括号里是最小–最大值。

**机器负载。** 测量期间这台机器上一直跑着一个 ML 训练任务。每轮开始时记录的 GPU 利用率为 27–98%（多数在 90% 以上），显存占用 12.8 GB，系统内存 47 GB 里用了约 25 GB。表里的量都是进程内的 CPU 时间。帧率含串行等 GPU 的时间，受训练任务影响，只作参考，不作结论依据；GPU 计时没有测。

| 场景 | 情形 | 每帧重建网格 | prepare p50 ms | 每帧 CPU µs | 构建 µs | 顶点变换 + 上传 µs | 每帧上传 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| canvas | 静止 | 0 / 332 | <0.01 | 182 [160–218] | 0 | 0 | 0 |
| canvas | 滚动 | 0 / 332 | 3.8 [3.7–4.0] | 6745 [6441–7093] | 0 | 6059 [5715–6394]（90%） | 9.2 MB |
| canvas | 合成缩放 | 0 / 332 | 4.5 [4.5–4.9] | 7789 [7403–8937] | 0 | 7170 [6511–7981]（89%） | 9.1 MB |
| canvas | 重录 | 332 / 332 | 33.2 [31.1–34.4] | 34976 [33555–36438] | 25769 [23759–25793]（71%） | 8454 [7561–8903]（24%） | 9.3 MB |
| mermaid | 静止 | 0 / 156 | <0.01 | 246 [244–263] | 0 | 0 | 0 |
| mermaid | 滚动 | 0 / 156 | 4.1 [4.0–4.3] | 6100 [6011–6547] | 0 | 4880 [4775–5136]（79%） | 8.1 MB |
| mermaid | 合成缩放 | 0 / 156 | 3.9 [3.9–4.3] | 7074 [6894–7168] | 0 | 6312 [5943–6357]（88%） | 8.0 MB |
| mermaid | 重录 | 156 / 156 | 24.2 [23.9–24.8] | 23392 [22986–26825] | 17419 [16940–19630]（74%） | 2908 [2719–2950]（12%） | 0 |

“构建”是展平、`i_overlay`、`lyon` 与边带四段之和；“顶点变换 + 上传”是 `push_path`、`PathBuffers::upload`（逐块比较并写入）和提交时把这些字节拷进上传环三段之和。括号里是占每帧 CPU 的比例。Mermaid 重录时几何不变，新顶点与上一帧逐字节相同，比较后不上传。

重录时构建内部的拆分（µs/帧，括号为占每帧 CPU 的比例）：

| 场景 | 展平 | `i_overlay` | `lyon` 三角化 | 抗锯齿边带 / 阴影带 | 其他 | 图表解析 + 布局 | 录制 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| canvas | 318（0.9%） | 16321（45%） | 5892（17%） | 2843（8%） | 327（0.9%） | — | 59 |
| mermaid | 136（0.6%） | 8713（37%） | 4435（19%） | 3832（16%） | 348（1.5%） | 578（2.3%） | 77 |

滚动时变换与上传的拆分：

| 场景 | `push_path` µs | 比较 + 写入 µs | 拷进上传环 µs |
| --- | ---: | ---: | ---: |
| canvas | 1769 | 1866 | 2484 |
| mermaid | 1439 | 1717 | 1778 |

- **静止：第二帧完全复用。** 没有一个网格被重建；painter 连路径都不碰（prepare 不到 0.01 ms，零上传）。录制按（`paint_key`、布局尺寸、主题代数）缓存，场景按录制缓存网格，两层都命中。
- **滚动与合成缩放：不重新网格化，但每帧要搬全部顶点。** 重建数为 0，合成缩放也一样。可这两种帧 79–90% 的 CPU（4.9–7.2 ms）花在搬顶点上：`push_path` 每帧在 CPU 上把每个顶点变换到场景坐标，`PathBuffers::upload` 比较并重写整份约 8–9 MB 的顶点数组，提交时再拷一次进上传环。索引不变，没有上传。这和网格化无关，也不需要 GPU 覆盖率才能解决：把顶点按节点局部坐标留在 GPU 上，每次 draw 传原点与仿射即可（保留文本条目就是这样做的）。
- **重录：构建占大头。** 每帧要 17–26 ms（canvas 每条路径约 78 µs，mermaid 每个网格约 112 µs）。其中 `i_overlay` 最大（37–45%），`lyon` 17–19%，边带 8–16%，展平不到 1%。Mermaid 的解析与布局只占 2%。
- 网格偏重：canvas 每个网格平均 406 个顶点，mermaid 767 个（展平容差 0.05 逻辑 px，加上圆角连接与边带）。顶点数直接决定上面滚动时的搬运量。

**结论：不做（不用 GPU 端解析覆盖率替换展平 + `i_overlay` + `lyon`）。** 判据如下：

1. 稳态帧（静止、滚动、合成缩放）里重新网格化的开销超过每帧 0.5 ms（60 Hz 帧预算的 3%）。实测为 0，不满足。
2. 某个受支持的工作负载每帧都重录路径，且在先做完下面第 2、3 项这类更便宜的 CPU 改进之后，构建仍超过每帧 4 ms（60 Hz 预算的四分之一）。原始构建是 17–26 ms，超过了开销门槛；但只有内容持续重录时才会出现，例如把缩放或动画放进 `paint_key` 的画布，或每次按键都重新解析的图表（这种通常有节流，不会到 60 Hz）。框架自己的路径都不会每帧重录：Markdown 图表只在源码改变时重录。
3. 普通文档首帧一次性构建造成超过 50 ms 的卡顿。实测是 17–26 ms，不满足。

另外，GPU 端覆盖率不是免费的：曲线仍要上传，描边、虚线和路径裁剪仍要处理；Vello 式 compute 要用计算着色器（wgpu 的 GLES 后端只在 GLES 3.1 以上才有），还会把工作搬到一块正被其他任务占满的共享 GPU 上。更便宜、收益更确定的做法，按先后：

1. 路径顶点按节点局部坐标常驻 GPU，每次 draw 传变换。可省掉滚动与合成缩放帧 79–90% 的 CPU，不碰网格化。（已实现，见下文。）
2. 让 `lyon` 能直接处理的路径绕开 `i_overlay`：没有裁剪的简单填充，以及描边（用 `lyon` 的 `StrokeTessellator`）。`i_overlay` 占构建的 37–45%。
3. 复核展平容差和圆角连接的密度（每个网格 406 / 767 个顶点）。

如果做完 2、3 之后，某个产品仍需要每帧重录 50 条以上的路径（按每条 78–112 µs 计约 4 ms），再重新评估 GPU 覆盖率方案。

**之后：路径顶点常驻 GPU（第 1 项，已实现）。** 网格的顶点与索引在某一帧第一次画它时写进 painter 的一块共享 arena（每个渲染目标一块），保持节点局部坐标，之后不再改动；每次 draw 只送一条 48 字节的实例（仿射、原点、不透明度，以及裁剪、渐变与遮罩的下标）。同一节点相邻的网格，实例相同且在 arena 里相邻时，仍合成一次 draw。抗锯齿边带不在 CPU 上重算：`path.wgsl` 的 `path_screen_extrude` 在顶点着色器里按 draw 的仿射把节点空间的斜接向量映射到屏幕，与原来 CPU 上的 `screen_extrude` 是同一算法。所以任何缩放下边带都是一个物理像素，不需要按缩放阈值重新生成边带，也不用屏幕空间导数。arena 用 `Weak` 记住网格：场景放掉网格的下一帧就回收，120 帧没画过的也回收；常驻字节超过 64 MiB 时只留上一帧画过的。死区多于活区时压缩，用量不到四分之一时缩小。`resource_counters()` 新增 `path_meshes` 与 `path_mesh_bytes`。

同样的命令与机器，前后各 3 轮的中位数（方括号为最小–最大）。测量时训练任务仍在跑，每轮开始时 GPU 利用率 19–100%，显存占用 13.3–13.7 GB。

| 场景 | 情形 | prepare p50 ms 前 → 后 | 每帧 CPU µs 前 → 后 | 顶点变换 + 上传 µs 前 → 后 | 每帧上传 前 → 后 | draw 数 前 → 后 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| canvas | 滚动 | 3.86 → 0.14 | 7124 [6738–7789] → 427 [317–438] | 6267 → 32 | 7.1 MB → < 1 KB | 2 → 16 |
| canvas | 合成缩放 | 4.51 → 0.14 | 8066 [7807–8571] → 321 [238–603] | 7336 → 26 | 7.6 MB → < 1 KB | 3 → 22 |
| canvas | 重录 | 33.6 → 28.6 | 34637 [32343–37586] → 30427 [28900–31635] | 9572 → 5246 | 9.0 MB → 7.4 MB | 3 → 3 |
| mermaid | 滚动 | 4.59 → 0.52 | 6769 [6300–7533] → 858 [825–990] | 5310 → 22 | 6.7 MB → 8 KB | 30 → 32 |
| mermaid | 合成缩放 | 4.04 → 0.15 | 7253 [6912–7738] → 481 [405–510] | 6412 → 23 | 6.7 MB → 8 KB | 28 → 30 |
| mermaid | 重录 | 24.9 → 22.2 | 24979 [23774–26595] → 24236 [23619–25561] | 3034 → 3472 | 0 → 6.6 MB | 28 → 30 |

- **滚动与合成缩放：路径的 CPU 从 5–7 ms 降到 22–32 µs。** 每帧只上传实例（canvas 几十条；mermaid 的 8 KB 主要是文字），整帧 CPU 降到 0.3–0.9 ms。mermaid 滚动剩下的 prepare 是文字与 Runtime 的工作，不是路径。
- **重录没有变差。** canvas 省掉了 CPU 变换，搬顶点从 9.6 ms 降到 5.2 ms。mermaid 重录时几何不变，原来逐字节比较后不上传；现在每个新的 `Arc` 都重新写进 arena（6.6 MB），搬顶点多了约 0.4 ms，但省掉的变换抵消了它，整帧 CPU 与 prepare 都没有变差。
- **静止帧不变。** painter 本来就跳过路径。前后两个版本穿插各跑 3 轮，每帧 CPU 中位数 canvas 201 → 194 µs，mermaid 171 → 203 µs，都在驱动提交 130–330 µs 的波动之内。
- **画面不变。** `retained_paths_paint_what_cpu_transformed_vertices_did_under_any_scale` 把常驻路径与按原来方式在 CPU 上变换的同一批网格逐像素比较（椭圆、三角形、1px 描边、带渐变的旋转方块），节点缩放取 0.5、1、1.5、2，另加 rotate(25deg) scale(1.4, 0.7)。每个通道相差不超过 2/255。
- **第 2 项（简单路径绕开 `i_overlay`）没有做。** 边带生成依赖 `i_overlay` 给出的干净轮廓：外轮廓与洞的方向固定，互不相交。绕开它就要先判断轮廓是否简单、方向是否正确；描边改用 `StrokeTessellator` 又拿不到可以生成边带的轮廓。这不是小改动。重录构建仍是 17–23 ms，`i_overlay` 仍是其中最大的一段。

命令（仓库根目录；S 为临时目录；每次只运行一个 GPU 进程）：

```sh
export CARGO_PROFILE_RELEASE_DEBUG=limited
cargo build -j 4 --release -p nana-ui --bin nana-gpu-scene-benchmark --features gpu,rich-text
cargo build -j 4 --release -p nana-ui --example stack-sampler --features hosted,bundled-fonts
# 每个场景（canvas | mermaid）和情形（none | scroll | scale | rerecord）各跑 3 轮
WGPU_BACKEND=dx12 target/release/nana-gpu-scene-benchmark.exe --scenario perf/scenarios/gpu-scene-paths-canvas.json \
  --path-animation scroll --serialize --sample-seconds 5 --output $S/plain-canvas-scroll.json
WGPU_BACKEND=dx12 target/release/examples/stack-sampler.exe --delay-ms 3000 --duration-ms 6000 --interval-us 2000 \
  --out $S/canvas-scroll.folded -- target/release/nana-gpu-scene-benchmark.exe \
  --scenario perf/scenarios/gpu-scene-paths-canvas.json --path-animation scroll --serialize --sample-seconds 9 \
  --output $S/sampled-canvas-scroll.json
# 每帧 CPU µs = stack-sampler 打印的 thread_cpu_ms × 1000 ÷（bench 帧率 × wall_ms ÷ 1000）
python perf/stack_attribution.py --path-stages --cpu-us-per-frame 6745 $S/canvas-scroll.folded
```

## 挂载宿主纹理（`HostTexture` + `GpuTextureView`）

着色器输出、预览视口、离屏渲染纹理与普通按钮一样，是界面树上占据布局、受裁剪、可命中的节点。首次接入请使用默认路径：把画面画到可采样纹理上，在树上挂载 `GpuTextureView`，并用同一个字符串槽位名在 `host_textures()` 中登记。

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop
```

`run_runtime` 会创建进程唯一的一份 `GpuContext`（附加窗口只新建 Surface 并共享该上下文；外部事件循环则通过 `EmbeddedRuntime` 与 `HostedGpuShared::from_device` 注入现有设备，切勿再次 `request_device`）。

宿主创建及恢复共享设备时，自动启用适配器支持的 BC／ASTC 纹理压缩能力，供内容生产端在同一设备上采样压缩纹理；这不会改变 UI 自身的纹理存储策略。不支持的能力不请求，生产端按设备实际能力选择格式或回退。外部注入设备和应用离屏宿主须在创建设备时自行请求所需的压缩能力。

:::api

```rust view
// 树上：和 Button 一样占布局
let preview = cx
    .mount_view_root(document_id, || view! {
        <Texture resource="preview" />
    })?
    .root::<GpuTextureView>();
```

```rust rust
// 树上：和 Button 一样占布局
let preview = cx
    .mount_view_root(document_id, || widget(GpuTextureView::new("preview")))?
    .root::<GpuTextureView>();
```

:::

在 `RuntimeProgram` 中登记并按帧更新该纹理槽：

```rust
// 1. initialize / rebuild_gpu：在宿主设备上创建纹理并登记槽位
let gpu = context.gpu();
let texture = gpu.create_texture(&GpuTextureDescriptor {
    label: Some("preview"),
    width,
    height,
    format: GpuTextureFormat::RGBA8_UNORM_SRGB,
    usage: GpuTextureUsages::SAMPLED | GpuTextureUsages::COPY_DST,
})?;
self.textures.register(
    "preview",
    HostTexture::new(&texture),
    width,
    height,
    HostTextureAlphaMode::Opaque,
);

fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
    Some(self.textures.clone())
}

// 2. flush 之前更新像素或替换纹理
fn prepare_window_frame(&mut self, id: WindowId, context: &RuntimeProgramContext<Self::Message>) {
    // 上传 CPU 像素：context.gpu().write_texture(&texture, region, &pixels, bytes_per_row)
    // 替换纹理对象或尺寸：host.replace_texture(&new_texture)；仅内容更新：host.invalidate()
    // 注意：切勿为了换纹理而去修改 Runtime 节点；窗口遮挡或 0 尺寸时仍可能调到这里，不可假定随后必有 present
}

// 3. present 之后才可释放上一帧被替换下的旧纹理
fn window_frame_presented(&mut self, _id: WindowId, _context: &RuntimeProgramContext<Self::Message>) {}

// 4. 设备丢失恢复：程序实例保留，仅在新的 context.gpu() 上重建 GPU 资源
fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {}
```

- **槽位是唯一身份**：视图只记槽名（`TextureSlotName`），每帧绘制时按槽名取注册表里当前的绑定。`HostTexture::new` 自己分配 id；往同一个槽登记一个新句柄、`replace_texture` 换纹理对象（`generation` 递增）或 `invalidate` 刷新内容（`version` 递增），宿主都会重绘用到这个槽的窗口，不用改 Runtime 节点。设备重建时宿主先清空注册表，再调 `rebuild_gpu`；程序在新设备上重新登记即可。若 `HostTexture::device_generation()` 与当前 painter 的设备代际不符，整帧会以 `ScenePaintError::StaleHostTexture` 拒绝。
- **线性工作空间要求**：进入 Scene 的 `HostTexture` 采样结果必须处于预乘线性 scRGB 空间——普通不透明/直通 alpha 的 sRGB 图片可使用 `*_SRGB` 纹理格式交由硬件采样器解码；带半透明的预乘内容请使用线性 `UNORM` 或浮点格式，勿将 gamma 空间预乘字节误标为 `SRGB`。

### GPU 合同与 wgpu 逃生口

#### 逻辑 GPU ABI（Issue #185）

普通渲染扩展可通过 `nana_gpu::ResourceTable` 声明 uniform / storage 缓冲、采样/存储纹理、采样器、动态切片与可选资源数组，并通过 `ShaderInterface` 绑定可移植 WGSL 与顶点/实例布局（`GpuContext::create_resource_layout` 与 `create_resource_group` 会在创建不透明句柄前校验 `DeviceGeneration`、usage、范围与可选能力，公开 API 不泄漏 `wgpu::BindGroupLayout`）：

| 核心合同类型 | 职责说明 |
| --- | --- |
| `GpuContext` | 进程唯一的 GPU 设备句柄：携带 `generation()`、`capabilities()`、粘性丢失状态 `is_lost()`，提供 `create_texture` / `write_texture` / `begin_frame` |
| `FrameContext` | 单帧独占编码器：`submit()` 提交并返回 `GpuSubmission`；drop / `discard()` 立即作废并自动回滚 painter 登记在该帧上的保留目标状态 |
| `GpuTexture` / `GpuRenderTarget` | 携带 `DeviceGeneration` 的 2D 纹理与渲染目标（跨代际资源在入口即被拒绝） |
| `GpuTextureFormat` / `GpuTextureUsages` | 后端中立的不透明格式与用途标志 |

当自定义渲染器确实需要访问底层原生 WGPU 对象时，显式启用 `wgpu-interop` feature（提供 `GpuContext::from_wgpu` / `wgpu()`、`FrameContext::wgpu_encoder()`、`GpuTexture::from_wgpu` / `wgpu_view()`、`ScenePass::wgpu()` 等，交出的是合同背后的**同一个**设备与编码器）。框架内部 crate（`nana-ui`、`nana-frame-exchange`、`nana-ui-devtools`）则通过隐藏的 `nana_gpu::__framework` 访问后端，由 `scripts/check-engine-boundary.py` 在 CI 中强制把关。

::: tip 队列工作不需要提交守卫
NanaUI 的 wgpu fork 让 `Surface::configure`（窗口缩放）只等仍在使用该 surface 交换链图像的工作，不再等整个设备空闲，其他线程同时提交也不会让它以 `GpuWaitTimeout` 失败（见 `third_party/wgpu/README.md`）。`third_party/wgpu-fork-tests` 在 Windows 上验证这一点：另一个线程提交的同时，在 DX12 与 Vulkan 上反复重配置一个窗口的 surface。因此通过 `wgpu-interop` 直接调用裸 `queue.submit` / `write_texture` / `write_buffer` 时可以在任意线程进行，不必持锁；原来的 `gpu.wgpu().lock_submission()` 已标记为弃用，返回的守卫不再与任何东西互斥。`GpuContext` 内部只保留 `submission_order` 互斥锁，让帧提交、上传批与 native-export 的 signal 按队列顺序排列。
:::

#### 原生纹理导出（`native-export`）

独立创建 WGPU 设备、且没有既有 device-lost callback 的外部生产端，使用 `GpuContext::from_wgpu_with_loss_tracking` 明确把 callback 交给同一 `GpuContext`；所有 clone 的 `is_lost()` / `lost_report()` 共享状态。原 `from_wgpu` 继续保留宿主 callback，不会自动替宿主观察丢失；已有宿主仍自行转发给 embedded runtime。OpenVR / Electron 参考生产端采用前者，避免声明检查 `is_lost()` 却从未安装通知。`controlled_device_loss` 集成测试实际调用 `device.destroy()`，验证 callback 所有权、粘性丢失与替换代际拒绝；这不代表硬件 driver reset 或外部 compositor 的恢复已实测。

在 Windows + DX12 设备上启用 `native-export` feature（对应能力位 `GpuCapability::NativeTextureExport`）后，`NativeExportPool` 可在宿主同一份设备上创建 3 张共享的 `BGRA8` 纹理与一把共享围栏并导出为 NT 句柄，供外部 D3D11 / Spout 消费方零 CPU 回读地跨 API 采样。完整围栏时序（`2k + 1` 就绪、`2k + 2` 释放）与消费方合同请见 [窗口外输出 · DX12 共享纹理](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/output.md#dx12-共享纹理native-export)。

## 跨线程最新帧交换（`FrameExchange` 与 `FrameBinding`）

当实时画面在独立线程上生产（如模型渲染、导播合成、视频解码）时，使用 `nana-frame-exchange` 的 `FrameExchange` 将完成的帧交给窗口，并在窗口侧用 `FrameBinding` 绑定到槽位（生产端仅需依赖 `nana-frame-exchange`）：

```rust
// 生产线程：每个节拍调用 poll，有新帧时录制并 copy_from
let mut exchange = FrameExchange::new(&gpu, frame_exchange::DEFAULT_CAPACITY, epoch, notify);
let inbox = exchange.inbox(); // 发送给 UI 线程

let mut frame = gpu.begin_producer_frame("producer");
scene.render(frame.wgpu_encoder()); // wgpu-interop
frame.submit();
match exchange.copy_from(&rendered, epoch) {
    CopyOutcome::Submitted => {}
    CopyOutcome::PoolFull => {} // 池满直接丢弃本帧，不阻塞等待
    CopyOutcome::EmptySource | CopyOutcome::IncompatibleSource | CopyOutcome::DeviceMismatch => {}
}
exchange.poll();

// UI 窗口线程
let mut binding = FrameBinding::new(
    context.gpu(),
    textures.slot("program"),
    HostTextureAlphaMode::Premultiplied,
);
// 在 prepare_window_frame 中调用：binding.prepare(Some(&inbox), |token| accept(token));
// 在 window_frame_presented 中调用：binding.presented(Some(&inbox), |token| accept(token));
```

- **GPU 内部复制与无锁交换**：`copy_from` 在纹理池内执行一次 `copy_texture_to_texture`（要求源纹理具备 `COPY_SRC`、单采样 2D、非深度格式）。单扇消费窗口建议分配 `3` 个槽位（在途复制、正在显示、已替换待 present），每增加一扇共享消费窗口多加 `2` 个槽位。
- **租约顺序与 Epoch 过滤**：`prepare` 换上新帧后，旧帧一直保留到 `presented` 才归还，生产端需等采样它的那次 UI 提交完成后才会复用该槽位。应用可通过 `set_epoch` 推进场景代次（立即隐藏收件箱中的旧代次帧；在新代次帧尚未发布且 `accept` 仍允许旧帧时，`FrameBinding` 会保持已显示的画面而不闪回透明空白）。

## GpuView：同 Pass 无中间纹理直写

当画面没有中间纹理、必须直接编码进当前 UI 渲染通道时，使用 `GpuView::new(slot_id)`（投影渲染器键为 `"gpu-view"` 的 `CustomRenderNode`，演示程序见 `gpu-view-demo`）：

- **`Inline` vs `Standalone`**：`GpuViewMode::Inline` 直接复用 painter 当前打开的目标 pass（通过 `ScenePass` 设置 viewport 与 scissor）；`Standalone` 则通过 `SceneGpuRenderContext::with_pass` 在同一帧、同一目标上另开一个 `Load / Store` pass。自定义渲染器输出的颜色必须为**预乘线性 scRGB**，管线缓存键应设为 `(context.gpu.generation(), context.target_format, context.presentation)`。
- **保持文档顺序的实例化合批**：若实现 `batch_capacity() > 1` 与 `draw_batch_in_pass`，painter 会把显示列表上**连续紧邻**的同渲染器节点切片交给它一次绘制；中间一旦出现任何普通 Quad、文字、图标、HostTexture 或合成组边界，切片即告终止，从而保证遮挡顺序与逐节点绘制逐比特一致。多节点共用同一着色器时请使用相同的 `(slot_id, revision)` 并将每节点差异放入 `params`。
- **实现 `preparation_version` 以允许帧缓存**：自定义渲染器默认每帧重新 prepare；**只要树上存在一个未实现 `preparation_version` 的自定义渲染器，painter 就会将整帧判定为不可缓存并重建全部绘制命令**。请像内置的 `DefaultGpuViewRenderer` 一样按 `(revision, params)` 实现该方法。

## 媒体与 GPU 槽位合同

| 节点 / 接口 | 底层槽位合同 | 行为说明 |
| --- | --- | --- |
| `GpuTextureView` | `"nana.host-texture"` + 宿主登记的槽位名 | 标准宿主纹理节点 |
| `GpuView` | `"gpu-view"` + 十进制 `slot_id` | 宿主需注册对应 `SceneGpuRenderer` |
| `Video` | `"nana.host-texture"` + `video:{id}` | 宿主推帧（有槽位时不画 `poster`；无槽位仅显示静态 `poster` 图片，不解码音频/视频） |
| `BrowserView` | Runtime 布局锚点 + 宿主原生子视图 | 仅 macOS `WKWebView`（Windows/Linux 返回 `Unsupported`），见下节 |

### 按实际绘制像素准备内容

`HostTextureRegistry::painted_extent(slot)` 返回该槽位在各渲染目标最近一次完整绘制中所需的最大设备像素尺寸（已计入 `ContentFit`、节点自身变换与当前 DPI 缩放系数；当同一槽位被列表缩略图与大图浮层同时引用时，自动取最大消费者尺寸）。生产端可通过 `painted_revision()`、`painted_changes_since(rev, &mut out)` 或 `subscribe_painted(callback)` 监听尺寸需求变化并按真实物理分辨率准备纹理，切勿自行用「上一帧的 `LayoutBox` × 缩放因子」手算代替。

## 应用内浏览器

`BrowserView` 是窗口宿主的原生子视图例外，而非整窗 WebView 壳：Runtime 节点负责布局、无障碍与可见性锚点，宿主管理原生引擎子视图（当前仅支持 macOS `WKWebView`，不进入离屏 Scene 截图）。

### 无头网页画面（WebSurface）

`WebSurface` 将平台网页引擎实例完全运行在窗口视图树之外，按 `max_fps` 截取页面帧（`WebFrame`：紧凑排列的 RGBA8、sRGB 空间预乘 alpha）并在后台线程送入应用提供的 `WebFrameSink`，UI 线程零像素转换、零排队开销。应用可自由决定帧的去向（如写入 `HostTexture` 槽位或自定义合成器）：

- **请求对账与原地更新**：程序在 `RuntimeProgram::web_surface_requests()` 中声明 `WebSurfaceRequest { id, policy, desc, restore_url, revision, command, frames }`（撤回请求即释放引擎），状态事件通过 `web_surface_event` 回流。修改 `desc`（尺寸、缩放、透明、帧率）原地生效；更换 sink 或安全策略则会重建实例。
- **临时弹出交互窗口（`ShowWindow`）**：可将同一个无头页面临时放进可交互的原生窗口（用于完成网页登录或点击页面内设置），期间 Cookie、页面状态不丢失且画面帧持续输出；用户关窗或发送 `HideWindow` 后页面回到屏外并上报 `WindowClosed`。
- **平台后端**：macOS 基于屏外无边框 `WKWebView`（自动检测连续 10 帧静止画面并降频至 4 fps，画面一动立即恢复；可通过 `cargo run -p nana-window --example web-surface-probe` 实测）；Windows 基于独立 STA 线程上的 WebView2 Composition Controller + `Windows.Graphics.Capture`（需系统安装 WebView2 Runtime 并随包分发 `WebView2Loader.dll`）；Linux `web_surface_support()` 返回 `false`。

## 按图离屏

当离屏纹理必须根据当前 Scene 图在采样**之前**动态编码生成时，树上仍挂载 `GpuTextureView`，并在宿主侧实现 `SceneResourceProducer`：

- **可见帧合批提交**：`encode_scene(scene, &mut frame)` 直接录制进宿主获取 Surface 后开启的同一个 `FrameContext`，与 UI 绘制在同一次 `queue.submit` 中提交（编码或绘制任一失败则整帧丢弃，生产者严禁自行提交编码器或提前谎报完成）。
- **隐藏窗口节拍（Hidden Tick）**：当窗口被遮挡、最小化时，`FrameDemand` 到期仍会调用 `prepare_window_frame` 并在无 Surface 的独立 `FrameContext` 上执行 `scene_resource_producers` 后立即提交（尺寸为 0 时仅 prepare 而不执行 producer 编码）。
- **单帧单目标铁律**：`FrameContext` 独占管理帧的提交与丢弃回滚。若前一帧绘制过某个 `RenderTargetId` 但尚未提交也未丢弃，再次绘制该目标会返回 `ScenePaintError::TargetInFlight`。设备丢失记在粘性的 `GpuContext::is_lost()` 上；外部通过 `EmbeddedRuntime` 注入设备时，宿主应在收到设备丢失通知后调用 `EmbeddedRuntime::notify_device_lost()`，并在新设备就绪后调用 `replace_gpu()`。

### GPU 节点的准备缓存与画面修订

`SceneGpuRenderer::preparation_version` 只控制 UI 命令准备的复用。按时间或外部状态绘制的节点即使资源和几何不变，像素也可能每帧变化；这类渲染器应让 `content_version` 返回 `None`。窗口保留输出、窗口外 PresentationDriver 与 Spout 共享输出据此按宿主请求重新绘制，同时仍复用稳定的准备命令。

静态或显式修订的渲染器可返回 `Some(revision)`，并在像素发生变化时递增。为保持已有消费者行为，`content_version` 默认沿用 `preparation_version`；动态渲染器需要显式区分二者。安装了动态渲染器但当前场景未引用它，不会使静态输出持续绘制。

通用文字与形状片元材质见 [通用材质](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/materials.md)。

### 逐帧应用载荷

`FrameExchange<E, M = ()>` 的 `M` 是随 GPU 拷贝一起发布的不可变应用载荷，与 epoch 和 `FrameToken<E>` 身份分离。通过 `new_with_payload` 构造，`copy_from_with_payload`（或 `wgpu-interop` 下的 `copy_from_wgpu_with_payload`）同时提交纹理、颜色元数据和载荷；`FrameLease::payload()` 与 `FrameBinding<E, M>::payload()` 只返回对应 lease / 当前绑定帧的数据。原有无载荷构造与复制接口保持兼容。

载荷在 pending copy、完成发布、绑定和呈现后退役之间与纹理共同保留；丢帧、代际失效和槽回收遵循原规则。更新载荷不切换 epoch、不清空最新帧，也不新增历史索引或扩大纹理池。消费者不得将旧纹理配上另读的最新生产状态。载荷类型由应用定义，框架不依赖应用的坐标或模型类型。
