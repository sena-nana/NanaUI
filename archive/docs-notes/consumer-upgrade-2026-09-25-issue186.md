# Issue #186：Native RHI 决策门审查

## 决策

当前结论为 **NO-GO**。WGPU 继续是 NanaUI 的唯一正式 GPU backend，正式路径保持：

这是前置证据不足的 NO-GO，不是 native A/B 已证明收益不足。
对应 [Issue #186](https://github.com/sena-nana/NanaUI/issues/186)；本记录不把未执行的验收项勾选为完成。

```text
Logical GPU ABI -> WgpuBackend -> wgpu
```

本 Issue 不创建 `nana-rhi`、`nana-hal`、Metal/D3D12/Vulkan renderer，也不让
Runtime、Scene 或 consumer 绕过 Logical GPU ABI 直接操作 native backend。

## 已核对的基础

- #183 的 `GpuContext` / `FrameContext` backend contract 已建立，普通公开接口不泄漏
  `wgpu::*`。
- #184 的 per-device policy、frame slot、transient pool、pipeline/resource lifetime 和
  submission retirement 已接入正式 WGPU 路径。
- #185 的 `ResourceTable`、logical binding、shader interface、capability/fallback 和
  opaque WGPU realization 已接入内建 quad、icon、mesh、motion、text、HostTexture、
  backdrop、destination 与 DefaultGpuView 路径。
- #171 的 `UiScene` 仍是 render-facing Presentation IR；没有新增 Logical Scene 或
  World authority。
- #177 的完整 realtime fixture 持续 GPU workload 证据尚未提供；
  [#184 交付记录](consumer-upgrade-2026-09-24-issue184.md) 明确将其列为未执行。
  Headless benchmark 和 HostTexture 探针不替代该应用 fixture。
  上述已实现契约也不等于跨 consumer、跨平台稳定性全部验收通过。
- 现有 `examples/native-rhi-probe` 只比较 macOS 上离屏 BGRA8 clear pass 的 WGPU
  与 Metal encode/submit completion 时间。它没有 NanaUI RenderPlan、Logical GPU ABI、
  真实 consumer draw data、presentation 或 device-loss 流程，因此不能作为本 Gate 的
  A/B 证据。

## Gate 条件审计

| 条件 | 状态 | 证据或缺口 |
| --- | --- | --- |
| 可复现的性能、能力或平台阻塞 | missing | 已检查的仓库材料未提供足以触发 Gate 的 WGPU/wgpu-core 性能归因、必需能力阻塞或反复平台故障证据；这不证明此类问题不存在。 |
| 至少两个真实 GPU-heavy consumer 使用同一 workload | missing | `gpu-scene-ui-live2d` 和 effect 场景仍明确为 unsupported；相邻 live2d-rs 存在真实 renderer 和 RenderPlan，但尚未接入 Nana Logical GPU ABI，不能直接提供合格 A/B。 |
| 同一 ABI、RenderPlan、shader semantics、content、resolution | not executable | 没有 native consumer lane 与对应 cross-backend RenderPlan。 |
| correctness 一致 | not executable | native probe 不输出 NanaUI 画面或可比的 correctness artifact。 |
| 性能、内存、上传、启动、present、恢复和兼容性对照 | not executable | 当前环境为 Windows；现有 direct Metal lane 不能运行。 |
| 维护成本与收益记录 | missing | 本记录保留了 WGPU 当前边界，但没有 native A/B 的真实收益或维护增量数据。 |
| 明确 GO / NO-GO 决策 | met | 前置证据不足，保持 NO-GO。 |
| NO-GO 时正式路径不受影响 | met | 改动仅涉及文档和独立 probe 报告，生产渲染器、依赖和设备创建路径未改动。 |

因此不能诚实地得出 GO。仅有“少一层抽象可能更快”的假设不触发 PoC，也不能用
不同 workload、shader、质量或分辨率构造 A/B。

第二消费者补查：本机相邻 `../live2d-rs` checkout 的 HEAD 为
`5d868d8cc196cc1e5e3393ef79b0d527e89d3033`。其 `crates/live2d-wgpu/Cargo.toml`
直接依赖 WGPU 30，`live2d-render` 提供自己的 RenderPlan；检查 crates 的 Rust 源码和
manifest 未找到 `nana-gpu` / `ResourceTable` 接入。这证明候选消费者可供后续迁移，
不证明已满足共享 ABI、跨后端 correctness 或性能比较。本轮仅只读检查，未修改该仓库。

## 重新开启条件

进入正式 PoC 前，#183–#185 与 #171 契约必须稳定，#177 持续 GPU workload 和第二真实
consumer 必须具备，并有性能、能力或平台阻塞之一的可复现证据。先排查高层
batching/upload/cache 以及现有 WGPU 能力与平台路径，记录为何不足以解决问题。

PoC 要产生以下证据后才能讨论 GO；不要求先有完整 PoC 报告才允许开展 PoC：

1. macOS 或其他目标平台的真实 native lane 与 WGPU lane；
2. NanaUI 外至少一个真实 GPU-heavy consumer（优先 live2d-rs），并能复用同一
   Logical GPU ABI、RenderPlan、draw data、shader semantics、model/content 和 resolution；
3. 可复现的性能、能力或平台阻塞证据；
4. correctness、CPU prepare/encode、submit、GPU pass、present latency、allocations、
   RAM/VRAM、upload、pipeline startup、device-loss/recovery、driver compatibility 和
   maintenance surface 的同口径报告。

PoC 限于一个目标平台；在 GO 决策前不建立长期 native backend 架构债。

GO 后另开实施 Epic；WGPU 保留 reference/fallback。维护成本评估必须包括
validation、state/lifetime/sync、backend divergence、driver 兼容、unsafe surface 和测试负担。

## 补充审查：避免错误归因

- 现有 WGPU lane 选择 HighPerformance adapter，而 Metal lane 使用系统默认设备；
  多 GPU Mac 上未证明为同一物理 GPU，adapter 名称也不足以证明身份相同。
- `submit_wait_ms` 包含阻塞完成等待，不是独立 GPU pass time 或 present latency。
  clear-only 结果不能推广到真实 draw、资源上传和多帧在途场景。
- 重新评估时固定硬件、驱动、构建配置、内容与采样口径，分别记录冷启动和稳态；
  缺失指标保持缺失，不能填零。性能阈值应在实验前按真实 workload 预算确定。
- schema v2 增加 `gate_evidence=false`、scope 和 limitations；旧 schema v1 没有标记
  也不代表合格证据。此 probe 不输出 GO 决策。

## 验证边界

本轮执行的编译、单元测试、engine boundary 和性能合同自检只能证明仓库合同与
smoke probe 的结构正确，不能替代 native GPU、视觉、present、device-loss 或跨平台验收。

本轮命令记录：

- `cargo test -p native-rhi-probe`：3 passed；包含实际报告 JSON 的证据标记回归测试。
- `cargo test -p nana-gpu --test contract`：并发运行未结束，被中止，不计通过。
  `cargo test -p nana-gpu --test contract -- --test-threads=1`：16 passed。
  单线程通过不能证明并发卡住问题已解决，也不证明 native device-loss/recovery 通过。
- `python scripts/check-engine-boundary.py`：OK。
- `python perf/contract.py --self-test`：OK。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。

没有更改 Cargo manifests、lockfile 或生产设备创建路径；lockfile 中原有的
`wgpu-hal` 是 WGPU 的传递依赖，不是新增的直接 native backend 依赖。
