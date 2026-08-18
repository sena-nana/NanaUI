---
name: nanaui-validation
description: Select and report functional validation for NanaUI changes. Use when planning, implementing, reviewing, or reporting tests, builds, UI snapshots, hosted GPU checks, native window evidence, performance benchmarks, dependency convergence, public compatibility, or Skill validation.
---

# NanaUI Validation

## Matrix

- **UI:** Test changed layout constraints, state transitions, persistence and real action wiring.
- **Visual:** Render the real workspace/gallery path with `ui-snapshots` and inspect affected PNGs.
  Keep `UserInterface::Cache` alive through readback and send a real redraw update.
- **GPU:** Test geometry, invalidation and resource lifecycle; run `hosted-gpu-demo` for Surface or
  shared-context changes.
- **Window:** Check the outcome contract and affected targets; require real platform evidence for
  native effects.
- **Performance:** Use the Issue #8 [`performance-contract.md`](../../../docs/performance-contract.md)
  for shared Scenario names, relative gates, work-counter invariants, and runner
  exit codes. Runtime/Scene benches and
  [`validate-runtime-performance.py`](../../../scripts/validate-runtime-performance.py)
  are the current semantic gates. Weekly
  `.github/workflows/runtime-performance.yml` (`ubuntu-latest` / `macos-latest`
  cron) is a stand-in, **not** a fixed benchmark machine.
  [`performance-baseline.md`](../../../docs/performance-baseline.md) is the
  **legacy Iced Gallery** series only — historical numbers, not the #8 contract.
- **Compatibility:** Review exports, serialized fields, manifests, lockfiles and consumers when
  public boundaries change.

## Checks

Select only the relevant layers:

```bash
cargo fmt --all -- --check
cargo check -p nana-ui --lib --no-default-features --locked
cargo check -p component-gallery --bin component-gallery --locked
cargo test --workspace --all-targets --locked
cargo check --workspace --all-targets --locked
cargo check -p nana-ui --all-targets --all-features --locked
cargo check -p component-gallery --all-targets --all-features --locked
cargo check -p vue-counter --all-targets --features windowed --locked
cargo check -p vue-counter --all-targets --no-default-features --features engine-v8,windowed --locked
(cd crates/nana-js-engine/fixtures/vue-sfc-compat && npm ci && npm run build)
cargo check -p nana-css-parity --all-targets --features webview-ref --locked # macOS
cargo clippy --workspace --all-targets --locked
cargo clippy -p nana-ui -p component-gallery --all-targets --all-features --locked --no-deps -- -D warnings
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
```

Application crates intentionally reject simultaneous QuickJS and V8 features. Use the explicit
legal-feature matrix above instead of workspace-wide `--all-features`.
Workspace Clippy remains a full diagnostic pass; the public NanaUI and Gallery path additionally
enforces `-D warnings` while legacy cross-package lint debt is retired separately.

For Skill-only changes, run `quick_validate.py` on each Skill, verify links and metadata, then run
`git diff --check`; do not add or rerun unrelated functional tests.

When the change is performance, Runtime incremental systems, virtualization, or
dirty/layout-stop behavior, also run the relevant subset:

```bash
python3 perf/contract.py --self-test
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-5k
python3 perf/runners/nana/run.py --scenario mutation-paint-only --output target/performance/issue8/nana-mutation-paint-only.json
python3 perf/runners/iced/run.py --scenario static-tree-100 --from-report docs/performance/2026-08-14-issue7-phase0-iced.json
python3 perf/runners/iced/run.py --scenario hover --from-report docs/performance/2026-08-14-issue7-phase0-iced.json   # expected exit 2
python3 perf/runners/nana/run.py --scenario hover --from-report docs/performance/2026-08-14-issue7-phase3-runtime.json   # expected exit 2 until a 10k hover case exists
python3 perf/runners/gpui/run.py --scenario virtual-list-10k   # expected exit 2
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --output target/performance/runtime.json
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-framework-benchmark -- --output target/performance/framework.json
cargo run --release --locked -p nana-ui-vue --features benchmark --bin nana-vue-runtime-benchmark -- --output target/performance/vue.json
cargo run --release --locked -p nana-ui-scene --features benchmark --bin nana-scene-benchmark -- --output target/performance/scene.json
python3 scripts/validate-runtime-performance.py \
  --runtime target/performance/runtime.json \
  --framework target/performance/framework.json \
  --vue target/performance/vue.json \
  --scene target/performance/scene.json
```

Relative P50 1.15× / P95 1.20× / P99 1.25× / memory 1.20× vs Iced/GPUI is
**not yet enforceable**. GPUI is an unsupported stub. Do not invent reference
timings. Native RHI same-RenderPlan A/B is **NO-GO** (#7 Gate B).

Report exact commands and results, separating regressions from environment or untested-platform
limits. Performance regressions that trip an in-force gate need a recorded
cause and waiver; silent merge is forbidden.
