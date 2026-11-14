# Layout and style (Rust path)

This page explains how to arrange containers, set semantic styles, control borders, and handle container padding on the Rust L3 path. If you write CSS in `.vue` or in a build-time `<style>` / `stylesheet!`, see [Layout (CSS subset)](layout.md) for the supported subset of CSS properties.

## Prefer `Stack` presets over hand-written low-level layout fields

[`Stack`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-runtime/src/view_components.rs) is the standard layout container on the Rust path. For common arrangements, use the preset constructors directly (in declarative views these correspond to `<Column>` / `<Row>` or `column()` / `row()`). When you need Grid, positioning, overflow, or paint properties, write them through `with_layout` or `from_layout` into the same [`LayoutStyle`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/box_layout.rs); there is no need to define a new layout component.

The Row/Column presets write `display: flex` and a direction directly and enter the same Flex formatting context; they do not establish a separate layout identity. `from_layout` keeps the display you pass in, and Grid is also expressed only through the same layout intent. Rust, `view!`, and the build-time `.vue` dialect share this one Runtime path. When comparing against equivalent CSS, spell out the preset's sizing and alignment defaults as well.

An authoring style is only spelling. A preset constructor, `<Row>` / `<Column>` in `view!`, `from_layout` with the preset's intent, and CSS written from an empty layout all express the same intent and produce the same context, participation, and layout result, both when the available width is generous and when it is narrower than the content (`issue204_row_column_and_css_share_structured_layout_results` in `tests/view_macro.rs`).

- The Row/Column presets size with `LengthSpec::Shrink`, written in CSS as `width: shrink` / `height: shrink` (a NanaUI extension, symmetric with `fill`). `fit-content` is a different length: when the available width is narrower than the content, it clamps to the available width, and `shrink` does not.
- `Stack::min_width(0)`, like CSS `min-width: 0`, records `allow_shrink`. That way the same declaration yields the same intent however it is written.

Positioning and scrolling are likewise only projections of Foundation capabilities (`issue204_position_and_scroll_css_and_rust_share_structured_results`). In three scroll containers — a ScrollView, an ordinary box with `overflow-y: scroll` written in Rust, and the same box written in CSS — put a sticky header, an absolute badge, and a body of text in each; all three authoring styles produce the same intent and results. The results compared are participation, containing block, clip, scroll range, and fragments. ScrollView only writes overflow; it does not bring a layout of its own.

- When CSS writes a physical edge such as `top` or `padding-left`, it also records a physical-edge entry, used to order it against logical edges such as `inset-inline-start`.
- When the same group has no logical edge, that entry has no effect. So after all CSS patches are applied (`apply` in `view/style.rs`), `LayoutStyle::drop_inert_physical_edges` drops it, so that CSS and Rust writing the same edge produce the same intent.
- When the same group has a logical edge, the entry is kept and the cascade order is unchanged.

A preset constructor, the view function `row()`, and `from_layout` with the preset's intent mount the same component with the same nodes and intent, and do the same layout work on the first frame, on local changes, and on idle frames (`issue204_facades_share_identity_and_layout_work` in `world/tests/layout_foundations.rs`). CSS is lowered at build time to `from_layout` plus style data and takes the same path. The only intentional difference between authoring styles is hit testing: Rust presets do not participate in hit testing by default, while `from_layout` and CSS layout boxes are clickable by default.

```rust
use nana_ui::runtime::{JustifySpec, SemanticColorRole, Stack};

// Toolbar: horizontal, width shrinks to content, children vertically centered; start-aligned by default, use JustifySpec::End to right-align
let toolbar = Stack::row(8.0).justify(JustifySpec::End);

// Action area that wraps automatically at narrow widths
let actions = Stack::bar(8.0).wrap(true);

// Main content area: fills the parent's remaining height
let body = Stack::fill_column(12.0);

// Outlined card container: set background, border role and width, and corner radius in one go
let card = Stack::column(7.0)
    .padding(16.0)
    .surface(SemanticColorRole::Surface)
    .border(SemanticColorRole::Border, 1.0)
    .radius(16.0);
```

The `&mut LayoutStyle` passed to a `with_layout` closure has a few in-place shorthands. They still write the same fields a CSS author would write: `set_absolute_inset(px)` (`position: absolute` plus `inset` on all four sides), `set_absolute_fill()` (all four sides 0, covering the containing block), and `set_fixed_width(px)` / `set_fixed_height(px)` (size, minimum, and maximum set to the same value, so it does not grow or shrink with content or flex). They return `&mut LayoutStyle` and can be chained. When you hold a `NodeStyle`, use `NodeStyle::layout_mut()` to get a writable layout (the shared default layout is copied first, so writes never leak through); you do not need to call `Arc::make_mut` yourself.

The layout result `LayoutBox` has `right()`, `bottom()`, `union`, `intersection` (`None` when disjoint), and `inset(amount)` (shrinks all four sides inward; if it overshoots, it collapses to 0 at the midpoint). Use them directly when computing overlays, safe areas, and hit regions.

In declarative views, child nodes are attached to a container through `.children((...))` or `.with(|c| c.add(...))` (see [L3: Building retained UI in Rust](l3-authoring.md)). `Stack` handles pure layout only; for containers with list or table semantics, use [`List`](../components/list.md) or [`Table`](../components/table.md).

| Preset constructor | Main axis | Sizing behavior | Typical use |
| --- | --- | --- | --- |
| `Stack::row(gap)` | Horizontal | Width shrinks to content, cross axis centered | Toolbars, button groups (start-aligned by default, `.justify(End)` to right-align) |
| `Stack::fill_row(gap)` | Horizontal | Fills the parent's remaining width | Multi-column segments across a full row |
| `Stack::column(gap)` | Vertical | Height shrinks to content, width fills the parent | Vertical page stacking |
| `Stack::fill_column(gap)` | Vertical | Fills the parent's remaining height and width | Main content area, panels with a bottom-pinned action bar |
| `Stack::bar(gap)` | Horizontal | Fills the full row width, children do not stretch automatically | Top bars, bottom bars, status bars |

::: tip Common pitfall: `column` vs. `fill_column`
If the main content area does not expand, or a bottom status bar or input box does not stick to the window's bottom edge, the cause is almost always a container written as the content-shrinking `Stack::column` where `Stack::fill_column` was intended.
:::

A Rust-preset `Stack` container does not participate in pointer hit testing by default (`pointer_events: false`), so it never intercepts clicks meant for what is below it. `Stack::from_layout`, which takes an already-resolved `LayoutStyle`, is hittable by default.

## Node style: `NodeStyle` and border rules

Controls accept custom style through `.style(NodeStyle)`. Always use [`SemanticColorRole`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/semantics.rs) semantic roles for color; never hard-code raw RGBA values (see [Visual and theme guidelines](look.md) for why):

```rust
use nana_ui::runtime::{NodeStyle, SemanticColorRole};

let style = NodeStyle::default()
    .surface(SemanticColorRole::Surface)      // background surface color
    .border(SemanticColorRole::Border, 1.0) // border: color role + width in one call
    .radius(8.0);
```

### A border needs both a color and a width

Internally a border is stored in two places: the color lives in `NodeStyle.border` (a semantic role) or `LayoutStyle.border_color`, and the width lives in `LayoutStyle.border_width`. **If either one is missing, the border is not drawn at all** (with no warning at compile time or run time). Therefore:

- **To draw a border**: always call `NodeStyle::border(role, width)` or `Stack::border(role, width)` to set color and width together; do not change only one of the fields.
- **To turn a border off**: set `border_width` to `Some(0.0)`, and also clear `border` and the border roles for interaction states (hover / focus).
- **A border takes layout space**: as with a CSS `border`, it sits between the box's edge and its padding, so the content box shrinks by it and a box sized by its content grows by it. Do not change a border's width for a state such as selected or focused, or the content moves with it; change only its color, or use a ring.
- **Cards with built-in appearance (`Card`)**: when you need a `1 px` border, use `Card::new().kind(CardKind::Outlined)` directly instead of layering a `NodeStyle` by hand. Background, border, and radius specified explicitly through `.style(...)` take precedence over the `CardKind` defaults.

### Ring: a state stroke that takes no layout space (`ring`)

`NodeStyle::ring(role, width)` / `Stack::ring(role, width)` draws a ring, the CSS `outline`: a stroke outside the border box that follows its corners and **takes no layout space**, so adding, widening or removing it moves nothing. Its color is a semantic role too (stored in `NodeStyle.ring` and resolved against the theme); its width is stored in `LayoutStyle.paint.outline`. A width of `0` removes the ring.

```rust
// Selected: an accent ring outside the 1 px border, layout unchanged
let card = Stack::column(0.0)
    .border(SemanticColorRole::BorderSoft, 1.0)
    .ring(SemanticColorRole::Accent, if selected { 1.0 } else { 0.0 });
```

The ring is drawn outside the box, so a parent's clipping (`overflow: hidden`) or a sibling right against it covers it; leave a gap about as wide as the ring.

### Theme-resolved semantic color mixing (`SemanticColorMix`)

When you need to mix two semantic colors by ratio, or lower only the opacity of a background or border itself, use `SemanticColorMix`:

- `SemanticColorMix::new(first_role, second_role, first_weight)`: mixes two semantic colors of the current theme in premultiplied-alpha sRGB space (`first_weight` ranges over `0.0..=1.0` with a precision of one ten-thousandth; it is re-resolved against the new palette automatically when the theme changes).
- `SemanticColorMix::alpha(role, alpha)`: lowers only the alpha of that surface or border color itself; **it does not lower the opacity of descendant text**.
- Set the base mix with `NodeStyle::surface_mix(mix)` and `border_mix(mix, width)` (whichever of these and `surface` / `border` is called last wins; the same-named methods are available directly on `Stack`). To change a container's layout inside a `bind` closure, use `Stack::layout_mut()` instead of replacing the whole container. Each `InteractionStyle` state (`base` → `selected` → `hovered` → `pressed` → `focused` → `disabled`) can override the base state in priority order.

## Key differences from Web CSS defaults

`LayoutStyle` field names follow CSS, but its defaults are **explicitly fail-closed**. Assuming browser CSS initial values will lead you astray:

1. **`flex-shrink` defaults to `0` (browser CSS: `1`)**: when a fixed-width row overflows, children keep their size and are not silently squashed. To shrink with the container, call `.shrink(1.0)` explicitly.
2. **`align-items` defaults to `Start` (browser CSS: `stretch`)**: children do not automatically stretch across the cross axis (`Stack` presets already set alignment for common uses; take care when constructing a `LayoutStyle` by hand). Block containers are the exception: a container with no `display` (`LayoutStyle::default()`, `NodeStyle::default()`), `display: Block` or `FlowRoot`, not laid along a row, is laid out as CSS block layout, which has no `align-items`, and a block-level child with an `auto` width fills the containing block as in CSS (except a child that names `align-self` or a width of its own, replaced content such as an image, and a child in an orthogonal writing mode). To keep children at their content width or centre them by `align_items`, make the container `display: Flex` (every `Stack` preset is), or give the child `align-self` or `width: Shrink`.
3. **`flex-direction` defaults to `Column` (browser CSS: `row`)**: a container with no direction stacks top to bottom. For horizontal layout, use `Stack::row` explicitly.

Key behavior consistent with CSS Flexbox: each child in a line lays out its inner content and measures its cross-axis height using the **used main size** the line actually assigns to it. For example, when a `Fill` column is narrowed next to a fixed-width sidebar, the 16:9 thumbnail and wrapping text inside it recompute their real height from the narrowed width; a stale height measured at full window width never lingers (unless the child itself pins its cross-axis `height` or carries its own `aspect_ratio`).

## Overlay positioning and container padding ownership

Modal dialogs, dropdown menus, drawers, and tooltips must use the built-in overlay controls (`Dialog`, `ActionMenu`, `Popover`, `Drawer`) anchored to the trigger slot. **Do not hand-write `position: absolute` or `fixed` to assemble an overlay.** The framework's internal `PositionSpec::Fixed` uses the window viewport as its coordinate basis and leaves the outer scroll, transform, and clip chain, but it stays in the original Runtime tree and still obeys structural opacity and stacking order.

Padding responsibilities by container layer are as follows (the framework does not zero padding automatically based on nesting depth; prefer the parent container's `gap` for spacing between siblings):

| Container | Default spacing responsibility |
| --- | --- |
| `AppShell` / `Workspace` | Manages region splitting and window chrome; does not add page margins for business content |
| `Stack` (`Column` / `Row`) | Only arranges children with an explicit `gap`; default `padding` is `0` |
| `Card` / `SettingsCard` | Owns the card's inner whitespace (`16 px` left/right, `14 px` top/bottom); adds no outer `margin` |
| `SettingsPage` | Owns the page whitespace of the inner scrollable body (top `20`, right `24`, bottom `24`, left `24 px`; `gap` between title and content is `16 px`) |

```rust
use nana_ui::runtime::{Card, Stack};
use nana_ui_core::PaddingSpec;

// 1. Several cards on a page: an outer Stack::column(16.0) controls the spacing between cards; Card has its own inner whitespace
let sections = Stack::column(16.0);
let card = Card::new();

// 2. Fields inside a card: Card already provides padding, so the inner Stack only sets gap and adds no second layer of padding
let fields = Stack::column(8.0);

// 3. Flush lists: whichever container layer needs to sit flush, set that layer's padding to 0 explicitly
let flush_card = Card::new().padding(0.0);
let flush_page = page.content_padding(PaddingSpec::uniform(0.0));
```
