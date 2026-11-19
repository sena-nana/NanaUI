---
title: "Core Contracts: nana-ui-core"
---

# Core contracts: `nana-ui-core`

This chapter goes into `nana-ui-core`, the cornerstone of the whole framework: it defines the foundation contracts shared across the repository — the Style Model (three layers: Tokens / Semantics / Layout), the theme stack, geometry primitives, the Workspace contract and Motion IR — and explains why these definitions must be extracted to the bottom layer. After reading it you will understand: which layer a style lands on when it travels from a CSS declaration or a Rust builder into a `LayoutStyle` field; how the semantic palette serves light and dark themes and WCAG contrast at the same time; how `WorkspaceModel` manages region layout and collapse animation without depending on a window backend; and why Motion allows only one timeline authority. This chapter follows [Modules and Layers](/learn/modules-and-layers) and is the shared prerequisite for the [CSS engine](/learn/css-engine) and the [text engine](/learn/text-engine).

## Responsibilities and boundaries

`nana-ui-core` is a data contract layer neutral to the graphics and window backends (`crates/nana-ui-core/src/lib.rs:1-9`): every style path converges on the Style Model (**Tokens + Semantics + Layout**); CSS compiled at build time maps onto that model, and the Rust API is its native entry point; actual painting stays in Runtime and UiScene.

- **A shared model, not a parsing implementation**: `core` only defines "what style data looks like", "how a theme compiles" and "how a region is declared"; it never parses any stylesheet or script. The file header states it plainly: "This crate must **not** depend on CSS parsers, JS engines, WebView, or window backends" (`crates/nana-ui-core/src/lib.rs:8-9`).
- **A zero CSS engine boundary**: CSS parsing is fully isolated outside `core`; the non-goals list in `style_model.rs` says that CSS parsing and the CSSOM must not enter this crate (`crates/nana-ui-core/src/style_model.rs:43-46`). The only contact point between `core` and CSS is `SemanticColorRole::from_css_token_name` (`crates/nana-ui-core/src/style_model.rs:341`, which only maps `--nana-*` variable names to semantic color roles, and returns `None` directly for an unknown `#hex` / `rgb()`) plus a small amount of single-value enum keyword parsing.
- **Entry-point neutrality**: the Rust builder and the build-time CSS compiler must produce the same style data, so the contract layer must sit lower than either of them; the five layer-2 workspace crates — text, CSS, charts, input, GPU (plus standalone `nana-icons-tabler`) — as well as layer-3 `nana-ui-diagram` all build on `core` (with layer-4 `nana-window` also depending on `nana-ui-platform`).
- **Diagnostics go through static descriptors**: it follows the static descriptor contract of `nana-diagnostics` and introduces no external logging crate — which is also one of its only three dependencies.

## Dependencies and neighbors

The dependency table in `crates/nana-ui-core/Cargo.toml` has only three entries: `nana-diagnostics`, `serde`, `serde_json` — no CSS parser, window library or GPU type:

| Neighbor crate | Collaboration |
| --- | --- |
| `nana-text` / `nana-ui-css` / `nana-ui-charts` / `nana-ui-input` / `nana-gpu` | Five layer-2 workspace crates (joined by layer-3 `nana-ui-diagram` and layer-4 `nana-window`) that build directly on `core`; typography vocabulary (`LineBreakSpec`, `WritingModeSpec`, etc.) is borrowed from `core` |
| `nana-ui-runtime` | Builds the retained tree on top of `core`; `NodeStyle` contains `LayoutStyle`, and a theme is compiled through `CompiledTheme` and attached to the world |
| `nana-ui-css` | The only CSS engine; maps style declarations onto `core`'s `LayoutStyle` (see [internal structure](#internal-structure) below) |
| `nana-diagnostics` | Provides `event!` / `metric!` / `fault!` static descriptors; `core` is one of the lowest consumers |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `StyleModelRef` | `crates/nana-ui-core/src/style_model.rs:839` | Per-node style read handle: `theme_appearance` / `metrics` / `palette` / `titlebar` / `on_media` / `opacity` (the comment stresses "small on purpose") |
| `SemanticColor` | `crates/nana-ui-core/src/style_model.rs:60` | Resolved semantic color in RGBA with `f32` components in `0..=1` |
| `PaintColor` | `crates/nana-ui-core/src/style_model.rs:70` | Author-space paint color: `Srgb` / `Oklch` / `Hsv` / `LinearScRgb`, with `to_srgb` / `to_linear_sc_rgb` conversion |
| `PaintColorSlots` | `crates/nana-ui-core/src/style_model.rs:165` | Color slots on `LayoutStyle`: foreground, background, four borders, outline and selection colors |
| `SemanticColorMix` | `crates/nana-ui-core/src/style_model.rs:393` | Semantic color mix weighted in ten-thousandths basis points; `resolve()` evaluates in premultiplied alpha space |
| `SemanticColorRole` | `crates/nana-ui-core/src/style_model.rs:257` | About 45 semantic color roles (including the code-highlighting token roles `Function` / `Builtin` / `Type` and so on); `from_css_token_name` lives at `:341` |
| `SemanticPalette` | `crates/nana-ui-core/src/style_model.rs:456` | Semantic palette: `dark()` / `light()` / `high_contrast()` / `for_system_contrast()` |
| `LayoutStyle` | `crates/nana-ui-core/src/box_layout.rs:3292` | The box-level layout slice of the Style Model: Flex / Grid / spacing / positioning / transform / writing direction and all other fields |
| `LengthSpec` | `crates/nana-ui-core/src/box_layout.rs` | Length intent: `Px` / percentages / `calc()` expressions, resolved against the containing block during layout |
| `RegionState` / `WorkspaceLayout` | `crates/nana-ui-core/src/layout.rs:94` / `:386` | Workspace region contract: `placement` / `scope` / size bounds / narrow-window collapse policy |
| `WorkspaceModel` | `crates/nana-ui-core/src/workspace_model.rs:85` | The only authority for region layout: persistent region table + drag-resize interaction + viewport facts + collapse transitions |
| `MotionTrack` / `MotionGraph` | `crates/nana-ui-core/src/motion/ir.rs` | Absolute-time animation tracks; sequential and parallel composition both compile down to this |
| `ThemeDefinition` / `CompiledTheme` / `ThemeRegistry` | `crates/nana-ui-core/src/theme/` | Theme definition → compiled artifact → registration and resolution |
| `LogicalRect` / `LogicalPoint` | `crates/nana-ui-core/src/geometry.rs:22` | Logical-pixel geometry primitives (distinct from the physical device pixels of the Scene layer) |
| `AccentRamp` | `crates/nana-ui-core/src/theme/tokens.rs:42` | Accent ramp: `base` / `strong` / `on_soft` / `text` + the `soft` family alpha + an explicitly specified `focus` |
| `MotionTokens` / `MotionRole` | `crates/nana-ui-core/src/theme/tokens.rs` | Token categories for motion durations and easing (`DEFAULT_MOTION` is `MotionTokens::DEFAULT`) |
| `ContentFit` | `crates/nana-ui-core/src/geometry.rs:44` | CSS `object-fit` semantics: `Fill` / `Contain` / `Cover` / `None` / `ScaleDown` |
| `ControlSemantics` | `crates/nana-ui-core/src/style_model.rs:914` | Component semantics control points: `size` / `button_kind` / `card_kind` / `status` |

## Internal structure

### 1. Style Model: Tokens + Semantics + Layout

The module header of `style_model.rs` (`crates/nana-ui-core/src/style_model.rs:19-24`) defines the three-layer ownership rule:

| Layer | Core content | Who writes it |
| --- | --- | --- |
| **Tokens** | `ThemeMetrics` / `ThemeAppearance` / `SemanticPalette` (`crates/nana-ui-core/src/theme/`) | Installed theme definitions; a single component must not overstep to invent its own tokens |
| **Semantics** | `crate::semantics`: component semantics such as `ButtonKind`, `ControlSize`, `CardKind` | Known semantic class names (such as `nana-btn--primary`) belong here; no new tokens are invented |
| **Layout** | `crate::layout` (workspace shell regions) + `crate::LayoutStyle` (box-level flex layout, `box_layout.rs`) | Flex / Grid / `gap` / `padding` / size declarations |

The accompanying color policy (`crates/nana-ui-core/src/style_model.rs:26-30`) states: known token names or semantic class names map to `SemanticColorRole`; an unknown `#hex` / `rgb()` **must not** be mixed into the official `ThemeTokens` — it may only remain as a restricted node paint color in `PaintColorSlots` or be discarded. This guarantees that a local color in business CSS cannot contaminate the global theme palette in reverse.

The two kinds of Layout slice are explicitly distinguished in the header of `layout.rs` (`crates/nana-ui-core/src/layout.rs:1-8`): `layout.rs` describes application shell regions (workspace region layout), `box_layout.rs` describes box-level Flexbox / Grid intent (`LayoutStyle`); both are input data for the painting pipeline, and neither is a browser CSSOM.

### 2. Semantic colors and the palette (`SemanticPalette`)

`SemanticPalette` provides the `dark()` (`:501`), `light()` (`:541`), `high_contrast()` (`:593`), `high_contrast_light()` (`:635`) and `for_system_contrast()` (`:678`) constructors; `with_accent_ramp()` (`:697`) re-derives the `accent_soft` family automatically when the accent color switches, so the background and the foreground text always stay in tune. The color lookup entry point `get_in(role, OpacityTokens)` (`:760`) has the complete fallbacks and system mapping built in.

Two design details worth noting:

- **Premultiplied mixing**: `SemanticColorMix` (`:393`) mixes two semantic colors in premultiplied alpha space using weights in ten-thousandths basis points; `resolve()` is the single evaluation exit.
- **Contrast guarantees guarded by tests**: focus-ring area contrast $\ge 3:1$ (WCAG 2.4.11) and focus text contrast $\ge 4.5:1$ (WCAG 1.4.3; measured `5.80:1` on the dark theme). The `focus` field in `AccentRamp` is specified explicitly instead of being derived from a fixed alpha because `0.50` alpha reaches `3.16:1` on the dark background `#181818`, while on a pure white background any alpha `< 1` multiplied in cannot reach `3:1` (`crates/nana-ui-core/src/theme/tokens.rs:42-69`).

### 3. Theme stack: `ThemeDefinition` → `CompiledTheme` → `ThemeRegistry`

The core principle of the theme system is written at the head of `theme/tokens.rs` (`crates/nana-ui-core/src/theme/tokens.rs:1-10`): **"the component names a step, the theme owns the value"** — a component only declares enum tiers (such as `RadiusTier::Md`, `MotionRole::HoverColor`), and the concrete values belong to the installed theme. Lookups are a compile-time-safe `match` over the enum rather than a runtime string hash lookup (`theme.get("radius.medium")` is strictly forbidden); bare constants (`space::*`, `type_scale::*`, `HAIRLINE`) are all derived from the `DEFAULT` struct, and `scripts/audit-theme-hardcoding.py` monitors whether component code gains new hard-coded literals.

### 4. Geometry primitives and the Workspace contract

`geometry.rs` defines logical-pixel primitives and window geometry constants: `LogicalPoint` (`:22`), `LogicalRect` (`:102`, which clamps negative widths and heights to `0` on construction), `ContentFit` (`:44-56`, corresponding to CSS `object-fit` and implemented by the painting side through UV sampling and clipping, without copying GPU textures), `ImageSampling` (`Resample` / `Mipmap`, `:92-98`) and `TITLE_BAR_HEIGHT = 36.0`.

The Workspace contract splits into a static description and an interaction model:

- **Static region description (`crates/nana-ui-core/src/layout.rs`)**: `RegionId` (`:15-24`), `RegionRole`, `RegionPlacement`, `RegionScope`, `NarrowBehavior` (`Shrink` / `Collapse` / `Overlay`) and `RegionState` (`:94`).
- **Interaction and transition model (`crates/nana-ui-core/src/workspace_model.rs`)**: `WorkspaceModel` (`:82`) manages persistent region sizes, drag-resize interaction, viewport facts and collapse transition animation in one place without depending on any window or rendering backend (`RegionTransition`, `:44`, evaluated by absolute time with `EaseInOutCubic` over `WORKSPACE_REGION_TRANSITION_DURATION = 260 ms`).

### 5. `LayoutStyle`: box-level flex layout intent

`box_layout.rs` (about 6,300 lines) carries the box-level layout slice `LayoutStyle` of the Style Model (`crates/nana-ui-core/src/box_layout.rs:3292`); its fields cover `paint_colors`, `direction` / `dir` / `writing_mode` / `text_orientation`, `flex_reverse` / `order` / `flex_wrap`, `display` / `position` / `z_index`, `transform` (paint-level 2D/3D transforms that do not change the Flex/Grid measurement box), `gap` / `padding` / `margin` (physical four sides TRBL + logical edges) and so on. The writing-direction enums are defined here too: `DirSpec` (`:48`) remaps logical edges and `text-align: start/end`; `WritingModeSpec` (`:65`) skips unsupported `sideways-*` in a fail-closed way; `TextOrientationSpec` (`:87`) follows UAX #50.

### 6. Motion / Motion IR: the only timeline authority

The header of `motion/mod.rs` states it plainly (`crates/nana-ui-core/src/motion/mod.rs:1-3`): **"This module is the only Motion timeline authority"** — Runtime's `AnimationSpec` is only a subset of timing and playback control, and sampling always goes through `evaluate_progress`; the CSS compiler must produce `MotionTrack` / `MotionGraph` and must not build a parallel clock; `PresentationStore` is a transient overlay layer built on top of these tracks, not a second timeline.

The IR itself lives in `crates/nana-ui-core/src/motion/ir.rs`: `MotionTrackId` (`:18`, restarting the same ID replaces it atomically), `MotionTargetId` (`:32`, mapped by Runtime to `StableNodeId`), `MotionValue` (`:46`, `Scalar` / `Color` / `Transform` / `Discrete`). `MotionValue::lerp` (`:93`) deliberately does not clamp easing progress back to `0..=1`, so that the intentional overshoot of springs and bounce curves is preserved; only color channels are range-clamped. The submodules divide the work cleanly: `eval` provides the single CPU evaluation entry `evaluate_track`; `descriptor` encodes compositor-safe tracks into the generational `MotionDescriptor` table carrying the version number `MOTION_DESCRIPTOR_VERSION`; `presentation`'s `PresentationStore` carries transient presentation overrides; `gpu` handles storage-buffer upload packing.

### 7. Why these contracts must sit at the bottom layer

Putting the Style Model, themes, geometry, Workspace and Motion in layer 1 comes from four hard engineering constraints:

1. **Multiple entry points produce the same data**: the Rust builder and build-time codegen must produce styles that can be compared field by field, so they must share the same target struct `LayoutStyle`.
2. **The common base of diamond dependencies**: the six layer-2 crates `text`, `css`, `charts`, `input`, `window`, `gpu` each depend only on `core`; if any one contract stayed in `runtime`, the six subsystems would be forced to depend on the retained tree in reverse or to copy the definitions each.
3. **A single source for typography vocabulary**: `nana-text` borrows 12 typography enums from `core` directly (guarded by a whitelist), which reduces the adaptation from `UiWorld` to the text engine to a zero-cost field pass-through.
4. **Keep it pure data and cheap to clone**: `core` has zero parsers, zero windows and zero GPU, which lets Runtime take the constant-time read strategy of "parse once when a style is written or a theme is switched, then only clone the `Arc<LayoutStyle>` when reading".

## Collaboration and data flow

Taking a card's style and motion as an example, the data flow crosses this layer's contracts in 4 steps:

1. **Entry-point write**: the Rust component API or `nana-ui-css` (build time / runtime) writes style declarations into `LayoutStyle` fields and `ControlSemantics`.
2. **Theme resolution**: Runtime installs a theme and gets a `CompiledTheme`; `StyleModelRef` exposes the current palette, metrics and appearance per node.
3. **Layout and painting consumption**: `RuntimeLayoutEngine` reads `LayoutStyle` and `WorkspaceModel` to compute geometry; the painting layer reads `PaintColorSlots` and `ContentFit`; Motion tracks are evaluated through `evaluate_track`, compositor-safe tracks are written into the `MotionDescriptor` table, and transient samples are never written back into the base style.
4. **Text joins in**: when a text node is measured, the font size, language and writing direction in `LayoutStyle` become `TextStyle` / `TextConstraints` for `nana-text`.

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-core/src/style_model.rs` | Style Model overview: the three-layer ownership rule, semantic colors/roles/palette, `StyleModelRef` |
| `crates/nana-ui-core/src/theme/` | `tokens.rs`, `definition.rs`, `compiled.rs`, `registry.rs`, `recipe.rs` (component recipes) |
| `crates/nana-ui-core/src/box_layout.rs` | `LayoutStyle` and the box-level flex layout vocabulary (about 6,300 lines) |
| `crates/nana-ui-core/src/layout.rs` | Workspace region contract (the boundary with box-level Flex is written in the file header) |
| `crates/nana-ui-core/src/workspace_model.rs` | The interaction authority for region layout and the collapse transition animation |
| `crates/nana-ui-core/src/geometry.rs` | Logical geometry primitives, `ContentFit`, window geometry constants |
| `crates/nana-ui-core/src/motion/` | `ir.rs` (Motion IR), `easing.rs`, `eval.rs`, `descriptor.rs`, `presentation.rs`, `gpu.rs` |

## Further reading

- [Modules and Layers](/learn/modules-and-layers): the layer-1 position of `nana-ui-core` in the dependency graph and its layer-2 neighbors
- [CSS engine](/learn/css-engine): how `nana-ui-css` maps style declarations onto the `LayoutStyle` of this chapter
- [Text engine](/learn/text-engine): the list of 12 typography vocabulary items that `nana-text` borrows from `core`
- [Contracts and Invariants](/learn/contracts-and-invariants): non-negotiable contracts such as "single color authority" and "presentation does not write back into the base style"
