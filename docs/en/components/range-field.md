# RangeField

The template tag is `<Slider>`, the function is `slider(min, max, step)`, and the type is `RangeField`. All three arguments are `f64`. The start is `min`: the function uses `RangeField::new(minimum, minimum, maximum, step)` internally.

## Basic usage

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Slider min=0 max=100 step=1 label="Volume" v-model={volume} />
}
```

```rust rust
use nana_ui::runtime::view::slider;

slider(0.0, 100.0, 1.0).label("Volume").model(volume)
```

:::

`min=0` in a template expands to `0_f64` with the suffix. In hand-written code the arguments are already `f64`, so write `0.0`.

`v-model={volume}` expands to `.model(volume)`. `volume` is a `Signal<f64>`. The preview during a drag is written back to it too, because the model listens to `RangeInput`.

## Visible values and commit

`RangeInput` is every visible value, including the preview while dragging, so it suits live preview. `RangeChanged` is a commit: the pointer lifts and the value changed, keyboard stepping, accessibility `SetValue`, `set_range_value`, or typing and restoring the default on a value-thumb slider. A cancelled drag does not commit. Every keyboard and accessibility step is a commit.

## Listening

`.on_input(|event: &RangeInput| …)` and `.on_change(|event: &RangeChanged| …)` listen to the two. In templates write `on:RangeInput={…}`, `on:RangeChanged={…}`.

`@input={save}` passes `save` straight through when it is a function, and the parameter must be `&RangeInput`. `@change={save}` likewise must be `&RangeChanged`. Other expressions expand to `move |_| { … }` and the event is ignored.

## Dragging

Dragging has its own start and end event, `RangeDragging { dragging }`. Keyboard and accessibility stepping do not count as dragging. It is not one of the two events above; use `.on(|event: &RangeDragging| …)`.

## Value and label on the track

By default the current value and unit are drawn beside the track. `show_value(false)` leaves only the track, and screen readers still read the value. `show_label(false)` draws no label and the track fills the control, while the label remains the screen-reader name; in a template it is `<Slider label="Volume" show_label={false}>`, and on `slider(..)` it is `.show_label(false)`. `show_value` is not in `<Slider>`'s field table, and `slider` has no method of that name either. To change it, call `show_value(false)` on the `RangeField`, or `.bind(|field| field.show_value = false)`.

## Thin rail

`rail(thickness)` draws the slider as a thin rail: only a track of that thickness and its filled part, spanning the whole control, with no label, value or field padding; the dot appears only when keyboard focus (focus-visible) lands on it. The control's own height does not change, so a 2px track can sit in a 16px hit area and the pointer picks values across the control's whole width. The label stays the screen-reader name. A thickness that is not a finite positive number keeps the regular look. The media bar's `Mini` density uses it for progress and volume.

```rust
let mut seek = RangeField::new(0.0, 0.0, 100.0, 1.0).label("Progress").rail(2.0);
Arc::make_mut(&mut seek.style.layout).height = Some(LengthSpec::Px(16.0));
```

## Gradient track

`track_gradient(stops)` draws the track as a gradient bar running from the minimum end to the maximum end through those stops, for scales where the value itself is a color, such as a hue bar or an opacity bar. A gradient bar draws no fill and adds no field padding; the bar is 12 wide, and the dot becomes a contrasting ring (a light ring with two thin dark edges, hollow in the middle to show the color it marks) that stands one ring above the bar, so it stays legible on any stop. The ring uses the theme's media foreground and media scrim colors, and is a light ring with dark edges in both light and dark mode. The track insets by half a ring at each end, so the ring stays inside the control at both ends too. The label, readout, keyboard stepping, dragging and screen-reader `Slider` semantics are unchanged; for a bare color bar, pair it with `show_value(false)`. Fewer than two stops keeps the regular look; the painter reads only the first eight stops, like any linear gradient. When `rail` is also set, the thin rail wins. `hue_spectrum()` is the 0-to-360-degree hue wheel unfolded (red, yellow, green, cyan, blue, magenta, back to red), and `ColorField`'s hue bar is it.

```rust
use nana_ui::runtime::{RangeField, hue_spectrum};

RangeField::new(200.0, 0.0, 360.0, 1.0)
    .label("Hue")
    .show_label(false)
    .show_value(false)
    .track_gradient(hue_spectrum())
```

## Value-thumb slider

`value_thumb(true)` writes the readout into the thumb: the thumb is a solid Accent capsule in `Full` radius, the readout (the number only, no unit, tabular figures) is centered inside it in the text color on Accent, and the track is a thin line filled up to the capsule. No separate readout is drawn on the right, and no readout slot is reserved. With `show_bounds(true)` and `default_value(Some(default))`, one row is:

```text
Min  ━━━━[ 137 ]━━━━  Max unit  ↺
```

```rust
use nana_ui::runtime::RangeField;

RangeField::new(137.0, 0.0, 200.0, 1.0)
    .label("Zoom")
    .show_label(false)
    .unit("%")
    .value_thumb(true)
    .show_bounds(true)
    .default_value(Some(100.0))
```

In a view it is `slider(0.0, 200.0, 1.0).value_thumb(true).show_bounds(true).default_value(default)`, and `default_value` can bind a `Signal<Option<f64>>`; the template is `<Slider value_thumb={true} show_bounds={true} default_value={Some(100.0)}>`.

- **Capsule.** Its height is the control's text line height plus one `sm` step (16 + 6 = 22 at the regular size), capped by the content box; its width is the widest of the minimum, maximum and current value (tabular figures) plus `md` on each side, at least two `page` (48), so the width does not change while dragging. The capsule center travels the track inset by half a capsule at each end, so the capsule always stays inside the track. The track breaks by one `xxs` step on each side of the capsule, showing the background through, so it stays visible over the fill. When disabled the capsule uses the disabled background and the text uses the disabled text color. When keyboard focus is visible, the focus ring follows the capsule's outer edge.
- **Component geometry.** `RangeGeometry` (via `UiWorld::range_geometry`)'s `value_thumb: Option<RangeValueThumb>` gives the capsule box (`capsule`), the readout (`text`), the break width (`gap`) and whether typing is active (`editing`); `track` is still the slider center's travel, which pointer value picking uses.
- **Dragging.** Pressing on the capsule starts a drag without jumping the value: the value starts from the value at press time and follows the pointer displacement. Pressing elsewhere on the track first jumps to that point and then drags relative to it.
- **Typing.** Pressing on the capsule and releasing after moving less than 2px, or pressing Enter while the slider has focus (Space and the screen reader's press also count), enters typing: the `NumberInput` that the slider assembles covers the capsule's position (`xl` wider than the capsule on each side), shows the current value, selects it all and takes focus, and the capsule steps aside. Enter or focus leaving commits: clamp to the range, snap to `step`, and if the value changed emit `RangeInput` and `RangeChanged`. Escape restores and emits nothing. When it ends, focus returns to the slider. `editing` says whether typing is active; `begin_range_edit(slider)` enters typing from code.

When it is placed in a view, the view assembles the reset-to-default button and the `NumberInput` used for typing, and every later write to the slider syncs them from its properties. When you create it by hand with `create_component`, call `assemble_range_field(slider)` once (or it assembles automatically after the first write). Both child nodes are absolutely positioned inside the slider and take no part in the row's layout; while they exist, the slider is their containing block.

## Bounds

`show_bounds(true)` writes the minimum before the track's left end and the maximum after its right end, at the same precision as the readout, in the caption size, muted color and tabular figures. The unit follows only the maximum: when the unit starts with a letter, one space separates it (`2.0 s`, `10 px`); when it starts with a symbol, it follows the number directly (`200%`, `90°`). The two are the layout parts `LayoutPartKind::Minimum` and `LayoutPartKind::Maximum`, with the track between them; with no label (`show_label(false)`) the minimum starts at the content box's start. When it is not a value-thumb slider, the right-hand readout still follows the maximum.

## Reset to default

`default_value(Some(value))` puts a compact 24×24 icon button at the end of the row (`IconButton::tool`, icon `Icon::Reset`), whose screen-reader name is the framework string `range_field.reset` (default "Reset to default"). The button is an independent screen-reader `Button` node and can be clicked by name. When the value equals the default (after snapping to `step`), the button is disabled: dimmed, not clickable, not focusable. A click is the same as `set_range_value(default)`: it emits `RangeInput` and `RangeChanged`. The button sits at the row end's `LayoutPartKind::TrailingContent` part. `default_value(None)` removes the button.

## Fine dragging

All sliders multiply the value change converted from pointer displacement by 0.1 while Alt (Option on macOS) is held during a drag. Pressing or releasing Alt mid-drag does not jump the value: at the switch it re-anchors on the pointer position and value at that moment, and dragging afterwards is relative to the displacement, never jumping back to the position under the pointer. Ordinary dragging without Alt still takes the value from the pointer position (including tick snapping). In code it is `update_range_drag_fine(document, pointer, x, fine)`; `update_range_drag` equals `fine = false`.

## Screen readers

When nobody names the slider and it is placed in a settings row, screen readers use the row label. When it has its own `label`, they use that. When there is a separate visible heading beside it, `labelled_by={caption}` (`slider(..).labelled_by(caption)`) uses that heading's text.

## Bounds and paging

`RangeField::new` repairs inconsistent bounds: non-finite or reversed ones collapse to `0.0..=1.0`, an illegal `step` collapses to one hundredth of the span, and the value is then clamped and snapped to the step. `slider` goes through this constructor. The page step defaults to ten times `step`; change it on the component with `page_step`.

## Properties

Fields are `value: f64`, `label: Option<Arc<str>>`, `unit: Option<Arc<str>>`, `disabled: bool`. `model` binds `value`, with `RangeInput` as the source, not a commit.

| Property | Type | Description |
| --- | --- | --- |
| `value` | `f64` | `model` binds this field, with `RangeInput` as the source, not a commit |
| `label` | `Option<Arc<str>>` | Screen readers use it when the slider has its own label. When nobody names it and it is placed in a settings row, they use the row label |
| `disabled` | `bool` | Field |
| `unit` | `Option<Arc<str>>` | The unit after the readout; screen readers read it after the value. When it follows the language, bind it in the view (`.unit(..)`), and the label no longer writes the unit in parentheses |
| `min` / `max` / `step` | `f64` | The three arguments of `slider`. The start is `min`; internally it is `RangeField::new(minimum, minimum, maximum, step)` |
| `show_value` | — | Not in `<Slider>`'s field table, and `slider` has no method of that name. By default the current value and unit are drawn beside the track; `false` leaves only the track, and screen readers still read the value |
| `show_label` | `bool` | `false` draws no label and the track fills the control, while the label remains the screen-reader name |
| `page_step` | — | Page step, ten times `step` by default |
| `rail` | `Option<f32>` | `rail(thickness)` draws only a thin rail of that thickness, spanning the whole control, with no label, value or padding, and the dot appears only when focus is visible; the control's height is the hit area |
| `track_gradient` | `Option<Arc<[GradientStop]>>` | `track_gradient(stops)` draws the track as a gradient bar: no fill or field padding, the dot becomes a contrasting ring, semantics unchanged. Fewer than two stops keeps the regular look; `rail` wins |
| `caution_from` | `Option<f64>` | `caution_from(value)` marks the track from that value to the maximum as a caution zone: its stretch of the rail and of the fill takes the theme's Warning color (solid on the rail as on the fill; the thumb marks where the value is). While the value is inside the zone the thumb (capsule or dot) takes the Warning color in place of the accent, and a capsule's readout takes whichever of the accent text color and the body text color contrasts more with it; a disabled range keeps its disabled thumb. A `markers` entry inside the zone, the one where it begins included, is drawn in the Warning color too. The readout, stepping and screen-reader semantics are unchanged and the value is not stopped at its edge; pair it with a `markers` entry there for a stop. A value outside the range or at the maximum marks nothing; `rail` and `track_gradient` win. The geometry is `caution: Option<RangeCaution>` on `RangeGeometry`. Builder only; not in `<Slider>`'s field table |
| `value_thumb` | `bool` | `value_thumb(true)` writes the readout into a wide capsule thumb and draws no right-hand readout; dragging on the capsule does not jump the value, and a click or Enter types. `rail` and `track_gradient` win. In `<Slider>`'s field table, same name on `slider` |
| `show_bounds` | `bool` | `show_bounds(true)` writes the minimum and the unit-bearing maximum at the track's ends. In `<Slider>`'s field table |
| `default_value` | `Option<f64>` | `Some(value)` puts a reset-to-default button at the row end, disabled when the value equals it; a non-finite value counts as `None`. In `<Slider>`'s field table, bindable |
| `editing` | `bool` | The value-thumb slider is typing. Read-only state, maintained by the start and end of typing |

## Events

There are two events, and both handlers take an event reference: `on_input` is `RangeInput`, `on_change` is `RangeChanged`.

| Event | Payload | Description |
| --- | --- | --- |
| `on_input`, template `on:RangeInput` or `@input` | `&RangeInput` | Every visible value, including the preview while dragging. A function is passed as is, and its parameter must be this reference; other expressions expand to `move \|\| { … }` and the event is ignored |
| `on_change`, template `on:RangeChanged` or `@change` | `&RangeChanged` | A commit: the pointer lifts and the value changed, keyboard stepping, accessibility `SetValue`, `set_range_value`, Enter or focus loss after typing on the value-thumb slider, or the reset button. A cancelled drag and Escape do not commit. Every keyboard and accessibility step is a commit |
| `.on` listening to `RangeDragging` | `&RangeDragging` | `{ dragging }`. The start and end of dragging. Keyboard and accessibility stepping do not count as dragging. It is not one of the two events above |

## Slots

| Slot | Description |
| --- | --- |
| None | The reset-to-default button and the `NumberInput` used for typing are assembled by the slider itself |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
