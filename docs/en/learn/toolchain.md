---
title: Authoring Toolchain
---

# Authoring Toolchain

The `view!`, `css!` and `stylesheet!` macros a developer writes, together with `.vue` single-file components, are compiled **at application build time** into declarative view function calls for `nana-ui-runtime` and static Style Model patch data. This chapter takes the two build-time compilation pipelines apart and follows templates, styles and scripts through every crate.

## Responsibilities and boundaries

| Crate | One-line responsibility | What it explicitly does not do |
| --- | --- | --- |
| `nana-ui-view-schema` | The single description table for built-in controls (`for_each_control!` intermediate representation) | Zero external dependencies, and no code generation logic of any kind |
| `nana-ui-view-macros` | Entry points for the `view!` / `css!` / `stylesheet!` and `#[derive(Store)]` procedural macros | Only forwards procedural macros; the core expansion lives in `view-codegen` |
| `nana-ui-view-codegen` | Expands the template element tree into Runtime view functions; calls `nana-ui-css` at build time to compile styles | Introduces no new runtime concept |
| `nana-ui-sfc` | Build-time compiler for `.vue` single-file components (`<script setup lang="rust">`) | Never enters the application runtime; its build output is standard Rust source and a source map |

Both compilation pipelines obey one hard rule: **the runtime receives compiled data, never CSS source waiting to be parsed** (`crates/nana-ui-runtime/src/view/style.rs:1-27`).

## Dependencies and neighbours

| Crate | Level | Normal in-workspace dependencies | Boundary note |
| --- | --- | --- | --- |
| `nana-ui-view-schema` | 0 | — | A dependency-free pure macro table, fed to both `runtime` and `view-codegen` |
| `nana-ui-css` | 2 | `nana-ui-core` | The one CSS engine, called only by `view-codegen` at build time |
| `nana-ui-view-codegen` | 3 | `nana-ui-core`, `nana-ui-css`, `nana-ui-view-schema` | Runs at host build time and compiles CSS rules into a static `StylePatch` |
| `nana-ui-view-macros` | 4 | `nana-ui-view-codegen` | A `proc-macro` crate, re-exported through `nana-ui-runtime` (the `view-macro` feature) |
| `nana-ui-sfc` | 4 | `nana-ui-view-codegen` | Runs in the application's `build.rs`; generated code refers to `nana_ui::runtime` through path strings |

## Key types and data

| Type / constant | File and line | Purpose |
| --- | --- | --- |
| `for_each_control!` | `crates/nana-ui-view-schema/src/lib.rs:49` | Tags, constructors, field write policies and event tables of the 26 built-in declarative controls |
| `Control` / `CONTROLS` | `crates/nana-ui-view-codegen/src/lib.rs:18` / `:53-77` | The static control metadata table produced by expanding `for_each_control!` |
| `Node` / `Element` / `Attr` | `crates/nana-ui-view-codegen/src/lib.rs:333-405` | The intermediate representation of the template syntax tree |
| `CompiledStyles` / `CascadeKey` | `crates/nana-ui-view-codegen/src/style.rs` / `crates/nana-ui-css/src/cascade.rs` | Build-time style compilation output and full CSS Cascade Level 5 priority key (`@layer`, `!important`, `Specificity`, `source_order`) |
| `CssDiag` / `DiagSeverity` | `crates/nana-ui-css/src/syntax/diag.rs` | Tiered build-time diagnostics: syntax/invalid-value/unsupported selector or at-rule => `Error` (compile error); safely-ignorable vendor-prefixed properties => `Warning` |
| `Component` / `Compiler` | `crates/nana-ui-sfc/src/parse.rs:23` / `lib.rs:115` | The four main blocks parsed out of a `.vue` file (name / props / script / template / style) and the build compiler |

## Internals

### 1. Pipeline one: the Rust procedural macro chain (`view!` / `css!` / `stylesheet!`)

On the Rust path, the `view!` macro and the declarative view functions are first-class equivalents, expanding to the same construction and mount calls:

```rust
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Button @activate={save}>"OK"</Button>
    </Column>
}
```

Four procedural macros are defined in `crates/nana-ui-view-macros/src/lib.rs` and re-exported by `nana-ui-runtime` under the `view-macro` feature:

| Macro | Entry point | Build-time expansion |
| --- | --- | --- |
| `view!` | `view-macros/src/lib.rs:17` | Parses the `<Tag>` template tree and hands it to `view-codegen`, expanding it into view function calls such as `column` / `text` / `each` / `when` |
| `#[derive(Store)]` | `view-macros/src/lib.rs:24` | Generates a `<Name>StoreFields` trait for the struct, giving every named field a path-tracking method |
| `css!` | `view-macros/src/lib.rs` | Takes a single-element CSS declaration block (Rust-token CSS or raw string `css!(r#"..."#)`) and compiles it via `nana-ui-css` into a `StylePatch` for `.css(...)` |
| `stylesheet!` | `view-macros/src/lib.rs` | Compiles `mod styles; .card { … }` (token or raw string form) into a module holding a static `Sheet` and `view::Class` constants of the same names |

#### Single source of truth: `nana-ui-view-schema`

The `for_each_control!` macro at `crates/nana-ui-view-schema/src/lib.rs:49` defines the template tags, constructors, field write policy (`write`) and events of the built-in controls in one place:

- `nana-ui-runtime` expands this table into a typed setter, `.model(...)` and event binding methods for each control;
- `nana-ui-view-codegen` expands the same table into the compile-time validation table for `<Tag>` templates — so a misspelled tag or attribute name fails at the exact source line during compilation, and adding a field to the table takes effect on both sides at once.
- The field write policy `write` (`lib.rs:23-40`) controls precisely how a value lands in the component: `set` (assign a plain field directly), `text_state` (replace editing text only when the content actually differs, preserving the caret position), `assign` (assign a number input and move the caret to the end), `clamp` (clamp a slider range), `span_low` / `span_high` / `span` (clamp a two-thumb span, pushing the other end along without allowing a crossing), `builder` (call the builder method of the same name to maintain derived parts, such as the HSV of a `ColorField`), `tab` (switch the selected tab and synchronize focus), `resource` (bind a host texture slot).

#### Build-time style compilation: `view-codegen/src/style.rs`

`crates/nana-ui-view-codegen/src/style.rs` provides `compile_inline` (`css!` and `style="..."`), `compile_styles` (`<style>`), and `compile_stylesheet` (`stylesheet!`), building directly on `nana-ui-css`'s `StyleOp` pipeline at compile time:

- Full CSS Cascade Level 5 `@layer` ordering (`CascadeKey`), `@media` (Kleene 3-valued logic via `Sheet::with_media` and `SheetMediaQuery`), `@container` (interval breakpoint buckets), `@supports`, `@keyframes` / `transition`, and `@font-face` (`Sheet::with_font_faces` wired to host font ingestion);
- Class and compound selectors, static child (`>`) and descendant (` `) combinators, subject and ancestor interactive pseudo-classes (`:hover` / `:focus` / `:focus-visible` / `:active` / `:selected`, `.card:hover .icon` → `track_interactive_state` + `ancestor_pseudo`), paint-only pseudo-elements (`::placeholder`, `::selection`, `::-webkit-scrollbar*`), element-scoped static `--*` custom property inheritance (hierarchical `VarScope`), and inline `style="..."` in `.vue` and `view!` (Layer 2 `.inline_patch`);
- Product builds emit zero-overhead pure-Rust `StylePatch(fn(&mut LayoutStyle))` function pointers via `emit_style_ops_rust` (calling `nana_ui_core::style_patch::set::*`, zero runtime JSON); dev hot reload produces `HotStylePatch::from_ops(ops)`;
- Diagnostics are strictly tiered: syntax errors, invalid values, unknown properties, and unsupported selectors or at-rules are compile errors, while known safely-ignorable vendor-prefixed properties and unused classes emit warnings — never silently dropped.

### 2. Pipeline two: the `.vue` single-file component compiler (`nana-ui-sfc`)

`nana-ui-sfc` compiles `.vue` files whose script is `<script setup lang="rust">` into standard Rust view functions in the application's `build.rs` (for example `TodoList.vue` produces `pub fn todo_list(...) -> impl IntoView`):

1. **Splitting into three blocks (`parse.rs:23`)**: parses out `<script setup>`, `<template>` and `<style>`, and records line and column offsets so error positions can be restored;
2. **Whole-component static signal analysis and constant folding (`lib.rs:413`)**: because the compiler sees the whole script, it automatically folds a `signal(v)` that is never written and never escapes into `constant(v)` (a property binding to a read-only constant assigns once when the node is built and creates no runtime `Effect`); detects circular `computed` dependencies and reports a compile error directly; and wraps the visible static dependency list as `__checked` (in debug builds it checks whether the actual reads go out of bounds, see `runtime.reactive.static_deps_mismatch`);
3. **Output (`$OUT_DIR`)**: writes the formatted `nana_views.rs`, a dependency report `nana_views.deps.md` and a source map `nana_views.map.json` (following `nana-sfc-source-map/1`). With `.hot(true)` enabled it also lifts purely static text into a hot-replaceable table `__NANA_HOT`, which works with `watch_templates` in `nana-ui-dev` to change text without a restart.

## Collaboration and data flow

```text
Pipeline 1  view! / css! / stylesheet! ──view-macros──▶ view-codegen (expand + build-time CSS) ──▶ Runtime view functions + StylePatch
Pipeline 2  .vue files ──nana-ui-sfc──▶ static signal analysis and folding + view-codegen expansion ──▶ $OUT_DIR/nana_views.rs
```

## Code map

| Crate | Core entry file | Size and role |
| --- | --- | --- |
| `nana-ui-view-schema` | `crates/nana-ui-view-schema/src/lib.rs` | 236 lines: the `for_each_control!` table, the only table of built-in controls |
| `nana-ui-view-macros` | `crates/nana-ui-view-macros/src/lib.rs` | 685 lines: entry points for the `view!`, `css!`, `stylesheet!` and `Store` procedural macros |
| `nana-ui-view-codegen` | `crates/nana-ui-view-codegen/src/{lib,style}.rs` | 1,873 + 1,504 lines: the template expander and the build-time CSS compiler |
| `nana-ui-sfc` | `crates/nana-ui-sfc/src/{lib,parse}.rs` | 1,030 + 638 lines: the `.vue` compiler and the static signal dependency analysis |

## Further reading

- [Authoring paths](/learn/authoring-paths): how the two authoring paths converge in `UiMutation` / `UiWorld`
- [CSS engine](/learn/css-engine): the internals of `nana-ui-css` called at build time
- [.vue dialect (guide)](/guide/scaling/sfc) · [Declarative views (reference)](/reference/reactive-view)
