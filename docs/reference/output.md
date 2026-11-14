# 窗口外输出（ExternalSurface）

`nana_ui::output` 是 `UiScene` 之后的呈现边界，启用 `gpu` 即可使用，不要求 `hosted`、winit 或 OS Window。设备、帧和提交继续由宿主的同一份 `GpuContext` / `FrameContext` 管理，所有 UI 绘制仍使用 `SceneWgpuPainter`。

普通 Window-only presentation 保持直接画入宿主取得的 Surface 目标。增加兼容消费方时，共享一个持久 canonical 输出；尺寸、像素密度、文字抗锯齿、alpha 或颜色合同不兼容时形成独立绘制组。框架没有隐式 CPU readback/upload 回退，也不为了启用输出能力给普通窗口增加目标纹理、pass、copy 或 submit。

## 无窗口入口与目标规划

`PresentationDriver` 接受调用方持有的 `RuntimeDocument`，以与窗口相同的 flush 流程完成样式、文字、布局和增量 Scene 提取，然后生成 retained 输出。

真实 Window 可见呈现、隐藏输出与 headless 入口都调用内部 `PresentationFrameDriver`：`prepare` 选择 direct、共享 canonical 或独立组并预留目标，`encode` 使用宿主已有帧绘制和 transfer，`finish` 绑定真实 submission 与租约。窗口 adapter 保留 Surface acquire/present、节拍和原生 transport；不再另写共享或重复绘制的决策。普通 direct 使用栈上目标描述，不创建输出池或执行 planner。

```rust
let mut driver = nana_ui::output::PresentationDriver::new(&gpu);
driver.configure(&consumers)?;
driver.render_document(
    &mut document, [640.0, 480.0], host_revision,
    [0.0; 4], Some(&textures), None,
)?;
// 非阻塞 poll 驱动完成回调；尚未完成时 sample 返回 NoCompletedFrame。
driver.poll();
let completed = driver.sample(consumer_id)?;
```

`render_document` 返回 Runtime 的 flush 结果，不保证 GPU 已提交。若 `driver.work().output_deferred` 大于零，宿主应稍后重试呈现；可使用 `render_scene(document.scene(), ...)` 重试同一份 Scene，无需重复业务 mutation。仅 `poll` 不会补交被跳过的生产帧。`render_scene` 的返回值分别说明各组 `Submitted`、`Reused` 或 `Deferred`。

所有不兼容组共用 driver 里的一个 `SceneWgpuPainter`，并录进同一个帧。每组的目标状态、uniform 与实例缓冲各自独立，共享的字形图集按录制顺序上传（见 [实时画面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md)）；移除某组时回收它的目标与不再使用的管线集。`PresentationDriver` 固定在创建时的 `GpuContext` 代际上：`configure` 拒绝其他代际的消费方并保留旧计划，设备丢失后 `render_scene` 返回错误且不提交；宿主换到新代际时要新建 driver。

无窗口宿主可用 `set_resource_fetch_host` 注入与窗口相同的 policy-gated HTTP(S) 图片传输，用 `set_image_update_waker` 让异步图片完成时唤醒自己的调度器；应在首帧前安装 waker。切换或清空 fetch host 会取消旧 egress 的未完成请求，旧缓存不会跨 host 使用。

`set_resource_producers(Some(Arc<SceneResourceProducerRegistry>))` 将图资源生产接入同一个输出帧：所有兼容/不兼容 UI 组共享一次 producer encode 和一次总提交，只有实际提交后通知 `submitted`，任何编码错误均丢弃整帧。`SceneResourceProducer::preparation_version` 默认 `None`，保持既有动态生产行为；提供稳定版本后，节点输入与版本均不变时不重新生产，未被引用的 producer 不会使输出 volatile。所有输出槽位饱和时在 producer 编码之前返回 `Deferred`。`PresentationError` 新增 `ResourceProduction`，穷尽匹配方需处理。

`consumers` 是已解析的 `OutputConsumer` 列表，包含稳定的 consumer / presentation identity、`RenderTargetRequirements` 与具体 adapter 的 `PresenterCapabilities`。`configure` 无变更时返回 `false`，不分配、不重建计划；配置失败保留原计划和已完成资源。仅增加或移除兼容 consumer 时保留原目标池和 revision。一个 driver 接受同一 presentation、同一逻辑视口；当前通用 driver 采用等比 `Contain`，支持 grayscale、预乘 alpha 的 retained 目标。直接 Surface、原生可渲染共享目标、subpixel 和 straight-alpha 的具体呈现由对应宿主负责，缺少实现时明确返回错误。

`RenderTargetPlanner` 在真实窗口输出与 driver 中调用。无结构变化时复用计划；内容修订不触发 topology 协商。其结果说明 consumer 如何到达目标：

| 路径 | 当前行为 |
| --- | --- |
| `DirectSurface` | 一个兼容组内只有有效 Window consumer 时保留 direct；独立窗口各自保留自己的 direct 路径。 |
| `SharedTexture` | 同设备 consumer 共同租用 canonical 纹理，同一组只渲染一次 UI。 |
| `GpuTransfer` | Window 最终呈现、原生导出复制等已实现的 GPU transfer。 |
| `NativeShared` | 仅 adapter 明确支持直接渲染至可导出的 canonical 资源时可选；DX12 export-copy 不冒充此路径。 |
| `Unsupported` | 没有实现的能力或无效目标，明确拒绝，不静默改走 CPU。 |

兼容性比较 presentation、viewport mapping identity、物理尺寸、profile、alpha、text AA、device generation 与 canonical ownership。`viewport_revision` 由宿主依据逻辑视口、DPR、fit 和原点的完整映射产生，不能拿内容 revision 代替。持久要求可向同组提升；native-copy transport 可消费同一个同设备 canonical，不额外渲染 UI。`OutputPlan::incompatibilities` 返回独立分组的具体原因。

SDR sRGB 的 `BGRA8_UNORM`、`BGRA8_UNORM_SRGB`、`RGBA8_UNORM`、`RGBA8_UNORM_SRGB` 可以共享绘制：最终 transfer 通过纹理视图处理通道顺序与 sRGB 编解码，保留 encoded 预乘通道值；存在 `BGRA8_UNORM` 消费方时优先以它作为 canonical。线性直写兼容 profile 仍独立分组。subpixel 与 grayscale、不同 alpha、HDR/P3 或不同像素密度不会被悄悄降级以取得共享。consumer 目标一律是单采样纹理，requirements 里没有采样数字段。光栅化 MSAA 属于 Scene painter 内部：painter 在 consumer 读取前完成 resolve。

原 `ApplicationState` / `RuntimeApplication` / `RuntimeProgramContext` 仍是窗口应用的兼容入口。无窗口应用通过 `RuntimeDocument` + `PresentationDriver` 驱动；不应把仍依赖 winit Window 的 `EmbeddedRuntime` 当作此入口。

## 持久资源、版本与租约

`ExternalSurface` 管理有界的 2–8 个持久 GPU 纹理槽，默认 3 个。稳态只推进 `content_revision`；resize、profile 或设备变化才创建新的资源代际。纹理槽在安全退休后循环复用，不逐帧创建、导出和销毁。consumer 应比较资源代际与内容版本，不能把每一帧占用的池 slot 当作永久不变的单个 handle。

- 内容不变时，`render` 在编码前返回 `Reused`：0 UI render、纹理重建、copy/blit、上传、submit 和 CPU 像素移动。consumer 可在 120/240 Hz 重复 sample，不能因此推动 NanaUI 刷新。
- 新内容仅在实际 GPU 提交完成后发布。`sample` 取得最近 completed 版本；新提交尚未完成时，原 completed 版本仍可用。
- `ExternalFrame` 可 clone；最后一份显示/提交引用退休之后才释放 producer slot。异步采样使用 `FrameContext::keep_alive_until_complete`，或在提交后调用 `ExternalFrame::retire_after`。clone 裸 `GpuTexture` 只保活分配，不预留 producer slot。
- slot 或 frame-slot 满时返回 `Deferred`，不阻塞等待。`poll` 只驱动非阻塞回调，不能用 `device.poll(WAIT)` 代替同步合同。
- 资源带 `DeviceGeneration`；错误设备、旧计划、旧资源代际和 producer teardown 均明确拒绝。失败 resize/configure 保留旧的有效 completed 结果。

分段宿主使用 `prepare` → `record` → `bind_submission`。`prepare` 独占预留槽位；丢弃计划释放未录制预留。录制后即使计划被丢弃，帧仍保留该槽位，直到 discard 或 GPU 完成。`record` 拒绝与准备阶段不一致的 Scene、viewport、registry 修订、alpha 或设备；`bind_submission` 只接受真正录制该计划的帧。discard 不发布内容或依赖；逆序绑定/完成也不能把旧 key 标成新内容。

迁移既有分段调用方时，`ExternalFramePlan` 现在是不可复制的一次性 reservation：录制时借用，绑定真实 submission 时移交所有权。不能缓存旧计划给后续帧重放，也不能用另一帧的 submission 补绑定。

`ExternalFrame` 携带真实 presentation profile、alpha 编码及稳定的 `OutputSurfaceId` / completed-revision ancestry。通过保留的嵌套 HostTexture 组合时，`record` 只检查 Scene 实际引用的槽位，并冻结其纹理、租约与 provenance，避免并发替换绕过检查。采样依赖闭包中已包含当前 producer 时返回 `CompositionCycle`，不编码 GPU 工作；普通 DAG 和多父消费可共享同一 child。这个检查针对已完成 revision 的因果依赖，不维护第二棵 UI/业务树，也不是对应用所有逻辑 endpoint 的全局图分析。低层宿主自行拆出裸纹理、剥离租约/provenance 或使用自定义 renderer 时，需自行维持其依赖与资源合同。

宿主应将没有包含在 Scene 修订中的外部内容纳入 `host_revision`。标准 renderer registry 使用实际被 Scene 引用的 renderer preparation version；未提供稳定版本的 renderer 才按 volatile 处理。缓存键同时包含 presentation / motion 修订和活跃动画的实际时钟；同一时刻重复消费不推动动画重画。异步图片结果在静态复用判定前收集，完成后推进 image revision，随后恢复静态复用。

HostTexture 静态判定只读取 Scene 实际引用槽位的存在性、实例、代际、版本、尺寸与 alpha，不再依赖整个 registry 的全局 revision。直接 `HostTexture::invalidate` 也会使对应输出失效；共享 registry 中未被引用槽位的更新不会让其他静态 presentation 重绘。

## 嵌套输出与兼容迁移

`EmbeddedSurfaceNode::from_frame` 使用一份 completed `ExternalFrame` 构造普通 HostTexture；父级继续使用自身的布局、裁剪、变换、opacity 和文档顺序。`update_frame` 保持 node / HostTexture identity，更新真实 child 来源、资源/内容版本，并自动重新登记仍归该 node 的槽位以发布尺寸、推进 registry revision 和唤醒父级；已移除或替换的槽位不会被重新创建。来自不同 producer 的相同 revision 数字不会被误认为同一帧。

同 producer、同资源代际的迟到旧 revision 返回 `StaleFrame`，不会把已经显示的新版本回退；切换 producer 仍允许相同 revision 数字。

注册表保留显示租约，正常 HostTexture painter 为每次父帧录制保留独立的 GPU 完成引用。一个 child 可被多父帧、多窗口反复采样；discard 只释放该次录制引用，不结束长期显示租约。旧 `bind_to_submission` 仍可作为提交后兼容 helper，多次调用分别保活，不能提前回收仍在显示的 child。

显式 `detach(self)` 消费节点，按 instance identity 原子撤销仍归它的注册槽，并释放节点自身引用；返回实际撤销数量。后来的 slot owner 不会被误删，父级会收到 revision / wake 通知。外部保存的 binding、HostTexture clone 和已录制 GPU 快照仍独立保活，直到各自安全退休。普通 Drop 继续允许注册表留显；兼容 `retire()` 只停止节点的显式 frame 访问，不等同于 detach。

`EmbeddedOutputMetadata` 包含逻辑/纹理尺寸、真实 profile/alpha、source、设备/资源/内容代际、独立的 endpoint generation 及可选 `EmbeddedInputMapping`。输入映射锚定父级权威节点和 geometry revision，不创建第二套坐标树；完整输入路由属于 Issue #243。仅能直接采样的预乘线性 scRGB合同进入普通 HostTexture；gamma 预乘、未编码成 sRGB 格式的 SDR、P3/HDR 等需要先做明确转换，不能静默按线性解释。

## 窗口输出

`RuntimeProgram::window_output(id)` 或 `ApplicationWindow::output` 返回 `WindowOutputConfig`；返回 `None` 停止输出并恢复 Window-only direct。

```rust
fn window_output(&self, id: WindowId) -> Option<WindowOutputConfig> {
    (id == TYPEWRITER).then(|| WindowOutputConfig::default()
        .with_extent(WindowOutputExtent::MatchWindow)
        .with_export(WindowOutputExport::SameDevice))
}
```

- **共享与独立绘制。** 透明 SDR、相同尺寸/视口、grayscale 的 Window + output 只绘制一次 gamma 预乘 canonical，再做最终 Surface transfer；straight 输出另作转换，不要求重画 UI。默认 output 是 1920×1080 `Contain`，且 opaque Window 可能使用 subpixel 与不同 alpha 合同；不兼容时各组独立绘制，仍共用宿主当帧提交，不二次布局。
- **静态与节流。** 输出不变时不录制输出；`max_fps` 只控制生产节拍。隐藏、最小化或遮挡窗口在 `while_hidden` 下继续排空消息/flush，并只为有变更的输出或 producer 编码；隐藏静态内容 0 UI render / submit。0 尺寸不产出帧。
- **适配与 alpha。** 支持 `Fixed`、`MatchWindow`、`Scale`，以及等比 `Contain` / `Cover`。`Premultiplied` 使用适合外部 compositor 的 gamma 空间预乘；`Straight` 对每个 canonical slot 使用对应的转换目标，额外 pass 可观测。
- **异步生命周期迁移。** `WindowOutputFrame::texture()` 保留回调内读取入口；异步消费必须取 `frame.lease()`。`WindowOutputLease` 可 clone，并在采样前对调用方帧使用 `keep_alive_until_complete`。不再依赖“两帧后复用”假设，慢 consumer 令池返回 `Deferred`，不会覆盖仍在使用的 slot。
- **槽位容量。** 窗口输出池的槽数是宿主在途帧槽数加一（默认 3 + 1 = 4，上限 8）：宿主在规划输出之前已占住当帧槽位，其余在途帧各占一个输出槽直到完成，已发布帧再占一个。少于这个数时，GPU 一落后（例如 Windows WARP）输出就会按 `SourceSlots` 周期性延后，窗口照常呈现而输出缺帧。
- **状态。** `Planned { plan }` 仅在 consumer / 尺寸 / 显示配置 / 设备等结构变化时报告实际计划；内容变化不反复通知。`Active`、`NativeUnavailable`、`NativeDeferred`、`NativeRetired`、`Failed`、`Stopped` 仅在状态改变时通知。native 不可用时仍可保留同设备输出；这不等于 CPU fallback。

迁移穷尽匹配状态或错误枚举的调用方时，需要处理新增的 `Planned`、循环依赖 / 过期计划拒绝，以及 native stage 的 `ProducerBusy`。后者表示已有生产 reservation，区别于消费方尚未释放资源，不应把两种状态合并为成功或擅自释放资源。

## 宿主适配与能力拒绝

以下区分具体消费路径与本机证据；接口、编译与 mock 不等同于外部平台实测：

| 消费方 | 入口与本机证据 |
| --- | --- |
| Window | 真实 `presentation-output-probe`，覆盖 direct、共享、独立分组、detach、resize 和隐藏静态输出。 |
| Nested | `EmbeddedSurfaceNode` 与真实 child → parent 像素测试，包括 120/240 次父提交。 |
| Headless | `PresentationDriver::render_document`，没有 Window 或 event loop。 |
| Native host | `PresentationDriver::present_to` 实现同设备目标 transfer；Windows `window-output-probe` / `native_export` 包含真实 D3D11 shared-handle import、GPU wait/copy/release。Windows WARP CI 已通过 native-export 及 240 次 native-host 消费子集，完整硬件/跨进程矩阵未验收。 |
| OpenVR / SteamVR | `openvr-output-probe` 通过 DX12 ready/release fence、同适配器 D3D11 共享纹理与 `IVROverlay_028` 提交到 SteamVR。默认采用官方 `DXGISharedHandle` 的持久纹理原子 `CopyResource` 更新；本机仅编译与协议检查，未执行 Windows/SteamVR。 |
| Web bridge | [Electron 44.9.1 参考宿主](https://github.com/sena-nana/NanaUI/blob/main/examples/web-output-bridge/README.md) + `web-output-producer`：跨进程 NT handle → managed `sharedTexture` → `VideoFrame` → WebGPU 绘制。本机覆盖真实 main 使用的协议模块；未执行 Windows/Electron GPU 路径。普通 Web API 无法导入任意 native GPU handle，必须由具体宿主安装并检测专用 bridge；通用 GPU transport 不代表 Electron bridge 已安装。 |

通用 `OverlayHostAdapter` 只负责同设备 transfer 与宿主提供的异步消费凭据，不提供或检测浏览器 native import。使用该 transport 的宿主对自己的 `FrameContext` 先 `record`，提交后以同一 submission 调用 `bind_submission`，再非阻塞 `poll`。NanaUI 的 copy 完成后才调用 transport。每个 transport 使用独立目标池，未释放的目标返回 `Deferred`；不得将一个原生单消费方 token 复制给多个独立消费者。

`OverlayRelease` 是需要宿主维持同步正确性的 unsafe 合同：只有外部 GPU 读取已完成且不会再次读取该版本，才可报告释放；adapter drop 或断连也不能提前释放 foreign 正在使用的资源。[OpenVR 官方纹理接口](https://github.com/ValveSoftware/openvr/blob/master/headers/openvr.h) 分别定义纹理提交、清除和 native handle 释放；本适配层不会把 `SetOverlayTexture` 的返回值推断成 GPU 完成证明。无法取得所需保证时，transport 返回 `Unsupported` 或保留目标，不能猜测等待帧数。

`OverlayAdapterError` 新增 `DeviceLost`。设备丢失后，完成回调不被用作有效像素的证明，`poll` / `record` / `bind_submission` 拒绝向 foreign transport 发布；已有未退休 receipt 保留其独立的资源责任。

Metal / Vulkan native import/export 未建立完整安全合同的能力明确不支持；同设备 GPU transfer 的通过不代表这些跨设备或跨进程能力已通过。

OpenVR 参考固定 [官方头文件版本](https://github.com/ValveSoftware/openvr/blob/0924064316de3effbcd1acf1e309182a2deb1c05/headers/openvr.h)。默认只分配一个 consumer 共享纹理，初始化提交一次 handle，此后按该接口合同执行原子 GPU 复制；局部 fence 仅证明 NanaUI 源已复制完成。compositor 的最终资源退休没有被证明，consumer 资源保留至进程退出。参考显式配置预乘 alpha flag；当前维护 workload 全部不透明，不构成透明边缘视觉验收。`--immutable-snapshots` 是有界三槽后 defer 的保守模式，不用于连续更新性能合同。没有将 `SetOverlayTexture` 返回值当作租约完成。

Electron 参考在生产端非阻塞观察 ready 后发送 handle。成功 import 后关闭临时跨进程句柄；消费者必须同时满足 renderer 的 `queue.onSubmittedWorkDone()` 和主进程 managed `allReferencesReleased`，才能确认 release。失败、断连或缺失凭据会隔离未完成资源并停止新输出；不能靠 IPC 返回或超时提前复用。只有明确能力缺失返回 `unsupported`，运行时验证、设备丢失和协议故障返回 `error`。两个参考均为 host fixture；不引入 WebView 产品外壳或 CPU fallback。


## DX12 共享纹理（`native-export`）

窗口输出支持 `WindowOutputConfig::with_suspended(true)`：在同一保留目标和原生共享池中提交一帧透明内容，随后不再请求隐藏输出的循环帧。窗口本身继续显示原场景，不能与透明输出共享这次栅格化。恢复为 `false` 强制提交当前场景，复用尺寸、设备和像素合同兼容的资源；撤销整个配置仍立即退役资源。配置撤销在程序更新时同步，不依赖隐藏窗口收到绘制事件。宿主负责为暂停设置有限保留期，以及按消费方协议发送透明帧；此接口不提供无限期缓存或完成超时即可复用的捷径。

`nana_gpu::NativeExportPool`（需启用 `native-export` feature，仅限 Windows + DX12 设备；对应能力位 `GpuCapability::NativeTextureExport`）持有三张带 `D3D12_HEAP_FLAG_SHARED` 和渲染目标标志创建的 `DXGI_FORMAT_B8G8R8A8_UNORM` 提交纹理（缺少渲染目标标志时 D3D11 拒绝打开共享 D3D12 纹理），每张纹理导出为一个 NT 句柄，并配一把共享围栏（shared fence）。它们在宿主自己的设备上包装为仅 `COPY_DST` 的纹理，不存在第二个设备或队列：

- `stage`：在调用方当帧中录制一次从已渲染纹理到下一个槽位的拷贝；
- `FrameContext::submit`：在提交顺序锁下，紧随 `queue.submit` 之后在设备队列上 signal 当帧的 *ready* 围栏值；
- `finish`：将已提交的拷贝包装为 `NativeFrameToken`。配置了 `WindowOutputExport::Native` 的窗口输出会对每个产出帧执行此流程。

第 `k` 次成功 stage reservation（从 0 起算）使用 ready 值 `2k + 1` 和 release 值 `2k + 2`。discard、提交后丢弃且未 `finish` 的 reservation 也消耗序号，后续值允许跳号；consumer 必须使用 token 给出的数值，不能从 `issued()` 或已交付帧数推算。否则丢弃的提交可能让下一次 ready wait 提前通过。消费方合同如下：

1. **按 `token.pool_generation()` 打开池（每个代际一次）**：在 `token.adapter_luid()` 指定的适配器上创建 D3D11 设备（`IDXGIFactory4::EnumAdapterByLuid`），对 `slot < NATIVE_EXPORT_SLOTS` 的每个 `token.texture_handle(slot)` 调用 `ID3D11Device1::OpenSharedResource1`，并对 `token.fence_handle()` 调用 `ID3D11Device5::OpenSharedFence`。只要该池还有任何 token 存活，句柄就保持有效；打开后的对象归消费方自己持有。
2. **消费每一帧 token**：先调 `token.accept_release()`，随后在消费方上下文上依次执行 `ID3D11DeviceContext4::Wait(fence, token.ready_value())` → 读取 `textures[token.slot()]`（例如 `CopyResource` 到发送端纹理）→ `Signal(fence, token.release_value())` → `Flush`。
3. **只能在 Wait 之后的 GPU 时间线上 signal release。** 若在生产端的 ready 信号落地前就提前 signal 了 release，生产端信号到达时会把围栏值往回拉，导致池永久挂起（defer）。
4. **未调用 `accept_release` 就直接 drop 的 token** 会由生产端在自己的队列上（排在 ready 信号之后）自动释放；消费方可以直接 drop token 来跳帧。

全程没有 CPU 等待。当消费方尚未释放上一帧时，`stage` 返回 `Deferred(ConsumerBusy)` 且不录制任何命令；窗口输出此时上报 `NativeDeferred`，并继续产出同设备纹理帧。若消费方超过 2 秒未释放，旧池退役（上报 `NativeRetired`）。宿主持有 `NativeExportRetirement`，直到最后一枚 token 归还且消费方 GPU 围栏实际到达 release；不能把已接受 token 的 drop 当作 GPU 完成。每扇窗口最多保留两份尚未释放的退役池，达到预算时继续报告 `NativeDeferred` 并停止新池分配；已有同设备输出仍可继续。尺寸变化、alpha/导出模式切换或设备重建同样会退役旧池，下一枚 token 将携带新的 `pool_generation`。除显式要求 `Straight` 外，像素均为 sRGB 编码、预乘 alpha。导出路径不做任何 CPU 回读；测试与 `window-output-probe` 中的 D3D11 回读仅用于验收核对。

```powershell
cargo test -p nana-gpu --features native-export --test gpu_loss_and_export native_export
cargo run -p nana-ui --example window-output-probe --features hosted,bundled-fonts,native-export
```

测试覆盖：导出帧并在同一适配器的 D3D11 设备上打开、逐字节复制比对、未释放帧使下一帧 defer、drop token 自释放、丢弃帧不污染池，以及最后一枚 token 释放后退役池关闭句柄。`window-output-probe` 让一扇透明合成窗口跑过四个阶段（可见/隐藏 × 动画/静态），并在 D3D11 设备上消费每一枚原生 token。阶段按进度推进而不是按固定时长：动画阶段拿到 30 帧输出即结束，静态阶段从输出空闲（复用已完成的帧）起观察 2.5 秒，空闲之前完成上一阶段最后内容的帧单独计为 `settle_frames`；任一阶段 60 秒内没走完即判失败。原因是软件适配器（CI 上的 WARP）前几帧要在 GPU 上执行数秒、稳态也只有每秒几帧，固定 2.5 秒的窗口实际是在判吞吐而不是功能。像素回读用 `D3D11_MAP_FLAG_DO_NOT_WAIT` 在后续帧上取：回读拷贝要等生产端的 ready 信号，而该信号可能排在一次需要事件循环继续运行才能完成的 present 之后，在事件循环线程上阻塞 `Map` 会互相等死。perf 模式（`--perf-output`）同理以 30 秒没有新 token 判为停滞，不再限制总时长。

Windows CI 以 `NANA_REQUIRE_DX12_NATIVE_TESTS=1` 执行 native-export 测试：缺少 DX12 adapter 直接失败，避免本地可选 skip 被误计为平台验收通过。CI 还运行上面的真实 Window/D3D11 import probe，保存其原始证据；配置了 CI 不等于本机已经执行 Windows 测试。

窗口路径同样按实际引用的 producer 签名及真实 GPU 设备代际保留生产缓存，仅成功 submit 推进版本；失败/discard 不推进。未引用的 registry 和稳定版本不会在隐藏 tick 中产生空提交，未实现 `preparation_version` 的 producer 继续采用动态语义。

## 性能、诊断与验收

`OutputWorkObservation`、`PresentationDriver::work`、`ExternalSurface::last_work`、Runtime `WorkCounters::output_*` 及 append-only diagnostics 记录真实规划、UI renders、提交、重建、复用、copy bytes、resolve/convert pass、readback 与 defer。未执行的阶段保持未观测，缺数据不能当作零；旧退休 diagnostics ID 不重复分配。

七类实际 GPU fixture 是 `output-static`、`output-multi`、`output-nested`、`output-conversion`、`output-slow`、`output-recovery`、`output-incompatible`（SDR 组与其 float working 组共用一个 painter、每帧一次提交，并报告共享 painter 的纹素字节 `painter_resources`）；另由真实原生 Window/Surface probe 验收 `output-direct`。后者还运行 attach/detach、兼容/不兼容输出、resize 与 hidden/static 阶段。headless target 不得冒充 direct Surface，resize recovery 不得冒充实际设备丢失。

```bash
cargo test --locked -p nana-ui --features hosted,bundled-fonts --lib output:: -- --test-threads=1
cargo run --release --locked -p nana-ui --features gpu --bin nana-output-benchmark -- --output target/performance/output/live.json
cargo run --release --locked -p nana-ui --example presentation-output-probe --features hosted,bundled-fonts -- --output target/performance/output/window.json
python3 perf/runners/nana/run.py --scenario output-direct --from-report target/performance/output/window.json --output target/performance/output/direct.json
python3 perf/contract.py --evaluate-invariants target/performance/output/direct.json
```

全部场景的 adapter、invariant 与报告命令见 [性能合同](https://github.com/sena-nana/NanaUI/blob/main/perf/README.md)。Windows/DX12、SteamVR、Electron foreign import 及实际 device-loss 需要对应平台或服务证据；没有执行时明确报告 unavailable，不能用编译通过或合成零计数替代。当前没有实现通用第三方框架/传输协议，也没有 CPU fallback。

额外六个 `transport-{overlay-static,overlay-changing,overlay-multi,overlay-slow,web-static,web-changing}` 用真实本地 GPU transfer 验证独立 consumer 池与退休，foreign retirement 明确为 mock。通过 `--transport-only` 独立执行，避免改变原七场景负载。三个 `transport-{native-host,openvr-native,web-native}` 只接收对应真实参考报告：30 次实际预热后，240 次测量，严格核对 fixture、平台路径、设备、尺寸、生产/消费工作量与 CPU 计时。缺指标、空负载或 mock 报告不能判为平台通过。

具体参考与九个 transport 门禁现已进入代码。Windows WARP 的 native-export、同适配器 D3D11 消费及受控 `device.destroy()` 通知已有实际运行证据；这不等于硬件 driver reset。SteamVR live、Electron 实际 GPU import/消费/双凭据释放、完整硬件与跨进程矩阵、缺失的 GPU execution time/VRAM 仍未验收。Electron hosted runner 能力探测因无 WebGPU adapter 未通过，未用 mock 或零帧代替消费成功。当前交付范围及原始证据见 [最终收尾记录](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/presentation-output-final-closeout-2026-10-26.md)；Issue 关闭不改变这些能力边界。


### 按 producer / consumer 追踪

`PresentationDriver::set_output_trace_enabled(true)` 启用每个实际目标组的 `output_traces()`。窗口入口实现 `RuntimeProgram::output_trace_enabled`，从 `window_output_trace` 收取直接 Surface、canonical 与实际 transfer consumer 的数值快照。默认关闭；启用时才分配冷路径状态。`OutputTraceSnapshot` 包含稳定 source/consumer identity、路径、格式、尺寸、设备/资源/内容代际、工作量、重建原因、defer/failure 状态与可选等待观察。

等待数据来自实际 `FrameContext` acquisition / `GpuSubmission`，不会从全局计数差值推断单个 consumer 的等待。未观察值保持 `None`。共享提交只有实际提交 owner 记一次，deferred 组不会占用其他组的提交数据。`PresentationDriver::work()` 保留已有 aggregate 语义；需要 producer/consumer 归属时读取 trace，不能把 aggregate 中的 consumer transfer 当作 producer 重绘。

正常 observe/query 只更新内存数值和 metric，不逐帧发 event。显式 `snapshot.capture_diagnostics()` 在 Debug 开启时发出九条具有同一 trace ID 的结构化记录，便于 nlog 重建；在非帧路径按需调用。`presentation-output-probe --trace-outputs` 保存真实窗口的逐项观察，独立于默认计时采样。

`presentation-output-probe --profile-stages` 单独打开阶段 metric 与 macOS thread CPU 时钟，区分 prepare、acquire、encode、submit、present 等 CPU 调用的墙钟时间；`--frame-rate 30` 可与默认 120 比较呈现等待。阶段及线程采样起点略有差异，不要求各段与整帧严格相等；这些数据均不是 GPU 执行时长。默认性能对照关闭上述 profiling/trace。

## 参见

- [实时画面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md) · [窗口](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md) · [架构合同](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/architecture.md) · [Issue #242 实施记录](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/issue242-delivery.md) · [GPU 与帧生命周期](/learn/gpu-frame)
