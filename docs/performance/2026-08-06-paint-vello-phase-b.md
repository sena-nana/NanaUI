# Phase B evidence — paint-vello (text + SVG + settle)

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

MVP #6 follow-on: readable UI on Vello + wgpu 30 without crates.io `anyrender_vello`.

## Scope

| Item | Status |
|------|--------|
| Latin + CJK text | **Done** — reuse `text_raster` (fontdb / ab_glyph) → Vello solid quads |
| Lucide / SVG paths | **Done** — kurbo `BezPath` + `Scene::stroke` (curve-aware; better than stub dots) |
| Soft radii | **Partial** — cheap heuristic for `button` / `input` / `textarea` / `select` only |
| CSS border / radius / shadow | **Deferred** — no stylo paint tree; document only |
| Host Device/Queue | Unchanged — no second Device; `Rgba8Unorm` STORAGE target |
| crates.io anyrender_vello | **Still forbidden** (wgpu 29) |

## Pin (unchanged from Phase A)

```toml
[patch.crates-io]
vello = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
vello_encoding = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
vello_shaders = { git = "https://github.com/euclio/vello", branch = "wgpu30" }
```

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

cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,paint-vello,evidence-png -- \
  home --complete-setup --png=docs/performance/lilia-home-vello-v8.png
```

## Results (2026-08-06 Phase B)

| Artifact | Result |
|----------|--------|
| Counter QuickJS vs V8 PNG | **Byte-identical** (`boxes=5`, texts `1` / `inc`) |
| Lilia home QuickJS vs V8 PNG | **Byte-identical** (`boxes=124`, CJK texts present, `composited=1`) |
| wgpu graph | **30.0.0 only** |
| HostTexture / `<nana-gpu>` | Composited after Vello (`composited=1`) |

### Engine settle (box-count timing fix)

Evidence hosts now call `settle_layout_stable` (pump until box count unchanged for 3
frames) before snapshot / PNG. This closes the Phase A “box counts may differ by
engine timing” gap for `vue-counter` and `nana-tauri-demo` under `paint-vello`.

## vs paint-stub (visual)

Same viewport / theme / settle; pixel mismatch is expected and small:

| Scene | Stub PNG | Vello PNG | Pixel diff |
|-------|----------|-----------|------------|
| Counter 800×600 | `vue-counter-stub-vs-vello.png` | `vue-counter-vello-quickjs.png` | **0.02%** |
| Lilia home 960×640 | `lilia-home-stub-vs-vello.png` | `lilia-home-vello-quickjs.png` | **0.96%** |

Primary drivers of residual diff:

1. SVG: stub samples polylines as square dots; vello strokes real Bézier paths with AA.
2. Soft corner radius on form chrome tags (vello only).
3. Vello area AA vs stub hard-edged triangles for text ink quads.

Text content and layout boxes match; both paths show Latin + CJK.

## Gaps remaining (not Phase B)

- Full CSS borders / radii / shadows / images (stylo paint / anyrender fork on wgpu 30)
- Parley / skrifa shaped text (still ab_glyph system-font raster)
- Pixel-perfect Lucide vs browser SVG

Optional later: fork anyrender onto wgpu 30; keep direct Vello as the default
`paint-vello` path until that lands.

Final Issue #5 acceptance commands:
[`2026-08-06-issue5-final-acceptance.md`](2026-08-06-issue5-final-acceptance.md).
