# SplitPane

`SplitPane` is two pieces of content plus an 8px drag handle. Its size comes from `nana_ui::runtime::SplitPaneModel`. Your content stays in the `first` and `second` slots.

## Basic usage

The handle takes no part in the content's layout, and the geometry has one writer. The control table has no `<SplitPane>`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::SplitAxis;
use nana_ui::runtime::{SplitPane, SplitPaneModel};

let model = SplitPaneModel::new(SplitAxis::Horizontal, 240.0, 120.0, 800.0);
view! {
    <Widget of={SplitPane::new(&model).surface(nana_ui::runtime::SemanticColorRole::Surface)}>
        <template #first>
            <Column />
        </template>
        <template #second>
            <Column />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::SplitAxis;
use nana_ui::runtime::{SplitPane, SplitPaneModel};

let model = SplitPaneModel::new(SplitAxis::Horizontal, 240.0, 120.0, 800.0);
widget(SplitPane::new(&model).surface(nana_ui::runtime::SemanticColorRole::Surface))
    .first(column())
    .second(column())
```

:::

## Model

In `SplitPaneModel::new(axis, default_size, min_size, max_size)`, `axis` is `nana_ui::SplitAxis::Horizontal` or `Vertical`.

`SplitPane::new(model)` starts with no content, and the view then puts it in with `.first(view)` and `.second(view)`. `SplitPane::from_model(model, first, second)` takes the two node ids directly.

`.apply(mutation)` pushes a `SplitPaneMutation` into the model. Both handle hit testing and the keyboard write into the model. To remember the width the user dragged to, read the model and hand it back on the next `from_model` or `apply`.

## Surface and handle

`.surface(role)` writes the semantic background, which is preserved across re-projection. `.handle` can be replaced with your own handle node; its first child is the 2px indicator bar.

## Assembly

`assemble_split_pane` is the `slot_assembler`. It wraps each slot in a shell, so a `ScrollView` inside the slot keeps projecting its own nodes and does not fight the split for size. It runs when the view is committed. After placing both pieces of content by hand, call it once yourself. Changing the model does not re-assemble automatically each time.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `axis` | `nana_ui::SplitAxis` | `Horizontal` or `Vertical`. The first argument of `SplitPaneModel::new` |
| `default_size` | — | An argument of `SplitPaneModel::new` |
| `min_size` | — | An argument of `SplitPaneModel::new` |
| `max_size` | — | An argument of `SplitPaneModel::new` |
| `.surface` | — | `role`. The semantic background, preserved across re-projection |
| `.apply` | `SplitPaneMutation` | Pushes the mutation into the model |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| None | None | Both handle hit testing and the keyboard write into the model |

## Slots

| Slot | Description |
| --- | --- |
| `.first` | The first piece of content. Absent at `new`. Template `#first`. `from_model` takes the node id directly |
| `.second` | The second piece of content. Template `#second` |
| `.handle` | Can be replaced with your own handle node; its first child is the 2px indicator bar |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md) · [Workspace](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/workspace.md)
