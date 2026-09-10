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

> **2026-09-10 更正**：下面这段归因是**错的**，按命令类型实测后被推翻。那 1528（后为 1503）
> 次里**没有一次是 quad**——1500 次是文字，1 次是图标，1 次 host texture，1 次 blit。
> 场景里 500 个 button 全部落在视口外，背景 quad 一个都没进 display list，而它们的
> **文字**因为 Text 图元不参与视口剔除全部进来了，其中 500 次在 GPU 上根本没有
> `pass.draw`（计数器虚报）。完整拆解、四种合并策略的实测对比与落地顺序见
> [`gpu-ui-draw-call-plan.md`](gpu-ui-draw-call-plan.md)。

（以下为当时的推测，保留以便对照）原因是 `push_quad`（`mod.rs:1384`）只合并**紧邻**的、同 scissor 的连续段。普通行按 document order 发 `Quad(slot 0)`、`Text(slot 2)`、`Icon(slot 3)`，中间的 Text/Icon 每次都打断 quad 的连续段，于是每行都要重开一次 draw。

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

> **2026-09-10 后续**：这一节说的「同一 atlas」限制已经不存在了——图标现在共享一张
> 打包 atlas，相邻图标无论字形是否相同都合成一次 draw。见下面
> [「已落地：图标共享一张打包 atlas」](#已落地图标共享一张打包-atlas)。

`crates/nana-ui/src/scene_paint/icon.rs` 加 `can_extend_run`，`mod.rs` 加 `push_icon`，
与既有的 `push_quad` / `push_mesh_draw` 同一套合并规则（scissor 相同、顶点连续、
同一 atlas）。`IconBatch` 的 N 个 item 因此塌成一次 draw。

`gpu-scene-ui-dense-2k`：1528 → 1503 draw calls。**收益小是预期之内**，不过原因与
当时的判断不同：该场景可见的图标只有 26 个（其余被视口剔除），这 26 个正好相邻，
一次就合完了，所以只省下 25 次。它真正帮到的是工具栏、
图标条和 `IconBatch` 这类图标本来就相邻的地方（回归测试
`adjacent_icons_batch_into_one_draw_whatever_their_glyphs` 断言 12 个相邻图标与
1 个图标的 draw call 数相同）。密集列表那 1500 次 draw **不是**靠重叠感知的批次合并解决的——它们全是文字，
需要的是跨节点的文字 run 合并与文字视口剔除，见
[`gpu-ui-draw-call-plan.md`](gpu-ui-draw-call-plan.md)。

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

## 已落地：屏外文字不再进 display list

`crates/nana-ui/src/scene_paint/text.rs` 的 `prepare_cryoglyph` 在建 area 之前
先跑一遍 cryoglyph 自己的 run 可见性判据（`TextRenderer::prepare` 会按
`TextArea::bounds` 的 Y 轴整段丢弃 layout run）。没有任何一条 run 落在带内的文字
直接返回 `None`：不建 renderer、不建顶点缓冲、不进 display list。

这修的是一个**计数虚报**：`text.rs` 原来无条件 `record_draw_call()`，而 cryoglyph 的
`render` 在 `glyphs_to_render == 0` 时根本不发 `pass.draw`。

| 场景 | draw calls |
|---|---|
| `gpu-scene-ui-dense-2k` | 1503 → **1003** |
| 其余四个场景 | 不变 |

差的正好是那 500 个落在视口外的 button 文字。判据是逐字抄 cryoglyph 的
（`TextArea::scale` 固定为我们一直传的 1.0），所以**像素不可能变**：一张含 24 条
跨上下边界、6 条跨左右边界、其中若干条旋转的文字的帧，改动前后整帧字节哈希相同
（`42750967d8bbac49`），只有 draw call 从 32 掉到 28。

回归测试 `text_below_the_clip_band_costs_no_draw_and_no_pixels`：8 条完全在视口下方的
label 与没有它们的同一棵树 draw call 相同、整帧像素相同；一条跨底边的 label 必须仍然
上墨。

## 已落地：相邻同 scissor 文字合并成一次 draw

`cryoglyph::TextRenderer::prepare` 本来就收 area 迭代器，一次 `render` 只发一次
`pass.draw`。改动把 cryoglyph 路径的 `prepare` 从「边遍历边发」改成**攒成 run、
建完 display list 再发一次**：

- `TextPipeline` 持有 `runs: Vec<Vec<PendingArea>>`，run `i` 准备进 `renderers[i]`；
- `mod.rs` 新增 `push_text_run`，与 `push_quad` / `push_icon` 同一套规则——只有
  document order 上**紧邻**且 **scissor 逐字节相同**的文字才并进上一条 run，
  中间任何一条 Quads / Mesh / Icon / HostTexture / Backdrop / PushGroup / PopGroup
  都终止它；
- run 里的 glyph 顺序就是 area 顺序，一次 draw 内的实例按顺序混合，所以
  text-shadow 仍然压在正文下面，重叠的两条 label 仍然后者在上；
- 一条未 flush 的 run 用 hash 指名它的 shaped buffer，所以 `ShapeCache` 满了要插入
  新段落之前必须先 flush（`at_capacity()` 那一处），否则 run 会引用被淘汰的 buffer。

| 场景 | draw calls | encode p50 /ms |
|---|---|---|
| `gpu-scene-ui-dense-2k` | 1003 → **4** | 0.0325 → **0.0012** |
| `gpu-scene-shader-nodes-256` | 387 → **132** | 0.0226 → 0.0139 |
| `gpu-scene-shader-nodes-256-independent` | 387 → **132** | 0.0191 → 0.0130 |
| `gpu-scene-host-textures-64` | 68 → 68 | 0.0087 → 0.0091 |
| `gpu-scene-ui`（参照） | 5 → 5 | 0.0013 → 0.0012 |

`gpu-scene-ui-dense-2k` 两步合计 **1503 → 4**：1 次文字（1000 条 label 一次画完）、
1 次图标（26 个）、1 次 host texture、1 次 blit。这就是这棵树的下限。
`shader-nodes-256` 的 387 → 132 是那 255 条 button/text 塌成 1 条。

回归测试：
- `adjacent_same_scissor_text_batches_into_one_draw`——8 条相邻 label 与 1 条 label
  draw call 相同，且合并后每一行仍然上墨；中间插一个 quad 则恰好多 2 次 draw。
- `merged_text_run_keeps_document_order_between_overlapping_labels`——两条完全重叠的
  label 合成一条 run 后，后者仍在上（把两者颜色对调，这条断言会挂）。
- `open_text_run_survives_shape_cache_eviction`——900 条不同段落（`SHAPE_CACHE_CAP`
  是 512）压过缓存，屏内那几行必须照常画出来（去掉 `at_capacity()` 那次 flush，
  这条测试会在 `expect` 上 panic）。

整帧像素证据：同一张跨边界 + 旋转文字的帧，两步改动前后字节哈希都是
`42750967d8bbac49`，draw call 32 → 28 → 9。

## 已落地：重叠感知的批次合并

前两步只合并 document order 上**紧邻**的同类命令。第三步让一个图元可以并进一条
**更早打开**的批次——也就是让它比自己的文档位置更早绘制——当且仅当它与那条批次之后
打开的所有批次**一个像素都不相交**。

为什么这是充分的：能被跳过的内容只可能落在「目标批次之后打开、且仍然打开」的批次里。
任何位置属于 GPU 合同的命令（`HostTexture` / `Custom` / `Backdrop` / `PushGroup` /
`PopGroup`）都会**无条件关闭全部打开的批次**，所以目标批次之后不存在已关闭的内容。
重叠的图元因此逐像素保持画家算法顺序，GPU 内容一次也不动。

三处实现要点：

- **包围盒必须是上界，不能是下界。** quad 按「最大外阴影 + outline」外扩，stroke 按
  「最大线宽 + 1px」外扩，带 filter / backdrop-filter 的 quad 没有矩形上界，直接按整张
  dest 处理（既进不了别的批次，也没人能跳过它）。文字的墨迹盒不是内容盒——
  `overflow: visible` 的文字会画到盒外——用的是**排版盒**，纵向按「请求行高比 1.25em
  自然行高少多少」外扩（正常行高时接近 0，`line-height: 1` 时几个像素），横向按 0.25em
  外扩覆盖 side bearing。
- **抗锯齿边不需要额外余量**：`physical_scissor` 把盒子向外取整到整像素。实测把 quad
  的余量从 1px 降到 0 会改变整帧哈希，降到 1px 不会——所以余量就是 1px 的 outline 基线。
- **MSAA 判定冻结在 document order 上**。`gpu_interleaved` 原来整个是扫描最终命令表
  算的；合并会把字形推成后缀，标志就会翻转并重建整张 dest。现在「字形之后来了 quad
  或 mesh」这一半在建表时按发射顺序算好，随 `PreparedBatch` 一起缓存；另一半
  （HostTexture / Custom / Backdrop / 分组命令）本来就不参与合并，仍旧读最终表。
  回归测试 `batch_merging_does_not_flip_the_dest_sample_count` 断言这一点（改回整个
  扫描最终表，它会挂）。

| 场景 | draw calls（基线 → 前两步 → 三步） | encode p50 /ms |
|---|---|---|
| `gpu-scene-ui-dense-2k` | 1503 → 4 → **4** | 0.0337 → **0.0012** |
| `gpu-scene-shader-nodes-256` | 387 → 132 → **16** | 0.0226 → **0.0062** |
| `gpu-scene-shader-nodes-256-independent` | 387 → 132 → **16** | 0.0191 → **0.0061** |
| `gpu-scene-host-textures-64` | 68 → 68 → 68 | 0.0087 → 0.0101 |
| `gpu-scene-ui`（参照） | 5 → 5 → 5 | 0.0013 → 0.0012 |

密集 UI 那条线前两步就到底了（全是文字），第三步对它是 0；它真正管的是
**「背景 quad 与自己的标签交错」**的列表。一棵 9 行 `(quad, label)` 的树：基线 40 次
draw，前两步 39 次，第三步 **26** 次；整帧字节哈希三种状态下都是 `f210e0828d88122b`。
`shader-nodes-256` 的 132 → 16 是同一回事——那 255 条 button/text/quad 塌成了几条。

回归测试（三条都做过反向验证）：
- `quad_and_label_rows_keep_a_constant_draw_count`——9 行 `(quad, label)` 与 1 行的
  draw call 相同，且每一行仍然画出自己的背景和标签。只看最后一条打开的批次，它给出
  19 vs 3。
- `a_quad_over_earlier_text_is_not_folded_ahead_of_it`——一个盖住前面几行标签的
  scrim，必须仍然压在那些标签上面。去掉重叠判定，标签会浮到 scrim 之上。
- `batch_merging_does_not_flip_the_dest_sample_count`——文字后面的 quad 被折进前面的
  批次之后，帧仍然是单采样。

`nana-ui` 全量 464 个测试通过。（`async_host_texture_mask_rebinds_after_image_completion`
在 HEAD 上就间歇性失败，是 URL 图片异步加载的竞态，与本轮无关。）

## 已落地：图标共享一张打包 atlas

`AtlasKey { icon, px }` 原来是**每个键一张 wgpu 纹理 + 一个 bind group**
（`icon.rs`，128 条 LRU）。`can_extend_run` 要求 run 内 atlas 相同，所以一条 20 个
**不同字形**的工具栏就是 20 次 draw，而同一个字形在不同渲染尺寸下也算不同的键
（`px = dest_px * 2`，所以一次 hover 缩放会造出好几张纹理）。

改成一张共享纹理：

- **分配器是按格宽分行的 shelf**。图标是正方形，一行只收自己那个格宽，所以行内不会
  碎片化。行是自上而下追加的，条目永不移动。
- **容量是面积不是条目数**，起始 256²（256 KiB），需要时倍增到
  `min(2048, max_texture_dimension_2d)`。这 256 KiB 是**每个渲染目标一次性**付的，
  比原来「每字形一张纹理」在典型窗口上的开销更大（20 个 16px 字形约 82 KiB）——这是
  那一个共享 bind group 的代价。起始尺寸按「真实 shell 一次重排都不用做」选：
  20 个 34px 格子加一个行字形只占 24 KiB。
- **满了就重排**（`repack`）：新建一张纹理，只放**这一帧用到的**字形（从各自的 SVG
  重新栅格化），因此重排同时就是淘汰策略与去碎片。新纹理的边长按「留出再一个工作集
  的余量」选，避免逐帧交替图标集时每帧重排。已经发出的顶点会被 `patch_frame_uvs`
  按新格子改写 UV，所以重排可以发生在建 display list 的中途。
- **每个格子留 1 texel 的透明边**。采样器在纹理边界而不是格子边界 clamp，而一个
  quad 最多能采到自己格子外半个 texel（旋转图标，或画得比 `MAX_ATLAS_PX` 还大），
  这一圈「谁的都不是」的像素就是不让它读到邻居的东西。边框像素**没有**复制成
  `ClampToEdge` 那样——复制先实现了又删掉了：造不出一张它会改变像素的帧，包括一个
  满格出血的自定义字形，因为栅格化把 UV 限制在 `[0, 1]`，而越界采样只发生在最后
  半个 texel 里。

`ATLAS_CAP` 这个条目上限随之消失（容量变成面积）。原来的
`live_oldest_icon_does_not_hide_idle_entries_or_retain_frame_peaks` 测的是那条按条目
计数的 LRU，已换成两条测新契约的：`a_full_sheet_keeps_this_frames_glyphs_and_drops_the_idle_ones`
（沉睡字形被回收、在用字形必须活下来、工作集小的时候纹理不长大）与
`a_frame_larger_than_the_sheet_grows_it_instead_of_dropping_glyphs`（一帧自己的字形
装不下时纹理必须增长，而不是丢字形）。两条都做过反向验证。

| 树 | draw calls |
|---|---|
| 一条 20 个**不同字形**的工具栏（+ 4 个裁剪面板 × 40 行） | 34 → **15** |
| 12 个不同字形 + 3 个面板 × 40 行 | 23 → **12** |
| 12 个不同字形 + 1 个面板 × 20 行 | 17 → **6** |
| 五个基准场景（`gpu-scene-ui` / `dense-2k` / `host-textures-64` / `shader-nodes-256` ×2） | 5 / 4 / 68 / 16 / 16，**不变** |

基准场景不变是因为它们只用一个字形（`Icon::File`）一个尺寸，早就已经是一次 draw；
这一步管的是工具栏、侧栏、图标条——真实 shell 上它占今天 draw call 的 **59%**。

**渲染差异：没有。** 两张对照帧改动前后字节哈希相同：

- 12 个不同字形 + 一个字形 5 种尺寸 + 3 个旋转图标 + 一个 200px 图标 + 一个被
  overflow 裁剪的图标 + 一个分数偏移的图标：`db94505a59558ebc`，draw call 25 → 5。
- 一个满格出血的自定义字形（相邻两个 + 旋转 + 放大）：`99c98d511c739fb8`，
  draw call 4 → 3。

`gpu_upload_bytes` 的记账口径不变（仍然按栅格化出的字节数计），所以那一列可以跨这次
改动对照：真实 shell 上 288,432 → 288,432。

## 已落地：静止帧不再重复扫描 GPU 节点

一帧里 `plan.custom_nodes` 原来被走**四遍**：`validate_scene` 里的 `frame_plan()`
（记忆化路径上仍然跑一遍 O(N) 的 `validate_plan_resources`）、`validate_scene` 自己的
循环、`paint` 里**第二次** `frame_plan()`、以及 `paint` 的 `resources` /
`renderer_versions` 键循环。后两遍与前两遍做的是同一组 `scene.primitive()` 与注册表
查找。

`validate_scene` 现在返回它解析出来的东西（`ResolvedCustomNodes`：host texture 绑定
键、renderer 版本键、`cacheable`），`paint` 直接用，不再调 `frame_plan()`、也不再走
一遍自定义节点。四遍变两遍。

静止帧（display list 全部命中缓存）的整个 `paint` + runtime flush，p50：

| 场景 | 改前 | 改后 |
|---|---:|---:|
| gpu-view × 64 | 0.0093 ms | **0.0072 ms** |
| gpu-view × 256 | 0.0329 ms | **0.0232 ms** |
| gpu-view × 1024 | 0.1470 ms | **0.0942 ms** |
| `gpu-scene-host-textures-64` | 0.0215 ms | **0.0175 ms** |
| `gpu-scene-ui-dense-2k`（参照） | 0.0022 ms | 0.0016 ms |

剩下的两遍都是**必需**的，不能再合：

- `paint` 那一遍必须每帧重算 N 个 `preparation_version` 才能判断 display list 能不能
  复用，这是 O(N) 的下限。
- `frame_plan()` 记忆化路径上的 `validate_plan_resources` **是承重的**，不是重复。
  它查的是「两个节点声明了同一个 resource 但 `(renderer, revision)` 不同」，而
  `revision` 的低 32 位是**内容版本**（`pack_gpu_revision`），一个视频面板每帧都会改
  它——`node_structure()` 故意不含 `revision`，否则每帧都要重编译渲染图。所以这个冲突
  可以在两帧之间凭空出现，每帧必须重查。要把它也变成 O(1)，得让 `UiScene` 增量维护
  一张 resource → `(renderer, revision)` 声明表（`rebuild_node_primitives` 是唯一的
  写入口，revision 变更都经过它）。**没有做**：1024 个节点上它值 0.024 ms/帧，而它要
  动的是 scene 的核心变更路径。

原始报告与完整拆解见
[`gpu-draw-call-residual-audit.md`](gpu-draw-call-residual-audit.md)。
