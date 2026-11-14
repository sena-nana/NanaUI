# ColorField

`ColorField` is a swatch plus a hex value. It is a leaf compound: when you write its properties, `assemble_color_field` builds the swatch, the hex input and the HSV picker itself, so you do not assemble it a second time. The picker is existing controls: `XYPad` for saturation / value and `RangeField` for hue, wrapped in its own `Popover` below the swatch.

## Frame

The whole field has one frame, the way Ant Design's ColorPicker and Mantine's ColorInput do it.

- The frame uses the text field's background and border, and its height is exactly one control height (decided by `.size`).
- The swatch sits inside the frame, with 3px on the top, bottom and left edges. Its edge is the control height minus the border and the padding on both sides, so Small is 20 and Medium is 24. The radius is the frame's radius (`RadiusTier::Sm`) scaled down by the ratio of the swatch's edge to the frame's height, so the swatch is a proportional copy of the frame and follows a theme's radius. The swatch's trigger uses `Popover::compact_trigger`, exactly the swatch's size, with no button padding.
- The hex input has no background, border or radius of its own; it fills the inside of the frame vertically and keeps the text field's padding on the right.
- While focus is in the field (hex or swatch), the frame paints a focus border, as `:focus-within` does. With `.invalid` the frame paints a Danger border, and with `.disabled` the frame switches to the Subtle background.

## What the picker looks like

The picker paints the color it takes its value from. Saturation / value is a picture pad (`XYPadSurface::Picture`, twice the height of a large pad): the background is the solid color of the current hue, a white layer fades out from left to right above it (saturation), then a black layer fades in from top to bottom above that (value) — the standard HSV square; when the hue changes, the square changes its background with it. Hue is a spectrum bar without a readout (`RangeField::track_gradient(hue_spectrum())`, 0 to 360 degrees: red, yellow, green, cyan, blue, magenta, back to red). Both mark the current value with a contrasting ring, with no crosshair axes and no accent fill. The hue bar is still a screen-reader `Slider` and can be stepped with the keyboard. When the host supports screen color sampling and `.eyedropper(false)` has not been set, an eyedropper `IconButton` sits to the left of the hue bar (`[ eyedropper | hue ]`); clicking it immediately closes the popover and invokes the platform screen color sampler, writing the picked sRGB color back while preserving the field's alpha (cancelling via Esc or right-click leaves the color unchanged).

The control table has no `<ColorField>`.

## Basic usage

The host cannot get the window handle.

`value` of `ColorField::new(value)` is RGBA in `0..=1`.

At construction it is clamped to a valid color, and `hue`, `sat` and `val` are computed.

`.size`, `.disabled` and `.invalid` are the same as for field controls.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ColorField;

view! {
    <Widget of={ColorField::new([0.29, 0.57, 0.85, 1.0]).label("Accent color")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::ColorField;

widget(ColorField::new([0.29, 0.57, 0.85, 1.0]).label("Accent color"))
```

:::

## Color values

All four RGBA components are in `0..=1`.

A change to the hex box's text writes back to `value`.

When the same color is written again, the open state and the HSV cursor stay where they are during the drag; only when the color changes is it recomputed from the new RGB.

When the same color is written again, the HSV cursor stays where it is during the drag; only when the color changes is it recomputed from the new RGB.

## Opening and closing

`.opened` is forced closed while disabled.

Opening and closing are written on the field's own `opened`.

Do not touch the `Popover` that assembly built; the next assembly overwrites it.

A click on the swatch toggles `opened`.

While disabled, clicking the swatch does nothing, and the picker stays closed.

## Assembly

A field built by `create_component` also runs `assemble_color_field` after a write.

A leaf compound's assembly hangs on writes: when you change `value`, `label` or `opened`, `assemble_color_field` rebuilds the swatch, the hex and the picker.

This differs from the Shell, which waits for its slots to be placed and then calls `assemble_*` explicitly.

Child identity is held by the runtime.

## Accessibility

The default accessible name is "Color"; use `.label` to replace it with your own copy.

The default accessible name is "Color".

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `value` | — | `value` of `ColorField::new(value)` is RGBA in `0..=1`. At construction it is clamped to a valid color, and `hue`, `sat` and `val` are computed. A change to the hex box's text writes back to `value` |
| `.label` | — | The default accessible name is "Color"; use `.label` to replace it with your own copy |
| `.size` | — | The same as for field controls |
| `.disabled` | — | The same as for field controls. While disabled, `.opened` is forced closed, clicking the swatch does nothing, and the picker stays closed |
| `.invalid` | — | The same as for field controls |
| `.opened` | — | Opening and closing are written on the field's own `opened`. A click on the swatch toggles it. Forced closed while disabled |
| `.eyedropper` | `bool` | Defaults to `true`; shows the screen eyedropper button to the left of the hue bar when the host supports screen color sampling, and hides automatically on unsupported platforms (such as iOS or Wayland without a portal) |

## Events

A commit emits `ColorChanged { value }`; a drag in progress emits `ColorInput { value }`.

Read `ColorChanged` for a commit, `ColorInput` for a preview during a drag.

| Event | Payload | Description |
| --- | --- | --- |
| `ColorChanged` | `{ value }` | A commit emits `ColorChanged { value }` |
| `ColorInput` | `{ value }` | A drag in progress emits `ColorInput { value }`. Read `ColorInput` for a preview during a drag |

## Slots

| Slot | Description |
| --- | --- |
| None | Child identity is held by the runtime. Do not touch the `Popover` that assembly built; the next assembly overwrites it |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
