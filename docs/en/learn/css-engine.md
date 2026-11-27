---
title: "CSS Engine: nana-ui-css"
---

# CSS engine: `nana-ui-css`

This chapter goes deep into `nana-ui-css`: the framework's only CSS engine, responsible for W3C CSS Syntax Level 3 lexing and parsing, CSS Cascade Level 5 ordering, lowering matched declarations directly into typed `nana_ui_core::StyleOp` operations, and emitting zero-overhead pure-Rust `StylePatch` code. By the end you can answer: how CSS compiles with zero external dependencies into `StylePatch` function pointers; how `@layer`, `@media`, `@container`, `@supports`, `@keyframes`, `@font-face`, pseudo-elements, and ancestor interactive selectors are wired end to end into the runtime; how the runtime's strict 4-layer style composition works; and how `tools/css-parity` uses 156 test fixtures to hold the engine to the browser box model.

## Responsibilities and boundaries

`crates/nana-ui-css/src/lib.rs` defines the engine's single authority and hard boundaries:

- **The one CSS engine in the repository, zero external dependencies, zero thread-local state**: `crates/nana-ui-css/Cargo.toml` depends only on `nana-ui-core` — zero third-party crates (no `serde_json`, no external CSS parser) and no `thread_local!` state. Lexing, parsing, cascade, property lowering, and Rust patch codegen live exclusively in `nana-ui-css`; `nana-ui-core`, `nana-ui-runtime`, `nana-ui-scene`, and `nana-ui` contain no CSS parsing code.
- **Direct `StyleOp` output — no JSON decoding, no intermediate diffing**: every property parser takes `&[ComponentValue]` directly and returns `Result<Vec<StyleOp>, CssDiag>`, targeting `nana_ui_core::StyleOp` and `style_patch::set::*` (generated from one declarative table `define_style_ops!` with **242** variants, 1:1 with `LayoutStyle::OVERLAY_LEAVES`), with zero string field-path lists and zero per-field JSON decoding at runtime.
- **Never measures a container**: `@container` lowers only into axis and half-open interval data (`ContainerQuery`) and a breakpoint plan (at most `MAX_CONTAINER_BREAKPOINTS = 16` breakpoints / 17 size buckets), which the Runtime evaluates against measured container extents during layout. Geometry measurement always belongs to the single authority, the Runtime layout engine.
- **No box model solving**: the engine outputs `StyleOp` / `StylePatch` intent data; actual Flexbox / Grid / Inline / Layers size solving runs in `nana-ui-runtime` (see [Layout engine](/learn/runtime-layout)).
- **Tiered compile-time diagnostics and defensive bounds**: block, function, unary-operator, and boolean-condition nesting is bounded by `MAX_NESTING = 32`; token-stream variable expansion enforces cycle detection and `MAX_EXPANDED_TOKENS = 16_384`; `@font-face` registration enforces `MAX_FONT_FACE_BYTES` (8 MiB) and `MAX_REGISTERED_FONT_BYTES` (16 MiB). Invalid declarations emit structured diagnostics, never clobber prior cascaded values, and are never silently swallowed.

## Dependencies and neighbors

The sole normal dependency of `crates/nana-ui-css/Cargo.toml` is **`nana-ui-core`** (zero external dependencies), making it a clean layer-2 crate. Its only direct framework consumer is build-time `nana-ui-view-codegen` (plus the dev-only hot-reload path), and the parity tool `tools/css-parity` depends on it directly:

| Consumer crate | When it consumes | Specific responsibility |
| --- | --- | --- |
| `nana-ui-view-codegen` | **Application build time** | Parses and cascades stylesheets for `view!`, `stylesheet!`, `css!`, and `.vue`, using `nana-ui-css`'s exhaustive codegen to emit static `StylePatch`, `SheetMediaQuery`, `FontFaceRule`, and motion data; in dev hot-reload mode emits `HotStylePatch::from_ops(ops)` |
| `tools/css-parity` | **Parity verification** | Maps CSS text across 156 fixtures onto `LayoutStyle` via `apply_css_text` / `compile_inline_css`, feeds it to `RuntimeLayoutEngine::layout_style_tree`, and compares geometry boxes |

## Key types and data

| Type / entry point | File | Purpose |
| --- | --- | --- |
| `StyleOp` / `style_patch::set::*` | `crates/nana-ui-core/src/style_patch.rs` | 242 typed leaf-write variants and direct setter functions generated together from one table `define_style_ops!` (`STYLE_OP_COUNT == LayoutStyle::OVERLAY_LEAVES`) |
| `StylePatch` / `HotStylePatch` | `crates/nana-ui-core/src/style_patch.rs` | Zero-overhead product function-pointer patch `StylePatch(fn(&mut LayoutStyle))` and dev-only hot-reload data patch `HotStylePatch { ops: Vec<StyleOp>, .. }` |
| `MediaQueryList` / `MediaEnv` | `crates/nana-ui-core/src/media_query.rs` | Backend-neutral `@media` contract with strong Kleene 3-valued logic (`Some(true)` / `Some(false)` / `None`) over viewport, color scheme, reduced motion, contrast, pointer/hover, and resolution |
| `FontFaceRule` / `FontFaceSource` | `crates/nana-ui-core/src/font_face.rs` | Backend-neutral `@font-face` descriptor (`family`, `sources`, `weight` / `weight_end`, `style`, `base_href`) with 8 MiB per-file / 16 MiB cumulative caps |
| `Token` / `ComponentValue` / `Span` | `crates/nana-ui-css/src/syntax/{tokenizer,parser,span}.rs` | W3C CSS Syntax Level 3 UTF-8 char-safe tokens, nesting-bounded (`MAX_NESTING = 32`) component values, and exact byte spans with 1-based line/column mapping |
| `VarScope` / `expand_vars` | `crates/nana-ui-css/src/syntax/vars.rs` | Hierarchical token-stream custom property scope and `var(...)` expander (nested fallbacks, cycle detection, and `MAX_EXPANDED_TOKENS = 16_384` cap) |
| `ComplexSelector` / `Specificity` | `crates/nana-ui-css/src/syntax/selector.rs` | Selectors Level 4 AST, quote-aware attribute selectors, `An+B`, separately AND-composed `:is()` / `:where()` / `:not()`, and exact specificity triple |
| `CssDiag` / `DiagSeverity` / `DiagCode` | `crates/nana-ui-css/src/syntax/diag.rs` | Span-attached tiered diagnostics: `Error` (compile error) vs `Warning` (safely-ignorable vendor-prefixed properties, etc.) |
| `ParsedSheet` / `ParsedRule` / `CascadeKey` | `crates/nana-ui-css/src/cascade.rs` | Parsed stylesheet (rules, keyframes, `@font-face`, `LayerRegistry`, diagnostics) and full CSS Cascade Level 5 priority key |
| `CompileContext` / `CompiledDecls` / `CompiledInline` | `crates/nana-ui-css/src/properties/mod.rs` | Thread-local-free compilation context and declaration/inline outputs (`StyleOp`s, transitions, animations, element-scoped `--*` custom properties) |
| `ContainerQuery` / `plan_container_queries` | `crates/nana-ui-css/src/lower.rs` | Lowered `@container` query (container name, axis, half-open intervals) and breakpoint bucket planner (`MAX_CONTAINER_BREAKPOINTS = 16`) |
| `emit_style_ops_rust` / `emit_media_query_list_rust` / `emit_font_face_rule_rust` | `crates/nana-ui-css/src/codegen.rs` | Exhaustive Rust source emitters (no wildcard arm) turning `StyleOp`, `MediaQueryList`, and `FontFaceRule` into direct Rust statements and constructors |
| `Sheet` / `StyleSite` / `SheetMediaQuery` / `InteractiveStateHandle` | `crates/nana-ui-runtime/src/view/style.rs` | Runtime stylesheet consumer: caches shared `Arc<LayoutStyle>` per class/pseudo/`@media` mask, drives `@container` responsive rules and ancestor interactive state signals |

## Internal structure

### 1. W3C CSS Syntax Level 3 foundation (`syntax/`)

The engine embeds a strict, zero-dependency W3C CSS frontend under `crates/nana-ui-css/src/syntax/`:

1. **Tokenizer (`tokenizer.rs`)**: UTF-8 char-safe CSS Syntax Level 3 lexer producing `SpannedToken { token: Token, span: Span }` across strings, comments, `url()`, hashes, dimensions, percentages, functions, and escapes.
2. **Parser (`parser.rs`)**: builds `ComponentValue` trees, `Stylesheet`, `QualifiedRule`, `AtRule`, `Declaration`, and CSS Nesting Level 1 `StyleBlockItem` lists, bounding every bracket/block/function/condition nesting level at `MAX_NESTING = 32`.
3. **Token-stream custom property expansion (`vars.rs`)**: `VarScope` stores raw `Vec<ComponentValue>` declarations hierarchically (`:root` → ancestor matched rules / inline styles → element's own rules / inline styles) and expands `var(--name, fallback)` directly on component-value streams before property parsing, supporting arbitrarily nested fallbacks (`var(--a, var(--b, 12px))`), cycle detection (`DiagCode::VarCycle`), and token-budget protection (`MAX_EXPANDED_TOKENS = 16_384`).
4. **Selectors, at-rules, and values (`selector.rs`, `at_rule.rs`, `values.rs`)**: parses Selectors 4 compound/complex selectors, `:is()` / `:where()` / `:not()`, `An+B`, pseudo-classes, and pseudo-elements; unifies lengths, `calc()` / `min()` / `max()` / `clamp()`, `ThemeExpr`, colors (`light-dark()`, `color-mix()`, Oklch/HSL/RGB/semantic roles), gradient stop fixup, and `@layer`, `@media`, `@container`, `@supports`, `@keyframes`, `@font-face`.

### 2. CSS Cascade 5 ordering and element-scoped custom properties (`cascade.rs`)

`parse_stylesheet` (and `parse_stylesheet_with_base`) handles CSS Nesting desugaring, parse-time `@supports` evaluation, `@layer` registration, and rule collection into `ParsedSheet`.

Matched declarations are ordered by `CascadeKey` following **CSS Cascade Level 5**:

```text
normal layered (ascending @layer declaration order)
  < normal unlayered
  < !important unlayered
  < !important layered (reversed @layer order — earlier declared layer wins!)
  ── ties broken by Specificity (ids, classes/attrs/pseudos, types)
  ── ties broken by source_order (later wins)
```

Element-scoped static custom properties (`--*`) are preserved verbatim as `CustomPropDecl` (`Vec<ComponentValue>`). While walking the static template tree at build time, view codegen maintains a hierarchical `VarScope` (`:root` / `[data-theme]` → ancestor matched `--*` and inline `style="..."` → current element's `--*` and inline `style="..."`), so `.dark-card { --bg: #111; } .dark-card .title { color: var(--bg); }` resolves cleanly at compile time. Unresolved `--nana-*` theme tokens stay as runtime-dynamic `PaintColor` / `ThemeExpr` values that follow theme switches and enclosing `ThemeScope`s live.

### 3. Direct property lowering: `properties/` → `Vec<StyleOp>`

`crates/nana-ui-css/src/properties/` is split into eight focused modules: `layout.rs`, `grid.rs`, `paint.rs`, `text.rs`, `theme.rs`, `transform.rs`, `motion.rs`, and `pseudo.rs`.

`compile_declarations` (for rule blocks) and `compile_inline_css` (for `css!` and `style="..."`) follow one pipeline:

1. Extract `--*` declarations into `CustomPropDecl`, and order `direction`, `writing-mode`, and `text-orientation` ahead of the remaining declarations so logical box edges (`margin-inline-start`, `padding-block`, etc.) in the same block resolve against the used writing context.
2. Expand `var(...)` on each declaration and dispatch to the domain parser, returning `Result<Vec<StyleOp>, CssDiag>`.
3. **Atomic non-clobbering semantics**: an invalid declaration returns `Err(CssDiag)` without emitting partial `StyleOp`s and never clobbers a value written earlier in the cascade.
4. Paint-only pseudo-element declarations (`::placeholder`, `::selection`, `::-webkit-scrollbar*`) go through `compile_pseudo_property` straight into `StyleOp::PlaceholderColor` / `PlaceholderOpacity`, `StyleOp::SelectionBackground` / `SelectionColor`, and merged `StyleOp::Scrollbar` (`merge_scrollbar_skin`).

### 4. End-to-end CSS capabilities

Every supported CSS construct is wired end to end from `nana-ui-css` → `nana-ui-view-codegen` → `nana-ui-runtime` (and the `nana-ui` host):

| CSS construct | `nana-ui-css` output | Runtime / host consumer |
| --- | --- | --- |
| `@layer` | `LayerRegistry` + `LayerOrder` | Full CSS Cascade 5 ordering in `CascadeKey` (including `!important` layer reversal) |
| `@media` | `lower_media_query_list` → `nana_ui_core::MediaQueryList` (Kleene 3-valued logic) | Emitted as `SheetMediaQuery` attached via `Sheet::with_media`; evaluated reactively against `MediaEnv` (`media_env_signal()`) across viewport size, color scheme, reduced motion, contrast, hover/pointer, and resolution |
| `@container` | `lower_container_query` → `ContainerQuery` (axis + half-open intervals) | `plan_container_queries` splits into at most `16` breakpoints (17 buckets) and attaches a Runtime `ResponsiveRule` on the nearest matching query container |
| `@supports` | Evaluated at parse time via `is_supported_declaration` | Matching branches merge directly into the cascade; non-matching branches are skipped |
| `@keyframes` / `transition` / `animation` | `KeyframesRule`, `CompiledTransition`, `CompiledAnimation` | Compiled into `nana-ui-core::motion` Motion IR tracks (`SheetTransition` / `SheetAnimation`); dynamic theme color keyframes re-resolve live on theme changes |
| `@font-face` | `lower_font_face_rule` → `nana_ui_core::FontFaceRule` | Emitted via `Sheet::with_font_faces` and attached to the root via `El::sheet(&SHEET)`; host registers faces into `nana-text`'s `FontSystem` through `nana_ui::ingest_font_face_rules` / `drain_world_font_faces` (enforcing 8 MiB / 16 MiB caps) |
| Interactive pseudo-classes (`:hover` / `:focus` / `:focus-visible` / `:active` / `:selected`) | 5-bit pseudo mask (top 5 bits `PSEUDO_BIT_SHIFT = 59`, leaving `MAX_CONDITIONAL_CLASSES = 59` conditional class bits) | Composed and cached in `StyleSite` as shared `Arc<LayoutStyle>` per active state bitmask |
| Ancestor interactive selectors (e.g. `.card:hover .icon`, `.row:active .label`) | Matched across ancestor/descendant template elements at build time | Emits `El::track_interactive_state(handle, mask)` on the ancestor and `El::ancestor_pseudo(class, handle, mask)` on the descendant — zero runtime DOM selector traversal |
| Pseudo-elements (`::placeholder`, `::selection`, `::-webkit-scrollbar*`) | Lowered directly to `StyleOp` (`Placeholder*`, `Selection*`, `Scrollbar`) | Written directly to `LayoutStyle` and rendered natively by text inputs, text selections, and scroll views |
| Inline `style="..."` and `.css(css!{...})` | `compile_inline_css` (resolved against the element's `VarScope`) | Emitted as Layer 2 `El::inline_patch(&INLINE_PATCH)`, sharing one contract across `.vue`, `view!`, and Rust function views |
| Element-scoped static `--*` custom properties | `CustomPropDecl` + hierarchical `VarScope` over the template tree | Resolved and inherited at compile time; `--nana-*` semantic tokens remain runtime-dynamic theme values |

### 5. Exhaustive codegen and strict 4-layer runtime composition

#### Zero-overhead `StylePatch` and dev-only `HotStylePatch`

`emit_style_op_rust` in `crates/nana-ui-css/src/codegen.rs` matches every variant of `nana_ui_core::StyleOp` **exhaustively with no wildcard arm** (adding a new `StyleOp` variant in `nana-ui-core` fails compilation until codegen handles it):

- **Product builds**: each compiled rule becomes a zero-overhead function pointer `StylePatch::new(|__nana_layout| { __patch::set::padding_left(__nana_layout, ...); ... })` calling `nana_ui_core::style_patch::set::*` directly. At runtime there is **zero CSS parsing, zero JSON decoding, and zero string field-path lookup**.
- **Dev hot reload**: when only static `<style>` declarations change in a `.vue` file, `nana-ui-dev` recompiles them via `nana-ui-css` into `HotStylePatch::from_ops(Vec<StyleOp>)` and injects them live without restarting.

#### Strict 4-layer composition on `El` / `NodeStyle`

`crates/nana-ui-runtime/src/view/style.rs` and `node.rs` enforce a strict 4-layer precedence independent of builder call order:

```text
Layer 0  Control default layout (e.g. Stack::row() defaults, intrinsic control layout)
   ↓
Layer 1  Stylesheet rules (.class / .class_when / interactive pseudos / @container / @media / .ancestor_pseudo)
   ↓
Layer 2  Author inline overrides (style="..." via .inline_patch in .vue and view!, .css(css!{...}), static Rust layout/paint builders)
   ↓
Layer 3  Reactive Rust prop bindings (signals and closures on El, evaluated after NodeBindings::composed_style)
```

Layer 2 inline `style="..."` or `.padding(20.0)` therefore always wins over Layer 1 stylesheet rules and `@container` / `@media` buckets, while Layer 3 reactive bindings (such as `.paint_opacity(sig)` or `.visible(sig)`) always win over Layers 0–2 on every update.

### 6. Authoring macro syntax, `.vue` scoping, and tiered diagnostics

- **Dual macro CSS syntax**: `css!`, `stylesheet!`, and `view!`'s `<style>` accept both **Rust-token CSS** and **raw string literals** (`css!(r#"..."#)`, `stylesheet!(r#"..."#)`, `view! { <style>r#"..."#</style> ... }`). In token mode, double quotes are preserved automatically for string-expecting properties (`grid-template-areas`, `font-feature-settings`, `font-variation-settings`, `font-family`, `content`, plus `url(...)` and attribute selectors), while non-string properties still accept `"1.5em"` / `"#9ecafe"` as a Rust-lexer escape hatch.
- **`.vue` `<style>` scoping rule**: `<style>` in `.vue` single-file components is **component-scoped by default** whether or not `scoped` is written; explicit global styles use `<style global>` or a standalone `stylesheet!`.
- **Tiered compile-time diagnostics (`CssDiag`)**:
  - **`DiagSeverity::Error` (compile error)**: syntax errors, unterminated comments/strings, `MAX_NESTING` overflow, invalid property values, unknown properties, unsupported selectors/combinators or at-rules, variable cycles (`VarCycle`), or expansion overflow (`VarDepthExceeded`) — emitted as `compile_error!` in macros and `Err` in SFC compilation.
  - **`DiagSeverity::Warning` (compile warning)**: known safely-ignorable vendor-prefixed properties (`-moz-*`, `-ms-*`, `-o-*`, and unmapped `-webkit-*`; note that natively mapped `-webkit-backdrop-filter`, `-webkit-mask-image`, `-webkit-line-clamp`, `-webkit-text-stroke*`, and `-webkit-text-emphasis*` compile normally without warning) and recoverable template hints such as unused classes.

## Parity verification with `tools/css-parity`

The workspace's `tools/css-parity` (package name `nana-css-parity`) uses **156 standard JSON test fixtures** (covering box model `T-B`, Flexbox `T-F` / `T-FX`, Grid `T-G` / `T-GB`, inline `T-I`, layout `T-L`, positioning `T-P`, sizing and `calc` `T-S`, viewport `T-V`, and writing modes / wrapping `T-W` / `T-WM`) to lock "CSS declaration → `StyleOp` → `LayoutStyle` → `RuntimeLayoutEngine` → geometry box" end to end:

```bash
cargo run -p nana-css-parity -- compare
```

Each fixture gives a styled node tree, a viewport size, and expected box coordinates (default tolerance `2.0 px`), measured through `tools/css-parity/src/measure.rs` using the exact same `RuntimeLayoutEngine::layout_style_tree` as production.

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-core/src/style_patch.rs` | Single-source-of-truth `define_style_ops!`: 242 `StyleOp` variants, `set::*`, `StylePatch`, `HotStylePatch` |
| `crates/nana-ui-core/src/media_query.rs` | `MediaQueryList`, `MediaCondition`, `MediaFeature`, `MediaEnv`, and Kleene 3-valued logic |
| `crates/nana-ui-core/src/font_face.rs` | `FontFaceRule`, `FontFaceSource`, `FontStyleSpec`, and 8 MiB / 16 MiB byte caps |
| `crates/nana-ui-css/src/syntax/` | W3C CSS Syntax Level 3 tokenizer, nesting-bounded parser, token-stream `VarScope`, Selectors 4, `at_rule`, `values`, `span`, and tiered `diag` |
| `crates/nana-ui-css/src/cascade.rs` | Stylesheet parsing, CSS Nesting desugaring, CSS Cascade 5 `CascadeKey`, `@layer` / `@media` / `@container` / `@supports` / `@font-face` / `--*` extraction |
| `crates/nana-ui-css/src/properties/` | Eight domain property compilers (`layout` / `grid` / `paint` / `text` / `theme` / `transform` / `motion` / `pseudo`) emitting `Vec<StyleOp>` directly |
| `crates/nana-ui-css/src/lower.rs` | `@media` → `MediaQueryList`, `@font-face` → `FontFaceRule`, `@container` → `ContainerQuery`, and `plan_container_queries` |
| `crates/nana-ui-css/src/codegen.rs` | Exhaustive Rust emitters (`emit_style_ops_rust`, `emit_media_query_list_rust`, `emit_font_face_rule_rust`) |
| `crates/nana-ui-view-codegen/src/style.rs` | Build-time consumer: matches template trees, maintains hierarchical `VarScope`, emits `Sheet` / `StylePatch` / ancestor interactive tracking, and surfaces tiered diagnostics |
| `crates/nana-ui-runtime/src/view/style.rs` | Runtime 4-layer composition, `StyleSite` caching, `SheetMediaQuery`, `InteractiveStateHandle`, and `__patch` re-exports |
| `tools/css-parity/` | 156 CSS box-model parity fixtures and comparison CLI |

## Further reading

- [Core contracts](/learn/core-contracts): the three-layer definition of `LayoutStyle` and the Style Model (the destination of property mapping)
- [Layout engine](/learn/runtime-layout): how Flexbox / Grid / Inline / Layers solve geometry once `LayoutStyle` is available
- [Authoring entry points](/learn/authoring-paths) · [Authoring toolchain](/learn/toolchain): how `view!`, `stylesheet!`, `css!`, and `.vue` styles compile into Style Model data at build time
