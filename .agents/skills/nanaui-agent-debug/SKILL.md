---
name: nanaui-agent-debug
description: Headless NanaUI Agent session for visual verification and UI operation without a window. Use when debugging a product Vue/JS or Runtime app, taking offscreen screenshots, dumping a11y/scene trees, clicking widgets, diagnosing a missed click or an invisible element, or verifying layout boxes. Do not put CPU readback on product Surface present.
---

# NanaUI Agent headless debug

Use `nana-ui-devtools` only. Product `run_runtime` present must not CPU-read back.
`nana-ui` must not depend on `nana-ui-devtools`.

## Entry points

| Target | How |
|---|---|
| Vue/JS artifact | `nana-agent-session --js <app.js> --stdio` (feature `agent-bin`) |
| Framework fixture | `nana-runtime-agent --fixture <name> --stdio` (feature `runtime-agent`, no V8) |
| Your own Rust app | `cli::runtime_main(build_my_document(), args)` from your crate's own dev binary |
| In-process | `RuntimeAgentSession::new(document, w, h)` / `VueAgentSession::new(engine, artifact, w, h)` |

No CLI can load an arbitrary Rust application from a path — the document is
built by a closure over your own component types. Use `runtime_main`; it gives
your binary exactly the flags below.

```bash
cargo run -p nana-ui-devtools --features runtime-agent --bin nana-runtime-agent -- --list-fixtures
```

```bash
printf '%s\n' '{"id":1,"cmd":"a11y","interactive_only":true}' '{"id":2,"cmd":"screenshot","path":"target/agent/before.png"}' '{"id":3,"cmd":"click","agent_path":"increment"}' '{"id":4,"cmd":"diff","baseline":"target/agent/before.png"}' | cargo run -q -p nana-ui-devtools --features runtime-agent --bin nana-runtime-agent -- --fixture counter --theme dark --stdio
```

Flags: `--width --height --scale --theme light|dark --out-dir --screenshot [name]
--a11y --stdio --gpu-probe --help`, plus `--js` (Vue) or `--fixture` /
`--list-fixtures` (Runtime).

## Commands

JSON lines in, JSON lines out. An `id` is echoed back with the command name, so
key replies by `id`, never by position. A malformed line is answered and the
session continues.

**Observe** — `a11y` (`role` `label` `agent_id_prefix` `root` `depth`
`interactive_only`), `semantic` (Vue only), `probe`, `hit_test` (`x` `y`),
`diagnostics`, `screenshot` (`path`), `diff` (`baseline` `candidate`), `gpu`, `info`.

**Act** — `click` (+`button:2` for secondary), `hover`, `scroll` (`x` `y` `dx` `dy`),
`key` (`key` `code` `alt` `ctrl` `meta` `shift`), `type` (`text`), `set_value` (`value`).

**Environment** — `viewport` (`width` `height` `scale`), `theme` (`mode`),
`clear` (`color`, `null` = follow the theme), `pump`.

## Addressing a node

Every acting command takes the same selector, inline on the command:

```jsonc
{"agent_id": "increment"}                 // Vue data-agent-id
{"agent_path": "list/row-0"}              // Rust L3 build/mount assembly keys
{"node": 42}                              // StableNodeId
{"role": "button", "label": "Save"}       // add "nth": 0 when several match
{"x": 120, "y": 48}                       // raw point
```

`agent_path` exists only for trees built with `build` / `mount`. A tree built
with `create_component` has none — use `role`/`label` there. An ambiguous
`role`/`label` fails and reports the count rather than picking one.

## Decision procedure

1. `screenshot` → read `pixels.unique_colors`. `<= 8` means nothing painted:
   stop and diagnose, do not report a pass.
2. `a11y` **with a filter**. An unfiltered dump of a real application costs more
   context than it returns. Content boxes must be `> 8px`.
3. A click did nothing → `hit_test` at the same point. If your node is not first
   in `hits`, read what is.
4. Something is missing from the PNG → `probe` it. `verdict` names the cause:
   `not_painted` / `zero_size` / `off_viewport` / `zero_opacity` / `occluded`,
   with `clips`, `effective_opacity` and `occluded_by` behind it.
5. Blank frame that still reports `ok` → `diagnostics` first. A Vue error is the
   usual cause.
6. Proving a change is visual and not merely semantic → `diff` two screenshots.

## Caveats

- `unique_colors` is necessary, not sufficient: a UI painting entirely the wrong
  thing also has many colours. It kills the flat-clear failure mode; it does not
  replace opening the PNG.
- `occluded_by` is hit-test order, not painted alpha. A `pointer-events: none`
  overlay covers a node visually without appearing there.
- `theme` sets the host theme. A Vue app that manages `documentElement.dataset.theme`
  itself can overwrite it on the next render.
- Screenshots follow the document's theme background and sample host textures
  and GPU-node renderers, so `GpuView`, Avatar and Thumbnail paint real content.
- No adapter → pixel commands fail and the GPU-backed tests skip visibly through
  `offscreen::pixels_available`. Check with `--gpu-probe` or `{"cmd":"gpu"}`.

## Feature tiers

`runtime-agent` (Vue-free, V8-free) → `agent` (adds Vue) → `agent-bin` (adds V8).
Use the smallest tier that does the job.

```bash
cargo test -p nana-ui-devtools --features runtime-agent --all-targets --locked
```
