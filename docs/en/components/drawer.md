# Drawer

`Drawer` is a modal surface that slides in from one side of the window. Its accessibility role is dialog. You supply the title, description, and slot content; the framework places it in the viewport and handles Escape and clicking outside.

There is no `<Drawer>` and no `open()`.

## Basic usage

`Drawer::new(title)` docks to the right by default.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Drawer;
use nana_ui::runtime::DrawerSide;

view! {
    <Widget
        of={Drawer::new("Filters")
            .side(DrawerSide::Right)
            .description("Only affects the current list")}
    >
        <template #body>
            <Text>"By time"</Text>
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::Drawer;
use nana_ui::runtime::DrawerSide;

widget(
    Drawer::new("Filters")
        .side(DrawerSide::Right)
        .description("Only affects the current list"),
)
.body(text("By time"))
```

:::

## Assembly

`assemble_modal_slots` is a `slot_assembler`; when the view commits, it places these slots into the surface.

After building it by hand, call `assemble_modal_slots` yourself.

Changing a title does not reassemble automatically.

## Opening and closing

Open it with `activate_overlay(host, drawer)`. The drawer must be a direct child of that `OverlayHost`.

Close it with `dismiss_overlay(host)`.

Closing first stops interaction and restores focus. The node stays in the tree while the exit animation runs.

You can also declare it: `.open(..)` opens on `true` and closes on `false`; `.model(signal)` also writes open and close done by the host back to the signal. See [Dialog](dialog.md#declarative-open-and-close).

## Scrim

The scrim behind a drawer is the same one dialogs use: the theme's `EffectTokens::modal_scrim` and `modal_scrim_blur`. See [Dialog](dialog.md#scrim).

## Presses

While a modal surface is open, presses that land on it are captured by the surface.

The small tolerance around sidebar, split, and Dock handles underneath applies only to handles that are not covered.

## Properties

`.side` takes `nana_ui::runtime::DrawerSide`: `Left`, `Right`, `Bottom`.

`.description` is written below the title.

`.close_policy` takes `nana_ui::DialogClosePolicy`. Escape, clicking outside, and the close button always emit `DialogCloseRequested` on the drawer itself first; for gestures the policy does not allow, only the request is emitted and the drawer stays open. `DialogClosePolicy::requests_only()` hands open state entirely to the application. See [Dialog](dialog.md#letting-the-application-decide-open-state).

`.initial_focus` decides whether focus lands on the surface, the first action, or a node you specify after opening.

| Property | Type | Description |
| --- | --- | --- |
| `title` | — | `Drawer::new(title)` |
| `.side` | `nana_ui::runtime::DrawerSide` | `Left`, `Right`, `Bottom`. `Drawer::new(title)` docks to the right by default |
| `.description` | — | Written below the title |
| `.close_policy` | `nana_ui::DialogClosePolicy` | Takes `nana_ui::DialogClosePolicy` |
| `.initial_focus` | — | Whether focus lands on the surface, the first action, or a node you specify after opening |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `DialogCloseRequested` | `trigger` | The user pressed Escape, clicked outside, or activated the close button; emitted before `close_policy` applies |
| `DialogToggled` | `open` | The host opened or closed it. Not emitted when the application writes `open` itself |

## Slots

The view slots are `.title_icon`, `.title_content`, `.body`, `.footer`, and `.close_action`.

| Slot | Description |
| --- | --- |
| `.title_icon` | Icon before the title, vertically centered in the header with the title and close button |
| `.title_content` | A view that stands in the header in place of the title text; see [Dialog](dialog.md#title-content) for how it is laid out. The title is still the accessible name |
| `.body` | View slot. In the example it is `<template #body>` |
| `.footer` | View slot |
| `.close_action` | View slot |

## See also

[Overview](index.md) · [Controls](../reference/components.md)
