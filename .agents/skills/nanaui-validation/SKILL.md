---
name: nanaui-validation
description: Select and report functional validation for NanaUI changes. Use when planning, implementing, reviewing, or reporting tests, builds, UI snapshots, hosted GPU checks, native window evidence, performance benchmarks, dependency convergence, public compatibility, or Skill validation.
---

# NanaUI Validation

## Matrix

- **UI:** Test changed layout constraints, state transitions, persistence and real action wiring.
- **Agent headless:** Operate Vue/Runtime without a window via `$nanaui-agent-debug`
  (`nana-agent-session` / `VueAgentSession`). Inspect a11y bounds and the PNG; do
  not treat semantic-only updates as visual proof.
- **Visual:** Render the real workspace/gallery path with `ui-snapshots` and inspect affected PNGs.
  Paint `UiScene` through `SceneWgpuPainter`; keep the snapshot painter alive through
  readback and send a real redraw update.
- **GPU:** Test geometry, invalidation and resource lifecycle; run `hosted-gpu-demo` for Surface or
  shared-context changes.
- **Window:** Check the outcome contract and affected targets; require real platform evidence for
  native effects.
- **Performance:** Issue #8 [`perf/README.md`](../../../perf/README.md):
  Nana work-counter / catalog / hotspot gates. Runtime/Scene benches and
  [`validate-runtime-performance.py`](../../../scripts/validate-runtime-performance.py)
  are the #8 semantic gates. Cross-toolkit runners are Issue
  [#12](https://github.com/sena-nana/NanaUI/issues/12) observation, not #8
  pass/fail. Weekly GHA is **not** a fixed machine.
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
cargo check -p vue-counter --all-targets --all-features --locked
cargo test -p nana-js-v8 --features engine --locked -- --test-threads=1
(cd packages/nanavue-runtime && npm test)
(cd packages/nanavue-components && npm test)
(cd crates/nana-js-engine/fixtures/vue-sfc-compat && npm ci && npm run build)
(cd tools/css-parity-webview && cargo check --all-targets --locked) # macOS; not a workspace member
cargo clippy --workspace --all-targets --locked --no-deps -- -D warnings
cargo clippy -p nana-ui -p component-gallery --all-targets --all-features --locked --no-deps -- -D warnings
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
cargo test -p nana-ui-devtools --features agent --lib --locked
```

V8 is the single product JS engine. `nana-js-v8` engine tests are feature-gated (`engine`) and
serialized (`--test-threads=1`). Workspace Clippy (`--no-deps -- -D warnings`) and the public
NanaUI / Gallery path both enforce zero warnings.

For Skill-only changes, there is no single skill-validation script. Verify Skill
frontmatter, links, and `git diff --check`. Choose `cargo test` / snapshots /
`hosted-gpu-demo` in proportion to the change; do not add or rerun unrelated
functional tests, and do not treat `cargo check` as GPU evidence.

When the change is performance, Runtime incremental systems, virtualization, or
dirty/layout-stop behavior, also run the relevant subset:

```bash
python3 perf/contract.py --self-test
python3 perf/contract.py --evaluate-invariants target/performance/issue8
python3 perf/runners/nana/run.py --print-plan --scenario static-tree-5k
python3 perf/runners/nana/run.py --print-plan --scenario gpu-scene-ui
python3 perf/runners/nana/run.py --scenario gpu-scene-ui-live2d   # expected exit 2
python3 perf/runners/nana/run.py --scenario mutation-paint-only --output target/performance/issue8/nana-mutation-paint-only.json
python3 perf/runners/nana/run.py --scenario hover --from-report perf/fixtures/nana-runtime-static-tree.json
python3 perf/runners/nana/run.py --scenario mutation-paint-only --from-report perf/fixtures/nana-runtime-static-tree.json
python3 perf/runners/nana/run.py --scenario static-tree-100 --from-report perf/fixtures/nana-runtime-static-tree.json
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

Relative cross-toolkit multipliers are not #8 acceptance; see
[`perf/README.md`](../../../perf/README.md). Do not invent
reference timings. Native RHI same-RenderPlan A/B is **NO-GO**.

Report exact commands and results, separating regressions from environment or untested-platform
limits. Performance regressions that trip an in-force gate need a recorded
cause and waiver; silent merge is forbidden.
