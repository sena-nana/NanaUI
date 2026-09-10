# 普通 UI 的 draw call：拆解、可行性与方案（2026-09-10）

这是**调查报告 + 方案**，没有任何实现改动。它回答三个问题：`gpu-scene-ui-dense-2k`
那 1503 次 draw 到底是什么；上一轮方案里的「重叠感知批次合并」值不值得做；如果做，
按什么顺序做、怎么验证。

结论先说：

- **那 1503 次 draw 里没有一次是 quad。** 1500 次是文字，1 次是图标（26 个已合并），
  1 次 host texture，1 次 blit。`docs/gpu-node-scale.md` 里「每行 Quad/Text/Icon
  交错打断 quad 连续段」的归因是错的——那个场景压根没有可见的 quad。
- **重叠感知合并在这个场景收益为 0。** 真正的杠杆是**跨节点文字 run 合并**，而且
  在这个场景里连「重叠感知」都不需要：1500 条文字命令本来就是 document order 上
  连续的，一条与 `push_quad` / `push_icon` 同规则的**相邻合并**就能把 1502 压到 4。
- **其中 500 次文字是完全在视口外的**，因为 `visible_operations` 对 Text 图元不做
  剔除。而且这 500 次里 GPU 上根本没有 `pass.draw`——是我们的计数器虚报了。

## 一、怎么量的

在 painter 的 display list 构建处挂了一个临时探针（`NANA_DRAW_MIX=1`），做两件事：

1. 按 `DrawCommand` 变体拆分命令数与每条命令覆盖的图元数，并记录每个图元的物理
   包围盒（`physical_bounds(transformed_aabb_projective(bounds, affine, persp), scale, scissor)`）。
2. 在这份**真实的** display list 上离线模拟四种合并策略，数出各自的 draw 次数。

模拟器的自校验：把它设成「今天的规则」（只能扩展最近打开的那个批次、文字不合并）时，
它在三个场景上**逐个复现**了实际的 `list_draws`（1502 / 302 / 27）。所以下面的
「→ N」不是估算，是在同一份命令序列上数出来的。

探针是临时的，已从工作区撤掉；补丁存在会话 scratchpad 的 `draw-mix-probe.patch`。

三个场景（同机、release、`--locked`、Apple M4 / Metal，1280×800）：

| 场景 | 组成 | 说明 |
|---|---|---|
| `gpu-scene-ui-dense-2k` | text 1000 / button 500 / icon 500 | 仓库里的基准场景 |
| `onscreen-300` | text 100 / button 100 / icon 100 | 同一个 builder，缩到全部在屏内 |
| `column-list` | text 5 / icon 5 / button 10，纵向列表 | 把 root 从 wrap-row 临时改成 column |

后两个是临时场景，不入库；它们的存在是因为基准场景本身不含可见 quad（见下）。

## 二、1503 次 draw 是什么

```
visited_primitives = {text: 1500, icon: 26, custom: 1}   （operations = 1527）
quad = 0 draws / 0 instances
mesh = 0 draws / 0 instances
icon = 1 draw  / 26 instances
text = 1500 draws
host_texture = 1
blit = 1
```

`1500 + 1 + 1 + 1 = 1503`。

场景的 root 是 `FlexWrap::Wrap` 的 row，子节点顺序是 1000 个 Text、1 个
GpuTextureView、500 个 IconGlyph、500 个 Button。实际落位：

- 1000 个 Text 占满 y 0..499；
- GpuTextureView 在 y 499..859；
- 图标从 y 499 起，只有 **26** 个露在视口里，其余被 `visible_operations` 剔除；
- 500 个 Button 全部在 y 859..2154，**已经在视口外**。

于是 button 的背景 quad 一个都没进来（可见 quad = 0），而 button 的**文字**
1500 − 1000 = 500 条**全进来了**。

### 为什么 500 条屏外文字没被剔除

`crates/nana-ui-scene/src/scene/visibility.rs:123`：

```rust
_ => return self_clip_bounds(scene, &primitive),
```

Text 落到这个分支，而 `self_clip_bounds`（`visibility.rs:167`）只在该节点自己带
overflow clip 时才给出包围盒，否则返回 `None`。`None` 在这个索引里的含义是
**保守包含**（文件头就写了）。所以一个没有自剪裁的 Text，无论飘到哪里都不会被剔除。
上面那句的注释说明了动机——「Ink may exceed a nominal text/stroke box」——动机是对的，
代价是这里 1/3 的 draw 全花在屏外。

### 这 500 次 draw 计数是虚报的

`crates/nana-ui/src/scene_paint/text.rs:1078` 无条件记 `record_draw_call()`，
但 cryoglyph 的 `TextRenderer::render` 在 `glyphs_to_render == 0` 时**直接返回，
不发 `pass.draw`**（`text_render.rs:352`）。而它的 `prepare` 会按
`TextArea::bounds` 的 Y 轴丢掉整段 run（`text_render.rs:76-85`），屏外文字的
glyph 数就是 0。

即：**这一帧真实的 `pass.draw` 次数约是 1003，不是 1503。** 计数器和现实差了 500。
`GpuWorkObservation::draw_calls` 是对外的性能合同，这个偏差必须修——要么在
painter 侧不为空包围盒的文字生成 `DrawCommand::Text`（顺带省掉 prepare），要么让
文字管线只在真的发出 draw 时记数。前者更根本。

## 三、四种策略在真实命令序列上的模拟

`今天` 列是自校验（模拟器复现实际值）。`重叠感知` = 方向 1；`文字 run` = 方向 2 的
最弱形式（只合并 document order 上相邻的文字命令）。

| 场景 | list draws（今天） | 仅重叠感知 | 仅相邻文字合并 | 两者 | 两者 + 32px tile 位图 |
|---|---:|---:|---:|---:|---:|
| `gpu-scene-ui-dense-2k` | 1502 | **1502** | **4** | 4 | 4 |
| `onscreen-300`（wrap-row） | 302 | 203 | 203 | **21** | 27 |
| `column-list`（纵向列表） | 27 | 18 | 23 | **5** | 23 |

（表里是 display list 的 draw 次数，实际 `draw_calls` 各再 +1 次 blit。）

读出来的四件事：

1. **基准场景上，方向 1 的收益精确等于 0。** 没有可见 quad 可合，图标已经是 1 次。
   1498 次「合并被拒」全部是因为文字之间不共享下标空间——也就是说，这个场景需要的
   不是重排，是文字合并。
2. **相邻文字合并单独就能把基准场景压到 4。** 因为 1000 条正文文字与 500 条
   button 文字在 document order 上各自连续，中间只隔着 host texture 和图标。
   这条规则与 `push_quad`（`mod.rs:1417`）/ `push_icon`（`mod.rs:1393`）完全同构，
   **不重排任何东西**，document order 逐字保留。
3. **真正需要方向 1 的是「quad 与自己的文字交错」的结构**：`onscreen-300` 的
   100 个 button 是 `(Quad, Text)×100`，方向 1 把 100 次 quad draw 压成 1 次
   （302 → 203）；纵向列表里 10 个 button 压成 1 次（27 → 18）。两者叠加时，
   纵向列表 27 → 5，**且 `rejected_overlap = 0`**。
4. **tile 位图不但没用，还更差**（21→27、5→23）。32px 的格子比精确矩形更粗，
   小字号文本和窄 quad 会在同一格里假碰撞。**每批次一个包围矩形就是对的粒度**，
   不要上 tile。

### 保守判定会不会退化：会，但只在换行流布局里

`onscreen-300` 两者叠加只到 21（`rejected_overlap = 16`），而结构相同的纵向列表
只要 5（`rejected_overlap = 0`）。原因是包围盒的形状：

- **纵向列表**：quad 批次的包围盒向下长，文字批次的包围盒也向下长，第 k+1 行的
  quad 在两者下方，永远不相交 → 全部合并。
- **wrap-row**：一行排满后换行，文字批次的包围盒变成一条**贯穿整行宽度**的带子，
  下一行的 quad 与这条带子必然相交 → 每换一行就要重开一个批次，约 10 行 ≈ 20 个批次。

真实 UI 的密集区域（列表、表格、表单、侧栏）都是纵向流，wrap-row 是相对少见的
（标签云、图标墙）。所以「保守判定退化成完全不合并」在实际树上不成立，但**退化是
真实存在的**，而且退化后并不比今天差（最坏就是回到每个图元一个批次）。

## 四、风险复核

### `DestPassCounts` 的既有断言

13 处断言里，与「命令顺序」有关的只有两处会被方向 1 触碰：

- `tests.rs:658`（stroke-only 帧 `msaa == 1`）和 `tests.rs:2459`（纯 UI
  `msaa == 1`）。这两个场景本来字形就是后缀，重排只会让它更是后缀，断言仍然成立。
- 其余 11 处断言的是 group / backdrop / color pass 数量。方向 1 的硬规则是
  `HostTexture` / `Custom` / `Backdrop` / `PushGroup` / `PopGroup` **无条件 flush
  全部打开的批次**，这些命令的相对位置一字不动，因此这 11 处不受影响。

### MSAA 快路径会翻转

`mod.rs:1018` 的 `gpu_interleaved` 是在 `commands` 上扫出来的。方向 1 一旦让字形
变成后缀，这个标志就会从 `true` 翻成 `false`，于是 `DestTarget::ensure` 用不同的
`msaa` 参数重建**整张 dest**。这是一次真实的重分配，不能当作顺手的好处收下：
方案里必须显式决定——要么在合并后重算标志并接受一次性重建（并为它写断言），要么
把标志的计算固定在**合并前**的命令序列上，让合并不改变 dest 形状。**建议后者**，
理由是合并的目的是减少 draw，不是改变 dest 的采样数；采样数的切换应当是单独一轮
的、有像素证据的改动。

顺带说明：上面三个场景里 `gpu_interleaved` 恒为 `true`（都有 host texture），
所以 MSAA 快路径在基准场景里本来就没生效，这条风险只对不含 GPU 内容的树成立。

### 跨节点文字 run 合并的真实难点

不在「能不能画对」，在于 `prepare` 的时机：

- `prepare_cryoglyph`（`text.rs:809`）现在是**边遍历边 prepare**，立刻返回
  `PreparedText { index }`，`index` 就是 `renderers` 的下标。要合并成一次
  `prepare([area0, area1, ...])`，就必须把 area **攒到 run 结束再 prepare**，
  于是 `DrawCommand::Text` 要携带的是「run id」，在 flush 时才解析成 renderer 下标。
- `AtlasFull` 重试（`text.rs:861-880`）从「重 prepare 一个 area」变成「重 prepare
  整个 area 列表」。语义不变，但失败面变大：一个 run 越长，一次 trim 后重试要重做的
  工作越多。**run 需要一个长度上限**（比如 256 个 area），这也顺带限制了最坏情况。
- `PreparedKind::Affine`（旋转/斜切文本）走的是自己的顶点缓冲，**不能并入**，必须
  截断 run。
- scissor 可以跨：`TextArea::bounds` 本身就是 per-area 的裁剪，与 painter 设的
  scissor 等价；合并后 pass 的 scissor 取并集即可。这一点上一轮的判断是对的。

代价是：**`cryoglyph::TextRenderer` 的数量从「每帧每条文字一个」变成「每帧每个 run
一个」**，这本身也是一笔可观的节省（今天 1500 条文字 = 1500 个 `TextRenderer`，
每个都带自己的顶点缓冲）。

## 五、建议

**值得做，但顺序和上一轮的方案相反。**

| 步 | 改什么 | 预期 | 实测 | 风险 |
|---|---|---|---|---|
| 1 ✅ | 屏外文字不进 display list + 计数器不再虚报 | `dense-2k` 1503 → ~1003 | **1503 → 1003** | 低 |
| 2 ✅ | 相邻文字 run 合并（与 `push_quad` 同构） | `dense-2k` 1002 → 4 | **1003 → 4** | 中 |
| 3 ✅ | 重叠感知批次合并（方向 1） | `column-list` 23 → 5 | **9 行 `(quad, label)` 39 → 26；`shader-nodes-256` 132 → 16** | 高。采样数判定已冻结在 document order 上 |
| — | 打包 icon atlas | 暂缓 | — | 基准场景里图标已经是 1 次 draw；等有真实的多字形图标条场景再说 |

**第 1 步的实现与这里原先的设计不同，原设计不成立。** 原计划改
`nana-ui-scene/src/scene/visibility.rs`，给 Text 一个「保守外扩的包围盒」。做不了：
`primitive.bounds` 是节点的 content box，不是墨迹盒。`overflow: visible` 的文字
（固定高度里放下三行、`wrap: false` 的长文本）墨迹可以远远超出这个盒子，外扩多少都不
是可证的上界，剔错了就是掉字。

改成在 painter 侧**逐字复刻 cryoglyph 自己的 run 可见性判据**：
`TextRenderer::prepare` 会按 `TextArea::bounds` 的 Y 轴整段丢弃 layout run
（`text_render.rs:76-85`），所以「没有任何一条 run 落在带内」等价于「这次
`render` 不会发 `pass.draw`」。这是**精确**判据不是保守估计，用的是已经排好版的 run
位置而不是节点盒，因此不可能掉字，也不需要 ink 外扩。代价是横向完全出屏的文字仍会
留下一次空 draw（cryoglyph 的 run 过滤只看 Y 轴）——纵向滚动的场景不受影响。

省掉 shaping 这件事因此**没有做**：shaping 发生在判据之前。省下的是 renderer、顶点
缓冲、atlas 走一遍，以及那次假 draw。

第 1 步和第 2 步**互相独立**，都不碰 display list 的顺序，加起来就把基准场景从
1503 打到 5。第 3 步是另一个数量级的工程量，收益只在「quad 与自己的文字交错」的
结构上，应当**单独一轮**，并且只在第 1、2 步落地后再评估是否还需要。

### 实际改的文件

- `crates/nana-ui/src/scene_paint/text.rs`：run 可见性判据；`runs` / `flushed`
  状态；`merge_runs`；`flush_runs`；`ShapeCache::at_capacity` 与 flush-before-evict；
  `frame_texts` 换成 `prev_frame_runs`。
- `crates/nana-ui/src/scene_paint/mod.rs`：`push_text_run`；建完 display list 后
  `flush_runs`。

run 长度**没有加上限**：cryoglyph 的 atlas 会自动扩到
`max_texture_dimension_2d`，`AtlasFull` 只在扩不动时才发生，一次重试重走整条 run 的
成本与今天逐条重试的总成本同阶。真正的上限来自 shape cache——满了就先 flush。
`PreparedKind::Affine`（旋转/斜切文本）自带顶点缓冲，`merge_runs` 直接拒绝，
天然截断 run。

第 3 步同样落在 `crates/nana-ui/src/scene_paint/mod.rs`（`Batching` / `OpenBatch`、
四个 `push_*` 改成走打开批次、`painted_bounds` 与各图元的外扩量、`glyph_then_quad`
冻结并进 `PreparedBatch`）、`clip.rs`（`overlaps_physical` / `union_physical`）和
`text.rs`（`PreparedText::ink`、`can_merge_runs` 放宽到任意更早的 run）。

一条批次不存 kind 也不存 scissor：识别候选的谓词直接读命令本身，命令里就带着它的
scissor，变体也就定死了 kind。开合组时也不需要三态——一个空开空关的组会把自己的
`PushGroup` 取回去，而它开着的时候什么都没发出过，所以不可能有批次指向它或它之后。

### 落地后的实测（2026-09-10，同机同口径）

| 场景 | draw calls（基线 → 1 步 → 2 步 → 3 步） | encode p50 /ms |
|---|---|---|
| `gpu-scene-ui-dense-2k` | 1503 → 1003 → 4 → **4** | 0.0337 → **0.0012** |
| `gpu-scene-shader-nodes-256` | 387 → 387 → 132 → **16** | 0.0226 → **0.0062** |
| `gpu-scene-shader-nodes-256-independent` | 387 → 387 → 132 → **16** | 0.0191 → **0.0061** |
| `gpu-scene-host-textures-64` | 68 → 68 → 68 → 68 | 0.0087 → 0.0101 |
| `gpu-scene-ui`（参照） | 5 → 5 → 5 → 5 | 0.0013 → 0.0012 |

第 3 步对密集 UI 那条线是 0（它全是文字，前两步已经到底），它管的是
`(quad, label)` 交错的列表：一棵 9 行的树基线 40 次 draw、前两步 39 次、第 3 步 26 次。

**渲染差异：没有。** 两张对照帧，整帧字节哈希在每一步前后都不变：
- 跨上下左右边界 + 旋转文字的帧：`42750967d8bbac49`，draw call 32 → 28 → 9。
- 10 行 `(quad, label)` + 一个 scrim + 屏外标签 + 图标条的帧：`f210e0828d88122b`，
  draw call 40 → 39 → 26。

`nana-ui` 全量 464 个测试通过。

### 第 3 步落地时改掉的两个设计判断

- **quad 的包围盒余量必须是 1px，不能是 0，也不该是 `visibility.rs` 的 2px。** 2px 是
  剔除用的保守余量，密集列表里它会让每一行的 quad 与上一行标签的墨迹盒相交，合并全部
  失效（实测 40 → 38，等于没做）。0 则会改变整帧哈希——quad 的抗锯齿边确实画到盒外。
  1px（= outline 基线）既保住了像素又让合并生效（40 → 26）。这三个数是逐个量出来的，
  不是推出来的。
- **文字墨迹盒的纵向外扩不能用一个行高。** 一个行高（14px 字号约 16.8px）会让每条标签
  的盒子盖住上下两行，同样让合并全部失效。改成「请求行高比 1.25em 自然行高少多少」，
  正常行高时接近 0，`line-height: 1` 这种压紧行高时才涨到几个像素——这正是墨迹真的会
  溢出排版盒的场合。

### 怎么验证

照抄 `scene_paint/tests.rs` 里已有的精确增量断言写法
（`adjacent_same_atlas_icons_batch_into_one_draw`）。已落地的三条：

- `text_below_the_clip_band_costs_no_draw_and_no_pixels`——8 条完全在视口下方的
  label 与没有它们的同一棵树 `draw_calls` 相同、**整帧像素相同**；一条跨底边的
  label 必须仍在底部几行上墨。去掉第 1 步的判据，这条断言给出 11 vs 3。
- `adjacent_same_scissor_text_batches_into_one_draw`——8 条相邻 label 与 1 条 label
  `draw_calls` 相同，且合并后每一行都上墨；中间插一个 quad 恰好多 2 次 draw
  （多一次 quad，多一次文字）。
- `merged_text_run_keeps_document_order_between_overlapping_labels`——两条完全重叠的
  label 先断言它们确实合成了一条 run（否则这条测试什么也没测），再断言后者在上。
  把两者颜色对调，断言会挂。
- `open_text_run_survives_shape_cache_eviction`——900 条不同段落压过
  `SHAPE_CACHE_CAP = 512`，屏内那几行必须照常画出来。去掉 `at_capacity()` 那次
  flush，这条会在 `expect` 上 panic。

第 3 步已加的三条：
- `quad_and_label_rows_keep_a_constant_draw_count`——9 行 `(quad, label)` 与 1 行
  draw call 相同，且每行仍画出自己的背景与标签（只看最后一条打开的批次：19 vs 3）。
- `a_quad_over_earlier_text_is_not_folded_ahead_of_it`——盖住前面几行标签的 scrim
  必须仍压在它们上面（去掉重叠判定，标签会浮上来）。
- `batch_merging_does_not_flip_the_dest_sample_count`——文字后的 quad 被折进前面的
  批次后，帧仍是单采样（改回扫描最终命令表，`msaa` 从 0 变 1）。

- 每一步都要重跑 `gpu-scene-ui-dense-2k` 并更新 `docs/gpu-node-scale.md`。
- **不用快照套件做视觉门禁**（本机字体栅格化与基线不符）；像素断言写成
  `tests.rs` 里的定点读取，整帧对照用字节哈希。

## 六、顺带：基准场景本身要修

`perf/scenarios/gpu-scene-ui-dense-2k.json` 的 `notes` 写着「Rows emit Quad/Text/Icon
in document order」，但它实际测到的是 1000 条正文文字 + 500 条**屏外**
button 文字，可见 quad 为 0。这个场景现在既不代表密集列表，报出来的 1503 里还有
500 是虚报。修法：把 `node_repeat` 降到全部在屏内（例如各 100），或者把 root 换成
带滚动裁剪的纵向列表——后者更接近真实密集 UI，也才是第 3 步真正要优化的形状。
