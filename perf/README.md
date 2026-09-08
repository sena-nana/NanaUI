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

## Vue vs Rust L3 输入成本

不在这套 Scenario 里，因为它测的不是一个 toolkit 跑一个负载，而是**同一个进程里**建立
三棵形状相同的树并驱动同一段手势——把它拆成两个 runner 会重新引入这份 README 开头就在
警告的不可比性。

```bash
for mode in bare listeners reactive; do
  node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs $mode 2000
done
cargo build --release -p nana-ui-devtools --features agent-bin --bin nana-hover-benchmark
./target/release/nana-hover-benchmark --rows 2000 --moves 400 --warmup 60
```

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
