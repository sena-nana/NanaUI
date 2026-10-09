# 实时画面

无窗口的 retained output、目标规划和多 consumer 路径见
[`Window-independent presentation`](output.md)。本页描述的 `GpuContext`、
`FrameContext` 与 `GpuTexture` 是该 output boundary 的唯一设备、提交和资源
所有者。

## Native RHI 决策门（Issue #186）

Issue #186 当前结论为 **NO-GO**。WGPU 仍是 NanaUI 的唯一正式 backend。正式路径保持 `Logical GPU ABI -> WgpuBackend -> wgpu`。现有 native probe 只覆盖离屏 clear-pass smoke workload。没有 NanaUI RenderPlan。没有第二个真实 GPU-heavy consumer。没有 presentation 或 device-loss A/B 证据。因此不能据此创建 `nana-hal` 或 native renderer。完整条件审计和重新开启条件见 [Issue #186 交付记录](../../archive/docs-notes/consumer-upgrade-2026-09-25-issue186.md)。

## WGPU 后端由应用选择

框架 crate 不替你的应用决定编进哪些图形后端。workspace 的 `wgpu` 关闭默认特性。只留 `std`、`parking_lot`、`wgsl`。`nana-ui` 与 `nana-ui-vue` 的默认特性 `wgpu-backends` 打开平台上全部后端（Windows 为 DX12/Vulkan/GLES，Apple 为 Metal，Linux 为 Vulkan/GLES）。只在启用 `gpu` 时生效。因此按默认特性依赖的应用行为不变。只发行部分后端的应用用 `default-features = false`。在自己的 `wgpu` 依赖上按平台列出后端。`hosted_context` 只在编进来的后端里选择 adapter。DX12 用哪个着色器编译器见 [两阶段启动](startup.md#dx12-着色器编译器)：exe 旁的 `dxcompiler.dll`，没有时用 FXC。框架自身的测试经 dev-dependency 打开全部后端。示例和平台宿主作为应用，自己打开 `wgpu-backends`。

## 统一 GPU policy（Issue #184）

`GpuContext::policy()` 按 `DeviceGeneration` 保存共享 pipeline registry 与工作计数。Scene painter 的静态 pipeline 通过它复用真实 WGPU 对象。`GpuWorkSink` 将上传和 buffer reallocation 记入设备统计。公开边界仍使用 Nana 类型。

### HDR 显示信息与呈现参数（Issue #283）

Surface 的 HDR 能力仍由 `SurfaceCapabilities::format_capabilities` 决定。
`wgpu::Surface::display_hdr_info` 返回的是当前显示器的运行时建议值，只用于
调节最后的 tone-map：Apple EDR 提供相对 headroom，Windows 可能提供峰值和
SDR reference-white 的 nit 值，浏览器或其他后端可能完全没有这些字段。缺失值
表示未知，不表示 SDR；不能用它单独决定是否选择 HDR surface。

Hosted surface 会在创建、surface 恢复/换设备时缓存一次 `DisplayHdrInfo`，并在
窗口移动、缩放或 resize 等显示事件中重新读取。值没有变化时不会触发重绘；值
变化时只刷新 presentation 参数，不重建 Runtime/Scene 状态。稳定帧不会轮询 OS，
Metal 的查询也始终在事件线程完成。安全的 tone-map headroom 取值是
`DisplayHdrInfo::tone_map_headroom().unwrap_or(1.0)`；SDR reference-white 缺失时
采用 80 nit 的 scRGB 约定。诊断会保留请求/实际色彩空间、fallback、headroom 和
reference-white 和 active encoding，便于解释为什么某帧走了 extended-linear、PQ/HLG 或 SDR。
嵌入式宿主如果收到平台自己的 HDR/显示参数通知，应在窗口线程调用
`EmbeddedRuntime::refresh_display_hdr_info()`；直接持有 `HostedGpuContext` 的宿主则调用
其同名方法。两者都只刷新发生变化的 surface，并请求一次重绘；调用应与宿主的
window/event-loop 线程一致（Metal 的 surface 查询有线程亲和性）。Surface 能力表的
变化仍由 resize/recovery/rebind 重新解析 profile；headroom 通知本身只更新 uniform。

Scene painter 的工作像素是 premultiplied linear scRGB；它们不是普通 sRGB 图片。

图表的标记（`ScenePrimitiveKind::Chart`）由 painter 的三条 chart 管线画：折线、面积、形状（矩形、环形扇区、点符号、指针），边缘都是一个设备像素的解析式过渡，不靠 MSAA。点、形状和样式数组按标记的 revision 常驻在 target 的 GPU 缓冲里；画面不变的帧不上传，被回滚的帧随 target 重建。管线在 painter 第一次遇到图表时才编译。
`FrameExchange` 只复制 producer 提供的纹理和格式，不做色彩转换。需要跨线程传递
色彩信息时，producer 调用 `copy_from_with_metadata`（或 raw-WGPU 对应方法），把
`FrameColorMetadata` 一起交给 lease；consumer 从 `FrameLease::color_metadata()` 或
`FrameBinding::color_metadata()` 读取。元数据至少应声明 primaries、transfer 和 alpha
语义，range 未知时也必须按未知处理。为保持旧代码可编译，旧 `copy_from` 仍可用，
但它携带的是 `Unknown`，绝不能由 consumer 默认为 sRGB。交换本身不转换像素；离屏
读回、截图、纹理导出或跨进程 frame exchange 必须保留这些元数据，或者在导出边界
明确转换到目标（例如 sRGB PNG）。不能把 extended-linear、PQ 或 HLG 的字节静默标成
sRGB；需要普通 sRGB 消费者时，先做一次显式 tone-map/编码，再写入带 sRGB 标识的资源。
`FrameBinding` 的默认表面合同只直接接受 full-range sRGB 编码或 80-nit linear scRGB，
且要求 alpha 与 binding 一致；已声明的 P3、PQ、HLG、limited-range、straight alpha
会留在 inbox 中而不会被误采样。应用应先在 producer 端转换，或实现自己的带转换的
consumer，而不能把被拒绝的帧反复请求重绘。

`nana-ui-devtools::offscreen` 是一个明确的 SDR snapshot 边界：`FORMAT` 固定为
`BGRA8_UNORM_SRGB`。跨导出边界应使用 `readback_image` / `paint_image` 返回的
`SnapshotImage`，再调用 `into_png_srgb` 和 `write_png_with_metadata`；它们会验证
premultiplied sRGB → straight-alpha sRGB 转换，并写入 PNG 的 sRGB 标记。旧的
`readback` / `write_png` 只为兼容保留，调用方必须自己遵守同一合同。offscreen
不能用于导出 extended-linear、PQ 或 HLG；HDR 导出必须使用带目标色彩空间和传递函数
的专用路径。带颜色 chunk 的 PNG 之外，不允许把未标记的 PNG 推断成 sRGB。

### 帧上传

renderer 不直接调用 `queue.write_buffer` / `write_texture`。`GpuWorkSink` 把写入追加到当前 `FrameContext` 的上传批（`__framework::frame_uploads`）。`FrameContext::submit` 时，设备上的待落地批（`GpuContext::write_texture`、`GpuContext::write_buffer`、被丢弃帧的写入）与本帧的批一起，用一次 `memcpy` 拷进 `UploadRing` 的一个已映射 chunk。一个上传 command buffer 按目标合并后逐段 copy。并在同一次 `queue.submit` 里排在本帧之前提交。语义与 queue 写入一致。所有写入在本帧命令之前、按调用顺序落地。但每帧不再为每次写入付一次 WGPU 内部的 staging 分配。

`UploadRing` 是整块 chunk 的池（已映射的 buffer 不能被提交使用）。每块只属于一次提交。经 `map_buffer_on_submit` 在该提交完成后重新映射归还。池上限 32 MiB。满时对最近一次提交做有界等待，再按需新建。`GpuContext::write_texture` / `write_buffer` 在帧外写入时进入设备待落地批。由下一次提交落地。自己经 `wgpu-interop` 提交原始 command buffer 并读取上传结果的调用方，先调用 `GpuContext::flush_uploads()`。被丢弃的帧的写入不会丢。renderer 的 CPU 镜像假定 queue 写入已经发生。所以它们在下一次提交时落地（`RetainedWrites` 仍照常回滚 painter 的保留状态）。

文本的 glyph 块与绘制顺序仍走文本自己的 ring。它们必须作为 encoder 内按录制位置的 copy。才能让同一次提交前对同一 target 的两次 paint 各自画到自己的文字（#224 的合同）。每帧只有一次 `write_buffer_with`。

没有帧的测试 / 离屏 encoder（`paint_encoder`）仍直接写 queue。

### 帧槽

`FrameContext` 在录制时占用 slot。discard 立即归还。submit 记录其 `SubmissionIndex`。完成回调归还。槽满时 `begin_frame()` 对最早已提交的帧做一次有界阻塞等待（`gpu.frame_slot_waits`）。不再忙等。若全部槽都被从未提交的录制占用（同一线程持有全部槽），等待永远不会结束。新帧不占槽，直接开始，并报告 `gpu.frame_slots_exhausted`。`try_begin_frame()` 在槽满时立即返回 `None`。FrameExchange 使用该路径返回 `PoolFull`。窗口呈现也用它，并且只在拿到槽之后才 `get_current_texture`：透明交换链的当前缓冲被取走后，窗口线程再去 `poll(Wait)` 会让整窗没有可合成的画面。拿不到槽时先非阻塞地 poll 一次（槽在完成回调里释放），仍然没有就跳过这一帧，已呈现的缓冲留在屏幕上，约 16ms 后重试，按需刷新的窗口也一样。

### 缓存与资源

pipeline 与 resource layout registry 是按 `DeviceGeneration` 的 `StampedCache`。命中只写一次时间戳。满时一次线性选择淘汰最久未用的八分之一。被淘汰的后端对象等到下一次提交完成才释放。pipeline 在 registry 的锁外编译：不同 key 可以在不同线程上同时编译，同一 key 的并发冷请求共用一次编译，编译期间帧槽、上传和池也不被挡住。transient buffer / texture 池按完整描述 key 持有、复用。超出预算淘汰。

普通 resize 保留 `GpuContext` 和静态 pipeline。device replacement 使用新 context。旧设备资源不能进入新设备。HostTexture 本身就是本设备上的纹理（generation 已校验）。没有需要「realize」的东西。此前按 identity/version 缓存 HostTexture 的 realization cache 已删除（它把连续内容的每个版本都塞进静态缓存）。`url(...)` 图片由 `SceneWgpuPainter` 持有的唯一一个 `UrlTextureCache` 负责。quad 背景、border-image 与 HostTexture mask 共用它。同一 URL 每个 painter 只抓取、解码、上传、保留一次（仍按 fetch host 分桶）。图片就绪后两条管线各自丢弃按 target 保存的 URL 绑定。

`GpuWorkObservation` 记录 renderer workload 的逻辑 payload 与 draw 统计。`GpuPolicyStats` 记录设备级计数：`upload_bytes`、`upload_writes`、`upload_copies`、`upload_flushes`、`upload_ring_allocations`、`upload_ring_waits`、`frame_slot_waits`、`frame_slot_stalls` 等。
迁移说明见 [交付记录](../../archive/docs-notes/consumer-upgrade-2026-09-24-issue184.md) 与
[上传重写](../../archive/docs-notes/consumer-upgrade-2026-09-28-issue184-uploads.md)。

着色器、预览视口、离屏纹理和按钮一样，是树上的一块内容。有位置。会被裁切。点得到。不是盖在界面上的一层。也不是抠出来的洞。

先把一扇普通窗口跑通。见 [创建应用](../guide/essentials/application.md)。再把画面挂上去。心智模型见 [框架如何运行](how-it-works.md)。

第一次接入：把画面画到可采样纹理，树上挂 `GpuTextureView`。Vue 用 `NanaGpu` / `<nana-gpu>`。Live2D 多层、预览视口都是这条。

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop
```

这个演示的生产线程用自己的管线画画面，所以要开 `wgpu-interop`。只上传 CPU 像素、或把 `GpuTexture` 交给 `FrameExchange` 的宿主用不到它（见 [GPU 合同与 wgpu 逃生口](#gpu-合同与-wgpu-逃生口)）。

`examples/runtime-host-fixture` 同结构。现场直写见 [GpuView](#gpuview)。

## 挂上去

`run_runtime` 创建**一份**设备。就是 `GpuContext`（`nana-gpu` 定义，`nana-ui` 再导出）。附加窗口只新建 Surface。共享同一个 `GpuContext`。已有外部事件循环时，用 `platform_host::EmbeddedRuntime` 和 `HostedGpuShared::from_device(instance, GpuContext::from_wgpu(..))` 注入宿主设备。不要再 `request_device`。

树上挂 `GpuTextureView::new("preview")`。`host_textures()` 登记同一 slot。`prepare_window_frame` 用 `context.gpu()` 更新纹理。

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

```rust
// initialize / rebuild_gpu：在宿主设备上建纹理，登记 slot
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
    HostTexture::new(1, 1, &texture),
    width,
    height,
    HostTextureAlphaMode::Opaque,
);

fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
    Some(self.textures.clone())
}

fn prepare_window_frame(&mut self, id: WindowId, context: &RuntimeProgramContext<Self::Message>) {
    // CPU 像素：context.gpu().write_texture(&texture, region, &pixels, bytes_per_row)
    // 换纹理时 host.replace_texture(&new_texture)，内容更新用 HostTexture::invalidate()
    // 不要为了换纹理改 Runtime 节点
    // 不能 present 时 FrameDemand 仍会走到这里，包括 0 维；不要假定随后有 Surface
}

fn window_frame_presented(&mut self, _id: WindowId, _context: &RuntimeProgramContext<Self::Message>) {
    // present 之后才能释放上一帧的纹理
}

fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
    // 设备丢失后程序实例还在，只把依赖旧设备的资源在 context.gpu() 上重建
}
```

`create_texture` / `write_texture` 在交给后端前就校验设备代次、usage、尺寸上限、区域与字节长度。出错返回 `GpuError`。不会触发 WGPU 校验 panic。它们不需要任何 wgpu 类型。自己录 pass 画纹理时才需要下面的逃生口。

`HostTexture` 用稳定 slot 和 generation 包住可采样的 `GpuTexture`。宿主替换或改尺寸时加 generation。NanaUI 只重建绑定。不拆布局。`revision` 的高 32 位是 generation、低 32 位是内容 version（`pack_gpu_revision`）。`HostTexture::device_generation()` 是纹理所在设备。它与 painter 的设备不同时（设备替换后没有在 `rebuild_gpu` 里重建），整帧以 `ScenePaintError::StaleHostTexture` 拒绝。不会把旧设备的纹理交给后端。

HostTexture 的像素在进入 Scene 时必须已经是 premultiplied linear scRGB；如果源是普通 sRGB 图片，使用 sRGB-typed texture 让采样器解码（只适合 straight/opaque 颜色），不要把 gamma-premultiplied 字节标成 sRGB。带透明度的 premultiplied 内容使用线性 UNORM 或浮点格式。这样纹理、背景、文字和自定义 GPU 节点都在同一个线性工作空间混合。

`HostTexture::instance_identity()` 区分使用相同公开 id 的不同 handle。克隆共享该身份。自接事件循环使用 URL 图片时，给 `SceneWgpuPainter::set_image_waker` 安装唤醒回调。完成通知后请求重绘。内建宿主已接入。没写尺寸的 `<img>` 按图片解码出的尺寸布局：每帧 present 之后用 `take_image_natural_sizes` 取这个目标的记录，用 `commit_image_natural_sizes` 交给画的那个文档，返回 `true` 时再画一帧。画家只记录，不写文档。内建宿主已接入。见 [布局 · 替换内容](layout.md)。离屏工具可用 `has_pending_images()` 等待资源后再次绘制。HTTP 获取与解码在后台进行。纹理上传仍使用宿主设备。Quad 与 HostTexture 蒙版共用缓存实现。每个缓存最多同时获取 4 个 HTTP 资源。闲置纹理最多保留 256 项／64 MiB。120 个绘制帧未使用后释放。当前帧工作集按需保留。栅格图片解码限制为 4096 像素边长和 64 MiB 分配预算。SVG 沿用 2048 像素边长上限。URL 图片默认按实际绘制的设备像素在后台重采样成单层纹理。`ImageSampling::Mipmap` 改为解码尺寸加 mip 链、三线性采样。见 [应用 API · 图片采样](application-api.md#图片采样)。

合成顺序就是文档顺序。`"nana.host-texture"` 在主 pass 里、在这个节点该出现的位置采样。不攒到帧尾。多层就是相邻的几张 `GpuTextureView`。不要绕过界面树去直写窗口 Surface。

### GPU 合同与 wgpu 逃生口

#### Logical GPU ABI（Issue #185）

普通 renderer 可用 `nana_gpu::ResourceTable` 声明 uniform/storage buffer、sampled/storage texture、sampler、动态 slice 和可选资源数组。再用 `ShaderInterface` 绑定 portable WGSL 与 vertex/instance layout。声明按 binding 排序，并拒绝重复槽位。可选数组和 indirect/multi-draw 等路径先查 `GpuCapabilities::capability`。不支持时返回结构化 fallback reason。

`GpuContext::create_resource_layout` 把声明映射为当前 WGPU 的 opaque `GpuResourceLayout`。它检查 `DeviceGeneration`、资源数组能力和 binding 上限。公共 API 不泄漏 `wgpu::BindGroupLayout`。pipeline/cache 的身份应把 `ShaderInterface::key()` 与 `ResourceTable::layout_key()` 纳入 Issue #184 的 per-device registry。内置 quad、icon、mesh、motion、text atlas、HostTexture、backdrop copy/blur、destination blit 和 DefaultGpuView 已使用同一逻辑 layout 路径。destination group layer 也使用该逻辑 layout。backdrop 的特殊 pass 操作与 reading-blend 的临时拷贝仍保留在 framework-only WGPU bridge 中。因为它们需要显式的 pass 操作。不能合理抽象的 WGPU 操作仍须显式使用 `wgpu-interop`，并自行遵守 submission/lifetime contract。

资源值通过 `GpuBuffer`、`GpuSampler`、`LogicalResource` 和 `ResourceSet` 提供。`GpuContext::create_resource_group` 在创建 opaque bind group 前检查代次、usage、范围和必需 binding。资源数组与缺失 optional resource 需要能力或 fallback outcome。不能静默把旧设备资源交给当前 pass。

WGPU 是唯一的后端。它不是扩展合同。普通路径只用 `nana-gpu` 的类型：

| 类型 | 是什么 |
| --- | --- |
| `GpuContext` | 进程唯一的设备。`generation()` 是 `DeviceGeneration`，设备替换后换新值；`capabilities()` 是后端、适配器、`max_texture_dimension_2d` 与用到的可选 feature；`is_lost()` 是粘性的丢失状态；`create_texture` / `write_texture` / `begin_frame` |
| `FrameContext` | 一帧的录制，独占 encoder。`submit()` 提交并返回 `GpuSubmission`；丢弃即作废，画过它的 painter 会自动重建受影响的 target |
| `GpuTexture` / `GpuRenderTarget` | 带设备代次的纹理与渲染目标。别的设备上的资源被拒绝，不会进后端 |
| `GpuTextureFormat` / `GpuTextureUsages` | 不透明的格式与 usage |

自带 shader 的 renderer、自己持有设备的宿主、需要 CPU 回读的工具，走显式的 `wgpu-interop` feature：`GpuContext::from_wgpu` / `wgpu()`（adapter、device、queue、`lock_submission()`）、`FrameContext::wgpu_encoder()`、`GpuTexture::from_wgpu` / `wgpu_view()`、`ScenePass::wgpu()`、`SceneGpuRenderContext::wgpu_encoder()`，以及 `nana_ui::wgpu` 再导出。它交出的是合同背后同一份对象。不会创建第二套设备。

框架自己的 crate（nana-ui、nana-frame-exchange，以及 JS WebGPU 门面所在的 nana-ui-vue、快照回读所在的 nana-ui-devtools）经隐藏的 `nana_gpu::__framework` 取后端。不打开 `wgpu-interop`。所以 Vue 应用不会被连带获得逃生口。Cargo feature 仍会跨依赖图统一。依赖图里任何一个 crate 打开它，整棵图都能用。所以守门的是 `scripts/check-engine-boundary.py`。这些 crate 的公开签名、字段、再导出、别名、类型头、trait 的方法与关联类型、公开类型的 trait impl 出现 `wgpu`，必须在 `wgpu-interop` 之下。`__framework` 只允许这些 crate 自己的源码使用。

提交守卫不可重入。持着 `lock_submission()` 时，不要再调用 `FrameContext::submit`、`GpuContext::write_texture`、`FrameExchange::copy_from` 这些会自己取守卫的方法。

#### 原生纹理导出（`native-export`）

Windows 上的 DX12 设备可以把纹理交给另一个 D3D 设备。打开 `nana-gpu`（或 `nana-ui`）的 `native-export` 特性。`GpuCapability::NativeTextureExport` 报告能否使用。`NativeExportPool` 在宿主那一份设备上建 3 张共享的 BGRA8 纹理和一个共享 fence，导出成 NT handle。它不另开 Device/Queue，也不回读 CPU。`stage` 把一张已画好的纹理复制进当前 `FrameContext`。`FrameContext::submit` 在 `queue.submit` 之后、仍持提交守卫时，在设备队列上 signal 这一帧的 ready 值。`finish` 交出 `NativeFrameToken`。消费端先 `accept_release`，在自己的 GPU 时间线上 `Wait(ready)`，读完再 `Signal(release)`。上一帧没释放时 `stage` 返回 `Deferred`，不等待。没被接收就丢掉的 token 由生产端自己 signal 释放。窗口输出（`WindowOutputExport::Native`）每一帧都走这条路。公开类型只有 Nana 自己的类型、`BorrowedHandle` 和 `i64` 的适配器 LUID。完整的消费端合同见 [Window-independent presentation](output.md#dx12-shared-textures-native-export)。

## 跨线程最新帧

画面在另一个线程上产出（模型渲染、导播合成、解码）时，用 `FrameExchange` 把完成的帧交给窗口。用 `FrameBinding` 把它绑到 slot。两者都在宿主那一个 `GpuContext` 上。生产端 crate `nana-frame-exchange` 建在 `nana-gpu` 上，并再导出 `GpuContext` / `GpuTexture` / `DeviceGeneration`。渲染库不必依赖 `nana-ui`。

```rust
// 生产线程：每个 tick 都 poll，复制只在有新帧时做
let mut exchange = FrameExchange::new(&gpu, frame_exchange::DEFAULT_CAPACITY, epoch, notify);
let inbox = exchange.inbox(); // 交给 UI
// 自己录 pass 的生产者：帧也走 FrameContext，提交自动持提交守卫
let mut frame = gpu.begin_frame("producer");
scene.render(frame.wgpu_encoder()); // wgpu-interop
frame.submit();
match exchange.copy_from(&rendered, epoch) { // rendered: GpuTexture
    CopyOutcome::Submitted => {}
    CopyOutcome::PoolFull => {} // 丢这一帧，不等
    CopyOutcome::EmptySource | CopyOutcome::IncompatibleSource | CopyOutcome::DeviceMismatch => {}
}
exchange.poll();

// 窗口
let mut binding = FrameBinding::new(context.gpu(), textures.slot("program"), HostTextureAlphaMode::Premultiplied);
fn prepare_window_frame(..) { binding.prepare(Some(&inbox), |token| accept(token)); }
fn window_frame_presented(..) -> RuntimeProgramUpdate {
    if binding.presented(Some(&inbox), |token| accept(token)) { RuntimeProgramUpdate::redraw(id) } else { RuntimeProgramUpdate::default() }
}
```

- **是 GPU 内复制，不是零拷贝。** `copy_from` 在 slot 池里做一次 `copy_texture_to_texture`。不回读 CPU。源纹理需要 `COPY_SRC`、单采样、单层 2D、非深度格式。否则返回 `IncompatibleSource`。别的设备上的纹理返回 `DeviceMismatch`。都不会触发 wgpu 校验错误。原始 `wgpu::Texture` 用 `copy_from_wgpu`（wgpu-interop）。
- **谁都不等谁。** 池满时 `copy_from` 直接返回 `PoolFull`。UI 读最新帧只做指针交换。GPU 完成通过 `on_submitted_work_done` 回报。只在生产端 `poll` 里处理。
- **提交守卫。** 窗口缩放时 `Surface::configure` 会等 GPU 空闲。同时从别的线程提交会让它 `GpuWaitTimeout`。`copy_from`、`FrameContext::submit`、`GpuContext::write_texture` 自己持提交守卫。生产端自己的 `queue.submit` / `write_buffer` / `write_texture`（wgpu-interop）要持 `gpu.wgpu().lock_submission()`。并在 `poll(Wait)`、sleep 或 surface 操作前放下。
- **容量。** 一个窗口需要 3 个 slot：在途复制、正在显示、已替换但未 present。每多一个绑定同一交换的窗口加 2 个。
- **Lease 顺序。** `prepare` 换帧后，旧帧留到 `presented` 才释放。两次 present 之间最多换一次。lease 归还后，生产端要等 UI 那次提交完成才复用该 slot。隐藏 tick 只 prepare 不 present。所以最多换一次就停住。生产端随后看到 `PoolFull`。
- **Epoch 与接受策略。** `E` 是应用自己的代次（视口、场景……）。`set_epoch` 立刻隐藏邮箱旧帧。旧 epoch 的在途复制不会发布。`FrameBinding` 在同一存活 exchange 等待替换且 `accept` 仍接受旧 token 时保留已绑定帧，不呈现透明占位，也不反复请求重绘；明确拒绝、exchange 关闭或设备变化仍清除。`accept` 是窗口的策略（可见、未过期）。`prepare` 在没有待取帧或待取帧被接受时确认唤醒。所以 `set_epoch` 的唤醒先到、替换帧后发布，也会再叫醒窗口。被拒绝的帧不确认唤醒。所以隐藏窗口不会每帧被叫醒。策略变化时由应用请求重绘。
- **唤醒。** `notify` 在生产线程调用。只负责调度窗口：`drop(window.request_redraw())` 或 `context.dispatch(..)`。不要在里面等。
- **设备重建。** `rebuild_gpu` 后用新 `GpuContext` 重建 exchange 和 binding。`DeviceGeneration` 不同的 inbox 不会被绑定。binding 只收一个 `GpuContext`。设备和代次不可能对不上。已发出的 lease 继续有效。最后一个持有者释放后，旧设备在后台线程销毁。
- **诊断。** `FrameExchange::stats()` 给出 submitted / published / superseded / pool_full / stale_epoch 与占用高水位。读取不在帧路径上分配。

## GpuView

没有中间纹理、必须写进当前 UI pass 时才用。`gpu-view-demo` 是演示。

`GpuView::new(slot_id)` 投影 `CustomRenderNode`。renderer 键是 `"gpu-view"`。

- `scene_gpu_renderers()` 返回 `None` 或空 registry：未注册的 renderer 会报错。
- 示例自行注册演示 painter；应用显式登记自己的 `SceneGpuRenderer`。

`GpuViewMode::Inline` 复用当前 dest pass。`Standalone` 在同一帧、同一目标上另开 pass。Renderer 不得 `request_device`。也不得提交宿主正在录的帧。

`SceneGpuRenderer` 的合同里没有 wgpu 类型：

- `prepare` 收 `SceneGpuPrepareContext { gpu: &GpuContext, target_format: GpuTextureFormat, presentation, bounds, scale_factor, dest_size, gpu_work }`。`target_format` 是当前 Scene 的线性 scRGB 工作目标；P3/HDR 通常是 `RGBA16Float`。`presentation` 描述最终 surface 的色域/传递函数。
- `draw_in_pass` / `draw_batch_in_pass` 收 painter 正开着的 `ScenePass`（`dest_size`、`set_scissor`、`set_viewport`、`restore_viewport`）和同样带 `gpu`、`target_format`、`presentation` 的上下文。
- `render`（独立 pass）收 `SceneGpuRenderContext`，用 `with_pass(label, |pass| ..)` 在当前目标上开一个 Load/Store 的 pass，viewport 是整个目标、scissor 是节点的 clip。

自定义 renderer 写入的 RGB 和 alpha 必须是 premultiplied linear scRGB；不要在节点 shader 中做 sRGB/P3/HDR 转换，也不要把最终 surface format 当作工作 format。所有节点最后由 painter 的一个 presentation blit 统一完成传递函数、P3 矩阵和按 display headroom 调整的 HDR shoulder。缓存管线时按 `(context.gpu.generation(), context.target_format, context.presentation)` 建键。

在 Nana 有自己的 shader ABI（#185）之前，录 draw 仍要经 `wgpu-interop`。`context.gpu.wgpu().device()` 建管线。`pass.wgpu()` 录 draw。按设备建的缓存以 `(context.gpu.generation(), context.target_format, context.presentation)` 为键。跨设备替换保留下来的 registry 不会拿旧设备的管线去画。不同格式的窗口交替也不必重建。内置的 `DefaultGpuViewRenderer` 就这样做。

### 批绘制

`SceneGpuRenderer` 有两个带默认实现的方法。不实现就是原来的逐节点路径：

```rust
fn batch_capacity(&self) -> usize { 1 }
fn draw_batch_in_pass(&self, nodes: &[SceneGpuBatchNode<'_>], pass, context) -> usize { 0 }
```

`batch_capacity() > 1` 时，painter 把 **display list 上连续**的同 renderer 实例、无 `dedicated_pass`、bounds 非空的节点作为一段交给你。返回值是这段前缀里你实际编码了几个。返回 0 就退回 `draw_in_pass`。你可以只吃掉前缀。例如只处理共享同一 clip 的那几个。

这**不是**把 GPU 内容攒到帧尾。run 是 display list 的连续切片。中间任何一条 Quad / Text / Icon / HostTexture / backdrop / 合成组边界都终止它。shader 节点不可能跨过一个 Button。document order 与不批处理时逐比特相同。变的只是 draw 次数。

内置的 `DefaultGpuViewRenderer` 是实例化的参考实现。一条 instance-step 顶点缓冲。N 个相邻同 renderer 节点一次 draw。不需要任何可选 device feature。**上限**：合并的是同一条管线的 run。每个节点一个不同 shader 时，下限就是每种管线一次 draw。

多节点共用一个 shader 时，用**一个** `slot_id`、**一个** `revision`。把差异放进 `params`。`params` 不参与 resource 冲突判定。也不使 frame plan 失效。对一部分节点 bump `version` 而对其余不 bump，会让整帧被拒（见下面「不要做的」）。需要各自独立 revision 时，给每个节点一个自己的 slot。渲染图按 renderer 而不是按 resource 建 pass。所以这不再随节点数增加 pass。

`SceneGpuRenderContext` 带 `dest_size`（目标的物理像素尺寸）。`with_pass` 开出的 Standalone pass 的 viewport / scissor 与主 pass 一致。

节点上的 `palette` 和 `seed` 走 `CustomRenderNode::params`。槽位见 `gpu_view_params`。Runtime 只搬运这串数。语义由 renderer 键定义。换 renderer 就换一套自己的槽位约定。

```bash
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

## 媒体槽（Canvas / video / iframe）

这些不是浏览器。可见输出仍然只走 Runtime → UiScene → `SceneWgpuPainter`。

默认 GPU 接入是 **字符串 slot** 的 `GpuTextureView` / `"nana.host-texture"`。`GpuView::new(slot_id: u64)` 只给 `"gpu-view"` 直写 pass。**不是** `HostTextureRegistry` 的键。`HostTexture::id` 是 painter 缓存键。同样不是 slot。

| 节点 | 槽位合同 | L1 行为 |
| --- | --- | --- |
| `<nana-gpu>` / `data-nana-gpu` | `"nana.host-texture"` + 宿主登记的 slot 名 | `GpuTextureView` |
| `<nana-gpu-view>` | `"gpu-view"` + 十进制 `slot_id` | `GpuView`。宿主必须显式注册对应 painter |
| `<canvas data-nana-canvas="{id}">` | `"nana.host-texture"` + `canvas:{id}` | 2D 像素来自 `nana-ui-web-api`（tiny-skia），hosted 路径由 `CanvasGpuBridge` dirty upload。`getContext("2d")` 只在 web-api shim 里存在，不是 Chromium 2D |
| `getContext("webgpu")` | `"nana.host-texture"` + `webgpu-canvas:{id}` | 同一套 HostTexture，不是第二套 Device |
| `<video>` / `nana-video` + `data-nana-video="{id}"` | `"nana.host-texture"` + `video:{id}` | Runtime `Video`。宿主推帧。有槽时不画 `poster` |
| `<video poster>`（无槽） | 无 CustomRenderNode；`poster` 走 `content_image` URL | 只显示 poster。不解码、不播 |
| `<iframe>` | 无 | 显式 skip（`skipped_replaced = iframe`），不加载 `src`。不是应用内浏览器 |
| `BrowserView`（宿主原生例外） | Runtime 锚点 + 宿主原生内容 | 当前仅 macOS `WKWebView`；Windows/Linux 明确 `Unsupported`。不是 `GpuTextureView`，不进入离屏 Scene。见 [应用内浏览器](#应用内浏览器) |

### 按实际绘制像素准备内容

`HostTextureRegistry::painted_extent(slot)` 返回这个 slot **当前的绘制需求**。各绘制目标最近一次全新绘制里，画到它的每个节点的设备像素尺寸逐边取最大。`ContentFit` 之后的目标矩形、节点自己的变换、当时的缩放因子都已经算进去了。宿主要按真实像素准备内容（放大一道 pass、按需重新解码、挑清晰度）时读它。

同一个 slot 被多个节点或多个窗口共享时（例如同一张封面同时出现在列表与悬浮卡），纹理只有一份。按最大的消费者准备。较小的消费者靠 mip 与过滤缩小。结果与绘制顺序无关。每个绘制目标每次全新 prepare 交一整份需求，覆盖自己上一份。复用上一帧（blit / 缓存批次）不改需求。绘制目标或画家被丢弃时撤回它的需求。

不要用「布局盒 + 自己再算一遍 `ContentFit` × 缩放因子」代替。那拿到的是**上一帧的布局**。窗口改尺寸时会差一帧。而且不包含节点的变换。还没有任何节点画到、或 slot 刚被 `remove` 时返回 `None`。某一边为 0 表示节点在树上但没有可见面积。

需求会变的消费方不必每帧扫全部 slot。`painted_revision()` 在任一 slot 的需求变化时递增。`painted_changes_since(rev, &mut out)` 给出其后变过的 slot。`subscribe_painted(callback)` 在变化时回调（在画家线程上）。用来唤醒宿主去按新尺寸准备内容。

它是只读的观测量，不是请求。登记多大的纹理仍由宿主决定。framework 不会因为这个数去重新分配宿主纹理。节点默认只采样第 0 层。宿主提供 mip 链时，节点用 `ImageSampling::Mipmap` 请求三线性采样。`url()` 图片由 framework 自己按同一口径重采样。

没有 `data-nana-canvas` / `data-nana-gpu` 的 `<canvas>` 是空盒子（`skipped_replaced = canvas`）。不会把 `src` 或 pixmap 写进 `content_image` 假装成 2D 位图。无槽且无 poster 的 `<video>` 同样是空盒子（`skipped_replaced = video`）。

`GraphCanvas` 默认画 Scene Quad / Stroke。`"graph-canvas"` 自定义 renderer 不会自动投影。未登记时整帧拒绝。

## 应用内浏览器

`BrowserView` 是现成的窗口宿主例外。不是整窗 WebView 产品壳。也没有独立 `browser` feature。Runtime 仍是布局/可访问性权威。原生子视图不进入离屏 Scene。只支持宿主实现的平台和矩形合成条件。`tools/css-parity-webview`（workspace 外）只对照盒模型。不得链进 `nana-ui`。

BrowserView 仍是树上的一块内容。Runtime 管布局 / 可访问性 / 生命周期。宿主管原生引擎和帧。它不是 Vue `webview`。也不是 `nana.webview` 的别名。`<iframe>` 继续 skip。不要改成会加载。

| 名字 | 是什么 |
| --- | --- |
| `WebView`（拟议） | Runtime 控件，声明 URL，投影 `CustomRenderNode`。不是 `GpuTextureView` 别名 |
| 槽 `webview:{id}` | 引擎画面登记到 `HostTextureRegistry` 的键。不是 HWND / NSView |
| `"nana.host-texture"` | 与 `Video` 同一个 Scene renderer，按文档顺序采样 |

URL、白名单、Cookie、引擎选型归**应用**（默认拒绝，localhost 不自动放行）。窗口句柄归宿主。普通控件拿不到。像素在 `prepare_window_frame` 更新同一 slot。禁止 present 后再盖原生 WebView。禁止第二套 Device。

拟议事件：`WebViewNavigated`、`WebViewTitleChanged`、`WebViewFailed`。后退 / 地址栏用现有控件拼。不要让 `WebView` 自绘浏览器 chrome。控件落地前用系统浏览器或应用自己的引擎。不要在树上叠一层原生 WebView。

### 无头网页画面（WebSurface）

`WebSurface` 是拟议 `WebView` 控件的引擎层，现在已经可用：宿主保持一个不进任何窗口视图树的网页实例，按 `max_fps` 截取画面，把 `WebFrame`（RGBA8、sRGB、sRGB 空间预乘）交给请求里的 `WebFrameSink`。sink 在后台线程上被调用，UI 线程不转换、不排队像素。帧去哪里归应用：登记到 HostTexture 槽、交给自己的渲染器都可以。

- 程序在 `RuntimeProgram::web_surface_requests()` 返回 `WebSurfaceRequest { id, policy, desc, restore_url, revision, command, frames }`；撤回请求就释放引擎。事件经 `web_surface_event(WebSurfaceNotice { id, revision, event }, context)` 回流。
- `desc`（尺寸、缩放、透明、帧率）原地生效；换 sink（另一个 `Arc`）或策略会重建实例。新实例只导航到 Navigate 目标或 `restore_url`，不重放窗口命令。
- `ShowWindow` 把**同一个**页面放进可交互的原生窗口（登录、点网页里的设置），Cookie 和页面状态不丢，期间帧照常输出；用户关窗或 `HideWindow` 后页面回到屏外，并报 `WindowClosed`。
- macOS：屏外无边框窗口里的 `WKWebView`，关闭遮挡检测以免 WebKit 停止渲染，`takeSnapshotWithConfiguration` 截图，CGImage 在工作线程转换。页面按 `desc.scale` 渲染（不按屏幕倍率），截图无需重采样。和上一帧相同的截图不交付；连续 10 张不变后降到每秒 4 张，一变就恢复。实测 960×540 稳定 30 fps，1080p 截图约 110 fps 上限。
- 代价：每张截图都要 WebKit 把整页重画一遍（macOS 上在 WebKit GPU 进程里）。持续动画的页面本身也按显示器刷新率渲染，这部分与截图无关，降低 `max_fps` 只能减掉截图那一份。实测 1280×720：静止页面合计约 5% 单核，持续动画页面在 30 fps 时约 30%（release）。
- Windows：每个实例一条 STA 线程，WebView2 组合控制器挂在 `Windows.UI.Composition` visual 上，用 `Windows.Graphics.Capture` 截取；需要 WebView2 Runtime，分发时带 `WebView2Loader.dll`。缺运行时以状态错误报告。
- Linux：`web_surface_support()` 为假，`WebSurface::new` 返回不可用。

`cargo run -p nana-window --example web-surface-probe [-- --window]` 实测帧率、页面是否持续刷新和透明度，并保存最后一帧；`--size WxH`、`--fps N`、`--seconds N`、`--static` 用来测开销（用 `ps` 连同 WebKit 进程一起测）。

## 按图离屏

离屏必须按 Scene 图、在采样**之前**编码时，仍挂 `GpuTextureView`。再实现 `SceneResourceProducer`。

可见帧：`encode_scene(scene, &mut frame)` 录进宿主在获取 Surface 后开始的那个 `FrameContext`。生产与 UI 绘制合并提交。成功后才以那次的 `GpuSubmission` 调用 `PreparedSceneResources::submitted`。生产者从 `SceneResourceEncodeContext::frame()` 取帧。经 wgpu-interop 的 `wgpu_encoder()` 录制。不得提交它。编码或绘制失败时整帧丢弃。生产者不能自行提交或提前报告完成。标准宿主在每条失败路径上先丢弃帧、再放弃 Surface 纹理。并将已获取但未呈现的目标标记为需要 Surface 恢复。下一次获取在原有设备上重建该目标。避免 DX12 的帧延迟等待信号在丢帧后阻止后续获取。自驱 surface 的宿主（wgpu-interop 下的 `HostedGpuContext`）同样应先丢弃帧和 view，再调用 `discard_frame` 或 `discard_surface_frame`。这些方法不会提交 GPU 工作或请求重绘。重试需求由应用决定。`hosted-gpu-demo` 展示了这条路径。

隐藏 tick：窗口遮挡、最小化或尺寸为零（即使仍可见）时，`FrameDemand` 到期仍调用 `prepare_window_frame`。尺寸可画时再在一个不带 Surface 的 `FrameContext` 上跑 `scene_resource_producers` 并立刻提交。不 flush UI。不 present。不调用 `window_frame_presented`。`submitted()` 表示这次 encode 已入队。不是「采样该纹理的 UI 帧已经呈现」。0 维仍 prepare。不跑 producer encode（与 Surface 拒绝 0 维 reconfigure 一致）。

Rust 可以保存 `registry.slot("preview")` 返回的 `TextureSlot`。内容已更新时调用 `invalidate()`。替换纹理时调用 `replace()`。通知只唤醒引用该 slot 的目标。不要为纹理内容更新改写 Runtime 节点。

`SceneWgpuPainter::new(&GpuContext, GpuTextureFormat)` 建在一台设备上。`paint(scene, &mut FrameContext, &GpuRenderTarget, ..)` 与 `paint_target` 收同一台设备的帧和目标。否则返回 `ScenePaintError::DeviceMismatch`。多目标宿主使用 `paint_target(RenderTargetId, ...)`。为每个目标保存准备好的绘制批次、投影、可写 GPU 缓冲、文字 atlas/renderers 和纹理绑定。着色器、管线、字体系统及文字整形缓存仍在同一设备上共享。目标关闭时调用 `remove_target`。普通 `paint` 对应单独的缺省目标状态。不能借它复用多个窗口的目标缓存。

**帧的提交与丢弃由 `FrameContext` 一处负责。** 文字的 instance 块与画序索引表（#224）是在帧的 encoder 里用 `copy_buffer_to_buffer` 写进 GPU 的。painter 记录完就当它们已经在 GPU 上。所以 painter 把画成功的目标登记进帧。`submit()` 结清登记。帧被丢弃（drop）时登记回滚。下一次画这个目标前 painter 先丢掉它的保留状态重建。不会从没落地的内容画。返回 `Err` 时什么都没登记。前一帧画过某目标、还没提交也没丢弃时再画它，返回 `ScenePaintError::TargetInFlight`。两帧可能乱序提交。而 painter 的账本假设的是录制顺序。`GpuContext::generation()` 标识设备代次。克隆上下文不改变代次。重建设备会改变代次。生产者应随新上下文重建资源，并录进宿主的帧。提交成功后的通知才确认该帧生产完成。

自定义 renderer 默认每帧重新准备。显式实现 `preparation_version` 后才允许复用。版本变化或 renderer 实例替换会重新准备。实际 `render` 仍在需要呈现的帧执行。**这一条对整棵树收费。** 只要有一个未实现该方法的自定义 renderer 在树上，painter 就把整帧判为不可缓存。重建全部 quad / 文字 / 图标的绘制命令。内置的 `DefaultGpuViewRenderer` 按 `(revision, params)` 实现了它。自己写 renderer 时请照做。

`SceneGpuPrepareContext` 带 `dest_size`（目标物理像素尺寸）。尺寸变化必然使 painter 的预备批次失效。所以准备阶段看到的就是这批命令实际编码时的尺寸。

## 渲染图的形状

`frame_graph` 按 **renderer** 建 preparation pass（`prepare:{renderer}`）。并把 document order 上**连续且同 renderer** 的 Custom 图元并进一个 `custom:{renderer}` pass。pass 数因此对节点数是常数。`FramePlan.operations` 逐字节不变。

`CompiledRenderGraph` 是公开类型。pass 的数量与 label 会随之变化。后端扩展不要把它们当稳定值。多个 renderer 同时存在时，`FramePlan.preparations` 的顺序是「先 renderer 名、再 resource label」。生产者写的是互不相同的外部资源。所以这个顺序没有语义约束。

## 不要做的

- 为界面另开一套 Device / Queue
- 把画面读回 CPU、编码成图片再贴回去
- 在 UI 画完之后再往 Surface 上盖一层实时画面
- 把 GPU 内容攒到帧尾一次性画，打乱和按钮的前后关系
- 同一资源在一帧里提交互相冲突的 revision（整帧会失败，不会挑一个用）
- 在 UI 线程等生产端或 GPU 完成来拿帧；用 `FrameInbox` 取最新帧
- 经 wgpu-interop 对共享设备的 `queue.submit` / `write_texture` / `write_buffer` 不持 `gpu.wgpu().lock_submission()`（窗口缩放时 `Surface::configure` 会等 GPU 空闲，并发提交会 `GpuWaitTimeout`）。守卫只包住提交本身，不得跨 `device.poll(Wait)`、sleep 或 surface 操作。`FrameContext::submit`、`GpuContext::write_texture`、`FrameExchange::copy_from` 已经自己持有它
- 丢弃一个 painter 画过的 `FrameContext` 之后，还假定它的内容已经上屏；或在它提交前用另一帧再画同一目标
- 在普通扩展的公开签名里暴露 `wgpu::*`：用 `nana-gpu` 的类型；确实要把后端交给别人时放在 `wgpu-interop` 之下
- 在 `window_frame_presented` 之前丢掉仍可能被采样的帧，或让 slot 在 binding 销毁后继续指向已归还的纹理
- 为 Android 另写一套 renderer，或把实验 GameActivity 宿主当成产品 GPU 路径。该宿主仍把 UiScene 交给 `SceneWgpuPainter`，不调用桌面的 `run_runtime`，也不是当前产品目标（见 [Android](android.md)）
- 把 `GpuTextureView` 或 `<iframe>` 当成能加载的浏览器
- 在 UI 画完之后把原生 WebView 盖在窗口上，或让控件拿 HWND / NSView 去挂引擎

设备丢失的唯一记录是 `GpuContext::is_lost()`（粘性，丢失后不会恢复，宿主换一个新的 `GpuContext`）与 `lost_report()`。设备在没有任何窗口时丢失。恢复等到下一个窗口的第一帧。用它的 Surface 选新设备。`run_runtime` 自己请求的设备由 NanaUI 安装丢失回调写入它。外部 GPU 的丢失回调归宿主所有。通过 `HostedGpuShared::from_device(instance, GpuContext::from_wgpu(..))` 注入时，NanaUI 不会安装或覆盖该回调。宿主应把通知转发到窗口线程。调用 `EmbeddedRuntime::notify_device_lost()`（它同样把上下文标为丢失，持有该上下文的生产线程可以查询 `is_lost()`，据此停下）。然后暂停原设备上的其他工作。并在新 GPU 就绪后调用 `replace_gpu()`。管理器在通知与替换之间保持挂起。不退出宿主事件循环。
