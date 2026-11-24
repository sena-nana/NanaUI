# nana-ui

English · [简体中文](README.zh-CN.md)

NanaUI's facade crate and the single crate applications depend on directly.

It provides application entry points (`NanaApplication`, `RuntimeApplication`, `RuntimeProgram`), the desktop host built on `winit` and `nana-window`, and `SceneWgpuPainter` for painting a `UiScene` into host WGPU frames. Controls, declarative views, and `AppContext` are re-exported through `nana_ui::runtime`, and `nana-gpu` contract types such as `GpuContext` and `FrameContext` are exposed here as well. Host textures and `CustomRenderNode` let GPU content enter the UI tree as ordinary nodes.

The retained tree, components, and layout live in [`nana-ui-runtime`](../nana-ui-runtime), and scene extraction lives in [`nana-ui-scene`](../nana-ui-scene); this crate wires them to native windows and the GPU.

## Example

```rust
fn main() -> Result<(), nana_ui::HostedRunError> {
    let identity = ApplicationIdentity::new(
        "dev.nanaui.counter",
        "NanaUI Counter",
        env!("CARGO_PKG_VERSION"),
    );
    NanaApplication::builder(identity)
        .diagnostics(DiagnosticsConfig::default())
        .run::<RuntimeApplication<Counter>>(
            WindowDescriptor::new("NanaUI Counter").initial_size(480.0, 320.0),
        )
}
```

`Counter` implements `ApplicationState`; the full example is in `examples/application-counter.rs`:

```bash
cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts
```

## Features

No features are enabled by default (`default = []`). Desktop applications typically enable `hosted` (plus `bundled-fonts`, `view-macro`, or any needed optional capabilities), or use `full` for the convenience desktop bundle.

- `hosted`: desktop host including windows, surfaces, frame scheduling, and platform accessibility, implying `gpu`.
- `gpu`: `SceneWgpuPainter`, host textures, frame exchange, and windowless output. Uses the platform's primary backend (`metal` on Apple, `dx12` on Windows, `vulkan` on Unix) unless combined with `wgpu-backends` or custom `wgpu` backend features.
- `wgpu-backends`: enables the standard cross-platform WGPU backends (`dx12`, `vulkan`, `gles`, `metal`) when `gpu` is enabled.
- `icu`: opt-in ICU4X locale formatting for numbers, dates, and plurals (`PlainFormatter` when disabled).
- `snippet-transforms`: opt-in regular-expression transforms in snippets (`${1/(.*)/${1:/upcase}/}`). When disabled, snippets still expand and link placeholders cleanly and transforms return values unchanged without linking the regex engine (saving about 0.7 MiB), emitting a one-shot `runtime.snippet.transform_unavailable` diagnostic event when invoked.
- `svg`: opt-in SVG image decoding for `url()` textures, `Image`, and SVG-backed host icons (`usvg`/`resvg`/`tiny-skia`, saving about 0.8 MiB when off; vector icons render from their own geometry without it). Effective only with `gpu`. Emits `resource.image_codec_unavailable` if an SVG is loaded while disabled.
- `http-fetch`: opt-in built-in `http(s)://` image/resource fetching (forwards `nana-ui-platform/fetch`). Without it (or a custom `FetchHost`), remote `http(s)://` requests are skipped and emit `resource.fetch_unavailable`.
- `image-codecs-extra`: opt-in JPEG, GIF, and WebP decoding in `image` (PNG stays available). Emits `resource.image_codec_unavailable` when an excluded format is decoded.
- `runtime-wgsl`: opt-in runtime WGSL compilation (`naga` / `wgpu` WGSL fallback; implied by `full`). Without it, `gpu` loads precompiled shader archives (`stages.msl.zz` on macOS 14+ / iOS 17+ and compressed Naga IR elsewhere).
- `full`: convenience desktop bundle enabling `bundled-fonts`, `components`, `hosted`, `http-fetch`, `icu`, `image-codecs-extra`, `runtime-wgsl`, `snippet-transforms`, `svg`, `syntax-highlighting`, and `wgpu-backends`.
- `bundled-fonts`: embeds and registers Noto Sans SC as the default UI font (about 4.2 MiB); without it, text falls back to system fonts.
- `components`: enables `calendar`, `charts`, `controls`, `diff-view`, `dock`, `graph-canvas`, `image-viewer`, `qr-code`, `rich-text`, and `terminal`. `dock` also carries the host adapters `nana_ui::dock` and `runtime_dock_window_update`.
- `syntax-highlighting`: syntax highlighting for code blocks.
- `view-macro`: `view!`, `css!`, and `stylesheet!` macros.
- `icons-tabler`: full Tabler icon catalog (`nana_ui::icons_tabler`).
- `packaged-resources`: `.nrpack` resource pack mounting and `NANA_PACKAGE_VALIDATE` self-check.
- `wgpu-interop`: exposes underlying WGPU objects on `GpuContext` for custom render pipelines.
- `native-export` (Windows only): exports window output as a DX12 shared texture.
- `accesskit-tree`: exports AccessKit `TreeUpdate` only, for hosts wiring their own platform adapter.
- `reactive-trace`: records reactive causality chains for debugging.
- `no-default-app-icon`: skips linking the default application icon (for final application binaries only).

## Tests

```bash
cargo test -p nana-ui --lib --features hosted,gpu -- --test-threads=1
```

Without `gpu` or `hosted`, modules gated behind those features are skipped and their tests do not run. GPU tests require an available adapter; see [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) for the full matrix.

See also [Quick Start](../../docs/en/guide/quick-start.md), [Application API](../../docs/en/reference/application-api.md), [Layout and Feature Migration](../../docs/en/guide/migration-layout-and-features.md), and [Live GPU Content](../../docs/en/reference/gpu.md).
