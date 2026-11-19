# IconGlyph

`IconGlyph` draws one icon. It is decorative: it takes no pointer input and cannot be focused. To make an icon clickable, use `IconButton` and put the icon and accessible name on the button.

There is no `<IconGlyph>` in the control table.

## Basic usage

`IconGlyph::new(icon)` takes a `nana_ui::runtime::Icon`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Icon, IconGlyph};

view! {
    <Widget of={IconGlyph::new(Icon::Folder).size(16.0).role(nana_ui::runtime::SemanticColorRole::Text)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Icon, IconGlyph};

widget(IconGlyph::new(Icon::Folder).size(16.0).role(nana_ui::runtime::SemanticColorRole::Text))
```

:::

When one row needs both an icon and something clickable, keep `IconGlyph` in the display-only position and give clicks to the button next to it.

## Size

The default side is the icon size of a small control, and the foreground color is `Muted`.

`.size` changes the side. Values below 0 are clamped to 0; width, height, and minimum width and height are all set to this side, and it no longer flexes.

`.role` switches to another semantic color.

`.style` replaces the whole node style; size-dependent builder calls must come after it, or they are overwritten.

## Projection

When projected, the node text is an empty string.

The visual is `IconVisual` (`WidgetVisual`), with no tooltip.

The accessibility state stays at its default; screen readers do not treat it as a button.

## Where icons come from

Icons in the shell catalog are constants on the type, such as `Icon::Close`, `Icon::Add`, and `Icon::Folder`.

A product's own glyphs go through `icons-tabler` or `Icon::from_data`.

## Tooltips

`IconButton::with_tooltip` uses the default `TooltipConfig`. That is the button's own tooltip, not a field of this glyph control.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `icon` | `nana_ui::runtime::Icon` | Taken by `IconGlyph::new(icon)` |
| `.size` | — | Changes the side. Values below 0 are clamped to 0; width, height, and minimum width and height are all set to this side, and it no longer flexes. The default side is the icon size of a small control |
| `.role` | — | Switches to another semantic color. The default foreground color is `Muted` |
| `.style` | — | Replaces the whole node style; size-dependent builder calls must come after it, or they are overwritten |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| None | None | Takes no pointer input and cannot be focused |

## Slots

| Slot | Description |
| --- | --- |
| None | None |

## See also

[Overview](index.md) · [Controls](../reference/components.md)
