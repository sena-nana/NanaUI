# List

`List` is a container for a set of entries. Its accessibility role is a list, and screen readers read it as one. Which entries go inside is up to you.

The entry control is `ListItem`, whose page is described separately in the overview.

There is no `<List>` in the control table.

## Basic usage

`List::new()` has no child nodes.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::List;

view! {
    <Widget of={List::new().label("Recent files")}>
        <ListItem>"main.rs"</ListItem>
        <ListItem>"lib.rs"</ListItem>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{list_item, widget};
use nana_ui::runtime::List;

widget(List::new().label("Recent files")).children((
    list_item("main.rs"),
    list_item("lib.rs"),
))
```

:::

`list_item` is the view function for an entry; the label is both the constructor argument and the visible text.

Selection, disabling and trailing accessories can be bound per entry, while the list or `each_virtual` emits selection requests for your `SelectionState<K>`.

## Clicks and selection

By default a plain `List` neither takes the pointer nor joins focus; to manage single or multi-item selection, set `List::multiselectable(true)` or listen for `ListSelectionRequested` (on `each_virtual`, call `.multiselectable(...)` and `.on_select(...)`):

- Pressing an interactive `ListItem`, `InteractiveCard`, or custom item card carrying `.aria_selected(...)` emits `ListSelectionRequested { item, gesture: SelectionGesture::Press(modifiers), step: None }` (or `VirtualSelectionRequest<K>` in `each_virtual::on_select`).
- While focus is inside a selection-active list, arrows (`ArrowUp`/`ArrowDown` in 1D lists, plus `ArrowLeft`/`ArrowRight` and row-striding `ArrowUp`/`ArrowDown` when `grid_columns > 1`), `Home`/`End`, and `PageUp`/`PageDown` emit `Move`; `Shift` extends from the anchor, `Ctrl+Shift` extends additively, `Ctrl+Space` emits `Toggle`, and `Ctrl+A` emits `SelectAll`.
- For custom cards, call `.hittable().focusable().aria_selected(...)`; the CSS `:selected` pseudo-class updates with the selected state.
- Virtual lists keep offscreen selections in `SelectionState<K>` and scroll to and focus keyboard targets via `VirtualListRef::reveal_and_focus(&key)`.

## Workspace

When you do not need an IDE-style workspace, a column of `List`s or a single `AppShell` is enough.

Regions, Dock and splits are left to `Workspace`.

## Style and length

The list does not sort or virtualize; wrap a long list in `ScrollView`, or manage the window yourself with a virtual list.

The default style is an empty `NodeStyle`.

For spacing, padding or a background, write it in `.style`, or put the list inside `Stack::column`.

## Accessible name

The list's accessible name comes from `.label`; without it, screen readers announce only the role, "list".

## Properties

`.label` gives the whole list an accessible name.

`.style` replaces the whole node style.

| Property | Type | Description |
| --- | --- | --- |
| `.label` | — | Accessible name for the whole list. Without it, screen readers announce only the role, "list" |
| `.multiselectable` | `bool` | Enables multi-selection semantics for assistive technology and `ListSelectionRequested` |
| `.grid_columns` | `usize` | Grid column count (values > 1 enable 2D arrow navigation across columns and rows) |
| `.style` | — | Replaces the whole node style. The default style is an empty `NodeStyle` |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `ListSelectionRequested` | `{ item, gesture, step }` | Emitted by selection-active `List` / `each_virtual` containers on press or selection key |

## Slots

| Slot | Description |
| --- | --- |
| None | None |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
