# Issue #7 Phase 0：基线与图形后端决策

> 状态注记：历史证据。产品 JS 引擎现为单一 V8；QuickJS 已移除。Iced 已从产品路径移除。

本阶段在 `2026-08-14`、NanaUI `f5687685197e337f0c8e245197722efcc170bf74` 上完成。测量环境为 macOS 26.5.2（25F84）、Apple M4、Rust 1.97.0。Phase 0 只建立可重复基线和迁移门禁，不据此声明真实窗口、跨平台或完整业务负载已经验收。

## 结论

- 保留 WGPU 作为默认 GPU 后端，不启动 Phase 7 的 Nana Native RHI 实现。
- 直接 Metal 在单个空 render pass 的总耗时 P95 只比 WGPU 低约 0.9%，不足以抵消新后端的资源状态、同步、恢复和多平台维护成本。
- 32 个空 render pass 的人为压力下直接 Metal 总耗时 P95 低约 16.9%，说明编码层存在可测差异，但该探针没有 draw、文本、上传、surface 或 present，不能外推到 NanaUI / Live2D 实际帧。
- Iced 仅作为 `engine/iced` 内的兼容实现；它不是 NanaUI 的长期公开 API。后续阶段按 Issue #7 的顺序迁移 identity/runtime/scene，避免同时改写 GPU 后端而失去可比较基线。

## 基线

### NanaUI / Iced 兼容渲染

机器可读结果见 [`performance/2026-08-14-issue7-phase0-iced.json`](performance/2026-08-14-issue7-phase0-iced.json)。报告现在显式记录 `MSAAx4` 和 `Bgra8UnormSrgb`，以免与 2026-07-30 的无 MSAA 报告直接误比。

| 场景 | CPU P95 | GPU submit/wait P95 | Total P50 | Total P95 | Total P99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| list-100 | 0.065 ms | 0.970 ms | 0.757 ms | 1.034 ms | 1.072 ms |
| list-1000 | 0.516 ms | 0.749 ms | 1.173 ms | 1.252 ms | 1.301 ms |
| gallery-controls | 0.089 ms | 1.787 ms | 1.391 ms | 1.875 ms | 2.091 ms |
| workspace-20-regions | 0.090 ms | 6.292 ms | 6.236 ms | 6.382 ms | 6.433 ms |
| workspace-50-regions | 0.218 ms | 15.986 ms | 16.022 ms | 16.200 ms | 16.218 ms |

`workspace-50-regions` 接近一帧预算，后续增量系统必须把它作为红线场景。与旧报告的主要差异来自 `cbace4c4` 后基准启用了 MSAAx4，而非本阶段引入的 Iced 合仓回退。

复测命令：

```bash
cargo run --release -p component-gallery --bin ui-benchmark --features benchmark --locked -- --output docs/performance/2026-08-14-issue7-phase0-iced.json
```

### QuickJS / V8

两者运行同一预构建 Vue runtime-core probe，各 21 次独立冷启动；宿主结果均为 `ok=true`、`count=2`、`createElement=3`、`insert=3`、`increment=1`。

| 引擎 | 冷启动中位数 | invoke 中位数 |
| --- | ---: | ---: |
| QuickJS | 5.763 ms | 0.175 ms |
| V8 150.4.0 | 2.572 ms | 1.496 ms |

此探针只比较小型 runtime-core 产物，不代表大型 Vue 树、内存或包体结论。两种引擎继续保持应用级互斥。

### CJK IME 与语义链路

- hosted runtime 保留窗口 ID、预编辑文本和选区。
- Vue 的提交顺序通过 `compositionend -> beforeinput -> input`，中文提交值为功能断言。
- QuickJS 与 V8 均通过真实 Vue SFC `fetch -> Response.clone -> json/arrayBuffer -> semantic tree` 验收。
- 复核中发现 QuickJS 的 JSON 宿主桥把 typed array 当作普通对象，导致真实 fetch 在 `fetchStart` 前失败；修复位于共享宿主值序列化边界，并以二进制参数和真实 SFC 往返验证。

### 视觉基线

离屏 WGPU 共生成 59 张快照并完成人工复核。历史上记录过 7 张基线的 SSIM，现仅将其
作为定位栅格差异的诊断数据；它不是正确性、组件晋级或视觉验收门槛。Iced 输出也不是
绝对视觉真值，Runtime 应以主题语义、字体度量和布局合同正确为准。

复核同时移除了设置页中的 `Vibrancy`、`Hosted GPU`、`nana-window` 等技术实现文案。界面只描述用户可理解的透明效果、设备支持和实色回退，且状态来自真实平台能力。

离屏快照不等同于真实桌面材质、窗口交互或跨平台证明。

### Live2D WGPU

只读复测 sibling `live2d-rs` revision `71e92d04ab1b377aae6dac66d6f1ec5f9bb6d033` 的 `wgpu-warm --profile medium --frames 300 --warmup-frames 60`：

- `renderer_frame_update` P95 0.087708 ms，P99 0.090042 ms；
- `wgpu_encode` P95 0.235334 ms，P99 0.240750 ms；
- `wgpu_queue_submit_cpu` P95 0.011958 ms，P99 0.012959 ms；
- `wgpu_gpu_frame_complete_blocking` P95 1.077042 ms，P99 1.130542 ms。

该证据表明当前 WGPU CPU 翻译并非主导瓶颈；macOS 无 NVML 的警告属于环境限制。

### 最小 Metal PoC

可重复探针位于 `examples/native-rhi-probe`，机器可读结果见 [`performance/2026-08-14-issue7-phase0-metal-poc.json`](performance/2026-08-14-issue7-phase0-metal-poc.json)。它采用 20 次 warmup、120 个交错且反向配对的样本，分别测量 1/32 个空 pass。

| 后端 | pass | Encode P50 | Encode P95 | Total P50 | Total P95 | Total P99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| WGPU Metal | 1 | 0.007625 ms | 0.008250 ms | 0.277625 ms | 0.298666 ms | 0.381166 ms |
| Native Metal | 1 | 0.004458 ms | 0.004875 ms | 0.266667 ms | 0.295959 ms | 0.418917 ms |
| WGPU Metal | 32 | 0.178125 ms | 0.236750 ms | 1.899125 ms | 2.137626 ms | 2.707250 ms |
| Native Metal | 32 | 0.105625 ms | 0.136833 ms | 1.610417 ms | 1.776375 ms | 2.049583 ms |

复测命令：

```bash
cargo run --release -p native-rhi-probe --locked -- --output docs/performance/2026-08-14-issue7-phase0-metal-poc.json
```

## Phase 7 重开条件

只有同时满足以下条件，才重新评估 `nana-hal -> nana-rhi`：

1. 用同一份 Nana-owned RenderPlan 覆盖真实 NanaUI 与 Live2D 工作负载，而不是空 pass；
2. WGPU 与 native backend 使用交错、正反序样本，分别报告 CPU encode、submit、GPU completion 的 P50/P95/P99；
3. 至少验证 Metal 与另一个目标平台后端，并覆盖 surface/present、resize、device loss、资源回收；
4. native backend 提供 WGPU 无法满足的能力，或在真实帧上得到稳定且有产品价值的收益；
5. `wgpu-hal` 保持 upstream-owned，unsafe 仅收敛在薄 `nana-hal`，Live2D 正常路径最多依赖到 safe `nana-rhi`。

## 阶段复核

- 已补齐：P99、可写入 JSON 的基准输出、渲染配置元数据、可重复 native Metal PoC、当前 QuickJS/V8/CJK/Live2D/视觉证据。
- 已修复：QuickJS typed-array 宿主传输根因，以及设置页泄露技术实现名称的问题。
- 已排除误判：workspace 旧/新报告的抗锯齿配置不同；当前 50-region 数值仍作为后续阶段性能红线。
- 未发现需要在 Phase 0 引入完整 RHI、第二套 retained world 或新的 UI 设置项；这些都会增加冗余且破坏后续阶段的因果验证。

据此，Phase 0 的实现、功能验证、性能解释和缺口复核均已闭环，可以进入 Phase 1 审核。
