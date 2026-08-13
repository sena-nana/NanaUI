# Issue #7 Phase 7：Nana RHI 决策门禁

本阶段在 `2026-08-14` 复核 Phase 0 的 WGPU/native Metal 数据、Phase 6 的 Nana-owned UiScene/RenderGraph，以及当前 `live2d-rs` workload。结论仍是：**不启动 `nana-hal` / `nana-rhi` 原生 backend 实现，WGPU 保持默认与正式 backend。** 这是 Issue #7 明确允许的 benchmark gate 结果，不是未完成实现。

## 当前证据

- NanaUI revision：`f5687685197e337f0c8e245197722efcc170bf74` 加本目标的未提交阶段改动；
- `live2d-rs` 仍为 Phase 0 已测的 `71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033`，工作树只读复核为 clean；
- Phase 0 单空 pass：native Metal 相对 WGPU total P95 仅低约 0.9%；
- Phase 0 32 空 pass：native Metal total P95 低约 16.9%，但 workload 没有 draw、text、upload、surface 或 present；
- Live2D WGPU warm：encode P95 0.235 ms，queue submit CPU P95 0.012 ms，当前 CPU translation 不是主导瓶颈；
- Phase 6 已建立 backend-neutral UiScene/RenderGraph，但标准 NanaUI 与 Live2D 尚未共同由一份 RenderPlan 在 WGPU/native 两端执行。

重复运行空 pass PoC 不会补足真实 workload、第二平台或 capability gap，因此本阶段没有制造一份数值更新但决策信息不变的报告。

## 重开条件复核

| Phase 0 重开条件 | 当前状态 |
| --- | --- |
| 同一 Nana-owned RenderPlan 覆盖真实 NanaUI + Live2D | 未满足；Scene ABI 已就绪，但没有 native consumer，也没有共同真实帧 |
| 交错/正反序报告 encode、submit、GPU completion P50/P95/P99 | 仅空 pass 与 WGPU Live2D 各自具备，未形成同 workload A/B |
| Metal + 另一平台，覆盖 surface/present/resize/device loss/resource retirement | 未满足 |
| native 提供 WGPU 缺失能力或稳定产品价值收益 | 未发现 |
| unsafe 仅在薄 nana-hal，Live2D 依赖 safe nana-rhi | 未启动；当前无需引入这层维护面 |

五项没有同时满足。此时创建 Device/Queue/Buffer/Texture/Pipeline/encoder 抽象只会复制 WGPU 已承担的资源状态、同步、恢复与平台兼容逻辑，也会让 Phase 6 的稳定 Scene ABI 与 GPU backend 改写同时发生，失去可归因基线。

## 本阶段收口

扩展 `scripts/check-engine-boundary.py`，除既有 Iced compatibility 方向外，持续禁止 `nana-ui-runtime` / `nana-ui-scene` 直接依赖 Iced、WGPU、wgpu-hal、Metal、D3D12 或 Vulkan 实现包。未来若重开 RHI，backend 必须位于 Scene 下游，而不能污染 retained Runtime/Scene contract。

门禁通过：

- 当前 NanaUI 只有 workspace `wgpu = 30.0.0` 主版本；
- Runtime/Scene dependency tree 不含 GPU/backend crate；
- 更新后的 engine/backend boundary check 通过；
- Phase 6 hosted WGPU 与性能门禁继续有效。

## 阶段复核

本阶段没有新增 native backend、shader 变体、UI 设置或伪造 backend selector。保留 WGPU 不是架构锁死：UiScene/RenderGraph 已把 retained state 与 GPU backend 解耦，满足未来用同一 plan 做受控 A/B 的前置条件。

因此 Phase 7 以明确的 **NO-GO** 决策完成。若未来五项重开条件同时具备，应建立新的真实 workload evidence phase，而不是从当前空 pass 百分比外推。当前可以进入 Phase 8，按相同门禁判断 Live2D native backend 与 NanaUI 的所有权边界。
