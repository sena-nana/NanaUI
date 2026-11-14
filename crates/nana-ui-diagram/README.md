# nana-ui-diagram

English · [简体中文](README.zh-CN.md)

NanaUI's native Mermaid engine, with no browser dependency.

It parses Mermaid source into a typed model, then lays it out into a `DiagramScene`: a vector display list of shapes and single-line text. Colors, corner radii, and line widths in the list are still theme roles, so switching themes needs only a re-recording, not a relayout. Parsing and layout both finish within a `Limits` budget and return `DiagramError::TooLarge` when it is exceeded; statements skipped during parsing are recorded as `Warning` and do not fail the whole diagram.

All 34 diagram types listed in `DiagramKind::ALL` are implemented natively, including flowchart, sequence, class, state, er, gantt, pie, mindmap, timeline, gitgraph, sankey, xychart, and more.

This crate does not touch fonts, the GPU, or the UI tree; text widths are supplied by the caller through `TextMeasure`. Recognizing `mermaid` fences in Markdown and recording against the theme happens in the `rich-text` feature of [`nana-ui-runtime`](../nana-ui-runtime); applications generally do not call this crate directly.

## Example

```rust
use nana_ui_diagram::{DiagramMetrics, FixedMeasure, Limits, render};

let scene = render(
    "flowchart LR\n  A[Start] --> B[End]\n",
    &FixedMeasure,
    &DiagramMetrics::DEFAULT,
    &Limits::DEFAULT,
)
.unwrap();
```

## Tests

```bash
cargo test -p nana-ui-diagram
NANA_DIAGRAM_BLESS=1 cargo test -p nana-ui-diagram --test diagram golden
```

The second command rewrites `tests/golden/` after an intentional change. The parse and layout benchmarks need the `benchmark` feature:

```bash
cargo run --release -p nana-ui-diagram --features benchmark --bin nana-diagram-benchmark
```

See also [Native diagram engine](../../docs/en/reference/diagram-engine.md) and [NativeMarkdown component](../../docs/en/components/native-markdown.md).
