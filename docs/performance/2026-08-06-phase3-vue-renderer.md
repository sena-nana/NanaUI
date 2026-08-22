# HISTORICAL — Phase 3 evidence — Vue Custom Renderer + blitz-dom + paint-stub

> 状态注记：历史证据。QuickJS 已移除；产品引擎为 V8。

Date: 2026-08-06

This is a 2026-08-06 evidence log for the removed Blitz / paint-stub path. It is
not current architecture. Product retained/render is now Runtime/UiScene drawn
by `SceneWgpuPainter`. Iced is a removed historical migration snapshot, not the
product view.

## Commands

```bash
# NOT RUNNABLE: crate `nana-ui-blitz` was deleted
cargo test -p nana-ui-blitz --features layout --lib
cargo test -p nana-js-quickjs --lib
cargo run -p vue-counter -- counter --clicks=2
cargo run -p vue-counter -- todo
cargo run -p vue-counter --features evidence-png -- counter --clicks=1 \
  --png=docs/performance/vue-counter-quickjs.png
# NOT RUNNABLE: feature `paint-stub` was deleted
cargo run -p vue-counter --no-default-features --features engine-v8,paint-stub,evidence-png -- \
  counter --clicks=1 --png=docs/performance/vue-counter-v8.png
# NOT RUNNABLE: feature `paint-stub` was deleted
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
