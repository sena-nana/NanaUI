---
title: "CSS Engine: nana-ui-css"
---

# CSS engine: `nana-ui-css`

This chapter goes deep into `nana-ui-css`: the framework's only CSS engine, responsible for stylesheet parsing, selector matching, cascade ordering, and mapping matched declarations onto the Style Model (`nana_ui_core::LayoutStyle`). By the end you can answer: the complete internal pipeline from a CSS source string to `LayoutStyle` fields (hand-written lexing/splitting → selector matching → cascade ordering → field mapping); what `@media`, `@import`, `@layer`, `@font-face`, `@keyframes` and `@container` each compile into; why the engine runs only at `view-codegen` build time; and how `tools/css-parity` uses 149 test fixtures to hold the engine to the browser box model.

## Responsibilities and boundaries

The file header of `lib.rs` sets the tone: "NanaUI's CSS subset, in one place" (`crates/nana-ui-css/src/lib.rs:1`). A stylesheet is standard CSS within the supported subset — selectors, cascade, custom properties (CSS variables), `@media`, `@import`, `@font-face`, `@keyframes` and interactive pseudo-classes — all mapped onto the single Style Model (`crates/nana-ui-css/src/lib.rs:3-6`):

- **The one CSS engine in the repository**: lexing, parsing, cascade and mapping are implemented only in this crate; "CSS parsing stays out of `nana-ui-core`, `nana-ui-runtime`, `nana-ui-scene` and `nana-ui`" (`crates/nana-ui-css/src/lib.rs:13-14`).
- **Never measures a container**: `@container` is compiled into interval and bucketing data that the Runtime evaluates during layout — "this crate never measures a container" (`crates/nana-ui-css/src/lib.rs:6-8`; `crates/nana-ui-css/src/css_container.rs:3`). Geometry measurement always belongs to the single authority, the Runtime layout engine.
- **No box model solving**: the engine's output is `LayoutStyle` intent data; actual Flexbox / Grid size solving runs in `nana-ui-runtime` (see [Layout engine](/learn/runtime-layout)).
- **Explicit subset and an auditable report**: the supported CSS subset is defined by [Layout (CSS subset)](/reference/layout); selectors or declarations outside the subset are never guessed at and are recorded uniformly in `UnsupportedCssReport` and `UnsupportedCssTally`.
- **Defensive input upper bounds**: stylesheet size, font size and `@import` nesting depth all have hard upper bounds (`MAX_STYLESHEET_BYTES`, `MAX_FONT_FACE_BYTES`, `MAX_REGISTERED_FONT_BYTES`, `MAX_IMPORT_DEPTH`) paired with an `is_blocked_href` check (`crates/nana-ui-css/src/lib.rs:34-39`).

## Dependencies and neighbors

The only ordinary dependencies of `crates/nana-ui-css/Cargo.toml` are `nana-ui-core` and `serde_json` (`:14-15`), making it a standard layer-2 crate. Its only direct consumer inside the framework is `nana-ui-view-codegen`, and the parity tool `tools/css-parity` depends on it directly:

| Consumer crate | When it consumes | Specific responsibility |
| --- | --- | --- |
| `nana-ui-view-codegen` (`crates/nana-ui-view-codegen/Cargo.toml:11`) | **Application build time** | Parses and matches class rules for `view!`, `stylesheet!`, `css!` and `.vue`, compiling them into static `StylePatch` data so Rust L3 views have zero CSS parsing cost at runtime |
| `tools/css-parity` (`tools/css-parity/Cargo.toml:17`) | **Parity verification** | Maps the CSS text in fixtures to `LayoutStyle`, feeds it to the Runtime layout engine and compares geometry boxes |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `StyleRule` | `crates/nana-ui-css/src/css_cascade.rs:103` | One style rule: selector list + declaration entries + `source_order` + `layer` |
| `Selector` / `CompoundSelector` | `crates/nana-ui-css/src/css_cascade.rs:114` / `:150` | The compound selector chain and one compound segment (including `Combinator`, `:125`) |
| `Specificity` | `crates/nana-ui-css/src/css_cascade.rs:262` | The standard triple `(ids, classes_and_attrs, types)` (each a `u16`) |
| `MediaEnv` | `crates/nana-ui-css/src/css_cascade.rs:424` | Media query evaluation environment (viewport width/height, light/dark, contrast, etc.) used by `@media` / `@import` |
| `MatchContext` / `MatchNode` | `crates/nana-ui-css/src/css_cascade.rs:291` / `:278` | Selector matching site: ancestor chain, preceding siblings, `:has()` bitmap, focus and media environment |
| `DeclarationEntry` | `crates/nana-ui-css/src/css_cascade.rs:86` | One property declaration: property name, raw value, `!important` flag |
| `StylesheetParseReport` / `UnsupportedCssTally` | `crates/nana-ui-css/src/css_cascade.rs:481` / `:563` | Per-parse diagnostic report and cumulative tally of out-of-subset constructs |
| `RuleIndex` | `crates/nana-ui-css/src/css_cascade.rs:1298` | Candidate rule index keyed by the `type` / `id` / `class` of the rightmost compound selector |
| `ParsedStylesheet` | `crates/nana-ui-css/src/css_interactive.rs` | Parse result buckets: static rules, interactive pseudo-class rules, generated pseudo-elements, motion rules, `@container` |
| `KeyframesRule` / `MotionStyleRule` | `crates/nana-ui-css/src/css_interactive.rs` | `@keyframes` percentage stops and transition/animation declarations, awaiting compilation into Motion IR tracks |
| `ContainerRule` / `ContainerQuery` | `crates/nana-ui-css/src/css_container.rs` | `@container` compilation output: container name, query axis and half-open interval set |
| `InteractiveStyleRule` | `crates/nana-ui-css/src/css_interactive.rs` | The `:hover` / `:focus` / `:active` rule buckets (not applied during the static cascade) |
| `FontFaceRule` / `FontFaceSrc` | `crates/nana-ui-css/src/css_at_rule.rs` | `@font-face` sources and descriptors (loaded by `StylesheetLoader`) |
| `LayoutStyleCss` | `crates/nana-ui-css/src/css_map.rs:1862` | Property mapping entry trait: `apply_css_property`, `apply_css_text`, `apply_class_layout_hints` |
| `CssPaintColor` | `crates/nana-ui-css/src/style.rs:30` | CSS color syntax parser, converted to `nana_ui_core::PaintColor` through `to_core()` (`:51`) |

## Internal structure

### 1. Lexing and parsing: a hand-written parser with zero external dependencies

The engine relies on no heavy external CSS parser; a stylesheet passes through four stations in order:

1. `strip_css_comments` (`css_cascade.rs:3037`) strips comments;
2. `split_rule` (`:3091`, splits the selector head from the declaration block at the first `{`), `split_selector_list` (`:3132`, splits the selector list on top-level commas) and `skip_at_rule` (`:3061`) identify and dispatch `@` rules;
3. selectors go through `parse_selector` (`:2024`) → `tokenize_selector_chain` (`:2339`) → `parse_compound` (`:2433`); declaration blocks go through `parse_declaration_entries` (`:1422`), which splits them into `DeclarationEntry`;
4. parse results are sorted by `ParsedStylesheet` into five buckets: static rules, interactive pseudo-classes, generated pseudo-elements (`::before` / `::after`), motion and `@container`.

Parse entry points increase in capability: `parse_stylesheet` (`:658`) → `parse_stylesheet_full` (`:663`) → `_with_layers` (`:671`) → `_with_options` (`:680`); the underlying `parse_stylesheet_full_with_cache` (`:701`) carries its own parse result cache.

### 2. Selector model and right-to-left matching

Selector matching is driven by `MatchContext` (`crates/nana-ui-css/src/css_cascade.rs:291-344`), which carries the current node's tag, id, classes, attributes, ancestor chain (direct parent first), preceding sibling list (immediate older sibling first), a precomputed `:has()` bitmap (`has_bits: u64`), `focus_within`, `is_empty`, `checked` and `MediaEnv`.

`selector_matches` (`:1957`) backtracks from right to left: `Child` (`>`) checks the direct parent strictly, `Descendant` (space) scans up the ancestor chain, and `AdjacentSibling` (`+`) and `SubsequentSibling` (`~`) scan the preceding sibling list. The candidate set is first filtered by `RuleIndex` (`:1298`) on the id / class / tag of the rightmost selector into a strict superset, then `selector_matches` runs against the candidate rules, making per-element matching cost $O(\text{candidate count})$ rather than $O(\text{total rule count})$. `:nth-child(An+B)` (`parse_an_plus_b:2980`), attribute selectors (`AttrSelector:223`) and `:is()` / `:where()` lists (specificity takes the branch maximum, `max_alts_specificity:2918`) are all decided at this layer.

### 3. Cascade ordering and custom properties (`var()`)

`matched_declaration_entries_from` (`:1253`) sorts matched declarations by the standard cascade key `(important, cascade_layer_key, specificity, source_order, decl_index)`. The full pipeline that rebuilds one element's style lives in `rebuild_layout_style` (`:1606`) and the indexed variant `rebuild_layout_style_indexed` (`:1647`):

```text
stylesheet normal layer → utility-class hints (shell_contract) → attribute styles → inline style
  → stylesheet !important layer (including !important hit by @container)
  → attribute/inline !important layer → logical box edges (margin/padding-inline/block) final resolution
```

Note: **writing direction (`direction` / `dir`) and writing mode (`writing-mode`) are always applied before the remaining declarations** (`:1701-1712`), because they decide which physical edge later logical properties such as `margin-inline-start` map onto. Custom properties (`--*`) are collected along the `:root` / `[data-theme]` and element inheritance chain through `collect_document_custom_properties_from_rules` (`:1458`) and `matched_custom_properties_indexed` (`:1520`), and `var(...)` is expanded through `with_active_css_vars` before property mapping.

### 4. The property mapping table: `LayoutStyleCss` (`css_map.rs`)

`crates/nana-ui-css/src/css_map.rs` (about 9,100 lines) writes CSS property key/value pairs into `nana_ui_core::LayoutStyle` (through the `LayoutStyleCss` trait, `:1862`). Color parsing goes through `CssPaintColor` (`style.rs:30`), preserves `Oklch` / `Hsv` / `Srgb` precision and applies the semantic color token policy through `map_css_color_for_tokens` (`:635`). Host utility classes (such as `gap-sm`, `p-*`, `w-full`, `flex-1`, `rounded*`) are isolated in `crates/nana-ui-css/src/shell_contract.rs`, keeping a clear boundary with standard CSS syntax parsing. The utilities mean what they mean in Tailwind: only `flex` sets `display: flex` (and not over a `display` the stylesheet wrote); `flex-row` / `flex-col`, `items-*` and `justify-*` each set their one property. So `flex-col items-center` without `flex` is a block container, which has no `align-items`, and its children are not centred: write `flex flex-col items-center`.

### 5. Where each `@` rule and pseudo-class compiles to

| CSS construct | Compilation product | Consumer |
| --- | --- | --- |
| `@import` | Rules expanded in place (preserving depth, `@layer` and media conditions) | `css_cascade.rs:858` (`apply_import`) |
| `@layer` | Layer ordinal (numbered in first-declaration order) | `css_cascade.rs:945` (`record_layer_names`) |
| `@media` / `@supports` | `MediaQueryList` + `MediaEnv` / `evaluate_supports_condition` | `css_at_rule.rs` |
| `@font-face` | `FontFaceRule` (`src` / `weight` / `style` / `stretch` descriptors) | The host adapter loads the bytes and registers them into `nana-text`'s `FontSystem` |
| `@keyframes` / `transition` | `KeyframesRule` / `TransitionShorthand` | Compiled into Motion IR tracks in `nana-ui-core::motion` |
| `@container` | `ContainerQuery` (container name, query axis and half-open interval set) | `plan_container_queries` splits into at most `17` size buckets; the Runtime picks a bucket from the container's measured size |
| `:hover` / `:focus` / `:active` | `InteractiveStyleRule` (`:focus-visible` maps to `Focus`) | Not applied in the static cascade; the host interaction-state layer overlays it on demand |

### 6. Consumption timing: build time only

- **Build time (`crates/nana-ui-view-codegen/src/style.rs:239`)**: calls `parse_stylesheet_full` to parse `<style>`, `stylesheet!` or `css!`, applies the declarations onto an empty `LayoutStyle` and diffs out the modified fields, producing static `StylePatch` values carrying a cascade-priority `Rank` (`@container` rules carry a bucket index alongside). Rust L3 views therefore combine patches by bit at runtime, with **zero CSS parsing cost**. A patch is generated assignment code: one statement per written field (`layout.paint.outline.width = …`), whose value is that field's own JSON, read as the field's type. An application links the decoding of only the field types its styles write. The `LayoutStyle` as a whole is never serialized: a patch, and the overlay of one `NodeStyle` layer on another (`LayoutStyle::overlaid`), both write field by field.

## Parity verification with `tools/css-parity`

The workspace's `tools/css-parity` (package name `nana-css-parity`) uses **147 standard JSON test fixtures** (grouped as Flexbox `T-F`, sizing and `calc` `T-S`, box model `T-B`, Grid `T-G`, inline `T-I`, layout `T-L`, positioning `T-P`) to lock the accuracy of "CSS declaration → `LayoutStyle` → `RuntimeLayoutEngine` → geometry box" end to end:

```bash
cargo run -p nana-css-parity -- compare
```

Each fixture gives a styled node tree, a viewport size and expected box coordinates (default tolerance `2.0 px`), and is compared through `measure::measure_layout` (`tools/css-parity/src/measure.rs:90`) using the exact same `RuntimeLayoutEngine::layout_style_tree` as production.

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-css/src/css_cascade.rs` | The engine core: token splitting, selector parsing, matching, cascade ordering and `RuleIndex` (about 5,600 lines) |
| `crates/nana-ui-css/src/css_map.rs` | CSS property to `LayoutStyle` field mapping plus length/font-size/calc parsing (about 9,100 lines) |
| `crates/nana-ui-css/src/css_at_rule.rs` | `@media`, `@supports`, `@import`, `@layer`, `@font-face`, `@keyframes` parsing and evaluation |
| `crates/nana-ui-css/src/css_container.rs` | `@container` query parsing, half-open interval arithmetic and bucket planning (`plan_container_queries`) |
| `crates/nana-ui-css/src/css_interactive.rs` | Parse-time buckets: interactive pseudo-classes, generated pseudo-elements, keyframe and transition rules |
| `crates/nana-ui-css/src/style.rs` | `CssPaintColor` color parsing and the semantic color token mapping policy |
| `crates/nana-ui-css/src/css_motion.rs` | `transition` shorthand syntax parsing |
| `crates/nana-ui-css/src/shell_contract.rs` | Host built-in utility class mapping (`gap-sm`, etc.) |
| `crates/nana-ui-view-codegen/src/style.rs` | Build-time consumer: compiles class rules into static `StylePatch` |
| `tools/css-parity/` | 147 CSS box model parity fixtures and the comparison CLI |

## Further reading

- [Core contracts](/learn/core-contracts): the three-layer definition of `LayoutStyle` and the Style Model (the destination of property mapping)
- [Layout engine](/learn/runtime-layout): how Flexbox / Grid solve geometry once `LayoutStyle` is available
- [Authoring entry points](/learn/authoring-paths) · [Authoring toolchain](/learn/toolchain): how `view!` and `.vue` styles compile into Style Model data at build time
