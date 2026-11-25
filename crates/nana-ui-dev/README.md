# nana-ui-dev

English · [简体中文](README.zh-CN.md)

Development-time hot reload for NanaUI applications, for development builds only.

It watches the sources and compiles incrementally in the background. On success it hands the window geometry and the state the application gave it to the next process, then re-execs in place. It does no in-process dylib hot swapping. With `.vue` views, a save that only changes static text or static `<style>` properties is sent straight into the running window without a rebuild. Which state survives a reload is up to the application; this crate only delivers `DevSignal` messages and handoff data.

Builds without `debug_assertions` are rejected with `compile_error!`, so the release entry point should be a separate `main.rs` that only calls `run_runtime` and does not reference this crate. The headless session itself is in [`nana-ui-devtools`](../nana-ui-devtools).

## Features

- `templates`: pulls in `nana-ui-sfc` and provides `watch_templates`, `watch_template_events`, `TemplateText`, and `TemplateStyle`, for hot swapping static text and static `<style>` patches in `.vue`.
- `headless`: pulls in `nana-ui-devtools` so the dev binary responds to `--stdio`, `--screenshot`, and `--a11y`, letting you verify a rebuild without opening a window.

## Example

The complete dev entry point is in `examples/l3-dev-entry.rs`:

```bash
cargo run -p nana-ui-dev --example l3-dev-entry
cargo run -p nana-ui-dev --example l3-dev-entry --features headless -- --a11y
```

## Tests

```bash
cargo test -p nana-ui-dev --features templates
```

This crate compiles only in the debug profile; do not add `--release`. Other checks are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Hot reload](../../docs/en/guide/scaling/hot-reload.md) and [Development hot reload contract](../../docs/en/reference/hot-reload.md).
