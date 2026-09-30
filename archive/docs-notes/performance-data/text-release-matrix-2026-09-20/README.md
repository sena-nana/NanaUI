# 文本 cutover 的性能矩阵（#99 §10）

`nana-text` + `NanaRenderer::text` 成为唯一产品文本栈之后重跑的一轮。记的是
**work counter**，不是毫秒：毫秒在这台机器上受背景负载影响能跳好几倍，而
「这一帧到底有没有碰文字」是结构性的，counter 说得准。

adapter：Apple M4（Metal）。四份原始数据在同目录下。

```bash
cargo run --release --locked -p nana-ui --features gpu \
    --bin nana-text-paint-benchmark -- --output text-paint.json
cargo run --release --locked -p nana-ui-scene --features benchmark \
    --bin nana-dirty-frame-benchmark -- --shape layout --position head \
    --engine nana-text --output dirty-frame-head-nana-text.json
cargo run --release --locked -p nana-ui-scene --features benchmark \
    --bin nana-dirty-frame-benchmark -- --shape layout --position head \
    --engine measure --output dirty-frame-head-measure.json
cargo run --release --locked -p nana-ui-scene --features benchmark \
    --bin nana-text-edit-benchmark -- --output text-edit.json
```

## 门禁

### static steady（`text-paint.json`，1 / 100 / 1k / 10k label）

| | |
| --- | --- |
| `shape_cache_misses` | **0** |
| `glyph_rasterized` | **0** |
| `glyph_upload_bytes` | **0** |
| `text_instance_rebuilds` | **1/帧，与 label 数无关** |

那个 1 不是 label：`Static` 这个 workload 靠每帧改一个 ticker 节点的文字
（`"."` 重复 1–3 次）来逼出重批。它在 1、100、1000、10000 四档都恰好是 1，
**与文字量无关**——这正是「静止文本的稳态帧不碰字形」的证据。同一帧
`glyph_resolve_requests = 2`，就是那个 ticker 的两个字形。

### paint-only（`color`，每帧改颜色）

`shape_cache_misses` / `glyph_rasterized` / `glyph_upload_bytes` /
`text_instance_rebuilds` 在 1/100/1k/10k × 60/120/240Hz **全部为 0**。
颜色是 presentation，走 run 行，不碰 instance。

### compositor-only motion（`opacity`、`transform-panel`，60/120/240Hz）

同样四项在 1/100/1k/10k 全部为 0。唯一的例外是 `transform-panel` 的 100 档：
`text_instance_rebuilds = 0.0167`，即 **60 帧里共 1 次**——采样窗口边界上的一次
重建，不是每帧的工作（10k 档是精确的 0）。

`transform`（整棵树旋转，最坏情况而不是常见情况）本来就会重解析字形，不是
compositor-only 门禁的对象。

### #33：head-dirty 不再有隐藏的近 O(N²) zero-work 路径

`dirty-frame-head-nana-text.json`，`--shape layout --position head`，
真的 `nana-text` 引擎，节点数 502 / 1002 / **2002 / 4002 / 8002**（#99 要的
2k/4k/8k），dirty 行 1 / 2 / 8 / 32 / 128：

| | 整个网格 |
| --- | --- |
| `text_nodes_shaped` | **0** |
| `text_bytes_hashed` | **0** |
| `layouts_created` | **0** |
| `text_shape.key_builds` | **0** |
| `text_nodes_revision_skipped` | = `text_nodes_considered` |

每个节点都是 O(1) 的代际比较就跳过，**一个字节都没有重新哈希**。flush 时间与
em 宽度测试 shaper（`--engine measure`）在噪声内相同——真引擎在这条路上不比
假 shaper 贵，因为两边都不做文本工作。

### TextInput / 编辑器（`text-edit.json`）

250 / 1000 / 4000 / 8000 行（最大 310 KB），type / delete / caret / vertical /
click × head / tail：

| 动作 | `paragraphs_relayout_from_edit` | `paragraphs_reshaped_from_edit` |
| --- | --- | --- |
| type、delete | **1** | 0 |
| caret、vertical、click | **0** | 0 |

一次编辑只重排**它自己那一段**，与文档大小无关；光标移动、上下移动、点击
一段都不排。8000 行 310 KB 上一次输入 `input_ms` p50 0.059 ms、`flush_ms`
p50 0.171 ms。

## 这一轮没覆盖的

不假装跑过，也不拿单测冒充基准：

| #99 §10 要求 | 状态 |
| --- | --- |
| constraint-only resize（`shape_runs_created == 0`） | **没有对应 workload**。约束变化复用 ShapedRun 由 `nana-text` 的 `constraint_only_relayouts` 计数与 `layout_engine.rs` 的用例守着，但没有一条端到端的 resize 基准 |
| text-heavy table | `perf/scenarios/text-table.json` 存在，但它的 §8.1 门禁读的是 bench 侧 `MeasureTextShaper` 的 `glyph_cache_*`，不是产品引擎；这一轮没跑 |
| wrapped paragraph | 只有单测覆盖（断行、省略号、`max_height` 截断），没有基准格 |
| atlas pressure | 单测 `a_corpus_larger_than_the_atlas_keeps_placing_glyphs_and_never_samples_a_stale_one` |
| multi-window shared Device | 单测 `a_second_window_on_one_device_reuses_the_first_windows_glyphs`、`closing_one_window_leaves_the_others_glyphs_placed` |
| IME composition | `text-edit` 有 `composition_updates` 计数，但这一轮的 action 集合里没有组字；IME 的正确性在 `nana-text` 的 `editable_text.rs` 用例里 |

前两项要补的话得新写 workload，属于 #8 的门禁工程，不在 #99 的 cutover 范围里。
