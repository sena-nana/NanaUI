# Phase A evidence — paint-vello (Vello + wgpu 30)

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

MVP #6: usable Vello paint path without downgrading workspace wgpu.

## Pin

```toml
# workspace Cargo.toml
[patch.crates-io]
vello = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
vello_encoding = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
vello_shaders = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
```

`linebender/vello` has no `wgpu30` branch; this is PR [#1754](https://github.com/linebender/vello/pull/1754) head (`euclio:wgpu30`).
Do **not** use crates.io `anyrender_vello` / `blitz-renderer-vello` (wgpu 29).

## Commands

```bash
cargo tree -p nana-ui-blitz --features paint-vello -i wgpu   # → wgpu v30.0.0 only

cargo run -p vue-counter --release --no-default-features \
  --features engine-quickjs,paint-vello,evidence-png -- \
  counter --clicks=1 --png=docs/performance/vue-counter-vello-quickjs.png

cargo run -p vue-counter --release --no-default-features \
  --features engine-v8,paint-vello,evidence-png -- \
  counter --clicks=1 --png=docs/performance/vue-counter-vello-v8.png

cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-quickjs,paint-vello,evidence-png -- \
  home --complete-setup --png=docs/performance/lilia-home-vello-quickjs.png
```

## Results (2026-08-06)

| Artifact | Result |
|----------|--------|
| Counter QuickJS vs V8 PNG | **Byte-identical** |
| Lilia home QuickJS / V8 PNG | Both render; box counts may differ by engine timing (acceptable) |
| wgpu graph | **30.0.0 only** |
| HostTexture / `<nana-gpu>` | Composited after Vello (`composited=1` on lilia home) |

## Gap vs blitz-renderer-vello

Phase A drew solid layout-box fills only. **Phase B** adds system-font text + Lucide
SVG strokes; see [`2026-08-06-paint-vello-phase-b.md`](2026-08-06-paint-vello-phase-b.md).
Still missing: Parley/skrifa shaping, full CSS borders/radii/shadows, images, stylo
paint tree. Optional later: fork anyrender onto wgpu 30.
