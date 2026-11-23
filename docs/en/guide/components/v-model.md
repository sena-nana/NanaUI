# Two-way binding

`v-model` does two things: it displays the signal, then writes the change the control emits back. The function form is `.model(signal)`.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <TextInput placeholder="New task" v-model={draft} />
        <Checkbox v-model={agreed}>"Agree"</Checkbox>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{checkbox, column, text_input};

column().gap(8).children((
    text_input().placeholder("New task").model(draft),
    checkbox("Agree").model(agreed),
))
```

:::

`draft` is `Signal<String>` and `agreed` is `Signal<bool>`. `.model` binds the field first, then listens to that control's change event and calls `set` in the handler. You do not wire a read and a write pair yourself.

Writing `v-model` on a control that has no `v-model` is a compile error when the template is compiled.

## Binding to a store field

`v-model` also takes store paths (`Store`, `Subfield`, `Variant`, `Item`). A path both reads and writes, so hand it to `.model` directly, with no second handler:

:::api

```rust view
use nana_ui::runtime::view;

todos.each(|t| view! {
    <Row gap=4>
        <Checkbox v-model={t.done()}></Checkbox>
        <TextInput v-model={t.title()} />
    </Row>
});
```

```rust rust
use nana_ui::runtime::view::{checkbox, row, text_input};

todos.each(|t| row().gap(4).children((
    checkbox("").model(t.done()),
    text_input().model(t.title()),
)));
```

:::

The write goes through the path's own `set`: only bindings that read this row's field rerun, and `store_with_history` counts it in the current step. After the row is deleted, a late write does nothing. The path's value type has to match the field type in the control table; when they differ (a `u32` in the store and an `f64` wanted by `NumberInput`, say), bind the field and convert yourself in `.on_change`. `.model` takes a `Writable<T>`, which `Signal<T>` and all three path kinds implement.

## What can be bound

Only these controls register `model` in the control table. The signal type has to match.

| Control | Field | Event listened to for the write |
| --- | --- | --- |
| `TextInput` | `value: String` | `TextChanged` |
| `TextArea` | `value: String` | `TextChanged` |
| `NumberInput` | `value: f64` | `NumberChanged` |
| `Checkbox` | `checked: bool` | `ToggleChanged` |
| `Switch` | `checked: bool` | `ToggleChanged` |
| `Slider` | `value: f64` | `RangeInput` |
| `RangeSpan` | `span: (f64, f64)` | `RangeSpanInput` |
| `Curve` | `values: Vec<f64>` | `CurveInput` |
| `Select` | `value: Option<Arc<str>>` | `SelectChanged` |

Read the slider row carefully: `.model` listens to `RangeInput`, every visible value during a drag, not to the `RangeChanged` emitted on release. The signal follows the preview while dragging. For the committed value only, wire `.on_change` yourself and do not use `.model`. See [Events](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/components/events.md).

A text field writes back through its text state: when the new value equals the current one it is not replaced, so echoing one edit back to the signal leaves the caret and selection where they were. The `value` of `TextChanged` is converted to `String` and then `set`.

A `Select` signal is `Signal<Option<Arc<str>>>`. Selecting an item writes `Some(value of that item)`. The options themselves come from `.options(vec)`, each one a `SelectOption::new(value, label)`.

:::api

```rust view
use std::sync::Arc;

use nana_ui::runtime::view;
use nana_ui::runtime::SelectOption;

let options = vec![
    SelectOption::new("day", "By day"),
    SelectOption::new("week", "By week"),
];
view! {
    <Select options={options} v-model={period} />
}
```

```rust rust
use std::sync::Arc;

use nana_ui::runtime::view::select;
use nana_ui::runtime::SelectOption;

select()
    .options(vec![
        SelectOption::new("day", "By day"),
        SelectOption::new("week", "By week"),
    ])
    .model(period)
```

:::

`period` has the type `Signal<Option<Arc<str>>>`. The placeholder text comes from `.placeholder`, and it is not the current value.

A switch's and a checkbox's label is the accessible name. A switch with an empty label goes into a settings row so the row label names it. See [Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/accessibility.md).

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/components/slots">
    <p class="next-step-link">Named slots</p>
    <p class="next-step-caption">How dialogs, forms and menus hold their content.</p>
  </a>
  <a class="next-step" href="/guide/essentials/forms">
    <p class="next-step-link">Form bindings</p>
    <p class="next-step-caption">v-model and .model in one page of form fields.</p>
  </a>
  <a class="next-step" href="/guide/components/events">
    <p class="next-step-link">Events</p>
    <p class="next-step-caption">Why preview and commit are wired separately.</p>
  </a>
</div>
