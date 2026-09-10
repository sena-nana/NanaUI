# GPU 节点规模与 draw call 基线（2026-09-10）

这是**改动前的基线**，不是优化成果报告。它回答两个问题：一棵树上放几百个 shader 节点，成本长在哪；框架层的 draw call 到底有多少。

## 环境及可比性

- macOS 26.6.2；Apple M4；wgpu Metal 后端（adapter 报告 `Apple M4 (Metal)`）。
- release 构建，`--locked`。HEAD 为 `1100b020b9384f58fb0e744d50f0b2857973d28c`，工作区含本轮基准改动（新场景、`node_repeat`、`frame_graph` 报告字段），不含任何优化改动。
- 离屏目标，`offscreen-submit` 模式，每档 3 帧预热后取 20 帧。**不测 Surface 呈现。**
- 单机单次，没有跨机对照，也没有历史基线。下面的绝对毫秒数只用于「同一台机器上改动前后」的对照，不能用来宣称跨框架结论。

原始报告：[`performance-data/gpu-node-scale-2026-09-10/`](performance-data/gpu-node-scale-2026-09-10/)。

```bash
cargo build --release --locked -p nana-ui --features gpu,hosted,bundled-fonts --bin nana-gpu-scene-benchmark
```
```bash
./target/release/nana-gpu-scene-benchmark --scenario perf/scenarios/gpu-scene-shader-nodes-256.json
```

这四个场景是 NanaUI 内部的优化行，**不在** Issue #8 的 `harness_ids` 里，也没有 Iced / GPUI 对照；它们登记在 `perf/scenarios/catalog.json` 的 `nana_gpu_scale_ids`。

## 一张表

| 场景 | 节点 | draw calls | 每帧重建 display list | uniform 上传 /B | graph pass | graph 资源 | 建图 /ms | batch p50 /ms | encode p50 /ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `gpu-scene-ui`（参照，1 个 GPU 节点） | 4 | 5 | 否 | 0 | 4 | 2 | 0.008 | 0.000 | 0.0013 |
| `gpu-scene-host-textures-64` | 67 | 68 | 否 | 0 | 130 | 65 | 0.719 | 0.000 | 0.0116 |
| `gpu-scene-shader-nodes-256`（共享 slot） | 578 | 642 | **是** | 12,800 | 261 | 3 | **15.72** | **0.806** | 0.0263 |
| `gpu-scene-shader-nodes-256-independent` | 578 | 642 | **是** | 12,800 | 258 → 516 | 258 | **26.22** | 0.805 | 0.0247 |
| `gpu-scene-ui-dense-2k`（纯普通 UI） | 2002 | **1528** | 否 | 0 | 4 | 2 | 0.217 | 0.000 | 0.0347 |

`batch p50 = 0.000` 表示这一帧命中了 `PreparedBatch` 缓存，整份 display list 被复用。

## 三个结论

### 一、建图成本是超线性的，而且是结构变更时付

只改 `gpu-view` 节点数（其余不变），单测 `frame_graph()`：

| shader 节点 | graph pass | 建图 /ms | 相对上一档 |
|---:|---:|---:|---:|
| 1 | 6 | 0.026 | — |
| 4 | 9 | 0.013 | — |
| 16 | 21 | 0.027 | — |
| 64 | 69 | 0.405 | — |
| 128 | 133 | 1.923 | ×4.7 |
| 256 | 261 | 16.89 | **×8.8** |

节点数翻倍，建图时间乘 ~8.8。这与 `crates/nana-ui-scene/src/graph.rs:186-222` 的形状一致：hazard 叉积是 O(P²)，而拓扑发射每次都重扫全部 pass 并对每个 pass 重算 `deps.iter().all(...)`，`deps` 因为所有 pass 都在 `target` 上互相 hazard 而长到 O(P)。

pass 数本身是 `frame_graph`（`composition.rs:171-201`）在每个 Custom 图元前 flush 一次 `ui-standard` 造成的：共享 slot 时 2N+5，独立 slot 时再加 N 个 `prepare:` pass 和 N 个外部资源（258 个资源 / 516 个 pass），建图从 15.7 ms 涨到 26.2 ms。

`frame_plan()` 有 `OnceLock` 记忆，所以这**不是每帧成本**——它是「增删一个 shader 节点」要付一次的悬崖。

### 二、一个未版本化的自定义 renderer，让整棵 UI 每帧重建 display list

`batch_rebuilds` 这一列把话说死了：

- 256 个 shader 节点的场景，**每帧** `batch_rebuilds = 1`，`batch p50 = 0.806 ms`。
- 2002 个节点的纯 UI 场景（树大 3.5 倍），`batch_rebuilds = 0`，`batch p50 = 0.000 ms`。
- 64 个 host texture 的场景也是 0——HostTexture 走的是 `PreparedBatch` 的 `resources` 键，不影响缓存。

原因在 `crates/nana-ui/src/scene_paint/mod.rs:437-443`：自定义 renderer 的 `preparation_version` 返回 `None`（trait 默认值，`DefaultGpuViewRenderer` 没实现）时 `cacheable = false`，于是 `visible_operations` 加上每一个 `quads.push` / `text.prepare` / `icons.prepare` 和全部 `upload()` 每帧重来。**一个 `GpuView` 节点就足以让整棵树付这笔钱。**

同一列还显示 `gpu_upload_bytes = 12,800`，即 256 个节点每帧各写一次 50 字节的 uniform——`DefaultGpuViewRenderer` 每个 `PrimitiveId` 一个 buffer、一个 bind group、一次 `write_buffer`。

### 三、框架层 draw call 的大头不是 GPU 节点

`gpu-scene-ui-dense-2k`：2002 个普通节点，**1528 次 draw call**，而且这是一个完全静止、display list 已缓存的帧。

原因是 `push_quad`（`mod.rs:1384`）只合并**紧邻**的、同 scissor 的连续段。普通行按 document order 发 `Quad(slot 0)`、`Text(slot 2)`、`Icon(slot 3)`，中间的 Text/Icon 每次都打断 quad 的连续段，于是每行都要重开一次 draw。

对比：256 个 shader 节点的场景是 642 次 draw（2N + host texture + 若干 quad/text + blit），确实随节点线性增长，但绝对量比密集 UI 小一个量级。

## 已落地：`DefaultGpuViewRenderer::preparation_version`

`crates/nana-ui/src/default_gpu_view.rs` 实现了 `preparation_version`，返回
`(revision, params)` 的哈希。改动前后（同机、同一次会话，其余不变）：

| 场景 | `batch_rebuilds` | batch p50 /ms | 每帧 uniform 上传 /B | draw calls |
|---|---|---|---|---|
| `gpu-scene-shader-nodes-256` | 1 → **0** | 0.806 → **0.000** | 12,800 → **0** | 642 → 642 |
| `gpu-scene-shader-nodes-256-independent` | 1 → **0** | 0.805 → **0.000** | 12,800 → **0** | 642 → 642 |
| `gpu-scene-ui` | 0 → 0 | 0.000 → 0.000 | 0 → 0 | 5 → 5 |
| `gpu-scene-host-textures-64` | 0 → 0 | 0.000 → 0.000 | 0 → 0 | 68 → 68 |
| `gpu-scene-ui-dense-2k` | 0 → 0 | 0.000 → 0.000 | 0 → 0 | 1528 → 1528 |

即：一棵含 256 个 shader 节点的树，每帧省下约 0.8 ms 的整树重 batch 和 12.8 KB 的
uniform 写入；draw call 不变（那是批绘制接口的事），其余场景无回归。

原始报告在 [`performance-data/gpu-node-scale-2026-09-10/after-preparation-version/`](performance-data/gpu-node-scale-2026-09-10/after-preparation-version/)。

**读这一列时注意**：`batch_rebuilds` 只在 `quad.rs:660` / `mesh.rs:442` 的 `upload()`
里记录，所以一帧里若没有任何 quad / mesh 工作，这个计数器会保持 0 而不论 display list
是否重建。上面五个场景都含普通 UI，计数器有效；写针对性测试时必须给场景配上真实的
quad，否则断言是空的（`scene_paint/tests.rs` 的
`default_gpu_view_versions_preparation_and_tracks_param_changes` 因此挂了一个 quad 父节点）。

## 已落地：`frame_graph` pass 合并 + 线性建图

两处，都在后端中立的 `nana-ui-scene` 内：

- `crates/nana-ui-scene/src/graph.rs`：`compile()` 的拓扑发射从「每发射一个 pass
  就重扫全部 pass、每次重算 `deps.iter().all(...)`」换成预先算好入度的 Kahn；
  `add_pass` / `add_resource` 的线性查重换成 `HashSet`。发射顺序不变（仍是最小
  就绪下标优先），`CompiledRenderGraph` 的内容一字不改。
- `crates/nana-ui-scene/src/scene/composition.rs`：preparation pass 按 **renderer**
  分组而非按 resource；document order 上**连续且同 renderer** 的 Custom 图元并进
  一个 `custom:{renderer}` pass。`FramePlan.operations` / `custom_nodes` 逐字节不变
  ——run 被下一个普通图元或另一个 renderer 截断，shader 节点不可能跨过一个 Button。

| 场景 | graph pass | 建图 /ms |
|---|---|---|
| `gpu-scene-host-textures-64` | 130 → **4** | 0.719 → **0.048** |
| `gpu-scene-shader-nodes-256` | 261 → **6** | 15.72 → **0.072** |
| `gpu-scene-shader-nodes-256-independent` | 516 → **6** | 26.22 → **0.224** |
| `gpu-scene-ui`（参照） | 4 → 4 | 0.008 → 0.011 |
| `gpu-scene-ui-dense-2k`（参照） | 4 → 4 | 0.217 → 0.223 |

更要紧的是**形状**变了。改动前后按节点数扫描（共享 slot）：

| shader 节点 | 改前 pass / 建图 ms | 改后 pass / 建图 ms |
|---:|---:|---:|
| 64 | 69 / 0.405 | 6 / 0.059 |
| 128 | 133 / 1.923 | 6 / 0.041 |
| 256 | 261 / 16.89 | 6 / 0.064 |
| 512 | 517 / （未测，按 ×8.8 外推约 150） | 6 / 0.129 |
| 1024 | — | 6 / 0.232 |

独立 slot（每个节点一个外部资源，最坏情况）：256 / 512 / 1024 个节点分别是
0.196 / 0.403 / 0.999 ms，资源数 258 / 514 / 1026，pass 恒为 6。

即：pass 数对节点数**常数**，建图时间**线性**。原来「加一个 shader 节点要重付一次
O(P³)」这堵墙没有了。

**可观察的变更**：`CompiledRenderGraph` 是公开类型，pass 的**数量与 label** 变了
（`prepare:{resource}` → `prepare:{renderer}`）。多个 renderer 同时存在时，
`FramePlan.preparations` 的顺序从「全局按 resource label」变成「先按 renderer 名、
再按 resource label」；生产者写的是互不相同的外部资源（冲突会整帧拒绝），所以这个
顺序没有语义约束。单 renderer 场景顺序不变。

## 已落地：相邻同 atlas 图标合并成一次 draw

`crates/nana-ui/src/scene_paint/icon.rs` 加 `can_extend_run`，`mod.rs` 加 `push_icon`，
与既有的 `push_quad` / `push_mesh_draw` 同一套合并规则（scissor 相同、顶点连续、
同一 atlas）。`IconBatch` 的 N 个 item 因此塌成一次 draw。

`gpu-scene-ui-dense-2k`：1528 → 1503 draw calls。**收益小是预期之内**——密集列表每行
是 `Quad, Text, Icon`，中间的 quad 把 icon 隔开，合并很少触发。它真正帮到的是工具栏、
图标条和 `IconBatch` 这类图标本来就相邻的地方（回归测试
`adjacent_same_atlas_icons_batch_into_one_draw` 断言 12 个相邻图标与 1 个图标的
draw call 数相同）。密集列表那 1500 次 draw 要靠重叠感知的批次合并（见下），
不是这一步能解决的。

## 已落地：`SceneGpuRenderer` 批绘制 + 实例化参考实现

`crates/nana-ui/src/scene_gpu.rs` 给 trait 加了两个**带默认实现**的方法（现有 renderer
一行不改也能编）：

```rust
fn batch_capacity(&self) -> usize { 1 }
fn draw_batch_in_pass(&self, nodes: &[SceneGpuBatchNode<'_>], pass, context) -> usize { 0 }
```

painter（`scene_paint/mod.rs::draw_custom_run`）在遇到 `Custom` 命令时向前扫描
**display list 上连续**的同 renderer 实例、无 `dedicated_pass`、bounds 非空的节点，
交给 renderer 一次；返回值是前缀中被编码的数量，返回 0 就退回逐节点 `draw_in_pass`。

**这不是「攒到帧尾」**：run 是 display list 的连续切片，中间任何一条 `Quads` /
`Mesh` / `Text` / `Icon` / `HostTexture` / `Backdrop` / `PushGroup` / `PopGroup`
都终止它，shader 节点永远不可能跨过一个 Button。document order 与批处理前逐比特相同，
变的只是 `pass.draw` 的次数。回归测试
`ordinary_ui_and_dedicated_passes_split_a_gpu_view_run` 断言了这两种截断。

`DefaultGpuViewRenderer` 相应改成实例化：一条 instance-step 顶点缓冲装
`{rect, color_a, color_b, params}`（`params.zw` 携带 dest 尺寸，因此不需要绑定
uniform、也不需要每帧上传），零可选 device feature，默认 limits。逐节点路径也走同一条
管线，只是 `draw(0..6, i..i+1)`。实例下标按「最小空闲优先」分配，所以准备顺序（即
document order）天然让一个 run 相邻；发生碎片时只多几次 draw，不会画错——
`draw_batch_in_pass` 只吃掉下标连续且 clip 相同的前缀。

| 场景 | draw calls |
|---|---|
| `gpu-scene-shader-nodes-256` | 642 → **387** |
| `gpu-scene-shader-nodes-256-independent` | 642 → **387** |

642 − 387 = 255 = 256 − 1：**256 个 shader 节点从 256 次 draw 塌成 1 次**。剩下的 387
是 text / button / quad / host texture，属于普通 UI 那条线。

`batched_gpu_view_run_paints_each_node_like_a_lone_node` 断言批处理下每个节点的像素与
它单独绘制时完全相同，且三个节点各自带着自己的调色板。

**上限要说清楚**：这合并的是同**管线**的 run。真正每个节点一个不同 shader 的场景，
下限就是每种管线一次 draw。框架能给的是（a）同管线自动塌缩，（b）把每节点的 CPU 开销
降到接近 0。

**顺带的合同修正**：`SceneGpuRenderContext` 新增 `dest_size`。此前
`DefaultGpuViewRenderer::render`（Standalone 路径）用 `bounds.x + bounds.width` 伪造
dest 尺寸去 `set_viewport`，那本来就是错的；实例化之后它还会污染共享实例数据。

## 全部改动的合并对照

| 场景 | draw calls | 每帧重建 | uniform 上传 /B | graph pass | 建图 /ms | batch p50 /ms |
|---|---|---|---|---|---|---|
| `gpu-scene-ui` | 5 → 5 | 0 → 0 | 0 → 0 | 4 → 4 | 0.008 → 0.033 | 0.000 → 0.000 |
| `gpu-scene-host-textures-64` | 68 → 68 | 0 → 0 | 0 → 0 | 130 → **4** | 0.719 → **0.056** | 0.000 → 0.000 |
| `gpu-scene-shader-nodes-256` | 642 → **387** | 1 → **0** | 12,800 → **0** | 261 → **6** | 15.72 → **0.067** | 0.806 → **0.000** |
| `gpu-scene-shader-nodes-256-independent` | 642 → **387** | 1 → **0** | 12,800 → **0** | 516 → **6** | 26.22 → **0.219** | 0.805 → **0.000** |
| `gpu-scene-ui-dense-2k` | 1528 → 1503 | 0 → 0 | 0 → 0 | 4 → 4 | 0.217 → 0.225 | 0.000 → 0.000 |

原始报告：[`after-batching/`](performance-data/gpu-node-scale-2026-09-10/after-batching/)。

## 这张表怎么用

后续每一步改完都重跑这四个场景，把 before/after 写回本文。判据分别是：

| 改动 | 判据列 | 现在 | 目标 |
|---|---|---:|---|
| `DefaultGpuViewRenderer::preparation_version` | `batch_rebuilds` / `batch p50` | ~~1 / 0.806 ms~~ **已达成 0 / 0.000 ms** | — |
| 共享 instance buffer + 淘汰 | `gpu_upload_bytes` / `reallocs` | ~~12,800 / 0~~ **已达成 0 / 0** | — |
| `SceneGpuRenderer` 批绘制 | `draw_calls` | ~~642~~ **已达成 387（custom 部分 256 → 1）** | — |
| `frame_graph` pass 合并 + 线性建图 | `graph pass` / `建图 ms` | ~~261–516 / 15.7–26.2 ms~~ **已达成 6 / 0.07–0.22 ms** | — |
| 重叠感知批次合并 | `gpu-scene-ui-dense-2k` 的 `draw_calls` | 1528 | 取决于实现，先量再定 |

**不用快照套件做视觉门禁**——本机字体栅格化与基线不符。正确性用 `crates/nana-ui/src/scene_paint/tests.rs` 的回读断言和 `cargo test -p nana-ui-scene` 覆盖。
