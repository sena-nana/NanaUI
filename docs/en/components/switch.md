# Switch

`switch(label)` creates a switch. `label` is both the constructor argument and the accessible name. The template tag is `<Switch>`, and child text and `label` both write this one field.

## Basic usage

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Switch loading={pending} v-model={muted}>"Mute"</Switch>
}
```

```rust rust
use nana_ui::runtime::view::switch;

switch("Mute").loading(pending).model(muted)
```

:::

`v-model={muted}` expands to `.model(muted)`. `muted` is a `Signal<bool>`. As with a checkbox, the control flips `checked` itself and then emits `ToggleChanged`; you see the switch move without the application writing it a second time. `model` writes `event.checked` back to the signal.

## Listening

`.on_change(|event: &ToggleChanged| …)` listens to this change yourself. In templates it is `on:ToggleChanged={|event: &ToggleChanged| …}`. `@change={save}` passes `save` straight through when it is already a function, and the parameter must be `&ToggleChanged`. Other expressions expand to `move |_| { … }` and the event is ignored. A three-parameter closure is `.on_cx`.

## Binding the display only

With `.checked(muted)` alone, the signal drives the display. A flick from the user changes the control's own field; the signal's next write paints over it. Use `model` to make the signal follow along.

## Disabled and loading

While `loading` is true the control takes no pointer input, cannot be focused, and is marked busy for accessibility. `disabled` likewise keeps it out of interaction. Both can be constants, signals or closures.

`checked`, `disabled` and `loading` can all be constants, signals or closures. Do not `.get()` a signal into a boolean at construction and stop updating.

## Settings rows

Inside a settings row, a non-empty label uses the control's own name, such as `switch("Mute")`. `switch("")` has no name, and the screen reader uses the row label instead. When the row label changes, the association is re-projected with it. This association lives in `UiWorld`, not in the control's own accessibility state, so re-projecting the control does not wipe it. Outside a settings row, when the name is written on another node beside it, declare the same association with `switch("").labelled_by(caption)` (template `<Switch labelled_by={caption} />`), where `caption` is the `NodeRef` of that text.

## Outside the field table

The component also has `hint`, `size`, `invalid` and `control_position`, which decides the side the switch is painted on. They are not in `<Switch>`'s field table. The default paints it at the row's end, `SwitchControlPosition::End`. It shares the one `ToggleChanged` with the checkbox.

## Layout

A switch's track and its text are one row: the track sits on the `control_position` side, one step of the theme's `switch.label_gap` away, and the remaining width is one column with the label on top and the hint below, separated by `xxs`. With neither label nor hint only the track remains, with no gap. These are the parts layout publishes (the track is `LeadingContent` or `TrailingContent`, the label `Label`, the hint `Hint`), and intrinsic measure, placement and painting all read this one copy: the height of a switch with a hint comes from measurement, so styles no longer write `min_height` and the control size tier is only a lower bound. The width fills by default; written as content width, only the track and the gap count, and the label text does not count toward the width. Flicking, hovering and the knob's movement change painting only, with no relayout. In a right-to-left direction the track stays on the physical side `control_position` names, not mirrored.

## Properties

The fields are `label: String`, `checked: bool`, `disabled: bool`, `loading: bool`. `model` binds `checked`.

| Property | Type | Description |
| --- | --- | --- |
| `label` | `String` | Constructor argument, and also the accessible name. Child text writes the same field |
| `checked` | `bool` | `model` binds this field and writes `event.checked` back to the signal. Constant, signal or closure |
| `disabled` | `bool` | Out of interaction. Constant, signal or closure |
| `loading` | `bool` | While true the control takes no pointer input, cannot be focused, and is marked busy for accessibility. Constant, signal or closure |
| `hint` | — | Not in `<Switch>`'s field table |
| `size` | — | Not in `<Switch>`'s field table |
| `invalid` | — | Not in `<Switch>`'s field table |
| `control_position` | `nana_ui::runtime::SwitchControlPosition` | Not in `<Switch>`'s field table. The default paints it at the row's end, `SwitchControlPosition::End`; `Start` paints the switch before the text |

## Events

The event is `on_change`, of type `ToggleChanged`. The handler receives `&ToggleChanged`.

| Event | Payload | Description |
| --- | --- | --- |
| `on_change`, template `on:ToggleChanged` or `@change` | `&ToggleChanged` | Emitted after the control flips `checked` itself. Shares this one kind with the checkbox. A function is passed as is and its parameter must be this reference; other expressions expand to `move \|\| { … }` and the event is ignored |
| `.on_cx` | — | A three-parameter closure |

## Slots

| Slot | Description |
| --- | --- |
| None | Child text and `label` both write this one field |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
