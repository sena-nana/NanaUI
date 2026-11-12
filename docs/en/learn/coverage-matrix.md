---
title: The 24-crate coverage matrix
---

# 24-crate coverage matrix: which chapter covers each crate, and what to look up

This page is the **complete reference ledger** for the [Architecture learning](/learn/) section: for all **24** crates under the workspace's `crates/`, each crate gets a four-part description — **responsibility boundary**, **dependency direction**, **key entry types (with `file:line` anchors)**, and **primary chapter** — plus a read-only verification command you can rerun.

```powershell
# Count the directories under crates/ (should print 24)
(Get-ChildItem crates -Directory).Count
```

## 1. Mainline crates (4 on the product path + 1 text engine)

| Crate | Role in one line | Primary chapter | Dependency direction | Key entry points and source anchors | Read-only verification command |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-core`** | Framework-wide shared vocabulary: Style Model, geometry, themes, Motion IR, WorkspaceModel | [Core contracts](/learn/core-contracts) | In: `nana-diagnostics`; Out: nearly every upper-layer crate | `LayoutStyle` (`box_layout.rs:3292`), `SemanticColorRole` (`style_model.rs:257`) | `cargo tree -p nana-ui-core --depth 1` |
| **`nana-ui-runtime`** | The single authoritative retained tree `UiWorld`: mutation, layout, input routing, focus, IME, accessibility, Shell/Dock/AreaLayout | [Retained tree](/learn/runtime-tree) · [Layout engine](/learn/runtime-layout) · [Input path](/learn/runtime-input) · [Workspace](/learn/workspace-shell-dock) | In: `view-schema`, `view-macros`, `core`, `text`, `charts`, `input`, `diagnostics`; Out: `nana-ui`, `scene`, `devtools` | `UiWorld` (`world.rs:628`), `UiMutation` (`mutation.rs:15`), `AppContext` (`framework.rs:1072`) | `rg "pub struct UiWorld" crates/nana-ui-runtime/src/world.rs` |
| **`nana-ui-scene`** | Backend-neutral drawable scene `UiScene` and frame program `FramePlan`, with no GPU dependency | [Scene extraction](/learn/scene-extraction) | In: `runtime`, `core`, `charts`, `diagnostics`; **no** `gpu` / `css` dependency | `UiScene` (`scene.rs:1357`), `apply_delta` (`scene.rs:1672`), `RuntimeDocument::flush` (`runtime_document.rs:109`) | `cargo tree -p nana-ui-scene --depth 1` |
| **`nana-ui`** | Host adapter: `SceneWgpuPainter`, host frame scheduling, window services, accessibility backend, headless output | [GPU and frames](/learn/gpu-frame) · [Windows and platforms](/learn/window-platform) | In: `gpu`, `window`, `platform`, `scene`, `runtime`, `frame-exchange`, `svg-raster`, `package`, `icons` and more | `SceneWgpuPainter` (`scene_paint/mod.rs:381`), `run_runtime_scene` (`scene_host.rs:99`) | `rg "pub struct SceneWgpuPainter" crates/nana-ui/src/scene_paint/mod.rs` |
| **`nana-text`** | The product's only text engine: font matching, OpenType shaping, UAX #14 line breaking, retained layout handles | [Text engine](/learn/text-engine) | In: `nana-ui-core`, `fontdb`, `skrifa`, `harfrust`, `unicode-*`; Out: `runtime`, `nana-ui`, `scene` (benchmark) | `TextEngine` (`engine.rs:25`), `Shaper` (`shaping/shaper.rs:27`), `Layouter` (`layout/engine.rs:157`) | `cargo test -p nana-text` |

## 2. Engine and contract crates (CSS / input / GPU / frame exchange, 4)

| Crate | Role in one line | Primary chapter | Dependency direction | Key entry points and source anchors | Read-only verification command |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-css`** | The only CSS lexer/parser/selector matcher/cascade engine, producing `LayoutStyle` patches | [CSS engine](/learn/css-engine) | In: `core`, `serde_json`; Out: `view-codegen` (build time) | `parse_stylesheet` (`css_cascade.rs:658`), `rebuild_layout_style` (`:1606`) | `cargo tree -p nana-ui-css --depth 1` |
| **`nana-ui-input`** | Canonical input events, bounded endpoints, the sequencer and the `HostServices` contract | [Input path](/learn/runtime-input) | In: `core`, `diagnostics`; Out: `runtime` (the only routing consumer), `platform` (re-export) | `CanonicalInputEvent` (`canonical.rs:44`), `InputEndpoint` (`:410`), `HostServices` (`host_services.rs:97`) | `cargo tree --workspace -i nana-ui-input --edges normal` |
| **`nana-gpu`** | GPU backend contract: `GpuContext` / `FrameContext` / `GpuTexture`, generations and submission order | [GPU and frames](/learn/gpu-frame) | In: `diagnostics`; Out: `nana-ui`, `devtools`, `frame-exchange` (through `__framework`) | `GpuContext` (`context.rs:358`), `FrameContext` (`frame.rs:38`) | `rg "pub struct GpuContext" crates/nana-gpu/src/context.rs` |
| **`nana-frame-exchange`** | Cross-thread bounded latest-frame exchange that owns neither the UI device nor submission rights | [GPU and frames](/learn/gpu-frame) | In: `gpu` (through `__framework`); Out: `nana-ui` (`FrameBinding`) | `FrameExchange` (`lib.rs:493`), `FrameLease` (`:370`) | `rg "copy_texture_to_texture" crates/nana-frame-exchange/src/lib.rs` |

## 3. Window and platform crates (2)

| Crate | Role in one line | Primary chapter | Dependency direction | Key entry points and source anchors | Read-only verification command |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-platform`** | Platform-neutral contracts: window descriptors/events/commands, input re-export, `fetch`/WebSocket/clipboard/paths | [Windows and platforms](/learn/window-platform) | In: `core`, `input`, `diagnostics`, `ureq`, `url`, `arboard`; Out: each host layer | `WindowDescriptor` (`window.rs:429`), `WindowEvent` (`:114`), `WindowCommand` (`:684`) | `cargo tree -p nana-ui-platform --depth 1` |
| **`nana-window`** | Native window execution: handles, system materials, frameless chrome, scaling, fullscreen, Underlay, headless `WebSurface` | [Windows and platforms](/learn/window-platform) | In: `nana-ui-core`, `window-vibrancy`, `url`; Out: `nana-ui` | `apply_system_material` (`material.rs:176`), `prepare_client_chrome` (`chrome.rs:32`) | `rg "pub fn apply_system_material" crates/nana-window/src/material.rs` |

## 4. Authoring toolchain crates (4)

| Crate | Role in one line | Primary chapter | Dependency direction | Key entry points and source anchors | Read-only verification command |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-view-schema`** | The single description table for built-in controls, `for_each_control!` | [Authoring toolchain](/learn/toolchain) | In: zero dependencies; Out: consumed by both `runtime` and `view-codegen` | `for_each_control!` (`lib.rs:49`) | Read `crates/nana-ui-view-schema/src/lib.rs` |
| **`nana-ui-view-macros`** | `view!` / `css!` / `stylesheet!` proc-macro entry points | [Authoring toolchain](/learn/toolchain) | In: `view-codegen`; Out: `runtime` (`view-macro` feature) | `view!` (`lib.rs:16`), `css!` (`:41`), `stylesheet!` (`:76`) | `rg "^pub fn" crates/nana-ui-view-macros/src/lib.rs` |
| **`nana-ui-view-codegen`** | Expands templates into declarative view functions, and calls the CSS engine at build time to compile styles | [Authoring toolchain](/learn/toolchain) | In: `core`, `css`, `view-schema` | `expand` (`lib.rs:287`), `compile_styles` (`style.rs:986`) | `rg "use nana_ui_css" crates/nana-ui-view-codegen/src/style.rs` |
| **`nana-ui-sfc`** | `.vue` single-file component (Rust script) build-time compiler | [Authoring toolchain](/learn/toolchain) | In: `view-codegen` (generated code points at `runtime` through paths) | `Compiler` (`lib.rs:115`), `component()` (`lib.rs:413`) | `rg "pub struct Compiler" crates/nana-ui-sfc/src/lib.rs` |

## 5. Supporting crates (9)

| Crate | Role in one line | Primary chapter | Dependency direction | Key entry points and source anchors | Read-only verification command |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-charts`** | Chart model: `ChartOption`, scales, layout, hit testing and GPU mark arrays | [Supporting crates](/learn/supporting-crates) | In: only `core` + `bytemuck`; Out: `runtime`, `scene`, `nana-ui` | `ChartOption` (`option.rs:105`), `ChartMarks` (`marks.rs:203`) | Read `crates/nana-ui-charts/Cargo.toml` |
| **`nana-diagnostics`** | `event!` / `metric!` / `fault!` / `span!` macros and static descriptors | [Supporting crates](/learn/supporting-crates) | In: zero ordinary dependencies; Out: the whole product line, starting at `core` | `event!` (`lib.rs:83`), the descriptor table in `framework.rs` | `rg "never renumber" crates/nana-diagnostics/src/framework.rs` |
| **`nana-svg-raster`** | CPU-side SVG rasterization (`resvg → pixmap`) | [Supporting crates](/learn/supporting-crates) | In: `resvg 0.48.1`; Out: `nana-ui` | `rasterize_document` (`lib.rs:58`) | `rg "pub fn rasterize" crates/nana-svg-raster/src/lib.rs` |
| **`nana-app-icon`** | Default application icon, PNG/ICO/ICNS encoding and Windows PE resource embedding | [Supporting crates](/learn/supporting-crates) | In: `platform` (optional), `image`, `flate2`, `embed-resource` | `rasterize` (`mark.rs:8`) | `rg "feature" crates/nana-app-icon/Cargo.toml` |
| **`nana-icons-tabler`** | Auto-generated Tabler vector icon constant table (about 323k lines) | [Supporting crates](/learn/supporting-crates) | In: the icon types from `core`; not a workspace member, trimmed by the linker on reference | the `*_ICON: IconData` constant family | Read the header comment of `crates/nana-icons-tabler/src/lib.rs` |
| **`nana-package`** | Runtime read-only side of the packaging contract (parses `package.json` and `.nrpack`) | [Supporting crates](/learn/supporting-crates) | In: pure Rust (`blake3`, `ruzstd` and so on); Out: `nana-ui` | `PackReader` (`pack/reader.rs:47`), `ResourceClass` (`pack/mod.rs:30`) | Read `crates/nana-package/src/lib.rs` |
| **`nana-packager`** | Build-side packaging CLI (6 subcommands) | [Supporting crates](/learn/supporting-crates) | In: `nana-package`, `nana-app-icon`, the C `zstd` encoder; not shipped with the application | the `Command` enum (`main.rs:32-38`) | `rg "enum Command" crates/nana-packager/src/main.rs` |
| **`nana-ui-dev`** | Shared development-time hot-reload foundation (release/dist builds hit `compile_error!` outright) | [Supporting crates](/learn/supporting-crates) | In: `ui`, `core`, optional `devtools` / `sfc` | `ReloadRequest` (`lib.rs:46`), the compile-time gate (`lib.rs:22`) | `rg "compile_error" crates/nana-ui-dev/src/lib.rs` |
| **`nana-ui-devtools`** | Development-time headless Agent automation sessions and offscreen snapshots | [Supporting crates](/learn/supporting-crates) | In: `core`, `platform`, `ui`, `scene`, `gpu` | `AgentSession` (`agent/session.rs:37`), `RuntimeAgentSession` (`agent/runtime.rs:26`) | `rg "product-facing UI" crates/nana-ui-devtools/src/lib.rs` |

## Coverage statistics

The five groups above total `5 + 4 + 2 + 4 + 9 = 24`, covering all 24 crates under `crates/`, 100%.

## Further reading

- [Modules and layers](/learn/modules-and-layers): the full dependency-depth table for the 24 crates and the list of 55 ordinary dependency edges
- [Evidence checklist](/learn/evidence-checklist): read-only verification commands and measured readings for the key architecture claims
- [Architecture learning overview](/learn/): the map of all chapters, organized by learning path
