# SettingsRow

`SettingsRow` is one row of a settings page: label and hint on the left, your control on the right. The actual setting values, such as theme, account, and paths, are still owned by your state.

## Basic usage

There is no `<SettingsRow>` in the control table. The outer layer is still `settings_row`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::settings_row;

settings_row("Mute").control(view! {
    <Switch checked={false}></Switch>
})
```

```rust rust
use nana_ui::runtime::view::{settings_row, switch};

settings_row("Mute").control(switch("").checked(false))
```

:::

## Arrangement

`SettingsRow::new(label)` is horizontal by default and does not wrap by width. `.hint` goes below the label. `.stacked(true)` always stacks vertically.

In the horizontal arrangement the control is vertically centered against the whole block of copy, including when there is a hint below the label: the control faces the combined center line of label and hint, so the visual centers of both sides match. What stands under the row (a hint opened from the help button, the notice line) does not count: the control still faces the name's line.

`.stack_below(width)` wraps by the row's actual layout width: narrower than the threshold it stacks, equal or wider it stays on one line. An invalid threshold counts as off. The control node is not rebuilt; width changes are handled by layout reflow. The example in the reference uses `480.0` as the application's own threshold.

## Hint placement

Where the hint goes depends on how the row is used, so the container that holds the rows decides, not each row:

- A settings page: visited seldom, read when visited. The hint goes under the name (`HintPlacement::Below`, the default).
- An inspector card or a form: visited often, glanced at while adjusting. The hint goes behind a help button after the name (`HintPlacement::Help`), and the row stays one line tall.

Write `hint_placement` on a `SettingsGroup` or a `SettingsCard` and every row inside follows it; in views, `settings_group(..)` and `group_block(..)` have a method of the same name. A container reaches every `SettingsRow` inside it, through content lines, keyed lists, the containers of `when` / `dynamic` and other groups alike; only a group or card that writes a placement of its own takes over its own part. For a card holding several groups, write it once on the card (`SettingsCard`) or on the outermost group. A row's own `.hint_placement(..)` wins over its container's; with no container writing one it is `Below`. When a container changes its placement, the rows inside follow.

```rust
use nana_ui::runtime::HintPlacement;
use nana_ui::runtime::view::{group_row, settings_group, settings_row};

settings_group(Some("Appearance".into()), vec![
    group_row(settings_row("Opacity").hint("Only affects the picture in the window, not the outputs").control(opacity)),
    group_row(
        settings_row("Cache folder")
            .hint("Optimized models are stored here")
            .notice("Takes effect after a restart")
            .control(folder),
    ),
])
.inset()
.hint_placement(HintPlacement::Help)
```

## Help button

With `Help`, a row with a hint puts a help button (`HELP_CIRCLE`) after its name, before any label accessory; a row without a hint gets none. The glyph is as tall as the body text, and the button's target is a heading tool's square (24×24 by default) hanging out of the name's line on every side, so it does not make the row taller. The button's accessible name is the hint itself.

- Hovering the button with a mouse: a small card previews the whole hint.
- Pressing it (mouse and touch alike): the hint opens in place under the row, in the same style as a hint under the name, across the content line; pressing again closes it. While it is open no preview appears. Touch has no hover, so on touch pressing is how it opens.
- Keyboard: the focus can stop on the button, Enter or Space opens and closes the hint, and a screen reader hears whether it is open (`expanded`).

Whether it is open is the row's own state, kept by the framework; the application does not hold it.

## Notice line

`.notice(text)` puts a line under the row about its value: something to be aware of (`ValidationIntent::Warning`, the default) or a problem (`.notice_tone(ValidationIntent::Danger)`). The notice belongs to the row: it is drawn under the name's line and above the hairline of `.divided(true)`, spans the content line, and lines up with the row's content at the start; it carries an icon in the semantic color (a warning triangle, or a circled cross for a problem), and a faint tint of the semantic color; a problem’s text is in the danger color too, a warning’s in the body text color (a warning color is often a bright yellow, which small text on its own tint does not read in), with the control radius (`RadiusTier::Sm`). An opened hint stands above the notice. `None` or an empty string takes no room and leaves no gap.

When a notice shows, the control at the end of the row still faces the name's line (with a hint under the name, the combined center line of name and hint), not the whole including the notice.

A Warning notice is read as the row's description; a Danger one, like `.error`, makes the row read as invalid with the notice as its screen reader value.

## Threshold of the view function

The view function `settings_row(label)` first does `SettingsRow::new("")`, then `stack_below(280.0)`, and then writes the label. 280 is the threshold used by assembled rows.

## Groups

`.divided(true)` draws a hairline below the row; do not draw it on the last row of a group, since the group's own edge closes it. When placed in a `SettingsGroup` directly after other content (key-value pairs, lists, hints), the group also has it draw a line above, so the boundary between a settings row and other content has a line on both sides. `.first_in_group` and `.last_in_group` mark the two ends of the group. `.loose` increases spacing.

Which rows are the ends depends on which rows are shown at the moment. Put the rows in a `SettingsGroup` (`widget(SettingsGroup::new()).children(..)`) and let it decide: whenever a member's visibility changes, the group finds the first and last shown members and writes `first_in_group` / `last_in_group` on the `SettingsRow`s placed directly in the group; rows in the group should not bind these two fields themselves. Other members (badges, values, a row of buttons) take positions too; they are just not written. Keyed lists and the containers of `when` / `dynamic` are seen through, and every row inside counts as a member. A `Divider` in the group is a separator: it shows only between two shown members.

## Control name

`.control(view)` puts the control after the copy. When the control has a non-empty name of its own, such as `switch("Mute")`, that name is used; when the name is empty, such as `switch("")`, segmented controls, sliders, and dropdowns, the row label is used. Assembly links the two through `set_labelled_by`. When the row label changes, the control's accessibility node is projected again.

## Label mark and accessory

`.label_mark(view)` places a category mark immediately after the label text, before the `Help` button and `.label_accessory(view)`; `.label_accessory(view)` puts a status pill or inline action after the help button on the same line, such as "Watermark removal [Not purchased]".

```rust
settings_row("Watermark removal")
    .label_mark(dlc_seal())
    .label_accessory(status_badge("Not purchased"))
    .control(button("Buy"))
```

The mark, help button, and accessory share the label line, separated by `space::XS` and vertically centered with the label. When the row narrows, the label yields first: it narrows and elides, while the mark and accessory keep their own widths. This holds for both horizontal and stacked arrangements. The row's accessible name is still the label; the mark and accessory are read by their own content.

A row with no mark, help button, or accessory is the same as before: the label sits directly in the copy, with no extra node layer. Only when at least one is present does the label get wrapped in a row. Make a slot's root a `when(..)` to show and hide it with state; for a hand-built row, set `label_mark` / `label_accessory` to `None` and then call `assemble_settings_row`, and the label goes back into the copy and the wrapping row is removed along with it.

## Label column (inspector layout)

`.label_column(Some(width))` (`SettingsRow::label_column(width)` for hand-built rows) switches the row to an inspector layout: the label takes a column of fixed width, the control starts at the column's right edge and grows to the end of the row, and the row is only as tall as the control, with no bands above or below. When several rows use the same width, their controls align on the same vertical line, even across cards. A label in this column is the secondary half of the row: `Muted` color, regular weight, so the values read more prominently. A label longer than the column is elided.

```rust
settings_row("Opacity")
    .label_column(Some(96.0))
    .control(scrub)
    .trailing(button("Reset to default"))
```

For the control to fill, set its width to `Fill` yourself; otherwise it uses its own width, starting at the column edge. `.stack_below` applies as usual: when the row is narrower than the threshold, the control moves below the label and fills, separated by only `space::XXS`. `None` is the original settings-page layout.

`.trailing(view)` (`trailing_child` for hand-built rows) puts the row's own action after the control on the same line, separated by `space::XXS`. When it fits, it keeps the width it asks for and the control yields the rest; when it does not fit, the trailing node yields, and the control keeps at least three control heights (the installed theme's `ControlSize::Medium`: 96 px in the default theme, 72 px in the compact theme), so even a trailing container that wants to fill the width (such as a plain `Stack::row`) cannot squeeze the control out. With a label column or a trailing node, the control and the trailing node go into one box; with a trailing node, each also sits in its own box (`settings-row-value` / `settings-row-trailing`). Once both are removed, the control returns to the row and the boxes are removed with them. The row label still names the control.

## Flashing

When the user jumps to a row from elsewhere, the application scrolls the page to that row and then calls `AppContext::flash_settings_row(row)` to make it flash: a faint accent background appears behind the row, holds for a moment, then fades out, leaving nothing behind.

```rust
let row = entity_ref::<SettingsRow>();
// In the view: settings_row("Watermark removal").entity_ref(row)
if let Some(row) = row.get() {
    cx.flash_settings_row(row)?;
}
```

The background uses the theme's `Selected` (the opaque light background of a selected item) and `Sm` corner radius; `AccentSoft` on a translucent theme laid over a group's tinted background is nearly invisible. The row has no horizontal padding of its own, so the background extends outward on each side by one list item padding (`ControlPadding::ListItem`), not touching the text, the same as a selected list row; vertically it covers the whole row. It is a node in the row that takes no part in layout, receives no pointer, and is invisible to screen readers, so it does not change the row's layout or hit testing.

The animation changes its opacity on the presentation layer, starting from the frame after the call (the frame that scrolls there is the frame it appears). The total duration reads the installed theme's `MotionRole::Attention` (1.5 seconds by default): for the first `MotionTokens::attention_hold` (default 0.2, one fifth) it holds full color, then fades out with the theme's standard easing. With reduced motion (`reduced_motion`) it does not fade: it holds full color for the same duration and is then removed. Calling again while it is still flashing restarts it. If the row is removed first, the background and its timeline go with it.

## Assembly

`assemble_settings_row` is a `slot_assembler`. It runs when the view commits; do not call it again in the view. After placing the control slot by hand, call it once yourself.

## Hint

`mount_settings_leaf_row` keeps the label, hint, and control slot. Even with no hint at first, you can later `update_component` to write `hint` as `Some`; writing `None` hides it.

## Error

`.error(reason)` says what is wrong with this row's value: a required field is empty, a port is in use, an identity code is incorrect. The reason is drawn on the notice line under the row, as a Danger notice; with a `.notice` as well, the error shows first. The hint under the name stays where it is; after writing `None` back, the notice line goes back to `.notice`, or folds away when there is none.

The row itself reads as invalid, and the screen reader value is the reason. The control's own red border is governed by the control's `invalid`; the row only says why: write both.

The notice line is built the first time there is a notice or an error and kept (hidden) after they clear, so the next one does not rebuild it.

When a reason newly appears on an existing row (from none to some), and focus is in the same document (for example on the submit button just clicked), focus moves to this row's control: the user lands where the change is needed, and the screen reader reads the field and the reason. A row built with an error, focus in another window, or only rewording the reason do not move focus.

## Workspace

If settings use a separate `WorkspaceController`, it does not override the main workspace's sizes and collapsed states.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `label` | — | `SettingsRow::new(label)`. Horizontal by default, no wrapping by width. The view function first does `new("")`, then writes the label |
| `.hint` | — | The hint: under the label, or behind the help button after the name, by placement. Can later be written as `Some` with `update_component`; writing `None` hides it |
| `.hint_placement` | `Option<HintPlacement>` | This row's own placement of its hint, over its container's. `None` follows the nearest `SettingsGroup` / `SettingsCard` that writes one, and `Below` when none does. Bindable in views |
| `.notice` | `Option<Arc<str>>` | The notice line under the row. `None` or empty takes no room. Bindable in views |
| `.notice_tone` | `ValidationIntent` | How the notice reads: `Warning` (default) or `Danger`. Bindable in views |
| `.error` | `Option<Arc<str>>` | What is wrong with the value. When set it is drawn on the notice line as Danger, over `.notice`, and the row reads as invalid. Bindable in views |
| `.stacked` | — | `true` always stacks vertically |
| `.stack_below` | — | Stacks when narrower than the threshold, same line when equal or wider. An invalid threshold counts as off. The view function uses `280.0`. The example in the reference uses `480.0` as the application's own threshold |
| `.divided` | — | `true` draws a hairline below the row. Do not draw it on the last row of a group; the group's own edge closes it |
| `.first_in_group` | — | Marks one end of the group |
| `.last_in_group` | — | Marks one end of the group |
| `.loose` | — | Increases spacing |
| `label_mark` | — | Category mark right after the label text (before the help button and `label_accessory`). In views use the `.label_mark(view)` slot |
| `label_accessory` | — | Accessory on the same line after the label. In views use the `.label_accessory(view)` slot |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| None | None | None |

## Slots

| Slot | Description |
| --- | --- |
| `.control` | Puts the control after the copy. `mount_settings_leaf_row` keeps the label, hint, and control slot |
| `.label_mark` | Goes right after the label text, before the help button and accessory. The label elides first; the mark keeps its width |
| `.label_accessory` | Goes after the label and help button on the same line. The label elides first; the accessory keeps its width |

## Methods

| Method | Description |
| --- | --- |
| `AppContext::flash_settings_row(row)` | Flashes this row: a faint accent background holds and then fades out, leaving no trace. Calling again restarts it |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md) · [Workspace](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/workspace.md)
