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
- **Packaging (Issue #226):** run `python3 scripts/check-package-boundary.py`, `cargo test -p nana-package -p nana-packager` (includes the Steam delta gates under `crates/nana-packager/tests/packager/steam_delta.rs`, runnable directly via `cargo test -p nana-packager --test packager steam_delta`), and, for runtime or layout changes, package `examples/package-fixture` and run `nana-packager validate <app> --run --tamper-suite` as the CI `packaging` job does. A foreign-target package reports its launch checks as not executed; say so. See `docs/reference/packaging.md`.
- **Performance:** use the repository's maintained performance contract and report measured regressions separately from unavailable platform or environment evidence.
- **Test hygiene:** verify target count caps, `test = false` / `doctest = false` hygiene, and anti-hang rules with `python3 scripts/check-test-hygiene.py` whenever adding or reorganizing tests. See `docs/reference/testing.md`.

## GPU load safety

An unthrottled probe once queued seconds of work and froze the whole desktop (the compositor waits behind the same GPU). Any probe, benchmark or test that loads the GPU must:

- bound its submissions in flight to 1–2 (wait on its own oldest submission, or submit through `GpuContext::begin_producer_frame`, which the producer budget bounds);
- stop at a wall-clock cap (about 20 s per run) and clamp its load arguments to the highest level verified safe (`surface-resize-probe --load` ≤ 16);
- default to a light load, and step it up one level at a time only while the desktop stays responsive;
- never run concurrently with another GPU load, including a second copy of a GPU test suite.

## Test concurrency

- Run at most one copy of a GPU-touching suite at a time, cap `cargo -j` to what the machine can share, and pass `--test-threads=<n>` to GPU-heavy suites when other sessions are building.
- Repeat a suite to check for flakes serially, never as parallel copies.
- Before reporting, confirm no test, probe or benchmark process you started is still running.

For a Skill-only edit, validate frontmatter, repository-relative links, naming, and `git diff --check`. Report exact commands and results, including what was not exercised. Do not claim consumer, GPU, or platform acceptance from `cargo check` alone.
