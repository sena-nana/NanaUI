# Issue #7 Phase 8：Live2D native backend / 共用边界

本阶段在 `2026-08-14` 复核 Issue #7 Workstream H。Phase 7 的 Nana RHI decision gate 为 NO-GO，因此本阶段同样明确：**不在 NanaUI 或 `live2d-rs` 新增 Metal/D3D12/Vulkan backend，不建立第二套 RHI。**

## Owner 与现状

- Cubism model state、drawable/mask、motion/expression 与 Live2D RenderPlan 属于 `live2d-rs`；
- NanaLive/业务宿主拥有 Actor 行为与何时产出画面；
- NanaUI 只拥有通用 UI layout、UiScene custom node、GPU resource composition 与窗口/Device/Queue 生命周期；
- NanaStudio 只应把 ActorComposite 当作可选 Source，不取得 Cubism 内部数据所有权。

只读复核 `live2d-rs` revision `71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033`：`live2d-core` / `live2d-render` 已独立于 WGPU，`RenderPlan` 含 masks、mask draws、main draws、offscreens 与 ordered commands，并通过 backend-neutral `RenderBackend` dispatch；WGPU 只存在于下游 `live2d-wgpu` crate。当前 owner 侧已经具备正确的 core/backend 分层，不需要 NanaUI 复制这些类型或改写其架构。

## NanaUI 接入边界

Phase 6 的通用 `CustomRenderNode` 和 `InvokeCustom` operation 已提供两种不泄露 Cubism 数据的路径：

1. 当前正式路径：业务在 NanaUI 唯一 WGPU Device/Queue 上渲染到 host texture，UiScene 以 opaque resource key 原位合成；
2. 未来扩展路径：应用/backend adapter 为 custom renderer key 注册同-pass RenderPlan consumer，使 Live2D draw 与普通 UI primitive 穿插。

NanaUI core 不新增 `Live2DNode` Rust 类型、Cubism drawable component、model file API 或动作/表情命令。`<nana-live2d-view>` 仍由应用注册的 native component/Host API 映射为通用 GPU node；这避免把一个产品/厂商协议固化成 framework ABI。

当前 host texture 路径共享 Device/Queue，不做 CPU readback、Base64、图片编码或额外子窗口；它是 Composite-only 的正确默认边界。只有业务明确需要 segmented layer insertion、协议协商成功且真实 workload 证明 offscreen texture pass 是瓶颈时，才值得实现同-pass custom backend。

## Native backend gate

Workstream H 的前置条件是 Workstream G 通过。Phase 7 已确认：

- 没有 NanaUI + Live2D 同 workload 的 WGPU/native A/B；
- 没有第二平台 native backend；
- 没有 surface/present/device-loss 完整证据；
- 没有 WGPU 无法提供的必需能力；
- Live2D WGPU warm encode/submit CPU 成本不是主导瓶颈。

因此新增 `live2d-metal` / `live2d-d3d12` / `live2d-vulkan` 会带来 shader、resource binding、mask/offscreen correctness、同步和设备恢复的三份维护面，而没有证据支持。即使未来重开，也必须共同依赖 safe `nana-rhi`；禁止 Live2D 独立再造 RHI。

## 阶段复核

本阶段只做 read-only owner/dependency/plan 审查与结论文档，没有修改 sibling `live2d-rs`，没有加入低价值 mock、backend selector 或技术 UI，也没有把通用渐变/host texture 验证冒充真实 Live2D 集成。

剩余真实集成缺口仍如实存在：尚无 Live2D RenderPlan 作为 UiScene custom operation 在同一复杂 frame graph 中执行的硬件证据。它不阻塞本阶段的 native backend NO-GO，却会继续作为产品集成验收项，不能在 Phase 9 被误报为完成。

Phase 8 以正确所有权和 **NO-GO** 决策完成，可以进入 Phase 9，审核 Iced 退出指标、最终 DoD 和仍需留存的兼容债务。
