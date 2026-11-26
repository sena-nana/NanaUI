# nana-text

English · [简体中文](README.zh-CN.md)

NanaUI's text engine, and the only place in the framework that measures and lays out text.

It handles font registration, matching, and missing-glyph fallback, shapes with OpenType, handles bidirectional text, line breaking, ellipsis, and vertical text, and finally produces an immutable `TextLayout`. Text nodes in the UI tree retain their layout through `TextLayoutStore`, and staleness is decided by comparing just two integers, `TextRevision` and `FontGeneration`. Caret, selection, hit testing, and IME composition are all built on `TextLayout`.

The enums used for layout (such as `LineBreakSpec` and `WritingModeSpec`) come from [`nana-ui-core`](../nana-ui-core). Atlas upload and painting are in [`nana-ui`](../nana-ui); this crate provides only glyph placements and optional CPU rasterization.

## Example

```rust
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    NativeTextEngine, TextConstraints, TextEngine, TextKind, TextSource, TextStyle,
    TextWorkCounters,
};
use nana_ui_core::TextWrapBreak;

let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
fonts
    .register_file(
        "crates/nana-ui-core/assets/fonts/NotoSansSC-Variable.ttf",
        &FaceDescriptor::default(),
    )
    .unwrap();
let mut engine = NativeTextEngine::new(fonts);

let layout = engine.layout(
    TextKind::Paragraph,
    &TextSource::new("Text measured and laid out by nana-text"),
    &TextStyle { font_size_px: 16.0, ..TextStyle::default() },
    &TextConstraints {
        max_width_px: Some(120.0),
        wrap: Some(TextWrapBreak::Word),
        ..TextConstraints::default()
    },
    &mut TextWorkCounters::default(),
);
```

## Features

- `raster`: rasterizes glyphs into bitmaps on the CPU (`nana_text::raster`). `nana-ui`'s `gpu` feature turns it on, and hosts that composite text themselves can use it on its own.
- `svg`: turns on `raster` and adds OpenType `SVG ` table glyph rasterization via `nana-svg-raster` (not in default features; `nana-ui`'s `svg` feature enables it).

## Tests

```bash
cargo test -p nana-text --features raster
```

The tests use only fonts checked into the repository. Other commands are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Text engine (learn)](../../docs/en/learn/text-engine.md) and [Text engine (reference)](../../docs/en/reference/text-engine.md).
