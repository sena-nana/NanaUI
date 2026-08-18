# Issue #8 performance harness

Shared Scenario schema and thin runners. The contract they must satisfy is
[`docs/performance-contract.md`](../docs/performance-contract.md).

```text
perf/
├── contract.py              # schema helpers + extractors
├── schema/                  # JSON Schema for Scenario and run reports
├── scenarios/               # shared workload definitions
├── runners/{nana,iced,gpui}/
├── micro/                   # reserved; no Issue #8 micro suite yet
├── baselines/               # reserved; do not invent a history database
└── reports/                 # generated runner output (gitignored)
```

## Invoke

From the repository root:

```bash
python3 perf/contract.py --check-schema
python3 perf/contract.py --self-test

python3 perf/runners/nana/run.py --list
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-100
python3 perf/runners/nana/run.py --scenario static-tree-100 --output /tmp/nana-static-tree-100.json
python3 perf/runners/nana/run.py --all --output-dir perf/reports

python3 perf/runners/iced/run.py --scenario static-tree-100 --output /tmp/iced-static-tree-100.json
python3 perf/runners/iced/run.py --scenario hover --output /tmp/iced-hover.json   # exit 2
python3 perf/runners/gpui/run.py --scenario virtual-list-10k --output /tmp/gpui-virtual-list-10k.json
```

GPUI is a stub: it prints `status: unsupported` and exits **2**. Exit **1** is a
real failure. Fake GPUI numbers are forbidden.

`--from-report` maps an already-produced `nana-*-benchmark` / `ui-benchmark`
JSON without invoking cargo. Use it in tests and when another job already ran
the binary.
