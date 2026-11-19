# ActionMenu

`ActionMenu` is an action menu attached to a trigger. Inside it is a `Popover` with alignment changed to the start edge and with the menu's own defaults for width, gap and padding. Opening and closing work as in popover: the state is written on `open`, and the control toggles it itself.

There is no `<ActionMenu>` in the control table: controls in templates are leaves, but a menu's items are its children. The item `<ActionMenuItem>` is in the control table (`label`, `accessible_name`, `leading_image`, `disabled`, `danger`, `active`, `@activate`). The Rust authoring style has `action_menu(label)`; the trigger text, accessible name and open state are `label`, `accessible_name` and `open` in `fields::action_menu`. When `label` is an empty string the menu closes and the trigger is not shown.

## Basic usage

`ActionMenu::new()` takes no title argument. Use `ActionMenu::trigger` for a text trigger and `trigger_icon` for an icon. In views, items go into the menu with `.children`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ActionMenu, ActionMenuItem};

view! {
    <Widget of={ActionMenu::new().trigger("File")}>
        <Widget of={ActionMenuItem::new("Save")} />
        <Widget of={ActionMenuItem::new("Delete").danger(true)} />
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ActionMenu, ActionMenuItem};

widget(ActionMenu::new().trigger("File")).children((
    widget(ActionMenuItem::new("Save")),
    widget(ActionMenuItem::new("Delete").danger(true)),
))
```

:::

## Items that follow data

Items can come from a keyed list: put `each(..)` (or a `Store`'s `keyed(..).each(..)`, or `when(..)`) inside `.children(..)`, alongside fixed items. There is no limit on the number of items, and existing items are not rebuilt when rows are added or removed.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ActionMenu;

view! {
    <Widget of={ActionMenu::new().trigger("Presets")}>
        <ActionMenuItem v-for={p in presets} key={p.id} @activate={apply(p.id)}>{p.name.clone()}</ActionMenuItem>
        <ActionMenuItem @activate={manage()}>"Manage presets…"</ActionMenuItem>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{action_menu, action_menu_item, each};

action_menu("Presets").children((
    each(presets, |p| p.id, |p| action_menu_item(p.name.clone()).on_activate(move || apply(p.id))),
    action_menu_item("Manage presets…").on_activate(manage),
))
```

:::

- A list column counts as one group of items: items in the group have the same spacing as the menu's own items, and an empty list takes no space and adds no extra gap.
- Activation, disabled state and accessible name belong to each item: in a row, use bindings (`.disabled(..)`, `.accessible_name(..)`) to follow the data rather than rebuilding.
- While open, the Up and Down arrow keys, Home and End move focus among enabled items (skipping disabled and hidden ones, wrapping at both ends), and the same applies when focus is on the trigger. Tab works as usual. When a row is inserted while open, focus stays on the original item. When the focused item is removed, focus lands on the item that takes its place (or the last item if there is none, or back on the trigger if no items remain). `AppContext::action_menu_items(menu)` returns all of the menu's items in order.

## Trigger

`.trigger(text)` is a text trigger.

`.trigger_icon(icon, label)` is a 28×28 icon button whose accessible name is `label`.

`.trigger(view)` in views works as in popover: it is display only, so do not put pressable controls inside it.

## Items

An item is `ActionMenuItem::new(label)`. For actions registered in `ActionRegistry`, call `ActionRegistry::action_menu_item(&id, &context, Some(&keymap))` to construct an `ActionMenuItem` carrying the live resolved label, disabled state, and shortcut hint.

`.hint` is one line of supplementary text; an empty string is dropped.

`.leading` places an icon.

`.leading_image(slot)` places a round image in the icon's position, drawing host texture slot `slot`, the same as an [Avatar](avatar.md) on the same slot (Cover, clipped to a circle, and the same `nana.host-texture` resource in the scene, named after the slot). When both are written, the image wins. Its side length is the line height of the row's text, one step larger than the icon, and the row does not grow taller; items without an image lay out as before. An empty slot draws a Subtle placeholder circle and the label position does not change; the convention of rejecting the frame for a missing slot is the same as Avatar. The image is decorative only; screen readers read only the label. In views, bind `.leading_image(..)` (`Option<TextureSlotName>`, field `fields::action_menu_item::leading_image`).

`.danger(true)` uses the danger color.

`.disabled` disables this item.

`.active` marks the current item.

## Placement

`.placement` and `.width` are passed to the inner popover.

As with popover, the menu surface is pinned to where the trigger is shown, and near a viewport edge it flips and then tucks back in.

If the page scrolls after opening, the surface stays where it is until the next layout.

## Open state

`.open` writes the open state directly.

Clicking the trigger, Escape, clicking outside, or writing `open` as `false` all close it; if focus is still on an item, it returns to the trigger.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `ActionMenu::new()` | — | Takes no title argument |
| `.trigger(text)` | — | Text trigger |
| `.trigger_icon(icon, label)` | — | 28×28 icon button whose accessible name is `label` |
| `.trigger(view)` in views | — | As in popover, display only; do not put pressable controls inside it |
| `.placement` | — | Passed to the inner popover |
| `.width` | — | Passed to the inner popover |
| `.trigger_style` | `NodeStyle` | Passed to the inner popover: layered over the trigger's existing style, replacing only the fields written; see [Popover](popover.md#trigger) |
| `.trigger_icon_size` | `f32` | Passed to the inner popover: the glyph side length of an icon trigger; the trigger's box does not change |
| `.open` | — | Writes the open state directly. As in popover, the control toggles it itself |
| `ActionMenuItem::new(label)` | — | An item |
| `.hint` | — | One line of supplementary text; an empty string is dropped |
| `.leading` | — | Places an icon |
| `.leading_image(slot)` | `Option<TextureSlotName>` | Round image in the icon's position, drawing a host texture slot; when written together with `.leading`, the image wins |
| `.danger` | — | `.danger(true)` uses the danger color |
| `.disabled` | — | Disables this item |
| `.active` | — | Marks the current item |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| None | None | Clicking the trigger, Escape, clicking outside, or writing `open` as `false` all close it; if focus is still on an item, it returns to the trigger |

## Slots

| Slot | Description |
| --- | --- |
| None | In views, items go into the menu with `.children` |

## See also

[Overview](index.md) · [Controls](../reference/components.md)
