# Layout Model, Feature Defaults, and Runtime Migration Guide

This guide summarizes the changes and migration steps across the core layout model, Cargo default features, build-time CSS migration warnings, runtime optional-feature degradation diagnostics, and GPU / platform boundaries.

## 1. Pure `Flex + Grid + Inline + Layers` (and `Overlay`) Layout Model

The core layout engine has removed legacy `Block` / `FlowRoot` / `InlineBlock` / `Contents` modes, `float` / `clear`, and experimental `baseline-source`:

| Legacy form | Migration target | Notes |
| --- | --- | --- |
| `display: block` / `DisplaySpec::Block` / `Block` control | Omit `display` or write `display: flex; flex-direction: column` (in Rust use `Stack::column(...)`) | Containers without an explicit `display` that are not horizontal arrange along the block axis and stretch normal children; explicit `flex` / `grid` / `layers` strictly follow their own `align-items` |
| `display: flow-root` | `display: flex; flex-direction: column` (or `Stack::column(...)`) | Independent `FlowRoot` formatting contexts no longer exist |
| `display: inline-block` / `DisplaySpec::InlineBlock` | `display: inline-flex` or `display: inline-grid` | Atomic container participating in inline formatting contexts (IFC); supports baseline alignment and shrink-wrapping |
| `display: contents` / `DisplaySpec::Contents` | Remove the extra wrapper node or move layout styles onto its children | The retained tree no longer preserves boxless pass-through wrappers |
| `float` / `clear` | `display: flex` or `display: grid` | Floats were removed from the core layout model; declarations are ignored |
| `baseline-source` | Standard first-baseline alignment | Experimental property removed |

### Atomic Baselines and `subgrid` Alignment

- **`inline-flex` / `inline-grid` atomic baselines**: Follows CSS Flexbox §8.5 and CSS Grid §11.8: exports the first baseline from inline/flex/grid children; non-`visible` clipping (such as `overflow: hidden`) no longer suppresses `inline-flex` / `inline-grid` baselines; when no line box exists among children, a baseline is synthesized from the first child's margin/border bottom edge (falling back to the container's bottom margin edge when empty).
- **Standalone `subgrid`**: `grid-template-columns: subgrid [line-name...]` and `grid-template-rows: subgrid [line-name...]` inherit resolved tracks and gutters from the parent grid span while accounting for the subgrid's own `border` / `padding` / `margin` on edge tracks and compensating for custom `gap` overrides.

### Build-Time CSS Migration Warnings

When removed syntax appears in `.vue`, `view!`, `css!`, or `stylesheet!`, `nana-ui-view-codegen` emits build-time diagnostics and ignores the removed declaration:

- `legacy block display (\`block\` / \`flow-root\`) was removed; use \`display: flex; flex-direction: column\` (or \`grid\` / \`layers\`) instead`
- `\`inline-block\` was removed; use \`display: inline-flex\` or \`display: inline-grid\` instead`
- `\`display: contents\` is not supported in NanaUI's retained Style Model; remove the wrapper or move layout styles onto its children`
- `CSS floats (\`float\` / \`clear\`) were removed; use \`display: flex\` or \`display: grid\` instead`
- `mixing \`subgrid\` with explicit tracks is not supported in ...; use standalone \`subgrid\` (with optional \`[line-name]\` groups)`

See [Layout (CSS Subset)](../reference/layout.md) and [Layout and Styling (Rust)](../reference/rust-layout.md) for full details.

## 2. Opt-In Cargo Feature Defaults

To minimize baseline binary size and avoid pulling optional cross-crate dependencies implicitly, `nana-ui`, `nana-ui-runtime`, and `nana-ui-scene` use `default = []`, and `nana-ui-platform` uses `default = ["clipboard"]`. Applications opt in explicitly by role:

| Feature | Crate | Default | Enabled capability | Behavior & diagnostics when off |
| --- | --- | --- | --- | --- |
| `hosted` / `gpu` | `nana-ui` | Off | Desktop host / `SceneWgpuPainter` with only the platform's primary backend (`metal` / `dx12` / `vulkan`) | No window host or GPU painter; non-Apple targets read compressed Naga IR by default, and macOS 14+ / iOS 17+ read precompiled `stages.msl.zz` by default |
| `wgpu-backends` | `nana-ui` | Off (in `full`) | Enables all standard cross-platform WGPU backends (`dx12` / `vulkan` / `gles` / `metal`) | Retains primary platform backend only |
| `runtime-wgsl` | `nana-ui` | Off (in `full`) | Runtime WGSL fallback compilation (`naga` / `wgpu` WGSL) | Uses precompiled pipelines only; missing/rejected built-in stages return an explicit build error and emit `gpu.shader.build_failed`, while custom WGSL `MaterialProgram`s are rejected with `gpu.material.rejected` |
| `icu` | `nana-ui` / `runtime` / `scene` | Off (in `full`) | ICU4X localized number, date, and plural formatting | Falls back to `PlainFormatter` and emits `resource.icu_unavailable` on first localized formatting use |
| `snippet-transforms` | `nana-ui` / `runtime` | Off (in `full`) | Snippet regex replacement transforms (`fancy-regex`) | Placeholder linking continues normally, transforms return text unchanged, and a one-shot `runtime.snippet.transform_unavailable` event is emitted |
| `http-fetch` | `nana-ui` (forwards `platform/fetch`) | Off (in `full`) | Built-in `ureq`-based `http(s)://` asset and image fetching | Without a custom `FetchHost`, remote requests are skipped and emit `resource.fetch_unavailable` |
| `image-codecs-extra` | `nana-ui` | Off (in `full`) | Enables JPEG / GIF / WebP decoding in `image` | Retains core formats such as PNG; unsupported formats emit `resource.image_codec_unavailable` |
| `svg` | `nana-ui` | Off (in `full`) | CPU-side SVG image rasterization (`nana-svg-raster`) | SVG images are treated as undecodable and emit `resource.image_codec_unavailable`; built-in vector icons do not depend on it |
| `fetch` | `nana-ui-platform` | Off (`default = ["clipboard"]`) | `NativeFetchHost` | Compiles only the `FetchHost` trait contract |
| `full` | `nana-ui` | Off | Convenience desktop bundle (includes the features above plus `bundled-fonts`, `components`, `syntax-highlighting`, `wgpu-backends`, etc.) | Suitable for out-of-the-box desktop apps |

To keep the previous full-featured desktop configuration in an application's `Cargo.toml`:

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["full", "view-macro"] }
```

Minimal desktop applications can enable only `hosted` (plus `wgpu-backends`, `bundled-fonts`, `view-macro`, etc. as needed). See [Application API — Cargo features](../reference/application-api.md#cargo-features).

## 3. Work Observation, Shaders, and Cross-Platform Color Sampling

- **Work counters and diagnostics**: `WorkCounters` (with `ThemeWorkCounters`, `I18nCounters`, `DynamicLayoutCounters`, `GpuWorkObservation`, `OutputWorkObservation`), `HeadlessTurn.work`, and `HeadlessRuntime::present(...) -> Option<WorkCounters>` preserve full structural work accounting without adding per-node bytes to `NodeRecord`, `Hierarchy`, `PublishedLayoutMeta`, `PassInput`, or `LayoutStyle`. `nana-diagnostics` emits append-only structural counters `runtime.style.nodes_resolved`, `runtime.hit_test.nodes_rebuilt`, `runtime.extract.nodes_extracted`, `runtime.extract.nodes_changed`, `layout.measure_nodes`, and `layout.placement_nodes` (see [Diagnostics and Application Paths](../reference/diagnostics.md)).
- **Precompiled shaders and pipeline fallback**: Apple targets set a deployment floor of macOS 14.0+ / iOS 17.0+ (`MSL 3.1`); painter construction uses lazy `ArchiveScope` + `ArchiveLease` so concurrent worker threads share one `stages.msl.zz` decompression and return resident uncompressed bytes to `0 B` when the scope exits. Pipeline builders expose fallible `try_new*` APIs that emit `gpu.shader.precompiled_fallback` or `gpu.shader.build_failed` (and `gpu.material.rejected` for rejected custom materials when `runtime-wgsl` is off). See [Live GPU Content](../reference/gpu.md).
- **Screen color sampling (`ColorField` / `ColorSampler`)**: Host and window backends unify cancellation and lifetime safety across platforms (native `NSColorSampler` on macOS; strict XDG Desktop Portal `Screenshot.PickColor` v2+ detection without X11 fallback under Wayland sessions and X11 visual-mask root sampling with colormap fallback on Linux; deferred fallback screen capture when `WDA_EXCLUDEFROMCAPTURE` is unavailable plus per-monitor dynamic DPI loupe scaling on Windows). Hosts that cannot sample colors call `set_eyedropper_supported(false)` to hide the eyedropper button and record failures via `window.color_sampler.failed`.

## 4. Removed Public APIs

While tightening framework boundaries, the following historical APIs were removed or replaced:

| Removed API | Replacement |
| --- | --- |
| `BuiltinComponents` / `NanaBuiltinComponents` / `NanaBuiltinIdentities` / `ApplicationState::BUILTINS` | Removed; built-in components are registered on demand in `ComponentRegistry` whenever referenced by `view!`, `.vue`, or `register_component::<C>()` |
| `SemanticSpec` / `SemanticOption` / `PreparedSemanticBinding` / `prepare_semantic_binding` / `ComponentBindKind` | Removed; implement and register custom components with `ComponentView` and `RegisterableComponent` (`app.register_component::<C>()` / `world.register_component::<C>()`) |
| `command_palette_items_from_json` | Parse JSON in consumer code (e.g., via `serde_json`) into `Vec<CommandPaletteItem>` and pass directly to `CommandPalette` |
| String tag-name built-in control construction | Use typed Rust builders (`Stack`, `Button`, `Widget`, etc.), `view!`, or `.vue` components |

### Headless API Changes

- `HeadlessTurn.flushes` was removed in favor of `HeadlessTurn.work: WorkCounters`.
- `HeadlessRuntime::present()` now returns `Option<WorkCounters>` instead of `bool`.
- Enabling `gpu` no longer pulls in `nana-ui-platform/fetch`, `naga`, or `GLES` implicitly.
