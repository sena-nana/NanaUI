---
name: nanaui-validation
description: Choose proportionate evidence for NanaUI Runtime, Scene, UI, GPU, window, dependency, and performance changes. Use when planning, reviewing, or reporting validation.
---

# NanaUI Validation

Select evidence from the changed contract; do not run an unrelated full matrix by default.

- **Runtime/UI:** test changed layout, state transitions, persistence, interaction, focus, input, and serialization contracts.
- **Scene/GPU:** test extraction, geometry, invalidation, texture replacement, frame lifetime, and document-order composition. A compile check is not GPU evidence.
- **Visual:** render the affected real workspace or component path and inspect the produced image plus any diff report. Re-record a baseline only when the visual change is intended and reviewable.
- **Window:** test the public outcome contract and require a real target-platform window for native effects, chrome, resize, or cleanup.
- **Compatibility:** when a public boundary changes, inspect exports, feature gates, manifests, lockfiles, serialized data, and every in-repository consumer.
- **Packaging (Issue #226):** run `python3 scripts/check-package-boundary.py`, `cargo test -p nana-package -p nana-packager` (includes the Steam delta gates in `tests/steam_delta.rs`), and, for runtime or layout changes, package `examples/package-fixture` and run `nana-packager validate <app> --run --tamper-suite` as the CI `packaging` job does. A foreign-target package reports its launch checks as not executed; say so. See `docs/packaging.md`.
- **Performance:** use the repository's maintained performance contract and report measured regressions separately from unavailable platform or environment evidence.

For a Skill-only edit, validate frontmatter, repository-relative links, naming, and `git diff --check`. Report exact commands and results, including what was not exercised. Do not claim consumer, GPU, or platform acceptance from `cargo check` alone.
