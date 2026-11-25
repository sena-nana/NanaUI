# PropertyGrid

`PropertyGrid` is the dense two-column panel you find in professional tool inspectors: rows are grouped into collapsible sections, with a label on the left of each row and the application's control on the right. The whole grid shares one label column width, which can be dragged. It has no template tag; put it in a view with `widget(PropertyGrid::new(sections))`.

This type is behind the Cargo feature `property-grid` (included in `components`).

## Basic usage

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{PropertyGrid, PropertyRow, PropertySection, ScrubField};

widget(PropertyGrid::new([
    PropertySection::new("transform", "Transform", [
        PropertyRow::new("x", "X").mixed(true),
        PropertyRow::new("angle", "Angle").disabled(Some("Controlled by the parent transformer")),
    ]),
    PropertySection::new("draw", "Draw", [PropertyRow::new("opacity", "Opacity")]),
]))
.children((
    widget(ScrubField::new(12.0).mixed(true)).key("x"),
    widget(ScrubField::new(0.0).unit("°")).key("angle"),
    widget(ScrubField::new(1.0).range(0.0, 1.0)).key("opacity"),
))
```

A child node in the view keyed by a row id is that row's control, such as `ScrubField`, `Select` or `ColorField`. A control whose row is no longer in the model is hidden and waits rather than being destroyed.

## Sections

- Pressing a section header collapses or expands that section and emits `PropertySectionToggled { id, collapsed }`.
- `collapsed_sections()` returns the currently collapsed sections. The application saves it together with the label column width, and restores them next time with `PropertySection::collapsed(true)` and `.label_width(width)`.
- When the view re-renders, sections the user collapsed and the column width they dragged are not overwritten.

## Label column

The label column is 96 px wide by default, and all rows share that width. A 6 px drag handle sits on the right edge of the label column; dragging it horizontally changes the label width of every row at once; the label column and the control column each keep at least 48 px. Releasing emits `PropertyLabelWidthChanged { width }`.

## Mixed values and disabled

- `.mixed(true)`: with a multi-selection, the objects' values differ. The label switches to the body text color, and a screen reader reads "mixed values". The control shows the mixed state itself; for example `ScrubField::mixed` shows "—".
- `.disabled(Some(reason))`: the control becomes semi-transparent, covered by a transparent mask so the pointer cannot reach it. The reason is shown below that row in small, subdued text, and a screen reader reads it as the description. To disable without an explanation, write `.disabled(None::<&str>)`.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `PropertyGrid::new(sections)` | `PropertySection` | Sections and rows |
| `.label_width(px)` | `f32` | Label column width, 96 by default |
| `PropertySection::new(id, title, rows)` | — | One section |
| `.collapsed(bool)` | `bool` | Whether the section is collapsed |
| `PropertyRow::new(id, label)` | — | One row; the control is a child node keyed by `id` |
| `.mixed(bool)` | `bool` | Mixed value |
| `.disabled(reason)` | `Option<text>` | Disabled, with the reason |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `PropertySectionToggled` | `id`, `collapsed` | A section was collapsed or expanded |
| `PropertyLabelWidthChanged` | `width` | Dragging the label column ended |

## Slots

| Slot | Description |
| --- | --- |
| Child nodes keyed by row id | The control for that row |

## See also

[ScrubField](https://github.com/sena-nana/NanaUI/blob/main/docs/components/scrub-field.md) · [DynamicForm](https://github.com/sena-nana/NanaUI/blob/main/docs/components/dynamic-form.md) · [Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
