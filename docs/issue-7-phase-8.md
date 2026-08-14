# Issue #7 Phase 8：Live2D native backend / 共用边界

本阶段在 `2026-08-14` 复核 Issue #7 Workstream H。Phase 7 的 Nana RHI decision gate 为 NO-GO，因此本阶段同样明确：**不在 NanaUI 或 `live2d-rs` 新增 Metal/D3D12/Vulkan backend，不建立第二套 RHI。**

## Owner 与现状

- Cubism model state、drawable/mask、motion/expression 与 Live2D RenderPlan 属于 `live2d-rs`；
- 业务宿主拥有领域对象行为与何时产出画面；
- NanaUI 只拥有通用 UI layout、UiScene custom node、GPU resource composition 与窗口/Device/Queue 生命周期；
- 消费应用只应把宿主合成结果当作可选输入，不取得 Cubism 内部数据所有权。

只读复核 `live2d-rs` revision `71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033`：`live2d-core` / `live2d-render` 已独立于 WGPU，`RenderPlan` 含 masks、mask draws、main draws、offscreens 与 ordered commands，并通过 backend-neutral `RenderBackend` dispatch；WGPU 只存在于下游 `live2d-wgpu` crate。当前 owner 侧已经具备正确的 core/backend 分层，不需要 NanaUI 复制这些类型或改写其架构。

## NanaUI 接入边界

Phase 6 的通用 `CustomRenderNode` 和 `InvokeCustom` operation 已提供两种不泄露 Cubism 数据的路径：

1. 当前正式路径：业务在 NanaUI 唯一 WGPU Device/Queue 上渲染到 host texture，UiScene 以 opaque resource key 原位合成；
2. 未来扩展路径：应用/backend adapter 为 custom renderer key 注册同-pass RenderPlan consumer，使 Live2D draw 与普通 UI primitive 穿插。

NanaUI core 不新增 `Live2DNode` Rust 类型、Cubism drawable component、model file API 或动作/表情命令。`<nana-live2d-view>` 仍由应用注册的 native component/Host API 映射为通用 GPU node；这避免把一个产品/厂商协议固化成 framework ABI。

当前 host texture 路径共享 Device/Queue，不做 CPU readback、Base64、图片编码或额外子窗口；它是 Composite-only 的正确默认边界。只有业务明确需要 segmented layer insertion、协议协商成功且真实 workload 证明 offscreen texture pass 是瓶颈时，才值得实现同-pass custom backend。

本阶段随后用 acceptance-only binary 对当前正式路径做了 macOS 实机验证。`component-gallery` 仅在 `live2d-acceptance` feature 下固定依赖 `live2d-rs` revision `71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033`；该依赖不会进入 NanaUI library feature 或公共类型。验收在同一个 Apple M4 / Metal Device/Queue 上执行真实 `live2d-wgpu::Renderer` 的 update/prepare/encode/submit，把含 4 个 clipping source 与 32 个 clipped drawable 的合成负载写入 512×512 host texture，再由 `GpuTextureView` 与 NanaUI 前后景控件合成。整个正常路径没有 CPU readback；CPU 读回只发生在验收末尾生成截图时。

80 个 measured frame 以 UI-only / UI+Live2D 交替且反转顺序采样。UI+Live2D composed total P50/P95/P99 为 1.032/3.656/4.227 ms，CPU P95 为 0.188 ms；UI-only total P50/P95/P99 为 0.545/2.475/3.371 ms。最终截图含 408 个 distinct RGBA color，并人工确认 Live2D 输出位于 NanaUI 标题栏和控制栏之间。机器可读结果见 [`performance/2026-08-14-issue7-live2d-composition.json`](performance/2026-08-14-issue7-live2d-composition.json)。该负载执行的是真实 Live2D renderer、mask 与 host-texture composition，但模型数据是合成的，不被提升为授权产品模型验收。

## Native backend gate

Workstream H 的前置条件是 Workstream G 通过。Phase 7 已确认：

- 没有 NanaUI + Live2D 的 WGPU/native-RHI A/B；现有 interleaved A/B 只比较 UI-only 与当前 WGPU composition 成本；
- 没有第二平台 native backend；
- 没有 surface/present/device-loss 完整证据；
- 没有 WGPU 无法提供的必需能力；
- Live2D WGPU warm encode/submit CPU 成本不是主导瓶颈。

因此新增 `live2d-metal` / `live2d-d3d12` / `live2d-vulkan` 会带来 shader、resource binding、mask/offscreen correctness、同步和设备恢复的三份维护面，而没有证据支持。即使未来重开，也必须共同依赖 safe `nana-rhi`；禁止 Live2D 独立再造 RHI。

## 阶段复核

本阶段没有修改 sibling `live2d-rs`，没有加入 backend selector 或技术 UI。新增 harness 运行 pinned owner revision 的真实 WGPU renderer，而不是渐变纹理 mock；它留在 Gallery acceptance feature 中，不扩大 NanaUI 产品 API 或默认依赖面。

硬件证据表明正式 host-texture 路径当前已满足帧预算，因此同-pass `CustomRenderNode` 不是缺口，也不应为了“看起来更原生”提前实现。仍未覆盖的是授权产品模型；如果后续产品要以具体模型规模设性能门禁，应把模型作为外部验收输入复跑同一 harness，而不是把 Cubism loader 或资产放进 NanaUI。

Phase 8 以正确所有权和 **NO-GO** 决策完成，可以进入 Phase 9，审核 Iced 退出指标、最终 DoD 和仍需留存的兼容债务。
