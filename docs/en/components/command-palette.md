# CommandPalette

`CommandPalette` only presents the actions you supply that are available in the current context. The actual dispatch, and persisting shortcuts, is yours. Its accessible role is dialog, so it goes through `activate_overlay`.

The control table has no `<CommandPalette>`.

## Basic usage

`CommandPalette::new(title, items)` takes items of type `nana_ui::runtime::CommandPaletteItem`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{CommandPalette, CommandPaletteItem};

view! {
    <Widget
        of={CommandPalette::new(
            "Commands",
            [CommandPaletteItem::new("file.save", "Save").shortcut("⌘S")],
        )}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{CommandPalette, CommandPaletteItem};

widget(CommandPalette::new(
    "Commands",
    [CommandPaletteItem::new("file.save", "Save").shortcut("⌘S")],
))
```

:::

## Items

In `CommandPaletteItem::new(action, label)`, `action` is a stable `ActionId`.

`.shortcut` is the shortcut text shown, and `.category` is the group name.

## Filtering

The default placeholder is "Search actions" and the empty result is "No actions available"; replace them with `.placeholder` and `.empty_label`.

The control filters by substring unless `.filtered_items(true)`, which means you have already filtered and sorted the items and the palette does not filter them a second time.

Changing the query still emits `Search`.

## Query and navigation

`.query(...)` and `set_query` change the query string and the input state together, and collapse the selected index to 0; read the current value with `query_text()`.

`set_query` returns `CommandPaletteEvent::Search`.

`navigate` takes an `ActionPickerNavigation`: `Previous`, `Next`, `First`, `Last`, `Confirm`, `Dismiss`.

Confirm reads the currently visible item and emits `Select(action)`.

Directional navigation emits no event when no item is visible.

## Opening

When open, the palette is a direct child of `OverlayHost`, opened with `activate_overlay(host, palette)`.

Close it with `dismiss_overlay(host)`, or collect it yourself in `Dismiss`.

`ActionRegistry` registers actions, dynamic availability/labels, and shortcut contexts; call `ActionRegistry::palette_items(query, &context, Some(&keymap))` to produce the item list handed to the palette, and forward palette events to `AppContext::dispatch_palette_event(&registry, &event, &context)` for unified validation and dispatch. The panel itself only displays the list you hand it.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `CommandPalette::new(title, items)` | — | The items are of type `nana_ui::runtime::CommandPaletteItem` |
| `CommandPaletteItem::new(action, label)` | — | `action` is a stable `ActionId` |
| `.shortcut` | — | The shortcut text shown |
| `.category` | — | The group name |
| `.placeholder` | — | The default placeholder is "Search actions" |
| `.empty_label` | — | The default empty result is "No actions available" |
| `.filtered_items` | — | `.filtered_items(true)` means you have already filtered and sorted the items and the palette does not filter them a second time |
| `.query(...)` | — | Changes the query string and the input state together with `set_query`, and collapses the selected index to 0; `query_text()` reads the current value |
| `set_query` | — | Returns `CommandPaletteEvent::Search` |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `CommandPaletteEvent::Search` | — | Changing the query still emits `Search`. `set_query` returns this event |
| `Select(action)` | — | Confirm reads the currently visible item and emits `Select(action)` |
| `Dismiss` | — | The `ActionPickerNavigation` taken by `navigate` includes `Dismiss`. Close with `dismiss_overlay(host)`, or collect it yourself in `Dismiss` |

## Slots

| Slot | Description |
| --- | --- |
| None | None |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
