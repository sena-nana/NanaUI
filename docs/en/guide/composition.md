# Layout and composition

The control pages explain how each control is used on its own; this page explains the rules for putting them together: who owns horizontal padding, how tall the header is, which line a trailing control lines up with, and which layer a notice uses. These rules hold across applications; NanaUI's defaults follow them, and consumers do not need to write their own compensation.

Each rule is followed by the interface that carries it. Where there is no interface, the rule says what to write instead.

## Content line

Inside a container, the left and right edges of text and controls line up on one vertical line; that line is the content line.

- **One layer owns horizontal padding.** When the card (panel, popover card) provides padding, the groups, rows and scroll areas inside it add none; when nesting, if the outer layer has it, the inner layer's is 0. Padding on both layers indents inner content past the title; on neither, it hugs the edge.
- Neither the `settings_group` card nor `group_row` **adds** horizontal padding; the hairlines between rows inset by `CONTENT_INSET` (`SpacingStep::Xxxl`). To put a row on the content line, use `group_row_on_line(view)`, or `group_row(content_line(view))`. When your own row constructor already carries this padding, use `group_row(view)` and do not wrap it again.
- `settings_row` has no horizontal padding of its own, so it also needs the content line when it goes into a card or group.
- `Card::new()` carries `SurfacePadding::Panel` by default. When you use `CardKind::Flat` as a group container and the rows inside put themselves on the content line, write `.padding(0.0)`, or the content indents one step past the hairlines.
- When a list row (`ListItem`) needs a highlight plate wider than its text while the text stays on the content line, use `.pill_bleed(true)`: the plate grows by one row padding and the text does not move.

## Spacing by relationship

Padding decides how far content sits from the container edge; sibling spacing says how two things relate. Values all come from steps (see "Steps only"); this section is about which step to pick.

- **Closer means more related, and adjacent levels differ at least twofold.** A separating distance is at least twice a joining distance: one step (2px) apart reads as no intent.
- **Every distance has exactly one owner.** Padding belongs to the container, sibling spacing to the `gap` of the column that holds them, and a row's top and bottom whitespace to the row. Nothing stacks and nothing compensates (one layer subtracting another's padding means two layers are fighting over the same distance).
- **One level of nesting, one step less padding.** An inner layer is never wider than the outer one and does not stack horizontal padding (see "Content line").

Relationships have names: `SpacingRole`. Pick by relationship, not among adjacent steps: `gap_step` takes the role directly (`gap_step(SpacingRole::Attach)`), and interfaces that take only pixels use `space::role(role)`. The role resolves to a step that follows the theme and density.

| Relationship | Role | Step | Provided by |
| --- | --- | --- | --- |
| A name and the description below it | `Copy` | `Xxs` (`Xs` in `SettingsRow`, whose label uses a tight `Body` line box) | `SettingsRow` |
| The icon and text inside one control | — | `Sm` | The control itself |
| Side-by-side icon tools | `IconTools` | `Xxs` | `heading_tools`; icon rows at the end of a block or row follow this step |
| Mixed small items at the end of a row (text buttons, keycaps, switches, badges) | `Cluster` | `Xs` | The row that holds them |
| Side-by-side text buttons | `Buttons` | `Sm` | The `gap` of the row that holds the buttons; `footer_bar` |
| Things with no frame, held together by distance: the title of a tinted block, the actions below a block, several cards in one group | `Attach` | `Sm` | `settings_group(..).inset()`, `.footer(view)` |
| A row's label and control | `Label` | `Md` (`Xxl` in a loose layout; stacked `Md` / `Xl` loose) | `SettingsRow` |
| Row to row | — | Single-line rows: `Sm` above and below middle rows (`Xxs` at the group edge, copy band at least `Small` control height); multi-line rows with a below-hint or stacked label: `Lg` above and below middle rows (`Xs` at the group edge), plus hairlines | `SettingsRow`; the group adds no `gap`, or the hairlines break |
| A raised card's title and the card | — | `Md` | `settings_group` (the card has an edge and a shadow, so the title can sit one step farther) |
| Group to group inside a card or panel | `GroupInCard` | `Xl` | The `gap` of the column that holds the groups |
| Group to group in a window pane | `GroupInPane` | `Xxxl` | Same |
| Group to group on a settings page | `GroupInPage` | `PageTight` | Same |

- **An element with no card around it belongs by distance alone.** A group title, the actions below a card, a description outside a card: its distance to its own group must be less than half its distance to the neighbouring group, or it reads as the tail of the previous one. Rows inside a card are enclosed by the card and are not subject to this.

## Groups

| Content to place | Use | Do not use |
| --- | --- | --- |
| One group of like settings on a settings page (title + rows + hairlines) | `settings_group(title, rows)` | A hand-built `Card` + `Divider` |
| A block of free content (description, chart, button row) | `Card` (`Surface` / `Raised`), keeping its padding | `settings_group` |
| A group with a tinted background, no border, and the title on the same line as the row text | `settings_group(title, rows).inset()`; when you build the title yourself, `group_block(children)` | A hand-built `Stack` with a `Subtle` background and padding |
| Rows and dividers already organized at the Runtime layer | `SettingsGroup` (it is what sits in the `settings_group` card) | Deciding yourself which divider to hide |

- Spacing between groups comes from the `gap` of the column that holds them; a group carries no margin of its own.
- Hide a row with `group_row(..).visible(cond)`: it takes its divider with it, including when the first row is hidden. Do not hide only the row's content, which leaves an empty row and a stray hairline.
- Groups are told apart by their title and card alone; add no decorative lines, color blocks or icons.

## Subgroups

When a group needs further division, pick one form by intent and do not invent another:

| Intent | Form |
| --- | --- |
| Several rows appear and disappear together, with their outer whitespace counted as one group | Put another `SettingsGroup` inside the group: it treats these rows as one member and finds its own first and last |
| A setting that depends on the row above (meaningful only after a switch is on) | The row right after it in the same group, shown and hidden with `group_row(..).visible(cond)`; no indent, no frame |
| The editor for the selected item in a list | `expansion_body(lead, children)`, expanded in place below that row, one at a time; the content aligns with the title text of the expanded row (`lead`) |
| Several objects in one group, each with its own settings | One card per object, stacked under one title |
| A key-value summary | One block placed after the settings rows (see "Dividers") |

- A container has at most four levels: view (segmented control or second-level page), group, subgroup, row. No block inside a block, and no group title inside a block: when a piece of content needs a title, it is another group.
- Do not express hierarchy with indentation; indentation belongs to trees (`TreeView`, draggable hierarchical lists).
- Every group has a title. The only exceptions are a status notice at the very front of a container, and the group that comes first in a view and is exactly what the view is named for (`settings_group(None, ..)`).

## Dividers

A divider separates two things only: adjacent settings rows in one group, and a fixed region from a scroll region. Every other separation is left to cards and spacing.

| Where the line goes | Interface |
| --- | --- |
| Between two adjacent settings rows in one group | `divided` on `SettingsRow`; `SettingsGroup` draws it only between two rows that are both shown, never at the group's ends |
| Between a fixed region and a scroll region (below a card header, above a fixed footer, below a dialog header) | `footer_bar()` for a fixed footer (top hairline, `Xl` top whitespace, `Buttons` gap); `Divider` below a card header; for a dialog, the theme's `DialogRecipe::header_divider` |
| Separating two clusters of entries in chrome such as a toolbar or the Dock | `Divider::vertical()` |

Places with no line:

- between groups, and between a title and a card;
- between list rows (`ListItem`): rows are separated by hover, selection and row height, and a line would read as a settings row;
- between key-value rows (`LabeledValue`);
- at a card's top and bottom edges, or around custom content inside a card;
- underlines, color bars or gradient lines used as decoration under a title;
- vertical lines in a card body: side-by-side things are separated by spacing or a segmented control.

- **A settings row always has a line on its edge.** When a settings row (`divided`) is adjacent to other content in the group (key-value rows, a list, a notice), there is a line at the junction whichever side the other content is on: the line below is the row's own bottom border, and the line above is added by `SettingsGroup` when the row follows other content, withdrawn when that content is hidden. The other content draws no line itself. A key-value block is still best placed after the settings rows, which reads most evenly.

## Heading row

- When the title bar holds a card's own title, use `AppTitleBar::edge_inset(the card's padding step)`: the slots stand on the card's content line, and you do not subtract the title bar's `Sm` inside the slot.
- Build the header with `heading_row(title, tools)`: the title gives up width and ellipsizes first, and tools keep their width. Put an icon or back arrow before the title in `.lead(view)`, so the title starts at the same point with or without it.
- **The row height is fixed to the text's line box.** Wrap tools in `heading_tools(..)`: compact tools (`IconButton::tool`, compact menus, target `COMPACT_TARGET`, 24px by default) are taller than the line box, so they overhang half above and half below at `heading_overhang()` and do not stretch the header. A header with tools is therefore as tall as one without, and titles on the same page line up.
- Content on the header that is not a tool but just as tall (a status pill beside the name) uses `heading_hang(view)` and overhangs the same way. A row shorter than the heading line box (the name line of a settings row) uses `line_hang(view)` and takes no row height.
- The overhang is computed from the theme's line box (`LineRole::Line`) and applies to titles from `Meta` to `Body` size. When the title is already taller than the target, use `heading_tools(..).inline()` and keep the tools in the row.
- **The overhanging part must not be clipped.** When the header sits at the top of a scroll area, the scroll area leaves the same distance above it: `ScrollView::heading_room()`.
- Tools act on the whole group; an action that moves a task forward is a text button placed below the content, not a heading tool.

## List rows

`ListItem` has three states, chosen by whether the row can do something right now:

| State | When | Interface |
| --- | --- | --- |
| Interactive | Clicking it or pressing Enter does something (open, select, toggle) | The default |
| Read-only | It shows a piece of information and can never be clicked | `.read_only(true)`: no hover or click response, text stays the normal color |
| Disabled | It could do something, but a condition currently prevents it | `.disabled(true)`: grayed out; also tell the user why (an inline note or a hint) |

- Do not write read-only as disabled: disabled gray says "not right now", and the user goes looking for how to enable it.
- **In a multi-line row, the leading and trailing ends align with the first line.** When a description sits below the name, the switch or button at the row's end lines up with the name line rather than centering on both lines. Rows with `ListItem::auto_height(true)` already do this (`TrailingAlign::FirstLine`); other rows can write `.trailing_align(TrailingAlign::FirstLine)`. The row then pads the first line into the position of a single-line row, and the leading and trailing ends center in a band of the same height, so trailing ends stay on one line when single-line and multi-line rows are mixed. Either property may be set first, set later, or toggled with `.bind`; the band follows the row.
- Single-line rows keep the default centering.

## Status layers

From lightest to heaviest:

1. **Inline note**: `callout(..).quiet()`, no background, no horizontal padding, one size smaller, placed inside a block that already has a content line as a line of explanation.
2. **Banner**: `callout(icon, tone, text)`, a background band colored by `StatusTone`, for cases that need attention without interrupting (incomplete configuration, a restricted feature). A link or button that goes to fix it can sit at the end of the row (`.trailing(view)`).
3. **Notification**: `Toast` / `ToastStack`, the result of an action (done, failed), disappearing over time and taking no space in the content.
4. **Dialog**: `ConfirmDialog`, which must be answered before continuing (irreversible actions such as confirming a delete).

- Use one layer per thing. Do not raise a notification on top of a banner, and do not add a banner to every row.
- **An empty list uses `EmptyState`, not a list row.** Writing "no items" as a list row paints it as clickable or disabled. `EmptyState` is a centered icon, title, description and one optional action.
- Show a failure where it happened: a field's failure goes below the field (`ValidationMessage`), the whole action's goes to a notification. A field inside a settings row uses the row's `.error(reason)`: the reason takes the hint's place, the row height does not change, and the control also writes `invalid` to outline it in red; when the reason first appears, focus moves to that field. Do not raise a notification for a field error.
- Write a number's unit as the field's `.unit(..)` (`RangeField`, `NumberInput`), not in parentheses in the label.

## Narrow containers

- When `SegmentedControl` cannot fit one line in the width it is given, it wraps by default (`wrap` is on), so option text is neither truncated nor spilled out of the card; when the width allows, it keeps one line and its own width. When it really must stay on one line, write `.wrap(false)` and make sure its container is wide enough.
- `.fill(true)` makes the segmented control fill the width, and each line shares it evenly when wrapping.
- A button row uses a row that wraps (`flex-wrap: wrap`); do not assume one line fits.
- Titles and text inside rows give up width and ellipsize first; controls keep their width.

## Icon and text alignment

- When an icon sits beside text, center it on the text's line box; do not nudge it down by pixels.
- CJK text in bold at larger sizes sits low in its ink and looks slightly lower than the icon. That is the font's ink position, not a layout error; there is no ink-centered alignment mode yet (see "Interfaces that do not exist yet").

## Steps only

- Spacing writes `SpacingStep` (`gap_step`, `padding_step`, `space::*`), type size writes `TypeRole`, corner radius writes `RadiusTier`, line width writes `BorderWidth`, control height writes `ControlSize`. Do not write numbers: steps follow the installed theme and density, numbers do not.
- A card's or dialog's own name uses `TypeRole::CardTitle` with `TextWeight::Bold`.
- Write numbers only where an interface takes pixels only, and take them from the step constants (`space::XXXL`). Those numbers hold only under default metrics: when the theme changes spacing, the steps follow and the numbers do not. Write a step wherever you can: a hairline inset uses `Divider::inset_step(CONTENT_INSET)`, not `inset(content_inset())` — `content_inset()` is only the pixel value under default metrics.
- The repository's own `scripts/audit-theme-hardcoding.py` can scan consumer sources too; see below.

## Scroll areas

- **The scrollbar runs in the padding, not over the content.** When the card has horizontal padding, the scroll area extends into it with `ScrollView::gutter_theme(ThemeToken::PanelPaddingX)` (that step is the card's own horizontal padding and follows the theme) and insets the content back: the content stays on the card's content line and the scrollbar is drawn in the empty padding; when the padding cannot fit the theme's scrollbar lane, the lane narrows. Do not write pixels such as `gutter(16.0)`: that equals the card padding only under default metrics and extends 6 too far under `COMPACT_METRICS`.
- **Bottom fade**: `scroll_foot_fade(height, surface)` goes into the positioned container that holds the scroll area, over its bottom edge, saying the list has not ended. When the scroll area uses `gutter_theme`, give the fade `.clear_of_theme(step)` (the same value): it goes into the same positioned container as the scroll area (the column in the card's content box, whose left and right edges are the content line), follows the scroll area into the padding on the left, and stops before the scrollbar lane on the right. Leave `height` of blank space at the end of the list so the last row can scroll out from under the fade.
- With heading tools at the top, add `.heading_room()` (see "Heading row").

```rust
use nana_ui::runtime::view::{scroll_foot_fade, widget};
use nana_ui::runtime::{ScrollAxes, ScrollView, SemanticColorRole, Stack};
use nana_ui::theme::ThemeToken;

let gutter = ThemeToken::PanelPaddingX; // The card's own horizontal padding
widget(Stack::column(0.0).with_layout(|l| {
    l.position = nana_ui::runtime::PositionSpec::Relative;
}))
.children((
    widget(
        ScrollView::new(ScrollAxes::Vertical)
            .gutter_theme(gutter)
            .heading_room(),
    )
    .children(groups),
    scroll_foot_fade(24.0, SemanticColorRole::Surface).clear_of_theme(gutter),
))
```

## Running the hard-coding scan on consumers

`scripts/audit-theme-hardcoding.py` scans NanaUI's own components by default. Give it `--root` and `--path` to scan application sources; `--lint` reports only newly appearing numeric literals:

```bash
python3 ../NanaUI/scripts/audit-theme-hardcoding.py --root . --path crates/app/src/ui --lint
```

- `--lint` lists every pixel, font-size and radius literal outside `space::` / `type_scale::` (file:line) and exits 1 when there is one.
- To accept the current state and stop only new ones, record the current counts with `--output baseline.json`, then `--check baseline.json` fails only when a count grows (the same gate NanaUI uses for itself).
- The scan skips `target`, `.git`, `.claude`, `node_modules` and test code, but does scan an application's `src/bin`. Reading a step on the same line does not shelter a pixel literal: the `12.0` in `.gap_step(SpacingStep::Md).padding(12.0)` still counts. `--format markdown` prints a count table with a header.

## Interfaces that do not exist yet

- **Ink-centered alignment.** Aligning an icon to the text's ink (rather than its line box) needs nana-text to hand glyph ink bounds to layout, which does not exist yet; where it is needed, the application moves only the vector icon and not the text (moving text changes the baseline it snaps to at each DPI).
- **A leading icon slot on `Card`.** The header has `heading_row(..).lead(view)`; `Card` does not paint a title itself, and the application builds the card header with `heading_row`.
