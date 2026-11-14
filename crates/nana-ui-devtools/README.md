# nana-ui-devtools

English · [简体中文](README.zh-CN.md)

Headless sessions and offscreen snapshots for NanaUI, for developer tools and tests.

It can run a Rust view document without opening a window, inject pointer, keyboard, and scroll input, export the accessibility tree, and take screenshots and compare pixels. Input goes through the Runtime's canonical input routing with no bypass. Sessions are exposed over a JSON-lines protocol, and the `nana-runtime-agent` binary can drive the built-in fixtures. To drive its own document, an application calls `agent::cli::runtime_main` in its dev binary or implements `AgentSession` for its own session.

Screenshots need CPU readback; that path exists only in this crate's `offscreen` module, and product presentation never goes through it. The hot-reload dev entry point is in [`nana-ui-dev`](../nana-ui-dev).

## Features

With no features on, this crate is empty. Each feature includes the one before it:

- `offscreen`: offscreen painting and CPU readback, reading and writing PNG.
- `runtime-agent`: adds the JSON protocol, `RuntimeAgentSession`, and the command line on top of `offscreen`.
- `headless-host`: adds `HeadlessHost` on top of `runtime-agent`, running a whole `RuntimeProgram` in the native host's hook order (with deterministic `HeadlessRuntime` manual clock/quiescence, target resolution, hit testing, scene probe, view inspection, unpremultiplied PNG export, and pixel diff).
- `reactive-trace`: view inspection results include the reason for each node's most recent change.

## Example

```bash
cargo run -p nana-ui-devtools --features runtime-agent --bin nana-runtime-agent -- \
  --fixture counter --a11y
```

## Tests

```bash
cargo test -p nana-ui-devtools --features runtime-agent
```

Without a working GPU adapter, pixel tests return early. Other checks are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Development hot reload contract](../../docs/en/reference/hot-reload.md) and [Evidence checklist](../../docs/en/learn/evidence-checklist.md).
