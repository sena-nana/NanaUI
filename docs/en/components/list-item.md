# ListItem

`list_item(label)` creates one list row. `label` is both the constructor argument and the readable main text.

The template tag is `<ListItem>`. Child text and `label` both write to this one field.

## Basic usage

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <ListItem detail="Appearance" selected={current} @activate={open}>"Settings"</ListItem>
}
```

```rust rust
use nana_ui::runtime::view::list_item;

list_item("Settings").detail("Appearance").selected(current).on_activate(open)
```

:::

When `open` is already a function, `@activate={open}` passes the function in directly. Written as an expression, it expands to `.on_activate(move || { … })`. When `disabled` is true, `Activate` is not emitted.

## Row end

When `detail` is non-empty and there is no content slot, it is drawn at the end of the row: right-aligned, at a small font size, without changing the row height. Its color is the row's main text color multiplied by the theme's `OpacityTokens::secondary_text` (0.55 dark, 0.90 light; the painter blends in linear light, so the same α recedes by a different amount in the two modes). It is one step weaker than the main text and changes along with the row's state (hover, selected, disabled, application-supplied color); at rest it is close to `muted`. With keyboard focus, the light theme's focus background is nearly solid, and the main text is already the weakest text on it, so detail uses the same `focus_text` as the main text and is no longer faded. Overlong main text is truncated with an ellipsis rather than wrapping and breaking the row height.

An empty `detail` string means nothing is drawn at the row end. It does not add row height.

## Secondary text right after the main text

`note` is drawn after the main text, at the same font size and color as `detail`, and likewise does not change the row height. It suits paired names where the main text is a display name and `note` is an identifier: `list_item("Angle X").note("ParamAngleX")`.

For a hover tooltip, use `ListItem::with_tooltip` on the component (`tooltip: Option<IconButtonTooltip>`, configured the same way as `IconButton`). It appears when the pointer rests on the row itself (not in the leading or trailing slots) past the delay, and suits supplementary information beyond the main text. An empty label or `None` gives no tooltip; disabled rows do not show it. A read-only row with a tooltip still takes the pointer to show it, but does not highlight on hover, is not a Tab stop, and does not emit `Activate`. The tooltip hangs on a zero-size anchor under the row (queried with `AppContext::component_tooltip`), so a row with a tooltip still draws its leading, content, and trailing slots.

Width goes first to the main text and the row-end `detail`; `note` only uses what is left between them and is truncated when it does not fit; when not even a short piece fits, it is not drawn. Like `detail`, it is not drawn when a content slot is attached.

The accessible name is `label`, `note`, and `detail` in that order; a `note` that could not be drawn is still read.

## Display-only rows

Rows that only show information (members in a room, an already-bound record) use `read_only(true)`. Such a row takes no pointer input, does not highlight on hover, is not a Tab stop, and does not emit `Activate`; its text keeps its normal color and is not drawn in disabled gray. Its accessibility role is still ListItem; it does not report disabled and offers no `Click` / `Focus` actions. Use `disabled` only for rows that can be operated but are temporarily unavailable.

```rust
list_item("Hiyori").detail("Host").read_only(true)
```

## Selection

The application writes `selected`. Activation only reports that the row was clicked; it does not change selection. The accessibility role is ListItem; `selected` maps to selected and `disabled` maps to disabled.

Rows that are not in a list (Dock entries, a row of pills) use `role` to say what they are, with no change in appearance: `ListItemRole::Button` reports as a button (still reported as selected when drawn selected); `ListItemRole::ToggleButton` reports as a toggle button, and `selected` is reported as "pressed" rather than "selected", for example the entry of an open task card. A pressed toggle button is still drawn as selected.

A row that pops up a column of items when clicked (a Dock entry that raises a column of pills) also sets `has_popup(true)`: screen readers announce it as a menu button. While the row has focus, ArrowUp / ArrowDown, the `ContextMenu` key, and Shift+F10 all emit `SecondaryPress` with `keyboard: true` (`focus` is `Some(RovingEdge::Start)` for ArrowDown, `Some(RovingEdge::End)` for ArrowUp, and `None` for the menu key). Open the column in your handler, then move focus into it with `AppContext::focus_roving_edge`. The column uses `.roving_focus(RovingFocusGroup::vertical())` so the arrow keys move between items, and receives `RovingFocusEdge` when moving past either end. See menu buttons and arrow-key focus groups in [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md).

## Inline tools

Row-end tools that appear only when needed (pin, close) use `.tools(…)` in place of `.trailing(…)`. They show while the pointer is over the row or keyboard focus is on the row or inside it, so Tab stops on the row first and then on the tools. When `keep_tools` is true they also show at rest (an active pin).

How they hold space while hidden is set by the component's `tools_rest`. The default `RowToolsRest::Yield` takes them out of layout so the label uses the full row, and the row relayouts when they show; Shift+Tab coming back from below skips the tools and lands on the row, and another Tab reaches the tools. `RowToolsRest::Reserve` keeps their width and just does not paint them, so showing them causes no relayout and the tools stay in the Tab order.

```rust
widget(ListItem::new("Models").tools_rest(RowToolsRest::Reserve))
    .keep_tools(pinned)
    .tools(icon_button(icons_tabler::PIN, "Pin"))
```

## Changing the control

`@activate={open}` is already in the example above. The function takes no arguments. When you need `ViewContext`, use `.on_cx(|_item, _event: &Activate, cx| …)`, written in a template as the three-argument `on:Activate={…}`.

## Following data

For a whole column that follows data, use `each`; see [View authoring](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/view.md). A fixed number of rows goes into `column().children((…))`. Row height, spacing, `auto_height`, `trailing_align`, `pill_bleed`, and `tools_rest` are on the `ListItem` component, not in the field table.

Slots are the row's own child nodes. Those with a `key` on their root can be found again through the assembly path. Those without a key are named by position.

## Properties

The fields are `label: String`, `note: String`, `detail: String`, `selected: bool`, `disabled: bool`, `read_only: bool`, `tooltip: Option<IconButtonTooltip>`, `role: ListItemRole`, `has_popup: bool`, and `keep_tools: bool`. There is no `model`. Selection is not toggled by the control itself.

| Property | Type | Description |
| --- | --- | --- |
| `label` | `String` | Constructor argument and also the readable main text. Child text writes to the same field |
| `note` | `String` | An empty string means none. When non-empty and there is no content slot, it is drawn right after the main text as secondary text; truncated or not drawn when it does not fit, and still included in the accessible name |
| `detail` | `String` | An empty string means nothing is drawn at the row end; adds no row height. When non-empty and there is no content slot, it is drawn at the row end |
| `selected` | `bool` | Written by the application. Selection is not toggled by the control itself. Maps to selected in accessibility |
| `disabled` | `bool` | When true, `Activate` is not emitted. Maps to disabled in accessibility |
| `read_only` | `bool` | Display only: takes no pointer input or focus, no hover highlight, no `Activate`, normal text color. Accessibility does not report disabled and offers no `Click` / `Focus` actions |
| `tooltip` | `Option<IconButtonTooltip>` | Hover tooltip; none for `None` or an empty label. Not shown when disabled; when read-only, the row takes the pointer only for it |
| `role` | `ListItemRole` | Defaults to `ListItem`; `Button`, `ToggleButton` (`selected` reported as pressed). Changes accessibility only, not appearance |
| `has_popup` | `bool` | The row pops up a menu. Screen readers announce it as a menu button; ArrowUp / ArrowDown emit `SecondaryPress` with `keyboard: true` |
| `keep_tools` | `bool` | The `.tools` tools also show at rest |

## Events

The event is `on_activate`, of type `Activate`. The handler takes no arguments.

| Event | Payload | Description |
| --- | --- | --- |
| `on_activate`, template `@activate` | No arguments | Type is `Activate`. Pass a function directly; an expression is wrapped as `.on_activate(move \|\| { … })`. Not emitted when `disabled` is true |
| `.on_cx`, template three-argument `on:Activate` | `&Activate` and `ViewContext` | When you need `ViewContext` |

## Slots and unrestricted children

A row does not restrict what components an application places inside it: use the named slots or pass arbitrary views directly to `.children(…)`. Children always lay out in the order leading (`leading`), content or label copy, any extra `.children(…)` nodes, and trailing (`trailing` / `tools`): body children pack from the inline start while the trailing slot stays pinned to the inline end.

- `.leading(…)` goes before the label at the row start.
- `.after_label(…)` places a seal or mark immediately after the main text (and non-empty `note`), while preserving main-text ellipsis priority, the right-aligned `detail`, and the full accessible name (`label + note + detail`).
- `.content(…)` replaces the built-in label copy; once attached, `note`, `detail`, and `after_label` are not drawn.
- `.children(…)` freely composes arbitrary application views in declaration order between leading and trailing.
- `.trailing(…)` stays pinned to the row end.

When keyboard focus (`FOCUS_SURFACE`) fills the row with `focus_surface`, `Muted` secondary text or icons placed on the row follow the row's `foreground_secondary` onto `focus_text` so they stay readable; descendants inside an intermediate container that paints its own background or installs `theme_colors` overrides (such as a tinted seal plate) keep that container's own colors.

When the child node in the leading or trailing slot is hidden (`hidden`, or `display: none`), the row lays out as if that slot did not exist: with the leading slot hidden, the label starts at the padding, and the trailing slot stays at the row end. When it is shown again, the original layout returns without rebuilding the row.

| Slot | Description |
| --- | --- |
| `#leading` / `.leading` | Goes before the label |
| `.after_label` | Follows the main text (and `note`) directly while keeping `detail` and the full accessible name |
| `#content` / `.content` | Replaces the built-in label copy. Once attached, `note`, `detail`, and `after_label` take no space |
| `.children` | Freely composes arbitrary application views in order between leading and trailing |
| `#trailing` / `.trailing` | Stays pinned to the row end |
| `.tools` | Takes the trailing position; shows on hover or when focus is inside the row. See Inline tools above |


## Multi-line rows

For a row with a description under the name, the row start and end should line up with the name line, not be centered on both lines together. `ListItem::trailing_align(TrailingAlign::FirstLine)` does this: the row pads the first line to where a single-line row's line would be (`(band height − line box) / 2 − hairline border` above and below; the band height is the control height of the row's `style.control_height` step, or of `size` when unset, counting the border the same way as a single-line row), and centers the row start and end in this band, so row ends line up when single-line and multi-line rows are mixed. Rows with `auto_height(true)` default to `FirstLine`; all others default to `Center`.

When the row's style already gives vertical padding (`padding_top` / `padding_bottom` on a side, in pixels or a theme step, or `padding`, or `control_padding_y`), that side keeps the author's value, and the first-line padding only fills sides that are not set. The band hangs on the content edge, so the row start and end still line up with the first line; when the padding is thinner than the first-line padding, the band extends past the row's edge.

The band is the row's own node: with `FirstLine`, each row-start and row-end slot gets one, between the row and the slot; with `Center`, there is no band and the slots are direct children of the row. When projected, the row writes the band's style from its current alignment, `size`, and theme, and moves slots into or out of the band when the alignment changes. So whether the slots come from the view builder's `.leading` / `.trailing` / `.tools` or are attached through `AppContext::set_list_item_slots`, and whether `auto_height` / `trailing_align` is set before or after the slots or switched at runtime with `.bind`, the slots and the row's vertical padding change together. The content slot does not go into a band; its first line is measured against the row's own line box.

## Three states

Rows that can be clicked to do something use the default; rows that only display information and are never clickable use `.read_only(true)` (no response to hover or click, normal text color); rows that could normally act but cannot right now use `.disabled(true)` (grayed out, with the reason explained). Do not write read-only as disabled. See [Layout and composition](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/composition.md#列表行).

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
