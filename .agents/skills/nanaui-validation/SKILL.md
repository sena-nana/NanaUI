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
- **Performance:** Compare the same scenario, profile, machine state, warmup and sample settings
  using [`performance-baseline.md`](../../../docs/performance-baseline.md).
- **Compatibility:** Review exports, serialized fields, manifests, lockfiles and consumers when
  public boundaries change.

## Checks

Select only the relevant layers:

```bash
cargo fmt --all -- --check
cargo check -p nana-ui --lib --no-default-features --locked
cargo check -p component-gallery --bin component-gallery --locked
cargo test --workspace --all-targets --all-features --locked
cargo check --workspace --all-targets --all-features --locked
(cd crates/nana-js-engine/fixtures/vue-sfc-compat && npm ci && npm run build)
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
```

For Skill-only changes, run `quick_validate.py` on each Skill, verify links and metadata, then run
`git diff --check`; do not add or rerun unrelated functional tests.

Report exact commands and results, separating regressions from environment or untested-platform
limits.
