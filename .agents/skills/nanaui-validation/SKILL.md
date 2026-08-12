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

Report exact commands and results, separating regressions from environment or untested-platform
limits.
