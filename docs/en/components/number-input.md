# NumberInput

`number_input()` creates a number input. It has no constructor argument; internally it starts from `NumberInput::new(0.0)`.

The template tag is `<NumberInput>`.

## Basic usage

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <NumberInput label="Count" placeholder="0" v-model={count} />
}
```

```rust rust
use nana_ui::runtime::view::number_input;

number_input().label("Count").placeholder("0").model(count)
```

:::

`v-model={count}` expands to `.model(count)`. `count` is `Signal<f64>`.

## Draft

The committed number lives in `value`. Typing changes only the draft, not `value`. Parsing happens on Enter or on losing focus. When it does not parse, the field returns to the last committed value and does not invent one. Stepping and the arrow keys prefer a draft that parses; a half-typed draft that does not parse falls back to the committed value. At a boundary, where one step cannot move, nothing changes.

Clicking into an unfocused field outside its stepper buttons, or entering it with Tab, selects all text by default so typing immediately replaces the number; a second click while already focused places the caret at the pointer, and pressing a stepper steps directly without selecting all.

## Writes

`assign` clamps the number the application gives into range. Discrete mode then aligns it to the precision and step. When the result written in equals the current one, the text is not replaced.

## Range, step and precision

Range, step and precision live on the `NumberInput` component: `range`, `minimum`, `maximum`, `step`, `precision`. They are not in the field table. `NumberInput::continuous` only clamps and does not align to a grid; in that mode `step` decides only the arrow and stepping increments.

Continuous mode uses `NumberInput::continuous(value)` instead of `number_input()`. That field's text takes the shortest round-trippable form, and `precision` does not eat the decimals.

## Units

`.unit("ms")` paints the unit after the number and before the step buttons: Muted, the same type size as the number. The unit is not part of the text — typing, parsing and the committed value never see it; the text area stops a gap short of the unit, so a long number does not run under it. The screen reader reads the unit after the value (`description`).

When the unit follows the language, bind `unit` in the view. The label no longer writes the unit in parentheses.

## Alignment

The number sits at the start edge by default. The style's `text-align` (`layout.text_align`) decides where a number shorter than the box goes: `End` at the end edge (the right in left-to-right writing), `Center` centered, `Left` / `Right` by physical direction. Caret, selection and clicks are all in the same place, with no conversion elsewhere. A number longer than the box is unaffected: it still shows from the start edge and scrolls with the caret when focused, as in CSS. [TextInput](https://github.com/sena-nana/NanaUI/blob/main/docs/components/text-input.md) behaves the same when single-line.

## Read-only

`read_only` takes no input. `disabled` takes none either. The step buttons are painted by this control itself, not two buttons placed outside it.

## Properties

The fields are `value: f64`, `label: Option<Arc<str>>`, `placeholder: Arc<str>`, `unit: Option<Arc<str>>`, `disabled: bool`, `read_only: bool`, `invalid: bool`. A write to `value` goes through `assign`, not a direct field change. `model` binds `value` and takes `event.value`.

| Property | Type | Description |
| --- | --- | --- |
| `value` | `f64` | The committed number. A write goes through `assign`, not a direct field change. `model` binds this field and takes `event.value` |
| `label` | `Option<Arc<str>>` | Field |
| `placeholder` | `Arc<str>` | Field |
| `unit` | `Option<Arc<str>>` | The unit painted after the number; see "Units" above |
| `disabled` | `bool` | Takes no input |
| `read_only` | `bool` | Takes no input |
| `invalid` | `bool` | The value is wrong and the box is drawn with a Danger border; the reason is written in the owning row's `error` |

## Events

The change event is `on_change`, of type `NumberChanged`. Enter confirmation emits `on_submit` (`NumberSubmitted`), and Escape cancellation emits `NumberCancelled`. Handlers receive `&NumberChanged`, `&NumberSubmitted`, and `&NumberCancelled`.

`.on_change(|event: &NumberChanged| …)` fires only when the committed value changes. In templates it is `on:NumberChanged={|event: &NumberChanged| …}`. There is no separate `on_input` during typing.

`.on_submit(|event: &NumberSubmitted| …)` fires when Enter confirms an editable field (outside IME preedit), even when the committed value did not change, allowing applications to exit edit mode or clear focus; Escape reverts the draft and emits `NumberCancelled` (also ignoring IME preedit).

`@change={save}` passes `save` straight through when it is a function, and the parameter must be `&NumberChanged`. Other expressions expand to `move |_| { … }` and the event is ignored.

| Event | Payload | Description |
| --- | --- | --- |
| `on_change`, template `on:NumberChanged` or `@change` | `&NumberChanged` | Fires only when the committed value changes. There is no separate `on_input` during typing. A function is passed as is and its parameter must be `&NumberChanged`; other expressions ignore the event |
| `on_submit`, template `on:NumberSubmitted` | `&NumberSubmitted` | Fires when Enter confirms an editable field (outside IME preedit), even when the value did not change; interactive style states re-sync if a listener clears focus |
| `on::<NumberCancelled>` | `&NumberCancelled` | Fires when Escape cancels editing on an editable field and reverts the draft (outside IME preedit), even when no draft differed from the committed value |

## Slots

| Slot | Description |
| --- | --- |
| None | None |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
