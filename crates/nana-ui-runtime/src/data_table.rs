//! The interactive half of a data table: column resizing from the header,
//! and the selection requests a press or a key in the table raises.
//!
//! The table still owns no data and no selection. A drag on a resizable
//! header's trailing edge reports [`TableColumnResized`]; the application
//! applies it to its [`crate::VirtualTableLayout`] (header and body cells are
//! placed from the same column extents, so they stay aligned, frozen columns
//! included). A press on a body cell, or a selection key while a cell has
//! focus, reports [`TableSelectionRequested`] with the row node and the
//! [`SelectionGesture`]; the application maps the row to its key
//! ([`crate::VirtualTableItems::row_key_of`]), applies the gesture to its
//! [`nana_ui_core::SelectionState`] and projects the result onto the mounted
//! rows with [`AppContext::sync_virtual_table_selection`].

use std::sync::Arc;

use nana_ui_core::{SelectionGesture, SelectionModifiers};
use nana_ui_input::InputModifiers;

use crate::{
    AppContext, DocumentId, Entity, FrameworkError, NodePointer, NodePointerHooks, StableNodeId,
    Table, TableCell, TableNavigation, TableRow, VirtualTableItems,
};

/// How far in from a header cell's trailing edge a press grabs the column
/// boundary, px.
pub const TABLE_RESIZE_HANDLE: f32 = 5.0;

/// Rows a Page Up / Page Down with Shift extends by, as the plain keys move.
const TABLE_PAGE_ROWS: usize = 10;

/// A header drag set `column` to `extent` px, already clamped to the limits
/// the header cell carries. Emitted on the [`Table`] on every move of the
/// drag (`finished: false`) and once more when it ends (`finished: true`); a
/// cancelled drag ends at the width it started from.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumnResized {
    pub column: Arc<str>,
    pub extent: f32,
    pub finished: bool,
}

/// A press on a body cell or a selection key asked for the selection to
/// change. Emitted on the [`Table`]; `row` is the [`TableRow`] node the
/// gesture is about and `cell` the cell it came from. Column-header rows
/// raise none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSelectionRequested {
    pub row: StableNodeId,
    pub cell: StableNodeId,
    pub gesture: SelectionGesture,
}

/// What the modifier keys held with a press or a key mean to a selection:
/// Ctrl or Cmd toggles, Shift extends.
pub fn selection_modifiers(modifiers: InputModifiers) -> SelectionModifiers {
    SelectionModifiers {
        toggle: modifiers.control || modifiers.meta,
        extend: modifiers.shift,
    }
}

/// A header cell's column drag in progress, in the cell's own px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ColumnResizeDrag {
    origin_x: f32,
    start: f32,
    last: f32,
}

impl TableCell {
    /// Whether a press at `x` (the cell's own px) grabs the column boundary.
    fn grabs_column_edge(&self, x: f32, width: f32) -> bool {
        self.column_header
            && self.resizable
            && self.column.is_some()
            && x >= width - TABLE_RESIZE_HANDLE
            && x <= width
    }

    fn clamp_extent(&self, extent: f32) -> f32 {
        let (min, max) = self.extent_limits;
        let extent = if extent.is_finite() { extent } else { 0.0 };
        extent.max(min).min(max).max(0.0)
    }
}

/// The table and row a cell sits in, when it sits in one.
fn table_of_cell(cx: &AppContext, cell: StableNodeId) -> Option<(Entity<Table>, Entity<TableRow>)> {
    let row = cx.world().parent_id(cell)?;
    let table = cx.world().parent_id(row)?;
    let row = Entity::<TableRow>::from_stable_id(row);
    let table = Entity::<Table>::from_stable_id(table);
    (cx.read(row, |_| ()).is_ok() && cx.read(table, |_| ()).is_ok()).then_some((table, row))
}

pub(crate) const TABLE_CELL_POINTER: NodePointerHooks<TableCell> = NodePointerHooks {
    moved: table_cell_moved,
    left: table_cell_left,
    pressed: table_cell_pressed,
    released: table_cell_released,
    wheel: |_, _, _| Ok(false),
};

fn table_cell_pressed(
    cx: &mut AppContext,
    cell: Entity<TableCell>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    let Some((table, row)) = table_of_cell(cx, cell.stable_id()) else {
        return Ok(false);
    };
    let Some(bounds) = cx.world().component_layout_box(cell.stable_id()) else {
        return Ok(false);
    };
    let (header, grabs) = cx.read(cell, |c| {
        (
            c.column_header,
            c.grabs_column_edge(pointer.x, bounds.width),
        )
    })?;
    if grabs {
        cx.update_component(cell, |c, _| {
            c.resize = Some(ColumnResizeDrag {
                origin_x: pointer.x,
                start: bounds.width,
                last: bounds.width,
            });
        })?;
        return Ok(true);
    }
    if !header {
        let gesture = SelectionGesture::Press(selection_modifiers(pointer.modifiers));
        cx.update_component(table, |_, view| {
            view.emit(TableSelectionRequested {
                row: row.stable_id(),
                cell: cell.stable_id(),
                gesture,
            });
        })?;
    }
    // The press goes on to focus the cell as before.
    Ok(false)
}

fn table_cell_moved(
    cx: &mut AppContext,
    cell: Entity<TableCell>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    let Some((drag, extent)) = cx.read(cell, |c| {
        c.resize
            .map(|drag| (drag, c.clamp_extent(drag.start + pointer.x - drag.origin_x)))
    })?
    else {
        return Ok(false);
    };
    if extent != drag.last {
        report_resize(cx, cell, extent, false)?;
    }
    Ok(true)
}

fn table_cell_released(
    cx: &mut AppContext,
    cell: Entity<TableCell>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    let Some(extent) = cx.read(cell, |c| {
        c.resize
            .map(|drag| c.clamp_extent(drag.start + pointer.x - drag.origin_x))
    })?
    else {
        return Ok(false);
    };
    report_resize(cx, cell, extent, true)?;
    Ok(true)
}

/// A cancelled drag puts the column back.
fn table_cell_left(cx: &mut AppContext, cell: Entity<TableCell>) -> Result<(), FrameworkError> {
    let Some(start) = cx.read(cell, |c| c.resize.map(|drag| drag.start))? else {
        return Ok(());
    };
    report_resize(cx, cell, start, true)
}

fn report_resize(
    cx: &mut AppContext,
    cell: Entity<TableCell>,
    extent: f32,
    finished: bool,
) -> Result<(), FrameworkError> {
    let column = cx.update_component(cell, |c, _| {
        if finished {
            c.resize = None;
        } else if let Some(drag) = &mut c.resize {
            drag.last = extent;
        }
        c.column.clone()
    })?;
    let (Some(column), Some((table, _))) = (column, table_of_cell(cx, cell.stable_id())) else {
        return Ok(());
    };
    cx.update_component(table, |_, view| {
        view.emit(TableColumnResized {
            column,
            extent,
            finished,
        });
    })
}

impl AppContext {
    /// Escape during an active header column-resize drag cancels the drag
    /// and restores the column's initial width.
    pub(crate) fn cancel_table_column_resize(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let captures = self.world().pointer_captures(document);
        let mut cancelled = false;
        for (pointer_id, node) in captures {
            let cell = Entity::<TableCell>::from_stable_id(node);
            let Ok(Some(start)) = self.read(cell, |c| c.resize.map(|drag| drag.start)) else {
                continue;
            };
            let _ = self.cancel_pointer_hook(document, pointer_id)?;
            report_resize(self, cell, start, true)?;
            cancelled = true;
        }
        Ok(cancelled)
    }

    /// Whether the window point `(x, y)` over `node` is on a resizable column
    /// boundary, or `node` is dragging one: the cursor shows a resize there.
    pub(crate) fn table_column_edge_at(&self, node: StableNodeId, x: f32, y: f32) -> bool {
        let cell = Entity::<TableCell>::from_stable_id(node);
        let Ok((dragging, candidate)) = self.read(cell, |c| {
            (
                c.resize.is_some(),
                c.column_header && c.resizable && c.column.is_some(),
            )
        }) else {
            return false;
        };
        if dragging {
            return true;
        }
        if !candidate {
            return false;
        }
        let (Some((x, _)), Some(bounds)) = (
            self.world().pointer_layout_position(node, x, y),
            self.world().component_layout_box(node),
        ) else {
            return false;
        };
        self.read(cell, |c| c.grabs_column_edge(x - bounds.x, bounds.width))
            .unwrap_or(false)
    }

    /// The focused table cell, the row and table it is in. A control inside
    /// a cell (an editor) keeps its own keys, so only a cell that itself has
    /// focus counts.
    fn focused_table_cell(
        &self,
        document: DocumentId,
    ) -> Option<(Entity<Table>, Entity<TableRow>, Entity<TableCell>)> {
        let id = self.world().focused(document)?;
        let cell = Entity::<TableCell>::from_stable_id(id);
        self.read(cell, |_| ()).ok()?;
        let (table, row) = table_of_cell(self, id)?;
        Some((table, row, cell))
    }

    /// A selection key while a table cell has focus: Shift with the arrows,
    /// Home / End or Page Up / Down extends from the anchor; Ctrl (Cmd) with
    /// ArrowUp / ArrowDown moves without selecting, Ctrl+Space toggles the
    /// focused row and Ctrl+A selects all. `modifiers` says which of the two
    /// the dispatcher saw.
    pub(crate) fn table_selection_key(
        &mut self,
        document: DocumentId,
        key: &str,
        modifiers: SelectionModifiers,
    ) -> Result<bool, FrameworkError> {
        let Some((table, row, cell)) = self.focused_table_cell(document) else {
            return Ok(false);
        };
        if modifiers.extend {
            let navigation = match key {
                "ArrowUp" => TableNavigation::PreviousRow,
                "ArrowDown" => TableNavigation::NextRow,
                "ArrowLeft" => TableNavigation::PreviousColumn,
                "ArrowRight" => TableNavigation::NextColumn,
                "Home" => TableNavigation::FirstRow,
                "End" => TableNavigation::LastRow,
                "PageUp" => TableNavigation::PageUp,
                "PageDown" => TableNavigation::PageDown,
                _ => return Ok(false),
            };
            return self.navigate_table_with(table, navigation, TABLE_PAGE_ROWS, modifiers);
        }
        if !modifiers.toggle || modifiers.extend {
            return Ok(false);
        }
        let gesture = match key {
            "ArrowUp" => {
                return self.navigate_table_with(
                    table,
                    TableNavigation::PreviousRow,
                    TABLE_PAGE_ROWS,
                    modifiers,
                );
            }
            "ArrowDown" => {
                return self.navigate_table_with(
                    table,
                    TableNavigation::NextRow,
                    TABLE_PAGE_ROWS,
                    modifiers,
                );
            }
            " " | "Space" => SelectionGesture::Toggle,
            _ if key.eq_ignore_ascii_case("a") => SelectionGesture::SelectAll,
            _ => return Ok(false),
        };
        if self.read(cell, |c| c.column_header)? && gesture == SelectionGesture::Toggle {
            return Ok(false);
        }
        self.update_component(table, |_, view| {
            view.emit(TableSelectionRequested {
                row: row.stable_id(),
                cell: cell.stable_id(),
                gesture,
            });
        })?;
        Ok(true)
    }

    /// After a keyboard move put focus on `cell`, ask for the selection to
    /// follow (none for a header row).
    pub(crate) fn request_table_move_selection(
        &mut self,
        table: Entity<Table>,
        cell: StableNodeId,
        modifiers: SelectionModifiers,
    ) -> Result<(), FrameworkError> {
        let cell_entity = Entity::<TableCell>::from_stable_id(cell);
        if self.read(cell_entity, |c| c.column_header).unwrap_or(true) {
            return Ok(());
        }
        let Some(row) = self.world().parent_id(cell) else {
            return Ok(());
        };
        self.update_component(table, |_, view| {
            view.emit(TableSelectionRequested {
                row,
                cell,
                gesture: SelectionGesture::Move(modifiers),
            });
        })
    }

    /// Project a selection onto a virtual table's mounted rows: each row and
    /// its cells get `selected` from `is_selected(row key)`, highlighted and
    /// reported selected to assistive technology. Rows not mounted need
    /// nothing; they are built from the same predicate when they scroll in.
    /// Rows already in the right state are not touched, so a call with an
    /// unchanged selection writes nothing. Returns how many rows changed.
    pub fn sync_virtual_table_selection<R, C>(
        &mut self,
        items: &VirtualTableItems<R, C>,
        mut is_selected: impl FnMut(&R) -> bool,
    ) -> Result<usize, FrameworkError>
    where
        R: Clone + Eq + std::hash::Hash,
        C: Clone + Eq + std::hash::Hash,
    {
        let mut changed = 0;
        for (key, row) in items.rows() {
            let selected = is_selected(key);
            let current = self.read(*row, |row| row.selected)?;
            let cells_differ = self.world().child_ids(row.stable_id()).iter().any(|cell| {
                self.read(Entity::<TableCell>::from_stable_id(*cell), |c| c.selected)
                    .is_ok_and(|cell_selected| cell_selected != selected)
            });
            if current == selected && !cells_differ {
                continue;
            }
            changed += 1;
            self.update_component(*row, |row, _| row.selected = selected)?;
            let cells = self.world().child_ids(row.stable_id()).to_vec();
            for cell in cells {
                let cell = Entity::<TableCell>::from_stable_id(cell);
                if self.read(cell, |c| c.selected != selected).unwrap_or(false) {
                    self.update_component(cell, |c, _| c.selected = selected)?;
                }
            }
        }
        Ok(changed)
    }
}

impl<R, C> VirtualTableItems<R, C>
where
    R: Clone + Eq + std::hash::Hash,
    C: Clone + Eq + std::hash::Hash,
{
    /// The key of the mounted row `node` is, or is inside: the row of a
    /// [`TableSelectionRequested`] or of a focused cell.
    pub fn row_key_of(&self, cx: &AppContext, node: StableNodeId) -> Option<&R> {
        let mut current = Some(node);
        while let Some(id) = current {
            if let Some(key) = self
                .rows()
                .find_map(|(key, row)| (row.stable_id() == id).then_some(key))
            {
                return Some(key);
            }
            current = cx.world().parent_id(id);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nana_ui_core::{
        LengthSpec, SelectionGesture, SelectionModifiers, SelectionState, SemanticColorRole,
        TableColumn, VirtualTableLayout,
    };
    use nana_ui_input::{
        InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, PhysicalKey, PointerInput,
        PointerPhase,
    };

    use super::{TableColumnResized, TableSelectionRequested};
    use crate::{
        AccessibilityRole, AppContext, DocumentId, Entity, HeadlessInput, LayoutViewport,
        ScrollAxes, ScrollView, Table, TableCell, TableRow, VirtualTableItems,
    };

    const WIDTH: f32 = 400.0;
    const HEIGHT: f32 = 200.0;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// A 100-row table of six 60 px columns, header row and first column
    /// frozen, under a scroll port, synced and laid out.
    struct Grid {
        cx: AppContext,
        scroll: Entity<ScrollView>,
        table: Entity<Table>,
        layout: VirtualTableLayout,
        items: VirtualTableItems<usize, usize>,
        selection: SelectionState<usize>,
    }

    impl Grid {
        fn new() -> Self {
            let mut cx = AppContext::new();
            let scroll = cx
                .create_component(
                    document(),
                    ScrollView::new(ScrollAxes::Both).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(WIDTH));
                        layout.height = Some(LengthSpec::Px(HEIGHT));
                    }),
                )
                .unwrap();
            let table = cx
                .create_component(
                    document(),
                    Table::new().label("Files").multiselectable(true),
                )
                .unwrap();
            cx.append_child(scroll, table).unwrap();
            let layout = VirtualTableLayout::new(
                std::iter::repeat_n(20.0, 100),
                (0..6)
                    .map(|column| TableColumn::new(format!("c{column}"), 60.0).limits(30.0, 200.0)),
            );
            let mut grid = Self {
                cx,
                scroll,
                table,
                layout,
                items: VirtualTableItems::default(),
                selection: SelectionState::multiple(),
            };
            // The viewport is known once the scroll port is laid out.
            grid.relayout();
            grid.sync();
            grid
        }

        fn relayout(&mut self) {
            let work = self.cx.take_system_work();
            self.cx.resolve_styles(&work.style).unwrap();
            self.cx
                .layout_document(document(), LayoutViewport::new(WIDTH, HEIGHT))
                .unwrap();
            self.cx.rebuild_hit_test(document());
        }

        fn sync(&mut self) {
            let selection = &self.selection;
            let columns = self.layout.columns().to_vec();
            self.cx
                .sync_virtual_table_retained_in(
                    self.scroll,
                    self.table,
                    &mut self.items,
                    &self.layout,
                    [0.0, 0.0],
                    0,
                    [1, 1],
                    &[],
                    |index| index,
                    |key| Some(*key),
                    |index| index,
                    |key| Some(*key),
                    |_, row| TableRow::new().selected(selection.is_selected(row)),
                    |row, _, column, _| {
                        if row == 0 {
                            TableCell::new(columns[column].key.clone()).header_for(&columns[column])
                        } else {
                            TableCell::new(format!("{row}/{column}"))
                        }
                    },
                )
                .unwrap();
            self.cx
                .sync_virtual_table_selection(&self.items, |key| selection.is_selected(key))
                .unwrap();
            self.relayout();
        }

        fn cell(&self, row: usize, column: usize) -> crate::LayoutBox {
            let cell = self.items.cell_entity(&row, &column).unwrap();
            self.cx
                .world()
                .component_layout_box(cell.stable_id())
                .unwrap()
        }

        /// Rows `0..` in data order, header first: what the selection is
        /// over is the body, row 1 on.
        fn order(&self) -> Vec<usize> {
            (1..self.layout.row_count()).collect()
        }
    }

    fn pointer(phase: PointerPhase, (x, y): (f32, f32), modifiers: InputModifiers) -> InputPayload {
        InputPayload::Pointer(PointerInput {
            modifiers,
            ..PointerInput::mouse(phase, x, y)
        })
    }

    fn key(name: &'static str, modifiers: InputModifiers) -> InputPayload {
        InputPayload::Key(KeyInput {
            physical: PhysicalKey(name.into()),
            logical: LogicalKey(name.into()),
            state: KeyState::Pressed,
            repeat: false,
            modifiers,
        })
    }

    const CTRL: InputModifiers = InputModifiers {
        control: true,
        ..InputModifiers::empty()
    };
    const SHIFT: InputModifiers = InputModifiers {
        shift: true,
        ..InputModifiers::empty()
    };

    #[test]
    fn dragging_a_header_edge_resizes_the_column_and_the_body_follows() {
        let mut grid = Grid::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        grid.cx
            .on(grid.table, move |_, event: &TableColumnResized, _| {
                sink.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let header = grid.cell(0, 1);
        assert_eq!((header.x, header.width), (60.0, 60.0));

        // The edge shows the resize cursor; the middle of the header does not.
        let edge = (
            header.x + header.width - 2.0,
            header.y + header.height / 2.0,
        );
        let cell = grid.items.cell_entity(&0, &1).unwrap().stable_id();
        assert!(grid.cx.table_column_edge_at(cell, edge.0, edge.1));
        assert!(!grid.cx.table_column_edge_at(cell, header.x + 10.0, edge.1));

        let mut input = HeadlessInput::bind(&mut grid.cx, document());
        let none = InputModifiers::empty();
        input
            .route(&mut grid.cx, pointer(PointerPhase::Down, edge, none))
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Move, (edge.0 + 30.0, edge.1), none),
            )
            .unwrap();
        // Far past the column's maximum: the report is clamped.
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Move, (edge.0 + 500.0, edge.1), none),
            )
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Up, (edge.0 + 40.0, edge.1), none),
            )
            .unwrap();
        let heard = std::mem::take(&mut *heard.lock().unwrap());
        assert_eq!(
            heard
                .iter()
                .map(|event| (event.column.as_ref(), event.extent, event.finished))
                .collect::<Vec<_>>(),
            // The release moves the pointer there first: a last preview,
            // then the same width finished.
            [
                ("c1", 90.0, false),
                ("c1", 200.0, false),
                ("c1", 100.0, false),
                ("c1", 100.0, true)
            ]
        );

        // The application applies it; header and body cells of the column,
        // and every column after it, line up again.
        let last = heard.last().unwrap();
        assert!(grid.layout.resize_column_by_key(&last.column, last.extent));
        grid.sync();
        for column in 0..4 {
            let head = grid.cell(0, column);
            let body = grid.cell(5, column);
            assert_eq!(
                (head.x, head.width),
                (body.x, body.width),
                "column {column}"
            );
        }
        assert_eq!(grid.cell(5, 1).width, 100.0);
        assert_eq!(grid.cell(5, 2).x, 160.0);
    }

    #[test]
    fn presses_and_keys_request_selection_and_the_projection_shows_it() {
        let mut grid = Grid::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        grid.cx
            .on(grid.table, move |_, event: &TableSelectionRequested, _| {
                sink.lock().unwrap().push(*event);
            })
            .unwrap();
        let mut input = HeadlessInput::bind(&mut grid.cx, document());
        let at = |grid: &Grid, row: usize| {
            let cell = grid.cell(row, 2);
            (cell.x + cell.width / 2.0, cell.y + cell.height / 2.0)
        };
        // Apply every request to the selection, as an application would.
        let drain = |grid: &mut Grid| {
            let order = grid.order();
            for event in std::mem::take(&mut *heard.lock().unwrap()) {
                let row = *grid.items.row_key_of(&grid.cx, event.row).unwrap();
                grid.selection.apply(event.gesture, row, &order);
            }
            grid.sync();
        };
        let click = |input: &mut HeadlessInput, grid: &mut Grid, row, modifiers| {
            let at = at(grid, row);
            input
                .route(&mut grid.cx, pointer(PointerPhase::Down, at, modifiers))
                .unwrap();
            input
                .route(&mut grid.cx, pointer(PointerPhase::Up, at, modifiers))
                .unwrap();
        };
        let selected = |grid: &Grid| {
            let mut rows = grid.selection.iter().copied().collect::<Vec<_>>();
            rows.sort_unstable();
            rows
        };

        click(&mut input, &mut grid, 2, InputModifiers::empty());
        drain(&mut grid);
        assert_eq!(selected(&grid), [2]);
        click(&mut input, &mut grid, 5, SHIFT);
        drain(&mut grid);
        assert_eq!(selected(&grid), [2, 3, 4, 5]);
        click(&mut input, &mut grid, 3, CTRL);
        drain(&mut grid);
        assert_eq!(selected(&grid), [2, 4, 5]);

        // A press on the header row asks for nothing.
        click(&mut input, &mut grid, 0, InputModifiers::empty());
        assert!(heard.lock().unwrap().is_empty());

        // The keyboard, from a focused body cell.
        let start = grid.items.cell_entity(&5, &2).unwrap().stable_id();
        grid.cx.focus_node(document(), start).unwrap();
        input.route(&mut grid.cx, key("ArrowDown", SHIFT)).unwrap();
        drain(&mut grid);
        assert_eq!(
            selected(&grid),
            [3, 4, 5, 6],
            "Shift extends from the anchor at 3"
        );
        input.route(&mut grid.cx, key("ArrowDown", CTRL)).unwrap();
        drain(&mut grid);
        assert_eq!(grid.selection.current(), Some(&7));
        assert_eq!(
            selected(&grid),
            [3, 4, 5, 6],
            "Ctrl moves without selecting"
        );
        input.route(&mut grid.cx, key(" ", CTRL)).unwrap();
        drain(&mut grid);
        assert_eq!(selected(&grid), [3, 4, 5, 6, 7]);
        input
            .route(&mut grid.cx, key("ArrowDown", InputModifiers::empty()))
            .unwrap();
        drain(&mut grid);
        assert_eq!(selected(&grid), [8]);
        input.route(&mut grid.cx, key("a", CTRL)).unwrap();
        drain(&mut grid);
        assert_eq!(grid.selection.len(), grid.order().len());
        assert!(grid.selection.apply(
            SelectionGesture::Press(SelectionModifiers::NONE),
            4,
            &grid.order()
        ));
        grid.sync();

        // Mounted rows show it: highlighted and reported selected.
        let row = grid.items.row_entity(&4).unwrap().stable_id();
        let cell = grid.items.cell_entity(&4, &2).unwrap().stable_id();
        let other = grid.items.cell_entity(&5, &2).unwrap().stable_id();
        assert_eq!(
            grid.cx.world().accessibility(row).unwrap().selected,
            Some(true)
        );
        assert_eq!(
            grid.cx.world().node_style(cell).unwrap().background,
            Some(SemanticColorRole::Selected)
        );
        assert_eq!(
            grid.cx.world().node_style(other).unwrap().background,
            Some(SemanticColorRole::Surface)
        );
        let spoken = grid.cx.world().project_accessibility(document());
        let table = spoken
            .iter()
            .find(|node| node.role == AccessibilityRole::Table)
            .unwrap();
        assert!(table.multiselectable);
    }

    #[test]
    fn ctrl_shift_arrow_adds_disjoint_range_in_table() {
        let mut grid = Grid::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        grid.cx
            .on(grid.table, move |_, event: &TableSelectionRequested, _| {
                sink.lock().unwrap().push(*event);
            })
            .unwrap();
        let mut input = HeadlessInput::bind(&mut grid.cx, document());
        let order = grid.order();
        let drain = |grid: &mut Grid| {
            for event in std::mem::take(&mut *heard.lock().unwrap()) {
                let row = *grid.items.row_key_of(&grid.cx, event.row).unwrap();
                grid.selection.apply(event.gesture, row, &order);
            }
            grid.sync();
        };
        let click = |input: &mut HeadlessInput, grid: &mut Grid, row, modifiers| {
            let cell = grid.cell(row, 2);
            let at = (cell.x + cell.width / 2.0, cell.y + cell.height / 2.0);
            input
                .route(&mut grid.cx, pointer(PointerPhase::Down, at, modifiers))
                .unwrap();
            input
                .route(&mut grid.cx, pointer(PointerPhase::Up, at, modifiers))
                .unwrap();
        };
        let selected = |grid: &Grid| {
            let mut rows = grid.selection.iter().copied().collect::<Vec<_>>();
            rows.sort_unstable();
            rows
        };

        click(&mut input, &mut grid, 2, InputModifiers::empty());
        drain(&mut grid);
        click(&mut input, &mut grid, 6, CTRL);
        drain(&mut grid);
        assert_eq!(selected(&grid), [2, 6]);

        let ctrl_shift = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::empty()
        };
        let focus = grid.items.cell_entity(&6, &2).unwrap().stable_id();
        grid.cx.focus_node(document(), focus).unwrap();
        input
            .route(&mut grid.cx, key("ArrowDown", ctrl_shift))
            .unwrap();
        drain(&mut grid);
        assert_eq!(selected(&grid), [2, 6, 7]);
    }

    #[test]
    fn scrolled_frozen_table_hits_drags_resizes_and_reorders_without_fingerprint_bump() {
        let mut grid = Grid::new();
        let selection_events = Arc::new(Mutex::new(Vec::new()));
        let sel_sink = Arc::clone(&selection_events);
        grid.cx
            .on(grid.table, move |_, event: &TableSelectionRequested, _| {
                sel_sink.lock().unwrap().push(*event);
            })
            .unwrap();
        let resize_events = Arc::new(Mutex::new(Vec::new()));
        let res_sink = Arc::clone(&resize_events);
        grid.cx
            .on(grid.table, move |_, event: &TableColumnResized, _| {
                res_sink.lock().unwrap().push(event.clone());
            })
            .unwrap();

        // Scroll both axes and resync frozen corner/header/column.
        grid.cx
            .scroll_to(grid.scroll, crate::ScrollOffset { x: 120.0, y: 200.0 })
            .unwrap();
        grid.sync();

        let mut input = HeadlessInput::bind(&mut grid.cx, document());
        let visual_center = |grid: &Grid, row: usize, column: usize| {
            let id = grid.items.cell_entity(&row, &column).unwrap().stable_id();
            let b = grid.cx.world().presentation_input_bounds(id).unwrap();
            (b.x + b.width / 2.0, b.y + b.height / 2.0)
        };
        let click = |input: &mut HeadlessInput, grid: &mut Grid, row, column| {
            let at = visual_center(grid, row, column);
            input
                .route(
                    &mut grid.cx,
                    pointer(PointerPhase::Down, at, InputModifiers::empty()),
                )
                .unwrap();
            input
                .route(
                    &mut grid.cx,
                    pointer(PointerPhase::Up, at, InputModifiers::empty()),
                )
                .unwrap();
        };

        // Corner and scrolled header raise no selection request; frozen column & body do.
        click(&mut input, &mut grid, 0, 0);
        click(&mut input, &mut grid, 0, 3);
        assert!(selection_events.lock().unwrap().is_empty());

        click(&mut input, &mut grid, 12, 0);
        click(&mut input, &mut grid, 14, 3);
        let rows: Vec<_> = std::mem::take(&mut *selection_events.lock().unwrap())
            .into_iter()
            .map(|e| *grid.items.row_key_of(&grid.cx, e.row).unwrap())
            .collect();
        assert_eq!(rows, [12, 14]);

        // Drag trailing edge of scrolled header (0, 3) and apply without changing fingerprint.
        let h3_id = grid.items.cell_entity(&0, &3).unwrap().stable_id();
        let h3 = grid.cx.world().presentation_input_bounds(h3_id).unwrap();
        let edge3 = (h3.x + h3.width - 2.0, h3.y + h3.height / 2.0);
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Down, edge3, InputModifiers::empty()),
            )
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(
                    PointerPhase::Move,
                    (edge3.0 + 35.0, edge3.1),
                    InputModifiers::empty(),
                ),
            )
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(
                    PointerPhase::Up,
                    (edge3.0 + 35.0, edge3.1),
                    InputModifiers::empty(),
                ),
            )
            .unwrap();
        let last = resize_events.lock().unwrap().pop().unwrap();
        assert_eq!(
            (last.column.as_ref(), last.extent, last.finished),
            ("c3", 95.0, true)
        );
        assert!(grid.layout.resize_column_by_key(&last.column, last.extent));
        // Equal-width column move preserves total_extent, detected via VirtualTableLayout::revision().
        assert!(grid.layout.move_column(2, 4));
        assert_eq!(grid.layout.column_index("c3"), Some(2));
        grid.sync();
        assert_eq!(grid.cell(0, 2).width, 95.0);
        assert_eq!(grid.cell(14, 2).width, 95.0);
        for column in [0, 2, 3, 4] {
            let head = grid.cell(0, column);
            let body = grid.cell(14, column);
            assert_eq!((head.x, head.width), (body.x, body.width));
        }
    }

    #[test]
    fn escape_and_pointer_cancel_restore_column_width_and_table_below_sibling_aligns() {
        let mut grid = Grid::new();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        grid.cx
            .on(grid.table, move |_, event: &TableColumnResized, _| {
                sink.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let header = grid.cell(0, 1);
        let edge = (
            header.x + header.width - 2.0,
            header.y + header.height / 2.0,
        );
        let mut input = HeadlessInput::bind(&mut grid.cx, document());
        let none = InputModifiers::empty();

        // Escape cancels drag back to initial width and drops capture.
        input
            .route(&mut grid.cx, pointer(PointerPhase::Down, edge, none))
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Move, (edge.0 + 30.0, edge.1), none),
            )
            .unwrap();
        input.route(&mut grid.cx, key("Escape", none)).unwrap();
        let events = std::mem::take(&mut *heard.lock().unwrap());
        assert_eq!(
            events.last().map(|e| (e.extent, e.finished)),
            Some((60.0, true))
        );
        assert!(grid.cx.world().pointer_captures(document()).is_empty());

        // PointerPhase::Cancel also cancels drag back to initial width.
        input
            .route(&mut grid.cx, pointer(PointerPhase::Down, edge, none))
            .unwrap();
        input
            .route(
                &mut grid.cx,
                pointer(PointerPhase::Move, (edge.0 + 25.0, edge.1), none),
            )
            .unwrap();
        input
            .route(&mut grid.cx, pointer(PointerPhase::Cancel, edge, none))
            .unwrap();
        let events = std::mem::take(&mut *heard.lock().unwrap());
        assert_eq!(
            events.last().map(|e| (e.extent, e.finished)),
            Some((60.0, true))
        );
        assert!(grid.cx.world().pointer_captures(document()).is_empty());

        // Table below an 80 px sibling inside a ScrollView offsets viewport & pinned header.
        let mut cx = AppContext::new();
        let scroll = cx
            .create_component(
                document(),
                ScrollView::new(ScrollAxes::Both).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(WIDTH));
                    layout.height = Some(LengthSpec::Px(HEIGHT));
                }),
            )
            .unwrap();
        let wrapper = cx
            .create_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let banner = cx
            .create_component(
                document(),
                crate::Stack::row(0.0).height(LengthSpec::Px(80.0)),
            )
            .unwrap();
        let table = cx.create_component(document(), Table::new()).unwrap();
        cx.append_child(scroll, wrapper).unwrap();
        cx.append_child(wrapper, banner).unwrap();
        cx.append_child(wrapper, table).unwrap();

        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(20.0, 50),
            (0..3).map(|c| TableColumn::new(format!("c{c}"), 80.0)),
        );
        let mut items: VirtualTableItems<usize, usize> = VirtualTableItems::default();
        let sync_once = |cx: &mut AppContext, items: &mut VirtualTableItems<usize, usize>| {
            let work = cx.take_system_work();
            cx.resolve_styles(&work.style).unwrap();
            cx.layout_document(document(), LayoutViewport::new(WIDTH, HEIGHT))
                .unwrap();
            cx.sync_virtual_table_retained_in(
                scroll,
                table,
                items,
                &layout,
                [0.0, 0.0],
                0,
                [0, 1],
                &[],
                |i| i,
                |k| Some(*k),
                |i| i,
                |k| Some(*k),
                |_, _| TableRow::new(),
                |row, _, col, _| TableCell::new(format!("{row}/{col}")),
            )
            .unwrap();
            let work = cx.take_system_work();
            cx.resolve_styles(&work.style).unwrap();
            cx.layout_document(document(), LayoutViewport::new(WIDTH, HEIGHT))
                .unwrap();
        };

        sync_once(&mut cx, &mut items);
        let head = items.row_entity(&0).unwrap().stable_id();
        assert_eq!(
            cx.world()
                .node_style(head)
                .unwrap()
                .layout
                .transforms
                .transform
                .unwrap()
                .f,
            0.0
        );

        cx.scroll_to(scroll, crate::ScrollOffset { x: 0.0, y: 120.0 })
            .unwrap();
        sync_once(&mut cx, &mut items);
        assert_eq!(
            cx.world()
                .node_style(head)
                .unwrap()
                .layout
                .transforms
                .transform
                .unwrap()
                .f,
            40.0
        );
    }
}
