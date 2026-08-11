# Phase 3 evidence — Vue Custom Renderer + blitz-dom + paint-stub

Date: 2026-08-06

## Commands

```bash
cargo test -p nana-ui-blitz --features layout --lib
cargo test -p nana-js-quickjs --lib
cargo run -p vue-counter -- counter --clicks=2
cargo run -p vue-counter -- todo
cargo run -p vue-counter --features evidence-png -- counter --clicks=1 \
  --png=docs/performance/vue-counter-quickjs.png
cargo run -p vue-counter --no-default-features --features engine-v8,paint-stub,evidence-png -- \
  counter --clicks=1 --png=docs/performance/vue-counter-v8.png
cargo run -p vue-counter --no-default-features --features engine-v8,paint-stub -- todo
cargo check -p vue-counter --features windowed
```

## Results

| Engine | App | clicks | texts | PNG |
|--------|-----|-------:|-------|-----|
| QuickJS | counter | 2 | `["2","inc"]` | `vue-counter-quickjs.png` (11 KiB) |
| QuickJS | todo | 0 | Todo / one / two … | — |
| V8 150.4.0 | counter | 1 | `["1","inc"]` | `vue-counter-v8.png` (11 KiB) |
| V8 150.4.0 | todo | 0 | same structure as QuickJS | — |

Phase 2 probe tests (`nana-js-quickjs` / `nana-js-v8`) still pass.

Architecture notes and remaining gaps: [`docs/vue-backend-deps.md`](../vue-backend-deps.md).
