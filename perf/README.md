# Issue #8 performance harness

Shared Scenario schema and thin runners. The contract they must satisfy is
[`docs/performance-contract.md`](../docs/performance-contract.md)
(living #8 DoD: Nana work-counter / catalog / hotspot + CI fail-closed;
micro / E2E / Gallery stress are out of #8).

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

## Invoke

From the repository root:

```bash
python3 perf/contract.py --check-schema
python3 perf/contract.py --self-test
python3 perf/contract.py --evaluate-invariants path/to/nana-text-table.json path/to/nana-virtual-list-10k.json
python3 perf/contract.py --evaluate-invariants target/performance/issue8

python3 perf/runners/nana/run.py --list
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-100
python3 perf/runners/nana/run.py --print-plan --scenario gpu-scene-ui
python3 perf/runners/nana/run.py --scenario gpu-scene-ui-live2d   # expected exit 2
python3 perf/runners/nana/run.py --scenario static-tree-100 --output /tmp/nana-static-tree-100.json
python3 perf/runners/nana/run.py --all --output-dir perf/reports

python3 perf/runners/iced/run.py --scenario static-tree-100 --output /tmp/iced-static-tree-100.json
python3 perf/runners/iced/run.py --scenario mutation-paint-only --output /tmp/iced-mutation-paint-only.json
python3 perf/runners/iced/run.py --scenario hover --output /tmp/iced-hover.json
python3 perf/runners/iced/run.py --scenario virtual-list-10k --output /tmp/iced-virtual-list-10k.json
python3 perf/runners/iced/run.py --scenario text-table --output /tmp/iced-text-table.json
python3 perf/runners/iced/run.py --scenario dock-workspace --output /tmp/iced-dock-workspace.json     # expected exit 2
python3 perf/runners/iced/run.py --scenario text-editor --output /tmp/iced-text-editor.json           # expected exit 2
python3 perf/runners/iced/run.py --scenario gpu-scene-ui --output /tmp/iced-gpu-scene-ui.json          # expected exit 2
python3 perf/runners/iced/run.py --scenario static-tree-50k --output /tmp/iced-static-tree-50k.json   # expected exit 2 (incomparable)
python3 perf/runners/gpui/run.py --print-plan --scenario static-tree-100
python3 perf/runners/gpui/run.py --scenario static-tree-100 --output /tmp/gpui-static-tree-100.json
python3 perf/runners/gpui/run.py --scenario gpu-scene-ui --output /tmp/gpui-gpu-scene-ui.json  # expected exit 2
python3 perf/runners/gpui/run.py --scenario static-tree-100 --from-report perf/fixtures/gpui-scenario-static-tree-100.json
```

GPUI ``engine/gpui-scenario-bench`` is a crates.io ``gpui`` 0.2.2 TestAppContext
adapter. Wired kinds match Iced; others stay **unsupported** (exit **2**).
``present_ms`` / ``frames_after_idle`` are omitted (not 0). Relative multipliers
stay off. Laptop fixtures are not a named fixed-machine baseline.

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
`invariants/` completeness vs `weekly/`, and gated ids:
[`performance-contract.md`](../docs/performance-contract.md) §8.1.
