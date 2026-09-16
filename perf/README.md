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
