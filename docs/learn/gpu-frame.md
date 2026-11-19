---
title: GPU 与帧生命周期
---

# GPU 与帧生命周期：`nana-gpu` 合同与宿主的一帧

本章回答三个核心问题：**`nana-gpu` 的类型合同（`GpuContext` / `FrameContext` / `GpuTexture`、设备代际与丢失、提交顺序）为什么这样设计**；**`nana-ui` 的 `SceneWgpuPainter` 如何在宿主的 GPU 上下文上组织渲染通道（pass）与保留状态**；以及**一帧从调度到 `present` 的完整 35 步生命周期经过了哪些环节**。本章承接 [Scene 提取](/learn/scene-extraction) 的 `UiScene`，向下衔接 [窗口与平台](/learn/window-platform)。

## 职责与边界

- **`nana-gpu` 是后端合同，WGPU 是其唯一实现**：`crates/nana-gpu/src/lib.rs:1` 规定 `GpuContext` 代表进程唯一的 GPU 设备（携带设备代际、硬件能力、粘性丢失状态、纹理创建/上传）；`FrameContext` 从 `begin_frame` 到 `submit` 或 drop 独占一个命令编码器（drop 即自动 `discard` 并回滚 painter 登记的保留状态写入）；`GpuTexture` 与 `GpuRenderTarget` 携带 `DeviceGeneration`，跨代际资源在入口即被拒绝。`wgpu-interop` feature 是显式的底层逃生舱。
- **宿主拥有设备与帧（`AGENTS.md` 核心合同）**：宿主拥有 `Window`、`Surface`、`GpuContext`、每一帧的 `FrameContext` 以及帧调度；`SceneWgpuPainter` 建立在宿主上下文之上，只往宿主的帧里录制绘制命令。依赖图里只有 NanaUI 自己的 wgpu fork（`third_party/wgpu`，沿用上游包名与版本 30.0.1） 这一份 wgpu，严禁创建第二个 `Device` / `Queue`，严禁在产品呈现路径上执行 CPU 回读。
- **`nana-frame-exchange` 只传递生产者帧**：在宿主已有的同一份 `GpuContext` 上提供有界最新帧交换池（内部通过 GPU 侧 `copy_texture_to_texture` 复制，不回读 CPU），自身不拥有 UI 设备或提交权（`crates/nana-frame-exchange/src/lib.rs:1`）。
- **GPU 内容是树上的一等节点**：通过 `CustomRenderNode` 或 `HostTexture`（`GpuTextureView`）挂载在树上，像普通按钮一样参与布局、裁剪、命中测试与文档顺序。

## 依赖与邻居

在依赖层级上：`core(L1) → gpu(L2) → frame-exchange(L3) → runtime(L5) → scene(L6) → nana-ui(L7)`。`nana-ui` 是唯一持有 `SceneWgpuPainter` 的 crate；`nana-frame-exchange` 依赖 `nana-gpu` 并再导出 `GpuContext`、`GpuTexture`、`DeviceGeneration` 与 `GpuTextureFormat`，使独立的渲染生产端无需依赖 `nana-ui`：

| 邻居 Crate | 协作关系 |
| --- | --- |
| `nana-ui-scene` | 提供 painter 的输入：`UiScene` 图元、`frame_plan()` 与 `instance` 代际号 |
| `nana-window` / `nana-ui-platform` | 管理原生窗口 Surface 与透明/合成呈现路径（见 [窗口与平台](/learn/window-platform)） |
| `wgpu` | 唯一图形后端；公开签名中的 `wgpu::*` 仅允许在开启 `wgpu-interop` 时出现在五个白名单 crate 中（由 `scripts/check-engine-boundary.py` 强制检查） |
| `nana-diagnostics` | 记录帧路径零 I/O 指标（`FRAMES_DISCARDED`、`FRAME_SLOTS_EXHAUSTED` 等） |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `GpuContext` | `crates/nana-gpu/src/context.rs:358` | 进程级设备句柄（`Arc<GpuInner>`，克隆廉价且共享同一代际） |
| `DeviceGeneration` | `crates/nana-gpu/src/context.rs:19` | `NonZeroU64` 设备代际号，由进程级单调计数器分配 |
| `GpuCapabilities` | `crates/nana-gpu/src/context.rs:204` | 从 adapter / device 读取的硬件能力与特性表 |
| `FrameContext` | `crates/nana-gpu/src/frame.rs:38` | 单帧上下文：独占编码器 + 保留写入账本 + 帧槽租约 + 帧内上传批 |
| `GpuSubmission` | `crates/nana-gpu/src/frame.rs:301` | 提交回执：帧号、设备代际、CPU 录制耗时与后端提交序号 |
| `RetainedWrites` | `crates/nana-gpu/src/frame.rs:360` | 跨帧保留写入账本：帧被丢弃时自动标记回滚，并拦截同一目标跨帧并发录制（`in_flight`） |
| `GpuTexture` / `GpuRenderTarget` | `crates/nana-gpu/src/texture.rs:183` / `:276` | 携带 `DeviceGeneration` 的 2D 纹理与可渲染表面目标 |
| `GpuDeviceState` | `crates/nana-gpu/src/policy.rs:232` | 每设备策略权威：上传环形缓冲、有界帧槽、管线注册表、退役资源回收 |
| `SceneWgpuPainter` | `crates/nana-ui/src/scene_paint/mod.rs:381` | 内置 WGPU 场景绘制器：设备级共享管线家族 + 每渲染目标独立状态 |
| `RenderTargetId` | `crates/nana-ui/src/scene_paint/mod.rs:463` | 宿主拥有的窗口或离屏渲染目标标识（`pub u64`） |
| `DrawCommand` | `crates/nana-ui/src/scene_paint/mod.rs:513` | 录制后的 GPU 绘制命令枚举（`Quads` / `Mesh` / `Text` / `Icon` / `HostTexture` / `Custom` 等） |
| `HostTexture` / `HostTextureRegistry` | `crates/nana-ui/src/gpu_texture.rs:370` / `:555` | 宿主纹理句柄（高 32 位代际 + 低 32 位内容版本）与共享槽位注册表 |
| `SceneGpuRenderer` / `SceneResourceProducer` | `crates/nana-ui/src/scene_gpu.rs:247` / `:325` | 同 pass 自定义渲染器合同与按图调度的离屏资源生产者合同 |
| `FrameExchange` / `FrameBinding` | `crates/nana-frame-exchange/src/lib.rs` / `crates/nana-ui/src/frame_binding.rs:25` | 跨线程最新帧交换池与窗口消费侧槽位绑定 |

## 内部结构

### 1. `nana-gpu` 核心合同：设备代际、粘性丢失与提交顺序

- **设备代际（`DeviceGeneration`）与粘性丢失**：每个 `GpuContext` 分配唯一的 `DeviceGeneration`（`context.rs:19`），`GpuContext::check_device`（`:715`）在遇到其他代际的纹理或渲染目标时直接返回 `GpuError::DeviceMismatch`。设备丢失原因记录在粘性的 `LossState` 中（仅保留**首次**丢失报告，`:312-325`，防止驱动回调风暴）。
- **提交顺序**：队列提交不再需要与 `Surface::configure` 串行化——NanaUI 的 wgpu fork 让重配置只等仍在使用该 surface 交换链图像的工作，而不是整个设备空闲，所以另一条线程同时提交不会触发 `GpuWaitTimeout`（见 `third_party/wgpu/README.md`）。`GpuInner.submission_order: Mutex<()>` 只负责让帧提交、上传批与 native-export 的 signal 按真实队列顺序排列；`wgpu-interop` 原有的 `lock_submission` 守卫已移除。
- **前置参数校验**：`create_texture`（`context.rs:459`）在调用底层 WGPU 前完成尺寸、usage、块对齐与格式用途校验，将所有非法输入转化为可恢复的 `GpuError`，绝不触发 WGPU 校验 panic。

### 2. `FrameContext`：独占编码器、合并上传环与丢弃回滚

每个 `FrameContext`（`crates/nana-gpu/src/frame.rs:38`）只有两条终结路径：

1. **`submit(mut self) -> GpuSubmission`（`:148`）**：先在锁外完成 `encoder.finish()`；随后获取 `submission_order` 互斥锁并打包上传批，将 `UploadRing`（`crates/nana-gpu/src/upload.rs:1`，`32 MiB` 映射块池，消除每次小写入在 Metal/DX12 上的独立 staging 开销）生成的上传命令缓冲排在帧渲染命令**之前**，在同一次 `queue.submit` 中提交；最后结清保留写入账本并在 `on_submitted_work_done` 回调中归还帧槽与瞬态缓冲。
2. **`discard(self)` 或 `Drop`（`:250 / :259`）**：丢弃编码器并累加 `FRAMES_DISCARDED`；帧内已排队的缓冲写入并入设备待落地批（由下一次提交落地，保证 CPU 侧镜像一致）；同时调用 `RetainedWrites::roll_back` 将本帧画过的渲染目标标记为已回滚（触发 `RETAINED_FRAME_DISCARDED`），强制 painter 在下次绘制该目标前重建保留状态。

### 3. `SceneWgpuPainter`：设备级共享管线与每目标状态

`SceneWgpuPainter`（`crates/nana-ui/src/scene_paint/mod.rs:381`）分为两层：

- **设备级共享管线（构造一次）**：quad、motion、mesh、icon、text、host_texture、backdrop 七个管线家族在构造时通过 `beside!` 宏在 scoped 线程上**并行编译**（最慢的 quad 管线留在当前线程，`:566`），`charts` 管线在首次遇到图表时惰性编译。macOS / Metal 默认从预编译 Metal 3.1 MSL 归档加载着色器（`msl_precompiled/mod.rs`），其余 WGPU 后端读取紧凑的 `b"NAGA1"` Naga IR 字节码（`naga_ir/codec.rs:17`），仅当显式开启 `runtime-wgsl` feature 时才在运行期解析 WGSL 源码。
- **每目标状态（`TargetState`，`:469`）、HDR/OKLCH 呈现与 ROV**：按 `RenderTargetId` 隔离存储预备批次（`PreparedBatch`）、目标纹理与各管线实例数据，通过 `swap_target_state`（`:935`）换入换出，并由 `RetainedWrites` 账本跟踪——若上一帧在提交前被丢弃，`begin_frame`（`:955`）会自动清空并重建该目标的保留状态；若同一目标已有另一帧在飞（in-flight），直接返回 `ScenePaintError::TargetInFlight` 防止乱序提交破坏状态。作者色保留 OKLCH/sRGB 精度，内部合成统一在**预乘线性 scRGB** 空间完成，`ScenePresentationProfile` 输出到 SDR sRGB、Display P3 或 HDR PQ+BT.2020；当适配器支持 `wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER`（`crates/nana-ui/src/hosted_context.rs:1475`）时，背景虚化与混合通道走光栅序视图（ROV）路径。帧级 CPU/GPU 工作量分账统一汇总于 `WorkCounters`（`crates/nana-ui-core/src/work.rs:83`）。

### 4. 宿主一帧的 35 步完整序列（`scene_host/present.rs`）

宿主单窗口重绘函数 `redraw`（`crates/nana-ui/src/scene_host/present.rs:114`）按以下精确顺序执行：

| 步骤序号 | 核心阶段 | 关键设计原因与源码细节 |
| --- | --- | --- |
| **1–2** | 回收上帧指标；检查设备丢失 | 若 `take_device_lost` 为真则执行 `recover_device` 创建新代际 `GpuContext` 并调用程序 `rebuild_gpu` |
| **3–4** | 排空程序消息并应用；处理窗口 resize | 将 `dispatch_program` 消息送入 `RuntimeProgram::update` |
| **5** | 调用 `program.prepare_window_frame(id, ctx)` | 应用在此更新本帧要采样的 `HostTexture` 或 `FrameBinding` |
| **6–11** | 输出同步、推进合成时钟、`document.flush`、更新光标、暂存无障碍、获取共享 `UiScene` | `document.flush(viewport, &mut text)` 产出 `RuntimeFrameUpdate`；若未收敛（`FrameDidNotSettle`）则还原脏工作并跳过本帧 |
| **12–13** | 解析 Surface 色彩配置与 HDR 参数；检查启动接管 | HDR headroom 作为 uniform 传入，不触发管线重编译 |
| **14** | **`reserve_scene_host_frame(gpu)`（`present.rs:12`）** | **必须在获取 swapchain drawable 之前先申请帧槽**：调用 `try_begin_frame`，若满则先 `poll(Poll)` 一次再试；仍无空闲帧槽则直接放弃本帧（计入 `FRAMES_SKIPPED`），让上一帧画面继续留在屏幕上，避免取走透明交换链缓冲后无画面可合成 |
| **15–16** | `acquire_frame` 获取 Surface 纹理并包装为 `GpuRenderTarget` | 遇到 Surface 过期或丢失则转入表面恢复流程 |
| **17** | `producers.encode_scene(scene, &mut recording)` | 将 `SceneResourceProducer` 的离屏准备 pass 录进同一个 `FrameContext`（失败则 `discard` 整帧） |
| **18–19** | 同步 `nana.host-texture` 槽位重绘订阅；注入 Windows 原生内容渲染器 | 仅订阅当前场景实际引用的宿主纹理槽 |
| **20–21** | 设置绘制参数；执行 **`painter.paint_target(...)`（`present.rs:385`）** | 按文档顺序录制全部可见图元；若失败则 `discard` 帧并标记重整 |
| **22–24** | 录制窗口外二次输出（`window_output`）；**`recording.submit()`**；注册完成回调 | 窗口与离屏输出共享同一次 `FrameContext::submit` |
| **25–27** | `present_frame` 上屏；记录呈现诊断；应用挂起的 Surface 重配置 | Surface 重配置严格排在 `present` 之后，只等该 surface 的工作 |
| **28–31** | 更新 `FrameDemand`；回写解码图片自然尺寸（`commit_image_sizes`）；提交 Windows 合成树；投递窗口外输出帧 | 图片自然尺寸在程序回调前写回，若有未设尺寸的 `<img>` 拿到尺寸则请求下一帧重排 |
| **32–35** | 发布无障碍增量；刷新 IME 光标盒；调用 **`window_frame_presented`** | 直到第 35 步，应用才可安全释放上一帧被替换的旧纹理 |

### 5. `wgpu-interop` 逃生舱与 `__framework` 内部通道

为什么同时存在 `wgpu-interop` 与 `nana_gpu::__framework`（`crates/nana-gpu/src/__framework.rs`）？因为 Cargo 的 feature 会在整棵依赖图上做并集（unification）——如果 `nana-ui` 或 `nana-frame-exchange` 通过开启 `wgpu-interop` 来访问底层 WGPU，所有依赖 `nana-ui` 的普通应用都会被迫连带开启逃生舱。因此框架自身源码通过隐藏的 `__framework` 模块访问原始对象，而公开签名中的 `wgpu::*` 严格受 `wgpu-interop` 门控并由 `scripts/check-engine-boundary.py` 在 CI 中守卫。

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-gpu/src/context.rs` | `GpuContext`、`DeviceGeneration`、能力表、粘性丢失状态、提交顺序与 `begin_frame` |
| `crates/nana-gpu/src/frame.rs` | `FrameContext` 的 `submit` / `discard`、`RetainedWrites` 回滚账本与 `GpuSubmission` |
| `crates/nana-gpu/src/texture.rs` | `GpuTexture`、`GpuRenderTarget` 与稳定 usage 标志 |
| `crates/nana-gpu/src/policy.rs` | 有界帧槽、瞬态缓冲池、`StampedCache` 管线缓存与退役资源回收 |
| `crates/nana-gpu/src/upload.rs` | `UploadRing` 批量合并上传环 |
| `crates/nana-gpu/src/{wgpu_interop,__framework,abi}.rs` | 显式逃生舱、框架内部通道与逻辑资源绑定 ABI |
| `crates/nana-ui/src/scene_paint/mod.rs` | `SceneWgpuPainter` 主体、每目标状态管理与 `paint_target` |
| `crates/nana-ui/src/scene_paint/{msl_precompiled,naga_ir}/` | 预编译 Metal 3.1 MSL 归档与紧凑 `b"NAGA1"` Naga IR 编解码器 |
| `crates/nana-ui/src/scene_host/present.rs` | 宿主 `redraw` 35 步完整生命周期与 `tick_hidden_gpu` |
| `crates/nana-ui/src/{gpu_texture,scene_gpu,frame_binding}.rs` | `HostTextureRegistry`、`SceneGpuRenderer` / `SceneResourceProducer` 与 `FrameBinding` |
| `crates/nana-frame-exchange/src/lib.rs` | `FrameExchange` 跨线程有界最新帧交换池与 `FrameColorMetadata` |

## 延伸阅读

- [Scene 提取](/learn/scene-extraction)：`UiScene` 与 `FramePlan` 是如何在 `flush` 阶段生成的
- [窗口与平台](/learn/window-platform)：原生窗口 Surface、透明合成路径与材质回退
- [实时画面与 GPU 集成合同（参考）](/reference/gpu) · [窗口外输出（参考）](/reference/output)
