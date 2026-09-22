# Performance harness

Shared Scenario schema and thin runners. #8 DoD is Nana work-counter /
catalog / hotspot + CI fail-closed. Cross-toolkit same-batch numbers are
[#12](https://github.com/sena-nana/NanaUI/issues/12) observation, not #8
pass/fail.

```text
perf/
├── contract.py              # schema helpers + extractors
├── schema/                  # JSON Schema for Scenario and run reports
├── scenarios/               # shared workload definitions
├── runners/{nana,iced,gpui}/
├── micro/                   # reserved; not #8 DoD (no micro suite)
├── baselines/               # reserved; do not invent a history database
└── reports/                 # generated runner output (gitignored)
```

## NanaUI 内部优化行

`catalog.json` 的 `nana_gpu_scale_ids` 列出四个 `GpuScene` / `UiOnly` 场景，量的是 GPU
节点规模与框架层 draw call：`gpu-scene-shader-nodes-256`、
`gpu-scene-shader-nodes-256-independent`、`gpu-scene-ui-dense-2k`、
`gpu-scene-host-textures-64`。它们**不在** `harness_ids` 里，不是 #8 DoD，也没有
Iced / GPUI 对照——那两个 runner 对 `GpuScene` 一律 unsupported，摆进跨框架表格只会
重新引入本文件开头警告的不可比性。规模写在 `params.node_repeat`，runner 必须原样回显，
否则 extractor 拒绝该报告。基线与判据见
[`docs/gpu-node-scale.md`](../docs/gpu-node-scale.md)。

## Windows 合成器静止期

`windows-composition-steady` 量的是宿主对 OS 合成器做了多少无效功：窗口几何静止之后，
`IDCompositionDevice::Commit`、visual tree 改动、native-content region 提取和原生 chrome
重写都必须停在 0，不管 GPU 还在以多高帧率出帧。六条 invariant 全是 `eq 0`。

它**不在** `harness_ids` 里，不是 #8 §8.1 目录 id，也不是 weekly DoD：数字只有 Windows +
DX12 真机跑得出来，没有合成 target 的机器**不要**写报告——宁可没有，也不要一份全零的假报告。
产出入口是 `cargo run -p nana-ui --features "hosted bundled-fonts" --example native-content-probe`，
它把静止期增量写到 `target/performance/windows-composition-steady.json`。判据与实现见
[`docs/window.md`](../docs/window.md)。

## Issue #98 保留期文本

`catalog.json` 的 `nana_text_ids` 是 #98 的三类文本门禁加 #99 的 constraint-only 门禁，都是一千个文本节点：

| id | 每帧动的是什么 | 判据 |
| --- | --- | --- |
| `gpu-scene-text-retained` | 其中一个标签换文本 | 静止稳态：只重建那一个，不栅格化、不传 atlas、不重传别人的 instance |
| `gpu-scene-text-paint-color` | 每个标签换前景色 | paint-only：不塑形、不排版、不栅格化、不传 atlas，也不重建 / 重传 instance |
| `gpu-scene-text-compositor-opacity` | 装标签的列表淡入淡出 | compositor-only：同上 |
| `gpu-scene-text-compositor-transform` | 装标签的列表转 −1.5° / 0° / +1.5° | compositor-only：同上，且每三帧进出一次恒等变换 |
| `gpu-scene-text-constraint-resize` | 每个标签在两档宽度间交替（#99） | constraint-only：不塑形、新 layout 不重新塑形、画笔不自排、不栅格化、不传 atlas；`text_layout_lookups ≥ 900` 防空转 |

它们都**不在** `harness_ids` 里，理由和 motion 那组一样——判据是这一帧让文本路径重做了
什么（`text_counters.*`），不是公共 CI 的 GPU timing。`text_counters` 里 Runtime 与画笔
两侧都有：`text_nodes_shaped` / `text_layouts_created` 是 Runtime 这一帧重新测量的，
`paint_shape_cache_misses` 是画笔拿不到 Runtime layout 时自己排的。

场景必须真的在动：`params.text_ticker`（换文本）或 `params.text_animation`
（`color` / `opacity` / `transform` / `resize`）二选一。不动的话 painter 直接复用上一帧的批次，那一帧
什么都没做，counter 全是 0，门禁永远不会红。extractor 核对报告里回显的这两个参数，跑了
不动的场景会被拒绝。ticker 的文字是定宽的（`tick 0007`）：`tick 9 → tick 10` 会把同一行
后面的标签挪一个数字宽，那是重排的成本，不是保留期的，不该由采样窗口碰没碰上它来决定门禁红绿。

```bash
for id in gpu-scene-text-retained gpu-scene-text-paint-color \
          gpu-scene-text-compositor-opacity gpu-scene-text-compositor-transform \
          gpu-scene-text-constraint-resize; do
  python3 perf/runners/nana/run.py --scenario "$id" --output "target/performance/issue98/$id.json"
done
# 标签数 × 动画频率的完整报告（不是门禁，是可复现数字）
cargo run --release --locked -p nana-ui --features gpu --bin nana-text-paint-benchmark -- --output target/performance/issue98/text-paint.json
```

判据与前后数字见 [`docs/text-engine.md`](../docs/text-engine.md) 的「保留期文本」一节。

## Issue #87 compositor motion

`catalog.json` 的 `nana_motion_ids` 列出 compositor-only 结构门禁与 1/100/1k/10k
scale、retarget、churn。它们**不在** `harness_ids`：验收是 work counter（UiWorld /
layout / style / extract / CPU sample 必须为 0），不是公共 CI GPU timing。
`#8` 的 `animation.json` 稀疏门禁（`animations_considered` /
`animation_deadlines_scanned`）继续独立存在。

```bash
python3 perf/contract.py --self-test
python3 perf/runners/nana/run.py --print-plan --scenario compositor-steady
python3 perf/runners/nana/run.py --scenario compositor-steady --output target/performance/issue87/nana-compositor-steady.json
python3 perf/runners/nana/run.py --scenario compositor-tracks-1k --output target/performance/issue87/nana-compositor-tracks-1k.json
python3 perf/runners/nana/run.py --scenario compositor-retarget --output target/performance/issue87/nana-compositor-retarget.json
cargo run --release --locked -p nana-ui-scene --features benchmark --bin nana-scene-benchmark -- --compositor --output target/performance/issue87/compositor.json
```

`--compositor` 一次 dump 全部 scale（transform / opacity / mixed）以及 retarget / churn。
`motion_descriptors_uploaded` 故意省略：该 binary 不 encode/submit。GPU descriptor
不每帧重传由 `SceneWgpuPainter::last_motion_work()` 的测试覆盖，不是这条门禁。

Inspector 入口：`UiWorld::inspect_motion()`（track / class / evaluator / base vs
presentation / CPU fallback / deadline / GPU handle / layout-paint-extract 影响），
`UiScene::annotate_motion_inspector` 补 layer 与 promotion 原因。打印合同：

```text
Node #123 transform
Class: Compositor
Evaluator: GPU
Layer: #7
Runtime samples/frame: 0
```


## Issue #89 文本迁移基准

`nana-dirty-frame-benchmark --shape layout --position head` 的 2k / 4k / 8k 三格是
Issue #33 的原始 workload，Epic #88 把它保留为 `nana-text` 的迁移基准。

它**不在** `catalog.json` 的任何 id 列表里，**没有时间门禁**，是人工前后对比：接进
scenario 合同需要新的 `kind`、extractor 和签入 fixture，等 `nana-text` 真有引擎可测再做。
CI 只保证它还在、还能编译、网格还没被收窄（`ci.yml` 的
`cargo check -p nana-ui-scene --features benchmark` 与
`the_migration_grid_still_spans_two_four_and_eight_thousand_nodes`）。

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 1000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 4000 --samples 150 --warmup 30
```

基线数字与「每阶段重跑并贴回」的规则在 [文本引擎](../docs/text-engine.md#33-迁移基准)。
`nana-text` 自己的结构化 correctness 门禁是 `cargo test -p nana-text --all-targets`，
与本目录的 work-counter 合同是两回事。

## Issue #101 theme / style 基线

`catalog.json` 的 `nana_theme_ids` 是 Issue #101 §4 的 theme/style 工作量基线：static idle、
1/100/1k/10k 控件、单节点 hover / focus、light↔dark、accent-only、density、以及大树头部的
style mutation。和 motion / text 那两组一样，它们**不在** `harness_ids` 里——判据是一次主题
变化让保留期流水线重做了什么（`work_counters.style_nodes_*` / `theme_reads` /
`*_from_style`），不是公共 CI 的时序。

```bash
python3 perf/runners/nana/run.py --print-plan --scenario theme-palette-switch
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/runners/nana/run.py --scenario theme-palette-switch --from-report target/performance/issue101/theme.json
# 不想重跑 benchmark 时，用归档的那份报告重放：
python3 perf/runners/nana/run.py --scenario theme-density --from-report docs/performance-data/theme-audit-2026-09-19/theme-work-counters.json
```

这组没有单独的 `perf/fixtures/` 副本：归档报告只留一份（带日期、带机器），extractor 与门禁本身由
`perf/contract.py --self-test` 的 `theme_baseline_tests` 覆盖——合成报告能测到真实报告测不到的
负例（workload 不匹配、scale 不回显、`considered != resolved + skipped`、counter 缺失）。

报告里 theme counter 与 `#8` frame counter 落在同一个 `work_counters` 对象：名字不冲突，
放在一起才能用 `style_processed` 去核对 `style_nodes_considered`。`frame_work` 取的是测量
窗口里那次 drain 自己的计数，不是 `last_work_counters`——空帧不会替换后者，否则 idle 那行
会报上一帧的数字。

这些数字是 **Phase 0 基线**，描述现状（包括 Issue #100 要收窄的地方），不是目标值。改动
后重跑并贴回 [`docs/theme.md`](../docs/theme.md) 的「性能基线」一节。

## Vue vs Rust L3 输入成本

不在这套 Scenario 里，因为它测的不是一个 toolkit 跑一个负载，而是**同一个进程里**建立
三棵形状相同的树并驱动同一段手势——把它拆成两个 runner 会重新引入这份 README 开头就在
警告的不可比性。

```bash
for mode in bare listeners reactive bare-scroll listeners-scroll reactive-scroll; do
  node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs $mode 2000
done
cargo build --release -p nana-ui-devtools --features agent-bin --bin nana-hover-benchmark
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60 --shape window
```

`--shape window` 是真实窗口重绘的序列，`--shape headless` 是 Agent 会话的替身；只量后者
会得出一条没有窗口会走的路径的结论。`--scroll` 驱动带滚动容器的同形状树。

结果、成因和已落地的修复见 [`docs/input-cost.md`](../docs/input-cost.md)。这是目前唯一测过
JS↔Rust 边界的基准：`nana-vue-runtime-benchmark` 不 import V8。

## Invoke

From the repository root:

```bash
python3 perf/contract.py --check-schema
python3 perf/contract.py --self-test
python3 perf/contract.py --evaluate-invariants path/to/nana-text-table.json path/to/nana-virtual-list-10k.json
python3 perf/contract.py --evaluate-invariants target/performance/issue8
python3 perf/contract.py --evaluate-relative \
  perf/fixtures/iced-scenario-static-tree-100.json \
  perf/fixtures/gpui-scenario-static-tree-100.json

python3 perf/runners/nana/run.py --list
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-100
python3 perf/runners/nana/run.py --print-plan --scenario gpu-scene-ui
python3 perf/runners/nana/run.py --scenario gpu-scene-ui-live2d   # expected exit 2
python3 perf/runners/nana/run.py --scenario static-tree-100 --output /tmp/nana-static-tree-100.json
python3 perf/runners/nana/run.py --all --output-dir perf/reports

python3 perf/runners/iced/run.py --scenario static-tree-100 --from-report perf/fixtures/iced-scenario-static-tree-100.json
python3 perf/runners/iced/run.py --scenario mutation-paint-only --from-report perf/fixtures/iced-scenario-mutation-paint-only.json
python3 perf/runners/iced/run.py --scenario hover --from-report perf/fixtures/iced-scenario-hover.json
python3 perf/runners/iced/run.py --scenario virtual-list-10k --from-report perf/fixtures/iced-scenario-virtual-list-10k.json
python3 perf/runners/iced/run.py --scenario text-table --from-report perf/fixtures/iced-scenario-text-table.json
python3 perf/runners/iced/run.py --scenario dock-workspace --output /tmp/iced-dock-workspace.json     # expected exit 2
python3 perf/runners/iced/run.py --scenario text-editor --output /tmp/iced-text-editor.json           # expected exit 2
python3 perf/runners/iced/run.py --scenario gpu-scene-ui --output /tmp/iced-gpu-scene-ui.json          # expected exit 2
python3 perf/runners/iced/run.py --scenario static-tree-50k --output /tmp/iced-static-tree-50k.json   # expected exit 2 (incomparable)
python3 perf/runners/gpui/run.py --scenario static-tree-100 --from-report perf/fixtures/gpui-scenario-static-tree-100.json
python3 perf/runners/gpui/run.py --scenario gpu-scene-ui --output /tmp/gpui-gpu-scene-ui.json  # expected exit 2
```

Iced/GPUI observation is fixture-only (`--from-report`). Live compile of
`engine/iced` / `engine/gpui-scenario-bench` is gone. Unwired kinds: exit **2**.
``--evaluate-relative`` is #12 observation, not multiplier CI.

`--from-report` maps an already-produced `nana-*-benchmark` /
`nana-gpu-scene-benchmark` / `scenario-bench` / Gallery `ui-benchmark` JSON
without invoking cargo. Use it in tests and when another job already ran the
binary. Iced Dock/TextEditor stay exit 2 even if a JSON says `status: ok`; do
not keep fake-ok files under `perf/fixtures/`.

`overscan_rows`: catalog Table (and list/tree) overscan is **8 rows**. Iced
copies that catalog param; Nana writes `mounted − visible`. Compare windows via
`list_overscan_px` / `table_overscan_y_px`. `window_ms` is index arithmetic
(Fenwick lookup may round to 0); judged work is `materialize_ms` +
`live_ui_entities`, not `window_ms`.

`--evaluate-invariants` judges Nana runner envelopes. Exit codes,
`invariants/` completeness vs `weekly/`, and gated ids are described
below in §8.1.
