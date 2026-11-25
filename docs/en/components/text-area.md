# TextArea

`text_area()` creates a multi-line input. The template tag is `<TextArea>`. It takes no constructor arguments; the initial text comes from `value`.

## Basic usage

The fields are `value: String`, `label: Option<Arc<str>>`, `placeholder: Arc<str>`, `disabled: bool`, `read_only: bool`. `model` binds `value`, converting `event.value` to `String` and writing it back.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <TextArea label="Notes" placeholder="Optional" v-model={notes} />
}
```

```rust rust
use nana_ui::runtime::view::text_area;

text_area().label("Notes").placeholder("Optional").model(notes)
```

:::

`v-model={notes}` expands to `.model(notes)`. `notes` is a `Signal<String>`. `value` writes as text state: when the string written back is the same as the current one, the text is not replaced and the cursor and selection stay where they were.

## Input

`.on_input(|event: &TextChanged| …)` listens to input yourself. In a template that is `on:TextChanged={|event: &TextChanged| …}`. The event also carries the selection.

`@input={save}` passes `save` through unchanged when it is a function, and the parameter must be `&TextChanged`. Other expressions expand to `move |_| { … }`, and the event is ignored.

Unlike `TextInput`, the multi-line control has no `on_submit`. Enter is a newline, not a submit.

## Read-only

`read_only` keeps focus, cursor, selection, find and copy, and rejects modification, replacement, cutting and pasting. Switching to read-only at runtime ends an in-flight input method composition and rejects a text drag that started before the switch. The program can still write new authoritative text. `disabled` takes no part in these interactions. To display read-only documents use `read_only`, not disabled styling as a substitute.

## Line numbers and undo

Line numbers, diagnostics, minimap and git gutter live on the `TextArea` component: `line_numbers`, `diagnostics`, `minimap`, `git_gutter`. They are not in the `<TextArea>` field table. Buffer revisions, LSP and git state are still fed by the application.

The optional `syntax-highlighting` feature enables a presenter named `"highlight"` on the same block, through `highlight(language)`, without building a second editor.

Editor text goes through `commit_editor_edit`. Typing, deleting and pasting all enter undo through it. Do not write `state.value` yourself; that does not enter undo. Document revisions, conflicts and persistence still belong to the application.

## Drag to resize

`resize_vertical(true)` on the `TextArea` component puts a grip in the bottom-right corner (showing `NsResize` on hover); dragging it changes only the height, emits no text events, and keeps the dragged height across declarative `reconcile` updates (clearing it when `resize_vertical` turns off). With `min_height` / `max_height` set the grip still shows, the dragged height stays inside the limits, and the edge follows the pointer again as soon as it comes back inside the range. A `content-box` box model needs an explicit pixel height (`height(px)`) to start from.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `value` | `String` | The initial text. `model` binds it, converting `event.value` to `String` and writing it back. When the string written back is the same as the current one, the text is not replaced and the cursor and selection stay where they were |
| `label` | `Option<Arc<str>>` | Field |
| `placeholder` | `Arc<str>` | Field |
| `disabled` | `bool` | Takes no part in the read-only interactions. Do not use disabled styling as a substitute when displaying read-only documents |
| `read_only` | `bool` | Keeps focus, cursor, selection, find and copy, and rejects modification, replacement, cutting and pasting |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `on_input`, template `on:TextChanged` or `@input` | `&TextChanged` | The type is `TextChanged`. The event also carries the selection. `save` is passed through unchanged when it is a function, and the parameter must be `&TextChanged`. Other expressions expand to `move |_| { … }`, and the event is ignored. There is no `on_submit` |

## Slots

There are no slots.

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
