# nana-ui-scene

English · [简体中文](README.zh-CN.md)

Turns the runtime UI tree into a drawable scene with no WGPU dependency.

`RuntimeDocument` owns one document's `AppContext` and `UiScene`. Calling `flush` per frame runs style resolution, text layout, box layout, and scene extraction in order, then applies the delta to `UiScene`. The scene retains primitives per node and slot, rebuilds only dirty subtrees, and takes a translation fast path on pure scrolls. Compositor layers, opacity groups, and render graphs are also decided here; all outputs are pure data.

Devices, textures, and pipelines live in `SceneWgpuPainter` inside [`nana-ui`](../nana-ui), while layout and hit testing live in [`nana-ui-runtime`](../nana-ui-runtime).

## Example

```rust
use nana_ui_runtime::{DocumentId, LayoutViewport, MeasureTextShaper, Stack};
use nana_ui_scene::RuntimeDocument;

let id = DocumentId::new(1).unwrap();
let mut runtime = RuntimeDocument::new(id);
runtime
    .context_mut()
    .create_component(id, Stack::column(0.0))
    .unwrap();

runtime
    .flush(LayoutViewport::new(320.0, 240.0), &mut MeasureTextShaper)
    .unwrap();
let scene = runtime.scene();
```

`MeasureTextShaper` performs font-free approximate measurement; use `nana_ui_runtime::NanaTextEngineShaper` for real font shaping.

## Features

No features are enabled by default (`default = []`).

- `icu`, `calendar`, `charts`, `controls`, `graph-canvas`, `image-viewer`, `rich-text`, and `components`: opt-in features forwarded to `nana-ui-runtime`.
- `benchmark`: builds scene benchmarks only.

## Tests

```bash
cargo test -p nana-ui-scene
cargo run --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
```

Some integration tests require matching component family features; without them they compile empty and pass. Remaining commands are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Scene Extraction](../../docs/en/learn/scene-extraction.md), [Layout and Feature Migration](../../docs/en/guide/migration-layout-and-features.md), and [Runtime and Scene Contracts](../../docs/en/reference/runtime-scene.md).
