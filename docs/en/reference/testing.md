# Testing and Gates

NanaUI enforces hard limits on test target count, debug symbol size, concurrency isolation, and anti-hang patterns so local runs and CI avoid linker bloat and never freeze the host GPU, windowing system, or desktop compositor.

The gate script lives in [check-test-hygiene.py](https://github.com/sena-nana/NanaUI/blob/main/scripts/check-test-hygiene.py), and the `cargo-nextest` timeout and serialization config lives in [nextest.toml](https://github.com/sena-nana/NanaUI/blob/main/.config/nextest.toml).

## Build Profile and Target Hygiene

- **Debug info and dependency optimization (`Cargo.toml`)**:
  - `[profile.dev]` and `[profile.test]` both set `debug = "line-tables-only"`, and `[profile.dev.package."*"]` sets `debug = false`. This keeps panic backtrace line numbers without paying full DWARF bloat during linking.
  - Crypto and compression leaf crates (`chacha20`, `poly1305`, `blake3`, `ruzstd`, `zstd-sys`) keep `opt-level = 3` in `profile.dev` so pack sealing and `nana-packager` tests do not crawl in debug builds.
- **Trim testable targets**:
  - Every `[[bin]]` and `[[example]]` must set `test = false` unless explicitly listed in `ALLOWED_TEST_BINS` or `ALLOWED_TEST_EXAMPLES` in `scripts/check-test-hygiene.py`.
  - Pure example, fixture, and verification crates (`examples/component-gallery`, `examples/reactive-sfc`, `examples/runtime-host-fixture`, `tools/css-parity`, `third_party/wgpu-fork-tests`) set `doctest = false` under `[lib]`.
- **Integration harness caps and domain grouping**:
  - Integration `[[test]]` harnesses are capped at **4** per package (`MAX_TESTS_PER_PACKAGE = 4`) and **30** across the workspace (`MAX_TESTS_WORKSPACE = 30`).
  - Add new coverage to an existing domain harness (such as `contracts` / `perf_contracts` in `nana-ui-runtime`, or `packager` in `nana-packager`) instead of creating one integration test binary per issue.
- **No per-issue test filenames**:
  - Files matching `issue<digits>*.rs` are forbidden under any `tests/` directory and under `crates/nana-ui-runtime/src/world/`. Runtime layout regressions are organized by topic under `crates/nana-ui-runtime/src/world/tests/`.

## Anti-Hang, Isolation, and Scale Rules

`scripts/check-test-hygiene.py` statically scans test code under `crates/`, `examples/`, `tools/`, and `third_party/`:

1. **No unbounded GPU waits**: test code must not call `wgpu::PollType::wait_indefinitely()`, or `PollType::Wait` without `timeout: Some(...)` (typically 10 s).
2. **No multi-second sleeps on default paths**: blocking `sleep(Duration::from_secs(N))` with `N >= 2` is disallowed unless the enclosing `#[test]` is marked `#[ignore]`.
3. **Collision-free scratch directories**: test code that combines `env::temp_dir()` with `process::id()` must include a monotonic atomic sequence counter (`AtomicUsize`, `AtomicU64`, `AtomicU32`, or `unique_test_suffix(`) so parallel test threads do not collide.
4. **No nested `cargo` invocations**: test code must not spawn nested `cargo build`, `cargo test`, or `cargo run` commands.
5. **Release-only 100k stress tests**: 100k-node/scale stress tests stay marked `#[ignore]` and run explicitly with `--release -- --ignored`, keeping default test inputs bounded.

## `cargo-nextest` Groups and Timeouts

`.config/nextest.toml` configures bounded timeouts and serialization groups:

- **Timeouts**: `profile.default` sets `slow-timeout = { period = "15s", terminate-after = 2 }` and `fail-fast = false` (`profile.ci` uses `period = "20s"`, and `nana-packager` overrides `period = "30s"`).
- **`gpu-serial` (`max-threads = 1`)**: serializes `nana-gpu`, `nana-frame-exchange`, `nana-ui`, `nana-ui-devtools`, `nana-wgpu-fork-tests`, `runtime-host-fixture`, `nana-mobile-probe`, and `binary(ui-snapshots)`.
- **`os-global` (`max-threads = 1`)**: serializes `nana-window`, `nana-diagnostics`, and `nana-ui-platform` so process-global singletons, environment variables, and native window state do not race.

## Running Tests Safely Locally

- **Never run two `cargo test` commands or GPU loads at once**: keep one Cargo invocation active at a time and cap build concurrency with `-j` (for example `-j 6`).
- **Serialize threads when invoking `cargo test` directly on GPU or window suites**: pass `-- --test-threads=1`.
- **Common commands**:

```bash
# 1. Static hygiene gate: target caps, naming, and anti-hang rules
python3 scripts/check-test-hygiene.py

# 2. CPU-only crate tests
cargo test -j 6 -p nana-ui-runtime --lib --locked -- --test-threads=4

# 3. Serial GPU / native window suites
cargo test -j 6 -p nana-gpu --locked -- --test-threads=1
cargo test -j 6 -p nana-ui --locked -- --test-threads=1
cargo test -j 6 -p nana-window --locked -- --test-threads=1

# 4. Packager Steam delta gate
cargo test -j 6 -p nana-packager --test packager --locked steam_delta
```

See also [Diagnostics and Application Paths](diagnostics.md) and the workspace rules in `AGENTS.md`.
