# Table

`Table` is a table retained in the tree. It does not compare data itself. The sort indicator on a column header and the column order are recorded by the adjacent `VirtualTableLayout`; the row order is sorted by you, because only you know how two rows compare.

## Basic usage

The visible controls are three types. `Table::new()` has the accessible role table, and `.label` gives the whole table a name. `TableRow::new()` is one row, and `.selected` marks the current row. `TableCell::new(value)` is one cell, whose text is that value. `.column_header(true)` projects the cell as a column header, and otherwise it is a data cell. Cells are clickable and focusable by default. The table and its rows take no pointer input themselves.

The control table has no `<Table>`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Table, TableCell, TableRow};

view! {
    <Widget of={Table::new().label("Files")}>
        <Widget of={TableRow::new()}>
            <Widget of={TableCell::new("Name").column_header(true)} />
            <Widget of={TableCell::new("Size").column_header(true)} />
        </Widget>
        <Widget of={TableRow::new()}>
            <Widget of={TableCell::new("main.rs")} />
            <Widget of={TableCell::new("12 KB")} />
        </Widget>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Table, TableCell, TableRow};

widget(Table::new().label("Files")).children((
    widget(TableRow::new()).children((
        widget(TableCell::new("Name").column_header(true)),
        widget(TableCell::new("Size").column_header(true)),
    )),
    widget(TableRow::new()).children((
        widget(TableCell::new("main.rs")),
        widget(TableCell::new("12 KB")),
    )),
))
```

:::

## Geometry

Geometry lives in `nana_ui::runtime::VirtualTableLayout`. `VirtualTableLayout::new(row_extents, columns)` takes each row's height and a set of `nana_ui::runtime::TableColumn`. `TableColumn::new(key, extent)` gives a column key and width. `.sortable(true)` marks this column's header as a sort control. `.limits(min, max)` bounds the width, and `.resizable` allows resizing.

`move_column(from, to)` reorders columns, keeping widths and the total width, with `to` the target index after removal. `VirtualTableLayout` answers only which column shows the sort indicator, and the column geometry.

## Sorting

You do the sort comparison. A header activation goes through `toggle_sort(column)`: ascending, then descending, then off. The third activation clears the sort instead of returning to ascending. A column that is not sortable, or not in the table, is rejected. `set_sort` publishes the indicator directly; `None` means no sort.

When a header is activated, you call `toggle_sort`, read the current `TableSort`, sort your own data by that column key and direction, and then change the order of the table's child nodes.

## Resizing columns

Build header cells as `TableCell::new(title).header_for(&column)`: that carries the column key, whether it resizes, and the width range `.limits` gave. Over the trailing 5 px of a resizable header (`TABLE_RESIZE_HANDLE`) the pointer shows a horizontal resize cursor, and dragging there emits `TableColumnResized { column, extent, finished }` on the `Table`: `finished: false` on each move of the drag, then `finished: true` on release, the width already clamped to the range. Pressing `Escape` or receiving a pointer cancel ends the drag at the width it started from (`finished: true`).

The table does not change its own geometry. When you receive it, call `layout.resize_column_by_key(&column, extent)` and sync again (`sync_virtual_table_retained_in`): `VirtualTableLayout::revision()` triggers a resync so header and body cells stay aligned, frozen columns and headers included (even when the `Table` sits below sibling content in a `ScrollView`). To resize only on release, handle only `finished: true`.

## Selection

Which rows are selected is your state, usually a `SelectionState<RowKey>` (see below). The table only reports requests:

- A press on a body cell emits `TableSelectionRequested { row, cell, gesture: SelectionGesture::Press(modifiers) }`. Ctrl (Cmd on macOS) toggles, Shift selects the range from the anchor, both together add a range.
- With focus on a body cell, the arrows, Home / End and Page Up / Down move focus and emit `Move`; Shift extends from the anchor and Ctrl+Shift adds a range without clearing disjoint selections; Ctrl with ArrowUp / ArrowDown moves focus without selecting; Ctrl+Space emits `Toggle`; Ctrl+A emits `SelectAll`.
- Header rows raise no selection requests.

`row` is the row node; `VirtualTableItems::row_key_of(&cx, row)` turns it into your row key. Hand the request to `SelectionState::apply(gesture, row_key, &order)`, then call `cx.sync_virtual_table_selection(&items, |key| selection.is_selected(key))`: mounted rows and their cells get `selected`, paint the theme's selection background and report selected to assistive technology; rows not mounted need nothing, as `build_row` builds them from the same predicate when they scroll in. With an unchanged selection this writes nothing and allocates nothing. `Table::multiselectable(true)` tells assistive technology several rows can be selected.

## SelectionState

`nana_ui::runtime::SelectionState<K>` is pure-data selection state tied to no control, shared by `each_virtual`, `List`, grids, virtual tables and `TreeView` (see [TreeView's Selection](https://github.com/sena-nana/NanaUI/blob/main/docs/en/components/tree-view.md#selection)) alike. It stores only your business keys: the selected set, the current item (where the keyboard is) and the anchor (where a Shift range starts). It stores no positions, rows or nodes, so a row scrolled out of view and never built stays selected.

Operations that need order (Shift ranges, arrow keys, select all, cleanup after the data changes) read it from a `SelectionOrder` you pass in: a slice of keys, or `keyed_order(&items, |item| item.id)`, narrowed with `.selectable(..)` to leave out disabled items. The order is never copied into the state.

- `SelectionState::single()` holds at most one item; `multiple()` any number.
- `apply(gesture, key, &order)` is the one entry point for pointer and keyboard; `navigate(step, modifiers, &order)` takes one `SelectionStep` and applies it.
- `reconcile(&order)` is called after the data gains, loses or reorders items: keys missing from the order are dropped; a current item that went away moves to the selectable item nearest where it was; reordering loses nothing.
- `is_selected`, `selected_in(&order)` (in order), `current`, `anchor`, `revision` (bumped on every change).

In a reactive view keep it in a `Signal<SelectionState<K>>` and bind rows with `.selected(sel.map(move |s| s.is_selected(&key)))`.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `.label` | — | On `Table::new()`, gives the whole table a name. The accessible role is table |
| `.multiselectable` | `bool` | On `Table::new()`, tells assistive technology several rows can be selected |
| `.selected` | — | On one row of `TableRow::new()`, marks the current row |
| `value` | — | The text of `TableCell::new(value)` is that value |
| `.column_header` | — | `true` projects the cell as a column header, otherwise it is a data cell |
| `.header_for(&column)` | `&TableColumn` | A header cell: column key, whether it resizes, width range |
| `.column` / `.resizable` | — | Set the column key, or whether dragging the trailing edge resizes, on their own |
| `row_extents` | — | Each row's height taken by `VirtualTableLayout::new` |
| `columns` | A set of `nana_ui::runtime::TableColumn` | Same as above |
| `key` | — | The column key of `TableColumn::new(key, extent)` |
| `extent` | — | The width of `TableColumn::new(key, extent)` |
| `.sortable` | — | `true` marks this column's header as a sort control |
| `.limits(min, max)` | — | Bounds the width |
| `.resizable` | — | Allows resizing |

## Events

| Event | Emitted on | Description |
| --- | --- | --- |
| `TableCellFocused` | `Table` | An arrow key moved focus to a cell |
| `TableColumnResized` | `Table` | A header drag resized a column; see “Resizing columns” |
| `TableSelectionRequested` | `Table` | A press or key asked for the selection to change; see “Selection” |

Sorting from a header is done by calling `toggle_sort` or `set_sort` yourself; see above.

## Slots

There are no named slots. Rows and cells are child nodes; see basic usage.

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
