# 实时画面

着色器、预览视口、离屏纹理和按钮一样，是树上的一块内容：有位置、会被裁切、点得到。不是盖在界面上的一层，也不是抠出来的洞。

先把一扇普通窗口跑通，见 [开始](start.md)，再把画面挂上去。心智模型见 [框架如何运行](how-it-works.md)。

第一次接入：把画面画到可采样纹理，树上挂 `GpuTextureView`。Vue 用 `NanaGpu` / `<nana-gpu>`，Live2D 多层、预览视口都是这条。

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop
```

这个演示的生产线程用自己的管线画画面，所以要开 `wgpu-interop`；只上传 CPU 像素、或把 `GpuTexture` 交给 `FrameExchange` 的宿主用不到它（见 [GPU 合同与 wgpu 逃生口](#gpu-合同与-wgpu-逃生口)）。

`examples/runtime-host-fixture` 同结构。现场直写见 [GpuView](#gpuview)。

## 挂上去

`run_runtime` 创建**一份**设备，就是 `GpuContext`（`nana-gpu` 定义，`nana-ui` 再导出）。附加窗口只新建 Surface，共享同一个 `GpuContext`。已有外部事件循环时，用 `platform_host::EmbeddedRuntime` 和 `HostedGpuShared::from_device(instance, GpuContext::from_wgpu(..))` 注入宿主设备；不要再 `request_device`。

树上挂 `GpuTextureView::new("preview")`，`host_textures()` 登记同一 slot，`prepare_window_frame` 用 `context.gpu()` 更新纹理。

```rust
// 树上：和 Button 一样占布局
let preview = cx.build(document_id, |ui| {
    ui.child("preview", GpuTextureView::new("preview"))
})?;

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

`create_texture` / `write_texture` 在交给后端前就校验设备代次、usage、尺寸上限、区域与字节长度，出错返回 `GpuError`，不会触发 WGPU 校验 panic。它们不需要任何 wgpu 类型；自己录 pass 画纹理时才需要下面的逃生口。

`HostTexture` 用稳定 slot 和 generation 包住可采样的 `GpuTexture`。宿主替换或改尺寸时加 generation，NanaUI 只重建绑定，不拆布局。`revision` 的高 32 位是 generation、低 32 位是内容 version（`pack_gpu_revision`）。`HostTexture::device_generation()` 是纹理所在设备；它与 painter 的设备不同时（设备替换后没有在 `rebuild_gpu` 里重建），整帧以 `ScenePaintError::StaleHostTexture` 拒绝，不会把旧设备的纹理交给后端。

`HostTexture::instance_identity()` 区分使用相同公开 id 的不同 handle，克隆共享该身份。
自接事件循环使用 URL 图片时，给 `SceneWgpuPainter::set_image_waker` 安装唤醒回调，
完成通知后请求重绘。内建宿主已接入；离屏工具可用 `has_pending_images()` 等待资源后再次绘制。
HTTP 获取与解码在后台进行，纹理上传仍使用宿主设备。
Quad 与 HostTexture 蒙版共用缓存实现；每个缓存最多同时获取 4 个 HTTP 资源，
闲置纹理最多保留 256 项／64 MiB，120 个绘制帧未使用后释放，当前帧工作集按需保留。
栅格图片解码限制为 4096 像素边长和 64 MiB 分配预算，SVG 沿用 2048 像素边长上限。

合成顺序就是文档顺序：`"nana.host-texture"` 在主 pass 里、在这个节点该出现的位置采样，不攒到帧尾。多层就是相邻的几张 `GpuTextureView`。不要绕过界面树去直写窗口 Surface。

### GPU 合同与 wgpu 逃生口

WGPU 是唯一的后端，但不是扩展合同。普通路径只用 `nana-gpu` 的类型：

| 类型 | 是什么 |
| --- | --- |
| `GpuContext` | 进程唯一的设备。`generation()` 是 `DeviceGeneration`，设备替换后换新值；`capabilities()` 是后端、适配器、`max_texture_dimension_2d` 与用到的可选 feature；`is_lost()` 是粘性的丢失状态；`create_texture` / `write_texture` / `begin_frame` |
| `FrameContext` | 一帧的录制，独占 encoder。`submit()` 提交并返回 `GpuSubmission`；丢弃即作废，画过它的 painter 会自动重建受影响的 target |
| `GpuTexture` / `GpuRenderTarget` | 带设备代次的纹理与渲染目标。别的设备上的资源被拒绝，不会进后端 |
| `GpuTextureFormat` / `GpuTextureUsages` | 不透明的格式与 usage |

自带 shader 的 renderer、自己持有设备的宿主、需要 CPU 回读的工具走显式的 `wgpu-interop` feature：`GpuContext::from_wgpu` / `wgpu()`（adapter、device、queue、`lock_submission()`）、`FrameContext::wgpu_encoder()`、`GpuTexture::from_wgpu` / `wgpu_view()`、`ScenePass::wgpu()`、`SceneGpuRenderContext::wgpu_encoder()`，以及 `nana_ui::wgpu` 再导出。它交出的是合同背后同一份对象，不会创建第二套设备。Cargo feature 会跨依赖图统一：Vue hosted（JS WebGPU 门面直接在宿主设备上录制）会连带打开它，所以真正守门的是 `scripts/check-engine-boundary.py`——nana-gpu / nana-frame-exchange / nana-ui 的公开签名出现 `wgpu` 必须在 `wgpu-interop` 之下。

## 跨线程最新帧

画面在另一个线程上产出（模型渲染、导播合成、解码）时，用 `FrameExchange` 把完成的帧交给窗口，用 `FrameBinding` 把它绑到 slot。两者都在宿主那一个 `GpuContext` 上，生产端 crate `nana-frame-exchange` 只依赖 `nana-gpu`，渲染库不必依赖 `nana-ui`。

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

- **是 GPU 内复制，不是零拷贝。** `copy_from` 在 slot 池里做一次 `copy_texture_to_texture`，不回读 CPU。源纹理需要 `COPY_SRC`、单采样、单层 2D、非深度格式，否则返回 `IncompatibleSource`；别的设备上的纹理返回 `DeviceMismatch`。都不会触发 wgpu 校验错误。原始 `wgpu::Texture` 用 `copy_from_wgpu`（wgpu-interop）。
- **谁都不等谁。** 池满时 `copy_from` 直接返回 `PoolFull`；UI 读最新帧只做指针交换；GPU 完成通过 `on_submitted_work_done` 回报，只在生产端 `poll` 里处理。
- **提交守卫。** 窗口缩放时 `Surface::configure` 会等 GPU 空闲，同时从别的线程提交会让它 `GpuWaitTimeout`。`copy_from`、`FrameContext::submit`、`GpuContext::write_texture` 自己持提交守卫；生产端自己的 `queue.submit` / `write_buffer` / `write_texture`（wgpu-interop）要持 `gpu.wgpu().lock_submission()`，并在 `poll(Wait)`、sleep 或 surface 操作前放下。
- **容量。** 一个窗口需要 3 个 slot：在途复制、正在显示、已替换但未 present。每多一个绑定同一交换的窗口加 2 个。
- **Lease 顺序。** `prepare` 换帧后，旧帧留到 `presented` 才释放；两次 present 之间最多换一次。lease 归还后，生产端要等 UI 那次提交完成才复用该 slot。隐藏 tick 只 prepare 不 present，所以最多换一次就停住，生产端随后看到 `PoolFull`。
- **Epoch 与接受策略。** `E` 是应用自己的代次（视口、场景……）。`set_epoch` 立刻隐藏旧帧，旧 epoch 的在途复制不会发布。`accept` 是窗口的策略（可见、未过期）；被拒绝的帧不确认唤醒，所以隐藏窗口不会每帧被叫醒，策略变化时由应用请求重绘。
- **唤醒。** `notify` 在生产线程调用，只负责调度窗口：`drop(window.request_redraw())` 或 `context.dispatch(..)`，不要在里面等。
- **设备重建。** `rebuild_gpu` 后用新 `GpuContext` 重建 exchange 和 binding。`DeviceGeneration` 不同的 inbox 不会被绑定；binding 只收一个 `GpuContext`，设备和代次不可能对不上。已发出的 lease 继续有效；最后一个持有者释放后，旧设备在后台线程销毁。
- **诊断。** `FrameExchange::stats()` 给出 submitted / published / superseded / pool_full / stale_epoch 与占用高水位，读取不在帧路径上分配。

## GpuView

没有中间纹理、必须写进当前 UI pass 时才用。`gpu-view-demo` 是演示。

`GpuView::new(slot_id)` 投影 `CustomRenderNode`，renderer 键是 `"gpu-view"`。

- `scene_gpu_renderers()` 返回 `None` 或空 registry：未注册的 renderer 会报错。
- 示例自行注册演示 painter；应用显式登记自己的 `SceneGpuRenderer`。

`GpuViewMode::Inline` 复用当前 dest pass；`Standalone` 在同一帧、同一目标上另开 pass。Renderer 不得 `request_device`，也不得提交宿主正在录的帧。

`SceneGpuRenderer` 的合同里没有 wgpu 类型：

- `prepare` 收 `SceneGpuPrepareContext { gpu: &GpuContext, target_format: GpuTextureFormat, bounds, scale_factor, dest_size, gpu_work }`。
- `draw_in_pass` / `draw_batch_in_pass` 收 painter 正开着的 `ScenePass`（`dest_size`、`set_scissor`、`set_viewport`、`restore_viewport`）和同样带 `gpu`、`target_format` 的上下文。
- `render`（独立 pass）收 `SceneGpuRenderContext`，用 `with_pass(label, |pass| ..)` 在当前目标上开一个 Load/Store 的 pass，viewport 是整个目标、scissor 是节点的 clip。

在 Nana 有自己的 shader ABI（#185）之前，录 draw 仍要经 `wgpu-interop`：`context.gpu.wgpu().device()` 建管线，`pass.wgpu()` 录 draw。按设备建的缓存以 `(context.gpu.generation(), context.target_format)` 为键：跨设备替换保留下来的 registry 不会拿旧设备的管线去画，不同格式的窗口交替也不必重建。内置的 `DefaultGpuViewRenderer` 就这样做。

### 批绘制

`SceneGpuRenderer` 有两个带默认实现的方法，不实现就是原来的逐节点路径：

```rust
fn batch_capacity(&self) -> usize { 1 }
fn draw_batch_in_pass(&self, nodes: &[SceneGpuBatchNode<'_>], pass, context) -> usize { 0 }
```

`batch_capacity() > 1` 时，painter 把 **display list 上连续**的同 renderer 实例、无 `dedicated_pass`、bounds 非空的节点作为一段交给你；返回值是这段前缀里你实际编码了几个，返回 0 就退回 `draw_in_pass`。你可以只吃掉前缀——例如只处理共享同一 clip 的那几个。

这**不是**把 GPU 内容攒到帧尾。run 是 display list 的连续切片：中间任何一条 Quad / Text / Icon / HostTexture / backdrop / 合成组边界都终止它，shader 节点不可能跨过一个 Button。document order 与不批处理时逐比特相同，变的只是 draw 次数。

内置的 `DefaultGpuViewRenderer` 是实例化的参考实现：一条 instance-step 顶点缓冲，N 个相邻同 renderer 节点一次 draw，不需要任何可选 device feature。**上限**：合并的是同一条管线的 run；每个节点一个不同 shader 时，下限就是每种管线一次 draw。

多节点共用一个 shader 时，用**一个** `slot_id`、**一个** `revision`，把差异放进 `params`——`params` 不参与 resource 冲突判定，也不使 frame plan 失效。对一部分节点 bump `version` 而对其余不 bump 会让整帧被拒（见下面「不要做的」）。需要各自独立 revision 时给每个节点一个自己的 slot；渲染图按 renderer 而不是按 resource 建 pass，所以这不再随节点数增加 pass。

`SceneGpuRenderContext` 带 `dest_size`（目标的物理像素尺寸），`with_pass` 开出的 Standalone pass 的 viewport / scissor 与主 pass 一致。

节点上的 `palette` 和 `seed` 走 `CustomRenderNode::params`，槽位见 `gpu_view_params`。Runtime 只搬运这串数，语义由 renderer 键定义；换 renderer 就换一套自己的槽位约定。

```bash
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

## 媒体槽（Canvas / video / iframe）

这些不是浏览器。可见输出仍然只走 Runtime → UiScene → `SceneWgpuPainter`。

默认 GPU 接入是 **字符串 slot** 的 `GpuTextureView` / `"nana.host-texture"`。`GpuView::new(slot_id: u64)` 只给 `"gpu-view"` 直写 pass，**不是** `HostTextureRegistry` 的键。`HostTexture::id` 是 painter 缓存键，同样不是 slot。

| 节点 | 槽位合同 | L1 行为 |
| --- | --- | --- |
| `<nana-gpu>` / `data-nana-gpu` | `"nana.host-texture"` + 宿主登记的 slot 名 | `GpuTextureView` |
| `<nana-gpu-view>` | `"gpu-view"` + 十进制 `slot_id` | `GpuView`。宿主必须显式注册对应 painter |
| `<canvas data-nana-canvas="{id}">` | `"nana.host-texture"` + `canvas:{id}` | 2D 像素来自 `nana-ui-web-api`（tiny-skia），hosted 路径由 `CanvasGpuBridge` dirty upload。`getContext("2d")` 只在 web-api shim 里存在，不是 Chromium 2D |
| `getContext("webgpu")` | `"nana.host-texture"` + `webgpu-canvas:{id}` | 同一套 HostTexture，不是第二套 Device |
| `<video>` / `nana-video` + `data-nana-video="{id}"` | `"nana.host-texture"` + `video:{id}` | Runtime `Video`。宿主推帧。有槽时不画 `poster` |
| `<video poster>`（无槽） | 无 CustomRenderNode；`poster` 走 `content_image` URL | 只显示 poster。不解码、不播 |
| `<iframe>` | 无 | 显式 skip（`skipped_replaced = iframe`），不加载 `src`。不是应用内浏览器 |
| `WebView`（未实现） | `"nana.host-texture"` + `webview:{id}` | 拟议控件 `nana.webview`。像素仍走 HostTexture；引擎、URL、权限在应用侧。见 [应用内浏览器](#应用内浏览器) |

### 按实际绘制像素准备内容

`HostTextureRegistry::painted_extent(slot)` 返回这个 slot **最后一次被画到的设备像素尺寸**：`ContentFit` 之后的目标矩形、节点自己的变换、当时的缩放因子都已经算进去了。宿主要按播放区真实像素准备内容（放大一道 pass、按需重新解码、挑清晰度）时读它。

不要用「布局盒 + 自己再算一遍 `ContentFit` × 缩放因子」代替：那拿到的是**上一帧的布局**，窗口改尺寸时会差一帧，而且不包含节点的变换。还没画过、或 slot 刚被 `remove` 时返回 `None`；某一边为 0 表示那一帧它没有可见面积。记录只在绘制和 `remove` 时变——节点从树上摘掉而 slot 没有 `remove` 时，读到的是它最后一次可见时的尺寸。

它是只读的观测量，不是请求：登记多大的纹理仍由宿主决定，framework 不会因为这个数去改采样或重新分配。

没有 `data-nana-canvas` / `data-nana-gpu` 的 `<canvas>` 是空盒子（`skipped_replaced = canvas`），不会把 `src` 或 pixmap 写进 `content_image` 假装成 2D 位图。无槽且无 poster 的 `<video>` 同样是空盒子（`skipped_replaced = video`）。

`GraphCanvas` 默认画 Scene Quad / Stroke。`"graph-canvas"` 自定义 renderer 不会自动投影；未登记时整帧拒绝。

## 应用内浏览器

合同草案，不是现成控件。没有 `WebView` 类型、没有 `browser` feature，Gallery 不得摆假浏览。`tools/css-parity-webview`（workspace 外）只对照盒模型，不得链进 `nana-ui`。

落地后仍是树上的一块内容，类比 `Video`：Runtime 管布局 / 裁剪 / 命中，宿主管引擎和帧。Vue tag 拟议 `webview`（`nana.webview`）。`<iframe>` 继续 skip，不要改成会加载。

| 名字 | 是什么 |
| --- | --- |
| `WebView`（拟议） | Runtime 控件，声明 URL，投影 `CustomRenderNode`。不是 `GpuTextureView` 别名 |
| 槽 `webview:{id}` | 引擎画面登记到 `HostTextureRegistry` 的键。不是 HWND / NSView |
| `"nana.host-texture"` | 与 `Video` 同一个 Scene renderer，按文档顺序采样 |

URL、白名单、Cookie、引擎选型归**应用**（默认拒绝，localhost 不自动放行）。窗口句柄归宿主；普通控件拿不到。像素在 `prepare_window_frame` 更新同一 slot。禁止 present 后再盖原生 WebView，禁止第二套 Device。

拟议事件：`WebViewNavigated`、`WebViewTitleChanged`、`WebViewFailed`。后退 / 地址栏用现有控件拼，不要让 `WebView` 自绘浏览器 chrome。控件落地前用系统浏览器或应用自己的引擎，不要在树上叠一层原生 WebView。

## 按图离屏

离屏必须按 Scene 图、在采样**之前**编码时，仍挂 `GpuTextureView`，再实现
`SceneResourceProducer`。

可见帧：`encode_scene(scene, &mut frame)` 录进宿主在获取 Surface 后开始的那个 `FrameContext`，生产与 UI 绘制合并提交，成功后才以那次的 `GpuSubmission` 调用 `PreparedSceneResources::submitted`。生产者从 `SceneResourceEncodeContext::frame()` 取帧，经 wgpu-interop 的 `wgpu_encoder()` 录制；不得提交它。编码或绘制失败时整帧丢弃，生产者不能自行提交或提前报告完成。标准宿主在每条失败路径上先丢弃帧、再放弃 Surface 纹理，并将已获取但未呈现的目标标记为需要 Surface 恢复，下一次获取在原有设备上重建该目标，避免 DX12 的帧延迟等待信号在丢帧后阻止后续获取。自驱 surface 的宿主（wgpu-interop 下的 `HostedGpuContext`）同样应先丢弃帧和 view，再调用 `discard_frame` 或 `discard_surface_frame`；这些方法不会提交 GPU 工作或请求重绘，重试需求由应用决定。`hosted-gpu-demo` 展示了这条路径。

隐藏 tick：窗口遮挡、最小化或尺寸为零（即使仍可见）时，`FrameDemand` 到期仍调用 `prepare_window_frame`；尺寸可画时再在一个不带 Surface 的 `FrameContext` 上跑 `scene_resource_producers` 并立刻提交。不 flush UI、不 present、不调用 `window_frame_presented`。`submitted()` 表示这次 encode 已入队，不是「采样该纹理的 UI 帧已经呈现」。0 维仍 prepare，不跑 producer encode（与 Surface 拒绝 0 维 reconfigure 一致）。

Rust 可以保存 `registry.slot("preview")` 返回的 `TextureSlot`。
内容已更新时调用 `invalidate()`，替换纹理时调用 `replace()`；通知只唤醒引用该
slot 的目标。不要为纹理内容更新改写 Runtime 节点。

`SceneWgpuPainter::new(&GpuContext, GpuTextureFormat)` 建在一台设备上；
`paint(scene, &mut FrameContext, &GpuRenderTarget, ..)` 与 `paint_target` 收同一台设备的
帧和目标，否则返回 `ScenePaintError::DeviceMismatch`。多目标宿主使用
`paint_target(RenderTargetId, ...)`，为每个目标保存准备好的绘制批次、投影、可写 GPU 缓冲、
文字 atlas/renderers 和纹理绑定。着色器、管线、字体系统及文字整形缓存仍在同一设备上共享。
目标关闭时调用 `remove_target`；普通 `paint` 对应单独的缺省目标状态，不能借它复用多个窗口的
目标缓存。

**帧的提交与丢弃由 `FrameContext` 一处负责。** 文字的 instance 块与画序索引表（#224）是在
帧的 encoder 里用 `copy_buffer_to_buffer` 写进 GPU 的，painter 记录完就当它们已经在 GPU 上。
所以 painter 把画成功的目标登记进帧：`submit()` 结清登记；帧被丢弃（drop）时登记回滚，
下一次画这个目标前 painter 先丢掉它的保留状态重建，不会从没落地的内容画。返回 `Err` 时
什么都没登记。前一帧画过某目标、还没提交也没丢弃时再画它，返回
`ScenePaintError::TargetInFlight`：两帧可能乱序提交，而 painter 的账本假设的是录制顺序。
`GpuContext::generation()` 标识设备代次：克隆上下文不改变代次，重建设备会改变代次。
生产者应随新上下文重建资源，并录进宿主的帧；提交成功后的通知才确认该帧生产完成。

自定义 renderer 默认每帧重新准备；显式实现 `preparation_version` 后才允许复用。
版本变化或 renderer 实例替换会重新准备，实际 `render` 仍在需要呈现的帧执行。
**这一条对整棵树收费**：只要有一个未实现该方法的自定义 renderer 在树上，painter 就把整帧
判为不可缓存，重建全部 quad / 文字 / 图标的绘制命令。内置的 `DefaultGpuViewRenderer`
按 `(revision, params)` 实现了它；自己写 renderer 时请照做。

`SceneGpuPrepareContext` 带 `dest_size`（目标物理像素尺寸）。尺寸变化必然使 painter 的
预备批次失效，所以准备阶段看到的就是这批命令实际编码时的尺寸。

## 渲染图的形状

`frame_graph` 按 **renderer** 建 preparation pass（`prepare:{renderer}`），并把 document order 上**连续且同 renderer** 的 Custom 图元并进一个 `custom:{renderer}` pass。pass 数因此对节点数是常数，`FramePlan.operations` 逐字节不变。

`CompiledRenderGraph` 是公开类型：pass 的数量与 label 会随之变化，后端扩展不要把它们当稳定值。多个 renderer 同时存在时，`FramePlan.preparations` 的顺序是「先 renderer 名、再 resource label」；生产者写的是互不相同的外部资源，所以这个顺序没有语义约束。

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

设备丢失的唯一记录是 `GpuContext::is_lost()`（粘性，丢失后不会恢复，宿主换一个新的 `GpuContext`）与 `lost_report()`。`run_runtime` 自己请求的设备由 NanaUI 安装丢失回调写入它。外部 GPU 的丢失回调归宿主所有：通过 `HostedGpuShared::from_device(instance, GpuContext::from_wgpu(..))` 注入时，NanaUI 不会安装或覆盖该回调；宿主应把通知转发到窗口线程，调用 `EmbeddedRuntime::notify_device_lost()`（它同样把上下文标为丢失，持有该上下文的生产线程与 JS runtime 都能看到）后暂停原设备上的其他工作，并在新 GPU 就绪后调用 `replace_gpu()`。管理器在通知与替换之间保持挂起，不退出宿主事件循环。
