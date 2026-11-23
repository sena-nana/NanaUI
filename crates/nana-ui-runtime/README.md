# nana-ui-runtime

English · [简体中文](README.zh-CN.md)

NanaUI's retained, renderer-independent UI runtime.

`UiWorld` is the single authority for the UI tree: node identity, topology, and logical state live here and expose only `StableNodeId`. Mutations are queued as `UiMutation` in a `MutationQueue`, validated in batch at the frame boundary, and committed atomically. Built on this tree are layout, input routing, focus, IME, accessibility, motion, built-in controls, and desktop components (`Shell`, `Workspace`, `Dock`, overlays). GPU content nodes (`CustomRenderNode`, `GpuView`) are declared here and participate in layout and hit testing alongside ordinary nodes.

The `view` module offers declarative ergonomics: signals directly update the bound node fields without re-rendering or tree diffing.

This crate holds no native window, surface, or GPU device, and does not parse CSS. Scene extraction lives in [`nana-ui-scene`](../nana-ui-scene), painting and hosting in [`nana-ui`](../nana-ui), and applications typically import these types via `nana_ui::runtime`.

## Example

```rust
use nana_ui_runtime::view::{IntoView, button, column, signal};
use nana_ui_runtime::{AppContext, DocumentId};

fn counter() -> impl IntoView {
    let count = signal(0u64);
    column().gap(12.0).children((
        nana_ui_runtime::text!("Count {count}"),
        button("Add").on_activate(move || count.update(|c| *c += 1)),
    ))
}

let mut cx = AppContext::new();
let document = DocumentId::new(1).unwrap();
let view = cx.mount_view_root(document, counter).unwrap();
```

## Features

No features are enabled by default (`default = []`).

- `icu` (opt-in): formats numbers, dates, and plurals with ICU4X and compiled CLDR data; when disabled, `PlainFormatter` is used.
- `calendar`, `charts`, `controls`, `diff-view`, `dock`, `graph-canvas`, `image-viewer`, `qr-code`, `rich-text`, `terminal`: optional control families; `components` enables all of them. `dock` is `Dock` / `DockPanel` / `DockWorkspace`; `Workspace` and `SplitPane` belong to the Shell and are always compiled. `rich-text` includes Markdown, Mermaid diagrams, and math formulas.
- `snippet-transforms` (opt-in): regular-expression transforms in snippets; when disabled, transforms return the input unchanged without linking `fancy-regex` and emit `runtime.snippet.transform_unavailable`.
- `syntax-highlighting`: syntax highlighting for code blocks.
- `view-macro`: `view!` template macro.
- `reactive-trace`: records causal chains for reactive updates to answer "why did this update?".

`layout-verify` and `benchmark` are for tests and benchmarks only.

## Tests

```bash
cargo test -p nana-ui-runtime
cargo test -p nana-ui-runtime --features view-macro --test contracts --test perf_contracts -- view_macro view_stack theme_layout
```

`view_macro`, `view_stack`, and `theme_layout` compile empty and report passing without `view-macro`. Benchmarks require `benchmark`:

```bash
cargo run --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark
```

Remaining commands are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Retained Runtime Tree](../../docs/en/learn/runtime-tree.md), [Declarative Views](../../docs/en/reference/reactive-view.md), [Layout and Feature Migration](../../docs/en/guide/migration-layout-and-features.md), and [Runtime and Scene Contracts](../../docs/en/reference/runtime-scene.md).
