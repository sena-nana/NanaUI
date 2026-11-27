# Classes and styles

To arrange a row or a column, start with the `gap` of `column()` / `row()`. When you need to write CSS, the stylesheet is compiled at build time and is no longer parsed at runtime. `view!`, `css!` and `stylesheet!` all sit behind `view-macro`. The full set of properties is not listed here; see [Layout (CSS subset)](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/layout.md) and [Layout (Rust)](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md).

A misspelled class name is a compile error on both sides.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn todos(empty: nana_ui::runtime::view::Signal<bool>) -> impl IntoView {
    view! {
        <style>
            .todos { opacity: 1; transition: opacity 120ms ease-out; }
            .todos.empty { opacity: 0.6; }
        </style>
        <Column class="todos" class:empty={empty} gap=8>
            <Text>"Tasks"</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

stylesheet! {
    mod todo_styles;
    .todos { opacity: 1; transition: opacity 120ms ease-out; }
    .todos.empty { opacity: 0.6; }
}

fn todos(empty: Signal<bool>) -> impl IntoView {
    column()
        .gap(8)
        .class(todo_styles::todos)
        .class_when(todo_styles::empty, empty)
        .children(text("Tasks"))
}
```

:::

The `<style>` in a template goes first and compiles into one `Sheet`. `class="todos"` expands to `.class(…)`, and `class:empty={empty}` expands to `.class_when(…)`. `stylesheet!` generates a `Class` constant for every class, with `-` replaced by `_`, so `.empty` above is `todo_styles::empty`. The constants of `mod todo_styles;` are visible only to the parent module; write `pub mod` to make them public.

The classes of one element must come from the same table. A `transition` (including `transition-delay`) on fixed classes, conditional classes, or interactive pseudos plays along the compositor track to the new value when a state change updates `opacity`, `transform`, `filter`, `width`, `height`, `background`, or `color`. `@keyframes` and `animation` also compile to `#87 MotionTrack` tracks and resolve theme colors live. Logical styles take the new value directly.

To write a few declarations for a single element, use inline `style="..."` directly in `.vue` or `view!` templates, or `.css(css! { ... })` in function views:

```rust
use nana_ui::runtime::view::{column, css};

column().css(css! { padding: 12px; opacity: 0.8 })
// Raw string literals are also supported, bypassing all Rust lexing limits:
column().css(css!(r#"padding: 12px; font-size: 1.5em;"#))
```

`css!`, `stylesheet!`, and `<style>` inside `view!` accept both Rust-token CSS and raw string literals (`r#"..."#`). In token mode, string-expecting CSS properties (`grid-template-areas`, `font-family`, `font-feature-settings`, `font-variation-settings`, `content`, plus `url(...)` and selectors) preserve double quotes automatically; on other properties, values Rust cannot tokenize (such as `"1.5em"` or `"#9ecafe"`) still go in double quotes as an escape hatch.

Element styles follow a strict 4-layer precedence regardless of builder call order: **Layer 0** (control defaults) → **Layer 1** (stylesheet rules: `.class` / `.class_when` / interactive pseudos / `@container` / `@media` / ancestor interactive) → **Layer 2** (inline `style="..."` / `.css(css!{...})` / static Rust builders) → **Layer 3** (reactive Rust prop bindings).

::: warning
Mixing classes from two stylesheets on one element fails an assertion at runtime. Write conditional classes and fixed classes in the same `stylesheet!`.
:::

Build time compiles end to end: class and compound selectors (`.todos`, `.todos.empty`), subject interactive pseudo-classes (`:hover`, `:focus`, `:focus-visible`, `:active`, `:selected`), static child (`>`) and descendant (` `) combinators plus ancestor interactive selectors (such as `.card:hover .title`) within a template `<style>`, paint-only pseudo-elements (`::placeholder`, `::selection`, `::-webkit-scrollbar*`), element-scoped static `--*` custom property inheritance, inline `style="..."`, plus `@layer`, `@media`, `@container`, `@supports`, `@keyframes`/`animation`, `transition`, and `@font-face`. Syntax errors, invalid values, unknown properties, and unsupported selectors or at-rules (such as sibling combinators `+`/`~`, tag/id/attribute selectors, or box-generating `::before`/`::after`) fail with a compile error; known safely-ignorable vendor-prefixed properties (such as `-moz-*`, `-ms-*`) emit warnings. Do not assume "if it is in the stylesheet it will take effect".

Class rules inside `@container` blocks are compiled in too and apply by container size. Declare the container with `container-type` and `container-name`:

```css
.card { container-type: inline-size; container-name: card; }
@container card (max-width: 300px) { .row { height: 40px; } }
```

When `card` is no wider than 300px, the `.row` inside it is 40px high. Runtime measures the size, and crossing a breakpoint relays out only the affected nodes. A plain rule written later still overrides a container rule. One element follows one axis of one container at a time, with at most 16 breakpoints; beyond that a warning is raised on the element. `@container` cannot be written in `css!`. For the conditions you can write and the parts that are not supported, see [Layout (CSS subset)](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/layout.md).

The containers of `each` and `when` accept `.class`, `.class_when` and `.css` as well. In templates, wrap a `v-for` or a `v-if` chain in `<Block class="strip">`; the class goes to the container, not to each row. A row's or branch's own `class` stays on the row.

## Surfaces that follow the theme

Theme color tokens in stylesheets (`var(--nana-*)`, semantic role names, `theme(…)`, `light-dark(…)`, `:root` / `:root[data-theme="dark"]` custom properties, and `color-mix(…)` with semantic roles) compile to theme-aware `PaintColor` values that resolve dynamically when the installed `ThemeId` or an enclosing `ThemeScope` (`El::theme_colors`) changes; hardcoded literal colors remain static. Every element on a view also has `.foreground`, `.background`, `.border` and `.radius` (`SemanticColorRole` and `RadiusTier`), all bindable.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{RadiusTier, SemanticColorRole};

view! {
    <Column gap=8 background={SemanticColorRole::Surface} radius={RadiusTier::Md}>
        <Text foreground={SemanticColorRole::Muted}>"Secondary note"</Text>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, text};
use nana_ui::runtime::{RadiusTier, SemanticColorRole};

column()
    .gap(8)
    .background(SemanticColorRole::Surface)
    .radius(RadiusTier::Md)
    .children(text("Secondary note").foreground(SemanticColorRole::Muted))
```

:::

`.border` writes only the border color. The width is in layout. With one of the two missing, no border is drawn. To write both at once on a component, use `NodeStyle::outline(role, width)` or `Stack::outline(role, width)`, see [Layout (Rust)](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md).

The built-in `nana.light` and `nana.dark` themes provide two sets of preset tokens; an application can also register custom themes through `ThemeDefinition + ThemeRegistry`. The current choice is persisted as a `ThemeId`, and components depend only on semantic roles, never on theme names or appearance hints.

Card padding and the `Stack` presets (`row`, `column`, `fill_column`) are also in the Rust layout page. The page edge and the card interior are two separate boundaries, and the framework does not clear one because of the other by nesting.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/conditional">
    <p class="next-step-link">Conditionals</p>
    <p class="next-step-caption">How v-if expands to when.</p>
  </a>
  <a class="next-step" href="/reference/layout">
    <p class="next-step-link">Layout (CSS subset)</p>
    <p class="next-step-caption">The properties a stylesheet can write, and the parts that will not take effect.</p>
  </a>
</div>
