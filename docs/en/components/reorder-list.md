# ReorderList

`ReorderList` is selection and dragging on a vertical list. It reports "the value that moved, and the value after it"; it does not change item order itself. Order, grouping and persistence are yours.

## Basic usage

There is no `<ReorderList>` in the control table.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ReorderItem, ReorderList};

view! {
    <Widget
        of={ReorderList::new([
            ReorderItem::new("a", "First item").selected(true),
            ReorderItem::new("b", "Second item"),
        ])
        .label("Order")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ReorderItem, ReorderList};

widget(
    ReorderList::new([
        ReorderItem::new("a", "First item").selected(true),
        ReorderItem::new("b", "Second item"),
    ])
    .label("Order"),
)
```

:::

After receiving `Reorder`, reorder your list by `source` and `before`, then write it back to `items`.

## Cross-list dragging

Within one document, lists with the same `.transfer_group("…")` accept drags from each other. There is no group by default; a drag stays in its own list.

- When a row dragged out of list A moves over list B in the same group, B computes the target by its own rules: the dragged row's `kind`, the `accepts_inside` and `accepts_sibling` of B's rows, and B's `accepts_root`, `.tree_drop` and depth, exactly as when dragging inside B, except that no row travels with the drag in B. With `.drag_preview(ReorderDragPreview::LiveGap)`, B opens a live gap with a dashed Accent slot; by default B draws the insertion line or the marker for dropping into a row. A no longer draws its own target, and the dragged row's outline and pointer following stay as before. When the pointer returns to A, A returns to normal.
- Setting `.drop_zone(Some(node_id))` on B extends cross-list transfer hit testing to an enclosing container node (such as an entire group card and its padding), so even an empty flat list opens its landing slot inside the card; hidden or collapsed lists (and their `drop_zone`) never intercept transfers or external drags.
- With `.drag_preview(ReorderDragPreview::LiveGap)`, the source list also emits `ReorderListEvent::Drag { source, phase, x, y, inside_group }` (in window coordinates, plus whether the pointer is over any list/drop zone in its `transfer_group`) so outer UI can take over when dragging into external containers or other windows.
- To feed an external drag (such as from another window or drawer) into a list, call `AppContext::set_reorder_external_drag(list_id, Some(ReorderExternalDrag { value, kind, x, y, size }))` (or `None` to clear), and call `AppContext::finish_reorder_external_drag(list_id)` on release to emit/return the drop and clear state.
- On release over B, when the target is valid, the event is emitted on B: tree lists emit `TreeDrop { source, intent }`, ordinary lists emit `Reorder { source, before }`. `source` is the dragged row's value in A; `intent` and `before` point at rows in B; row values in the same group must be distinguishable from each other. A emits nothing this time. Where B does not accept, no marker is drawn, and release sends nothing on either side.
- A tree list needs the pointer over one of B's rows to have a target; an empty tree list accepts nothing. Ordinary lists compute by vertical position, so an empty list can still drop at the end.
- A keeps the pointer capture throughout. Escape or `Cancel` during the drag makes A emit `Cancelled`, and B's marker retracts with it.
- Keyboard moves do not cross lists. Lists in different documents (windows) do not accept drags from each other.

## Features

This type is behind the Cargo feature `controls`. `components` turns it on.

## Items

`ReorderList::new(items)` takes `ReorderItem`. `ReorderItem::new(value, label)` is draggable, droppable, unselected and enabled by default.

`.draggable(false)` also writes `drop_target` as false; when you want a drop target without a drag source, follow it with `.drop_target(true)`.

`.selected` and `.disabled` are written on the row. `.tools(id)` marks child nodes inside a row where a drag never starts: pressing in that area does not drag no matter how far you move. Buttons in a row do not need to be registered as `.tools` to receive clicks; see [Gestures](#gestures).

## List

`.spacing` is row spacing (and vertical row gap in grid mode), `.column_gap` sets horizontal gap in grid mode, `.layout_mode(ReorderLayout::Column | ReorderLayout::Grid)` selects a vertical column (default) or a wrapping left-to-right 2D grid, `.size` is small by default, `.label` is the list's accessible name. `.drag_preview(ReorderDragPreview::Auto | ReorderDragPreview::LiveGap | ReorderDragPreview::Marks)` controls drag preview mode (`Auto` shifts flat live rows and keeps marks for trees; `LiveGap` enables unified live gap-opening and dashed landing slots across columns, 2D grids, trees, cross-list transfers, and external drags; `Marks` always draws outlines and insertion lines only). `.drop_zone(Some(id))` widens transfer/external hit testing to an enclosing node, and `.tree_drop(true)` allows dropping onto the tree. `.reorder(false)` makes the list select-only: no amount of movement after pressing starts a drag, and release emits `Select`. To keep only a few rows fixed, give those rows `.movable(false)`: they still select, and still accept other rows dropping next to them.

## Tree

A tree is flattened into rows in depth-first order, and each row writes its depth with `.depth(n)` (top level is 0): a row owns the rows just below it that are deeper. Leave collapsed child rows out of `items` and mark collapsed parent rows with `.folded(true)`.

- Dragging a row carries the whole subtree it owns: the dragged outline frames the whole block, and no position inside the subtree can be dropped into.
- `.kind("…")` writes what this row is. `.accepts_inside(..)` writes which kinds can drop inside this row, `.accepts_sibling(..)` writes which kinds can drop before or after it (becoming its sibling), both using `ReorderAccepts`: `All` (default), `Nothing`, or `ReorderAccepts::kinds([..])`. Rows with no `kind` are only accepted by `All`.
- Dropping before or after a row also needs its parent row (at top level, the list's `.accepts_root(..)`) to accept that kind.
- Where nothing accepts, no insertion line is drawn and release emits nothing, so every target the list reports is one you declared acceptable.

The targets of tree drag and drop:

- The top quarter of a row is before it, the middle drops inside it, and the bottom quarter is after it; when the row does not accept the dragged kind, the top and bottom halves compute before and after.
- For a row with expanded children, the segment below it is before its first child row.
- Below the last row of a nested run, the pointer's horizontal position selects the depth: each `.indent` to the left exits one level, landing after that level's ancestor.
- The insertion line indents by `depth × indent` for the target depth; the default indent is `space::LG`, and use `.indent(px)` to change it so it aligns with the row indent you draw yourself.
- Positions that would not change after dropping (the immediately adjacent siblings) draw no line and emit nothing. `Inside` leaves it to the application which end of the child rows it goes in.

## Keyboard

When a control in the list or in a row has focus, Alt plus an arrow key moves that row (the row when focus is inside a row, otherwise the selected row), together with its subtree:

| Key | Move | Event |
| --- | --- | --- |
| Alt+↑ | Before the previous sibling | `TreeDrop` `Before`; an ordinary list emits `Reorder` |
| Alt+↓ | After the next sibling | `TreeDrop` `After`; an ordinary list emits `Reorder` |
| Alt+→ | Inside the previous sibling, as its last child row | `TreeDrop`: `After` of the last child row when it has visible child rows, otherwise `Inside` |
| Alt+← | Out of the parent row, after the parent row | `TreeDrop` `After` |

Keyboard moves obey the same rules as dragging: they emit nothing when the position is not accepted, has no sibling, or does not change. `ReorderList::key_move(value, ReorderKeyMove)` computes that event directly.

## Custom-drawn rows

`.live_rows(true)` means rows are drawn by the child nodes you mount; the list still uses `items` for hit testing and dragging and no longer draws rows from labels. The switch is declared at construction and never inferred from the tree during projection.

During a drag, each row's box comes from the list's own direct children, so rows must be mounted directly under the list. When you build rows with keyed `each`, give the list to `.container(..)`; rows are then built directly into the list, and their nodes are kept by key across insertions, removals and reorders:

```rust
use nana_ui::runtime::view::{each, widget};
use nana_ui::runtime::{ReorderList, ReorderListEvent};

each(ids, |id| id.clone(), entry_row).container(
    widget(ReorderList::new(items).live_rows(true).label("Items"))
        .on(|event: &ReorderListEvent| { /* Reorder by source and before */ }),
)
```

`items` must correspond one-to-one with the rows: write back to `items` on reorder, insertion and removal (for example by binding `fields::reorder_list::items`; `drop_zone`, `transfer_group`, `accepts_root`, and `reorder` field writers live in the same module).

## Gestures

A drag must exceed 4px to count as a move.

Live-gap dragging (`.drag_preview(ReorderDragPreview::LiveGap)`, as well as live flat lists without `.tree_drop` under `ReorderDragPreview::Auto`):

- The dragged row (together with the child rows it owns) follows the pointer, stacked above other rows but below open menus (the layer is `REORDER_PREVIEW_Z_INDEX`, below `MENU_OVERLAY_Z_INDEX`).
- At the position where release would drop, the other rows smoothly make room for the dragged block (vertically in columns/trees, or by 2D `[dx, dy]` shifts across wrapping cells in `ReorderLayout::Grid`); `LiveGap` draws an Accent dashed rounded landing slot inside the vacated space (and for nested tree slots highlights the target parent box with an indented L-shaped tree connector). Where the target does not change or is not accepted, all rows return to their places.
- All of this is on the presentation layer only: row styles, layout boxes and target resolution do not move, and targets are still computed from the rows' original layout boxes. The framework drives the animation; the application does not tick it. With reduced motion the rows make room immediately, and the preview still follows the pointer.
- The vacated slot and dashed outline serve as the landing indicator; no separate insertion bar is drawn.
- On release (dropped or not), Escape and `Cancel`, the preview, dashed slot and the vacated space retract at once. After receiving `Reorder` / `TreeDrop` and writing back to `items` in the same frame, the row settles into the slot it made.

Tree drag and drop under `Auto`, `Marks`, and custom-drawn rows: after a move the list draws an accent outline around the dragged row and an insertion line at the position where release would drop; when tree drag and drop lands inside a row, it instead outlines that row and lays a faint accent fill over it. Positions where dropping would not change the order draw no line. With live rows these follow the child nodes' layout boxes and are drawn above the rows (a selected row's background does not cover them), so rows do not need to draw any drag state themselves.

Where you press decides who owns the press:

- Pressing on a row, not on a control (custom-drawn rows; the row node itself under `.live_rows(true)`, and the parts of a row that cannot take focus): the list takes the pointer at once. Releasing without moving is `Select`; moving more than 4px drags the row.
- Pressing on a control in the row (a focusable node under the row, such as a button): the press belongs to that control as usual. Releasing without moving is the control's own click, and the list emits nothing; only after moving more than 4px does the list take the pointer and drag the row, and the control does not receive the click this time. As in HTML5 drag and drop, the whole row can drag, including a drag that starts on a button.
- Pressing in a `.tools` area: the list leaves it alone, and moving does not drag either.

Rows that used to wrap buttons in a segment registered as `.tools` so the buttons received clicks can drop `.tools` after upgrading: buttons still click, and dragging from a button drags the row. Keeping `.tools` preserves the old behavior, where a press in that area does not drag.

`selected_value` reads the current selection. `is_dragging` tells you whether the gesture is still in progress.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `items` | `ReorderItem` | `ReorderList::new(items)`. After `Reorder`, reorder by `source` and `before` and write back |
| An item's value and `label` | — | `ReorderItem::new(value, label)`. Draggable, droppable, unselected and enabled by default |
| `.draggable` | — | `false` also writes `drop_target` as false |
| `.drop_target` | — | For a drop target without a drag source, follow `.draggable(false)` with `.drop_target(true)` |
| `.selected` | — | Written on a row. Unselected by default |
| `.disabled` | — | Written on a row. Enabled by default |
| `.folded` | `bool` | Written on a row. Marks a collapsed parent row in a tree (`false` by default) |
| `.tools` | — | The argument is `id`. Marks child nodes inside a row where a drag never starts. Buttons in a row do not need to be registered to receive clicks |
| `.spacing` | — | Row spacing (and vertical row gap in grid mode) |
| `.column_gap` | — | Horizontal column gap in grid mode; defaults to `.spacing` |
| `.layout_mode` | `ReorderLayout` | `Column` (default vertical column) or `Grid` (left-to-right wrapping 2D grid) |
| `.drag_preview` | `ReorderDragPreview` | `Auto` (default), `LiveGap` (live gap-opening + dashed landing slot), or `Marks` (outlines / insertion line only) |
| `.drop_zone` | `Option<StableNodeId>` | Enclosing container node extending cross-list/external hit testing (ignored while hidden) |
| `.size` | — | Small by default |
| `.label` | — | The list's accessible name |
| `.tree_drop` | — | `true` allows dropping onto the tree |
| `.reorder` | `true` | `false` makes the list select-only, with no dragging |
| `.depth` | `u16` | Written on a row. Depth in the tree, top level 0, default 0 |
| `.kind` | — | Written on a row. The row's kind, for `ReorderAccepts` to judge |
| `.accepts_inside` | `ReorderAccepts` | Written on a row. Which kinds can drop inside this row, `All` by default; with `Nothing` the row computes before and after by its top and bottom halves |
| `.accepts_sibling` | `ReorderAccepts` | Written on a row. Which kinds can drop before or after it, `All` by default; the parent row must accept too |
| `.accepts_root` | `ReorderAccepts` | Which kinds can drop at top level, `All` by default |
| `.indent` | — | Indent per depth level, used to indent the insertion line and by the pointer to select depth. Default `space::LG` |
| `.live_rows` | — | `true` means rows are drawn by the child nodes you mount. The list still uses `items` for hit testing and dragging and no longer draws rows from labels. Declared at construction, never inferred from the tree during projection |
| `.transfer_group` | — | Lists in the same document with the same group accept drags from each other; targets and events belong to the receiving list. No group by default |

## Events

The event is `ReorderListEvent`:

- `Select(value)` is a click on a row.
- `Secondary { source, x, y }` is a right click on a row body. `x` and `y` are window coordinates, used to anchor the context menu. The row surface does not take pointer input, so this event is emitted on the list.
- `Drag { source, phase, x, y, inside_group }` fires during an active `LiveGap` drag on the source list with window coordinates and whether the pointer is inside any list/drop zone of its `transfer_group`.
- In `Reorder { source, before }`, `before` equal to `None` means drop at the end.
- In `TreeDrop { source, intent }`, `intent` carries the target id and a `TreeDropPosition`: `Before`, `Inside` or `After`. `source` moves together with its subtree. Keyboard moves emit this event too.
- For a row dragged in from a list in the same group, this list emits `TreeDrop` (an ordinary list emits `Reorder`), and `source` is its value in the original list; see [Cross-list dragging](#cross-list-dragging).
- `Cancelled` is cancellation. Escape during a drag is cancelled by the framework, which releases the capture and the press record; the following release neither drops nor clicks the control inside the row where the press started. When you drive `apply_pointer` yourself, hand focus loss and touch loss to `ReorderListPointer::Cancel`.

Pointer coordinates are unprojected through the list's layout coordinates before hitting rows, so when the list is inside a `ScrollView` or an ancestor carries a transform, pressing, dragging, targets and right clicks all find the row under the pointer; `Secondary`'s `x` and `y` are still window coordinates.

| Event | Payload | Description |
| --- | --- | --- |
| `Select` | `value` | A click on a row |
| `Secondary` | `source`, `x`, `y` | A right click on a row body. `x` and `y` are window coordinates, used to anchor the context menu. The row surface does not take pointer input, so this event is emitted on the list |
| `Drag` | `source`, `phase`, `x`, `y`, `inside_group` | Source drag lifecycle, window coordinates, and group hit status under `LiveGap` |
| `Reorder` | `source`, `before` | `before` equal to `None` means drop at the end |
| `TreeDrop` | `source`, `intent` | `intent` carries the target id and a `TreeDropPosition`: `Before`, `Inside` or `After` |
| `Cancelled` | — | Cancellation. Escape during a drag is handled by the framework; when you drive the pointer yourself, hand focus loss and touch loss to `ReorderListPointer::Cancel` |

## Slots

| Slot | Description |
| --- | --- |
| Child nodes | With `.live_rows(true)`, mounted by you to draw. The list still uses `items` for hit testing and dragging and no longer draws rows from labels |
| `.tools` | Child nodes inside a row where a drag never starts. Pressing here does not drag even when moving |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
