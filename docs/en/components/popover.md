# Popover

`Popover` is a surface anchored to its own trigger. Press, focus, Enter, Space and the panel anchor are all this trigger. It toggles `open` itself and does not use `activate_overlay`.

There is no `<Popover>` in the control table.

## Basic usage

`Popover::new()` has empty trigger text, opens below, and is center-aligned.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ActionMenuItem, Popover};

view! {
    <Widget of={Popover::new().trigger("More")}>
        <Widget of={ActionMenuItem::new("Copy")} />
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ActionMenuItem, Popover};

widget(Popover::new().trigger("More")).children(widget(ActionMenuItem::new("Copy")))
```

:::

## Trigger

`.trigger(text)` writes the accessible name and is also the text a plain-text trigger shows.

`.trigger_icon(icon, label)` draws a 28×28 square button with the icon centered inside it, and `label` is the accessible name. The icon is the `LeadingContent` part published by layout (#205); changing `trigger_icon_size` publishes it again, and painting reads only this box.

`.bare_trigger(true)` removes the background and outline in the resting state. Hover, press and open still add a wash. When you do not pass it, the light background and outline stay. `ActionMenu` forwards it as well.

Do not put bare symbols into a text trigger.

For "icon + text + count", use the view's `.trigger(view)`, which corresponds to `Popover::trigger_content`.

That content is the trigger's own child node; it shows even while the panel is closed and does not count as a panel item.

Put display only inside it; do not place pressable controls there.

The trigger's width shrinks to its content, and the cross-axis alignment is decided by the parent: it is vertically centered in a centered horizontal toolbar, and inside a vertical container with the default stretch it does not fill the whole row either.

`.trigger_style(style)` changes the trigger's style. Fields written in `style` overlay the trigger's original style and fields not written stay as they were: writing only a width still lets hover and press change color, and the open look, `compact_trigger` tightening to the content, and the background removal of `bare_trigger` all remain. When you write a corner radius (`layout.border_radius` or per-corner `layout.paint.border_radii`) it is drawn as written instead of the theme's small radius; when you write `layout.height` the control height steps no longer apply; when you write horizontal or vertical padding the control padding steps no longer apply, and a text trigger's label is drawn inside your padding too (the label is the `TextContent` part published by layout, spanning the trigger minus the horizontal padding across and the whole trigger down, with measurement, shaping and painting reading the same box; painting used to always use the control step padding, so a smaller padding left the label too far right and squeezed narrow). This applies only to the trigger; the panel's width and padding are still set by `.width` and `.padding`. `ActionMenu` forwards it as well.

```rust
let mut round = NodeStyle::default();
let layout = round.layout_mut();
layout.width = Some(LengthSpec::Px(36.0));
layout.height = Some(LengthSpec::Px(36.0));
layout.border_radius = Some(18.0);
Popover::new().trigger_icon(Icon::Add, "More").trigger_style(round)
```

## Placement

`.placement` takes a `nana_ui::PopoverPlacement`: `Top`, `Bottom`, `Left`, `Right`, `Bottom` by default.

`.alignment` takes a `PopoverAlignment`.

When the preferred side does not fit and the opposite side does, it flips to the opposite side and is then pulled into the viewport.

After it opens, scrolling the page leaves the popup where it was until the next layout.

The surface is viewport-fixed and defaults to `isolate_from_ancestors = true`, so it does not inherit ancestor opacity, filters, blend modes, compositor layer opacity, or scroll edge fades, and the panel's overflow cannot clip it; the floating overlay keeps only its own open/close fade while the in-flow trigger remains subject to ancestor effects. Pass `.isolate_from_ancestors(false)` to inherit ancestor effects again.

## Closing

When the panel closes because of the trigger, Escape, a click outside, or your writing `open` as `false`, focus returns to the trigger if it was still on a panel item.

`.close_on_escape` and `.close_on_outside` are both on by default.

## Size

`.width` has a minimum width, and `.gap` and `.padding` collapse to 0 when below 0.

## Anchor

There is no "use an arbitrary node as the anchor" path; the anchor is the trigger itself.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `.trigger` | — | Writes the accessible name, and the text a plain-text trigger shows. `new()` has empty trigger text |
| `.trigger_icon` | — | `(icon, label)` draws a 28×28 square button with the icon centered; `label` is the accessible name |
| `.bare_trigger` | — | `true` removes the background and outline in the resting state. Hover, press and open still add a wash. When not passed, the light background and outline stay |
| `.trigger_style` | `NodeStyle` | Overlays the trigger's original style, replacing only the fields written; hover, press and size tightening are kept, and a written radius is drawn as written. `None` by default |
| `.trigger_icon_size` | `f32` | Glyph edge length (logical pixels) of an icon trigger, so it matches neighbouring tools set through `IconButton::icon_size`; the trigger's box does not change. `None` by default, which uses the standard glyph size |
| `.placement` | `nana_ui::PopoverPlacement` | `Top`, `Bottom`, `Left`, `Right`, `Bottom` by default. `new()` opens below |
| `.alignment` | `PopoverAlignment` | `new()` is center-aligned |
| `open` | — | Toggled by the control itself. Writing `false` closes it |
| `.close_on_escape` | — | On by default |
| `.close_on_outside` | — | On by default |
| `.width` | — | Has a minimum width |
| `.gap` | — | Collapses to 0 when below 0 |
| `.padding` | — | Collapses to 0 when below 0 |
| `.isolate_from_ancestors` | `true` | When `true`, isolates the floating overlay from ancestor opacity, filters, edge fades, compositor opacity, and overflow clips; pass `false` to inherit them |

## Events

The events are `PopoverToggled` and `PopoverClosed`.

| Event | Payload | Description |
| --- | --- | --- |
| `PopoverToggled` | — | An event of this page |
| `PopoverClosed` | — | An event of this page |

## Slots

`.children` are the items inside the panel.

| Slot | Description |
| --- | --- |
| `.children` | The items inside the panel |
| `.trigger(view)` | Corresponds to `Popover::trigger_content`. The trigger's own child node; it shows even while the panel is closed and does not count as a panel item. Put display only inside it; do not place pressable controls there |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
