# TreeView

`TreeView` draws the stable nodes, expansion facts, and hierarchy you give it as disclosure rows. It does not read paths and does not watch the file system. Collapsed children are not projected.

## Basic usage

`TreeView::new(nodes)` takes `TreeNode<Arc<str>>`. Leaves use `TreeNode::leaf(id, label)`; branches use `TreeNode::branch(id, label, expanded, children)`. `.icon` and `.disabled` are set on the node. Which rows are selected is not set on the nodes but given with `.selection(selection)`; see Selection below.

The tree's `.size` defaults to small. `.style` replaces the whole style and must come before builder calls that change layout. `<TreeView>` in the control table binds `nodes`, `selection`, and `label`, and `on_select` receives `TreeSelectionRequested`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{SelectionState, TreeNode, TreeView};

let mut selection = SelectionState::single();
selection.select_only("main".into());

view! {
    <Widget
        of={TreeView::new([
            TreeNode::branch(
                "src".into(),
                "src",
                true,
                [TreeNode::leaf("main".into(), "main.rs")],
            ),
            TreeNode::leaf("cargo".into(), "Cargo.toml"),
        ])
        .selection(selection)}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{SelectionState, TreeNode, TreeView};

let mut selection = SelectionState::single();
selection.select_only("main".into());

widget(
    TreeView::new([
        TreeNode::branch(
            "src".into(),
            "src",
            true,
            [TreeNode::leaf("main".into(), "main.rs")],
        ),
        TreeNode::leaf("cargo".into(), "Cargo.toml"),
    ])
    .selection(selection),
)
```

:::

The default width fills; the height is computed from the visible row count and padding. If you write a height yourself, that height is kept, which makes it easy to put the tree in a fixed viewport.

## Expansion

`apply_event(TreeViewEvent::Toggle(id))` flips a branch; the disclosure, the left and right arrow keys, and assistive-technology expand / collapse all go through it and emit `TreeViewEvent::Toggle`. `TreeViewEvent::Select(id)` only moves the row the keyboard is on (the active row, `active_id`) there; it does not change the selection.

`navigate` moves the active row over the visible rows according to `TreeNavigation`. Visible rows are read from `visible_rows`, with depth recorded in `TreeRowData`.

## Selection

Which rows are selected is your state: a `SelectionState<Arc<str>>`, the same type tables and lists use (see [Table's SelectionState](https://github.com/sena-nana/NanaUI/blob/main/docs/en/components/table.md#selectionstate)). Hand it to the tree with `.selection(selection)`; the tree only reads it to paint the selected background and report it to assistive technology. With `SelectionState::multiple()` the tree reports that it is multi-selectable.

The tree does not change the selection. It emits `TreeSelectionRequested { id, gesture }` on the `TreeView`:

- Pressing a row: `Press`. Ctrl (Cmd on macOS) toggles; Shift selects a range from the anchor.
- With focus on the tree, the up and down arrow keys emit `Move`; with Shift (and Shift + Home / End) they extend from the anchor; Ctrl with up or down moves the active row without changing the selection; Ctrl+Space emits `Toggle`; Ctrl+A emits `SelectAll`.
- An assistive-technology click is the same as a press, and focus the same as an arrow key.

On receipt, call `request.apply_to(&mut selection, &nodes)` (that is, `SelectionState::apply` with `TreeOrder::visible(&nodes)`) and hand the result back to the tree. If nothing changed there is nothing to hand back, and the tree recomputes nothing. In a reactive view, keep the selection in a `Signal` and bind `selection` to it.

After nodes are added, removed, or renamed, call `selection.reconcile(&TreeOrder::all(&nodes))` to drop rows that no longer exist. `TreeOrder::all` includes the children of collapsed branches, so collapsing a branch does not lose the selection inside it; Shift ranges, arrow keys, and select-all only walk visible rows. Disabled rows cannot be selected.

## Toggle columns

`.columns([TreeToggleColumn::new(icon, name)])` draws a column of icon toggles at the end of each row, for example visible or locked. A row's toggle state is `TreeNode::toggles`:

- An icon that is on uses the secondary text color; one that is off uses a fainter color.
- Pressing a toggle flips it without changing the selection.
- Holding and dragging along the column over other rows sets every row to the same value.

Each change emits one `TreeToggleChanged { id, column, on }`. The tree updates the state on the node itself; if the application does not accept it, it writes the node back.

When some rows have no toggle in a given column (for example group rows in a layer tree have no visibility of their own), `.hide_toggle(column)` removes that cell and `.hide_toggles()` removes all of the row's toggles. A removed cell is not drawn and cannot be pressed; a drag along the column skips that row, and the screen reader does not report that toggle. Column indices stay the same, and other rows are unaffected.

## Drag and drop

With `.reorderable(true)`, a row can be dragged onto another row, with three kinds of drop target:

- Pointer in the top quarter of the target row: `Before`, drawing a line at the row's top edge.
- Pointer in the bottom quarter: `After`, drawing a line at the row's bottom edge.
- Pointer in the middle: `Inside`, outlining the whole row.

What is dragged is the selection set, in tree order (when the drag starts from a selected row), otherwise just that row. No drop target is shown when the target is a dragged row itself or one of its descendants. Releasing emits `TreeDropRequested { rows, intent }`; the tree itself moves no nodes, and the application decides whether to accept. Dragging out of the tree and releasing emits `TreeDragReleased { rows, x, y }`, where `x` and `y` are window coordinates; the application can convert them into another control with `UiWorld::pointer_layout_position` to drop the rows into another list. Pressing an already selected row of a multiple selection holds its selection request until release: a drag takes the whole selection set, and releasing without dragging asks for only that row.

## Inline rename

With `.renamable(true)`, double-clicking a row or pressing F2 on the focused tree opens an input at the label's position with the old name selected:

- Enter, or focus leaving the tree: emits `TreeRenameRequested { id, text }` and returns focus to the tree.
- Escape: cancels without an event.

The tree does not change the node's name; the application decides whether to accept. `renaming` reads the row being renamed. The manual calls are `begin_tree_rename` / `end_tree_rename`.

## Dragging in from a list

When dragging from a `ReorderList` onto the tree, what comes back is the stable node plus `before`, `inside`, or `after`. The tree does not infer whether it is a directory or a file.

## Accessibility

The tree is a Tree to the screen reader, named by `.label(name)`, for example "Layers" or "Outline". Without it the tree has no name; the tree never uses a row's id or label as its name.

Each visible row is a TreeItem under the tree. These rows are not retained nodes but virtual nodes projected from the visible rows, following expansion and selection live. Each row carries:

- Name: the row's label.
- Level: 1 at the top level, plus 1 for each level deeper.
- Expanded: only on branch rows, indicating expanded or collapsed.
- Selected: whether it is in the selection you give.
- Disabled: rows of disabled nodes are disabled.

Collapsed child rows are not in the accessibility tree; they appear after expansion. While the tree has focus, the screen reader's focus is on the active row (where the keyboard last went, else the selection's current item); with no active row it is on the tree itself.

The screen reader can perform these actions on rows, with the same results and the same events as pointer and keyboard:

- Click: same as clicking the row; asks for only it to be selected and gives the tree focus.
- Focus: same as moving to the row with arrow keys; moves the active row here, asks for it to be selected, and gives the tree focus.
- Expand / collapse: branch rows only; same as clicking its triangle, emitting `TreeViewEvent::Toggle`.

Actions on disabled rows, on rows that no longer exist, and actions a row cannot perform (for example expanding a leaf, or expanding an already expanded branch) are rejected without an event.

### Toggle columns

Each toggle at the end of a row is a Checkbox under that row's TreeItem, also a virtual node, in column order. Each toggle carries:

- Name: the column name, the second argument of `TreeToggleColumn::new(icon, name)`, for example "Visible" or "Locked". The tree never uses the icon as the name, so the application must supply the column name and change it when the UI language changes; with an empty column name the toggle has no name.
- Checked: the toggle state for this row and column; off when the node does not set this column.
- Disabled: rows of disabled nodes have disabled toggles too.
- Bounds: the rectangle of the toggle icon within the row.

When toggle state changes (a press, a drag over, or the application writing the node back), the screen reader sees the new state immediately.

The screen reader can only click a toggle: the same as pressing it, which flips it and emits the same `TreeToggleChanged { id, column, on }`, and the tree updates the state on the node the same way. It does not change the selection or move focus. Disabled rows, rows that no longer exist, columns the tree does not have, and actions other than click are rejected without an event.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `nodes` | `TreeNode<Arc<str>>` | Stable nodes taken by `TreeView::new(nodes)` |
| `TreeNode::leaf(id, label)` | — | Leaf |
| `TreeNode::branch(id, label, expanded, children)` | — | Branch. `expanded` is the expansion fact |
| `.icon` | — | Set on the node |
| `.disabled` | — | Set on the node. Disabled nodes are never selected |
| `.size` | — | Defaults to small |
| `.selection(selection)` | `SelectionState<Arc<str>>` (stored as `TreeSelection`) | Your selection; reported as multi-selectable with `multiple()` |
| `.columns(columns)` | `TreeToggleColumn` | Toggle columns at the end of rows; state lives in `TreeNode::toggles`. The column name is each toggle's name for the screen reader |
| `.reorderable(bool)` | `bool` | Rows can be dragged before, inside, or after other rows |
| `.renamable(bool)` | `bool` | Inline rename on F2 or double-click |
| `.label(name)` | `Arc<str>` | The tree's name for the screen reader; rows use their own labels |
| `.style` | — | Replaces the whole style; must come before builder calls that change layout |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `TreeViewEvent::Toggle` | `id` | A branch was expanded or collapsed |
| `TreeSelectionRequested` | `id`, `gesture` | A press, key, or assistive-technology action asked to change the selection; see Selection |
| `TreeToggleChanged` | `id`, `column`, `on` | A row's toggle was set to `on`: a press, a drag over, or a screen reader click on the toggle |
| `TreeDropRequested` | `rows`, `intent` | Drop request; `intent` is the target plus `Before` / `Inside` / `After` |
| `TreeDragReleased` | `rows`, `x`, `y` | Rows were dragged out of the tree and released; `x` and `y` are window coordinates |
| `TreeRenameRequested` | `id`, `text` | Rename committed |

## Slots

No named slots. Nodes come from `TreeNode` data.

## See also

[Overview](index.md) · [Controls](../reference/components.md)
