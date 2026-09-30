# #99 收口：五道文本门禁的真机报告（2026-09-22）

macOS，Apple M4（Metal），`--release --locked`。每个 JSON 是
`python3 perf/runners/nana/run.py --scenario <id>` 的原样输出，`text_counters` 是**每个采样帧**
的平均值，`invariants` 是对它们的判定。门禁读的是 work counter，不是毫秒。

| 门禁 | 每帧动的是什么 | 塑形 | 新 layout | layout 查询 | 画笔自排 | 栅格化 | atlas 上传 | instance 重建 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `gpu-scene-text-retained` | 一千个标签里一个换文本 | 1 | 1 | 1 | 0 | 0.45 | 43 B | 1 |
| `gpu-scene-text-paint-color` | 每个标签换前景色 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `gpu-scene-text-compositor-opacity` | 容器淡入淡出 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `gpu-scene-text-compositor-transform` | 容器在 −1.5° / 0° / +1.5° 间转 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `gpu-scene-text-constraint-resize`（新） | 每个标签在 72 / 60 px 两档宽度间交替 | **0** | 0 | **1000** | 0 | 0 | 0 | 378 |

## 这一轮新增 / 收紧的

- **constraint-only（#99 §10）**：`gpu-scene-text-constraint-resize`。一千个会折行的标签每帧换
  宽度，内容和样式不变。判据：`text_nodes_shaped ≤ 0`、`text_layouts_reshaped ≤ 0`（新 layout 里
  没有一份需要重新塑形）、画笔不自排、不栅格化、不传 atlas。instance 重建不设门：字形真的挪了。
  另有一行 `text_layout_lookups ≥ 900` 防空转——宽度没传到文本的一帧会让其余每一行都「顺便」通过。
  两档宽度交替时引擎的 layout cache 两档都命中，所以 `text_layouts_created` 也是 0；宽度
  每帧都是新值时走的是「从已塑形的 run 重排」，那条由 CI 里的
  `a_width_change_relayouts_from_the_runs_it_already_shaped` 守着（`text_nodes_shaped == 0`、
  `constraint_only_relayouts == layouts_created`）。
- **static steady**：`gpu-scene-text-retained` 过去只约束 instance / 栅格 / 上传，现在加上
  `text_nodes_shaped ≤ 1`、`text_layouts_created ≤ 1`、`paint_shape_cache_misses ≤ 0`——
  那个 1 是每帧换字的 ticker，其余 999 个标签什么都不做。

## 仍然不在门禁里的

- IME 组字与 text-heavy table 的**产品引擎**计数：`perf/scenarios/ime.json` / `text-table.json`
  跑的是 em 宽度测试 shaper。编辑器每次组字只重排自己那一段由 `editable_text_node.rs` 的单测守着，
  表格由 `nana-text-paint-benchmark` 的 `table` / `table-scroll` workload 记录（不判定）。
- ~~这五道门禁都不在 CI 里真跑~~（已补上，见下）。

## 之后补上的：CI 里的判定

收口时 `perf/contract.py --evaluate-invariants` 把这五个 id 一律判成 **skipped**（「不是 §8.1
honest-ok id」），也就是说即使有人把真机报告交给它，门禁也打不红。现在：

- 判定器认 `catalog.json` 的 `nana_text_ids`：`text_counters.*` 全部量到且守住才是 ok，缺一条是
  skipped（不许把缺失当作通过），和其他门禁混在一起时缺席的一律 fail-closed。它们不进
  `SECTION_8_1_HONEST_OK_IDS`，§8.1 目录的完整性要求不变。
- PR CI（`runtime-work-invariants`）用 `perf/fixtures/nana-gpu-scene-text-*.json` 判定——这是
  本机（Apple M4）`nana-gpu-scene-benchmark` 的原样输出，和 `gpu-scene-ui` 的做法一样。
- 每周的 `macos-composition` 任务在真 GPU 上现跑五道门禁并判定。
- `--self-test` 用真正的判定器过一遍：安静帧 ok、任意一条计数打爆 failed、缺计数不是 ok，
  并核对两个 workflow 都接了这五个 id。
