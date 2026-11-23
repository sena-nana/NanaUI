//! A large virtual table allocates nothing while nothing changes (#117).
//!
//! Each case builds a table of `ROWS` rows by 40 columns under a scroll port,
//! syncs it once and warms up, then counts the allocations of a steady run:
//!
//! - re-syncing an unchanged viewport;
//! - scrolling inside the materialized window without frozen panes;
//! - projecting an unchanged selection onto the mounted rows.
//!
//! Each holds at 10 000 and at 100 000 rows, and scrolling frozen panes
//! (which moves their mounted nodes) costs the same at both: the cost
//! follows the window, never the data.

use crate::{ALLOCATIONS, COUNTING};

use nana_ui_core::{SelectionState, TableColumn, VirtualTableLayout};
use nana_ui_runtime::{
    AppContext, DocumentId, Entity, ScrollAxes, ScrollMetrics, ScrollOffset, ScrollView, Table,
    TableCell, TableRow, VirtualTableItems,
};

const COLUMNS: usize = 40;
const ROW_HEIGHT: f32 = 20.0;
const COLUMN_WIDTH: f32 = 80.0;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

struct Fixture {
    cx: AppContext,
    scroll: Entity<ScrollView>,
    table: Entity<Table>,
    layout: VirtualTableLayout,
    items: VirtualTableItems<usize, usize>,
    frozen: [usize; 2],
}

impl Fixture {
    fn new(rows: usize, frozen: [usize; 2]) -> Self {
        let mut cx = AppContext::new();
        let scroll = cx
            .create_component(document(), ScrollView::new(ScrollAxes::Both))
            .unwrap();
        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(ROW_HEIGHT, rows),
            (0..COLUMNS).map(|column| TableColumn::new(column.to_string(), COLUMN_WIDTH)),
        );
        cx.set_scroll_metrics(
            scroll,
            ScrollMetrics {
                viewport_width: 640.0,
                viewport_height: 400.0,
                content_width: layout.column_layout().total_extent(),
                content_height: layout.row_layout().total_extent(),
                origin_x: 0.0,
                origin_y: 0.0,
            },
        )
        .unwrap();
        let table = cx
            .create_component(document(), Table::new().multiselectable(true))
            .unwrap();
        let mut fixture = Self {
            cx,
            scroll,
            table,
            layout,
            items: VirtualTableItems::default(),
            frozen,
        };
        fixture.sync();
        fixture
    }

    fn sync(&mut self) {
        self.cx
            .sync_virtual_table_retained_in(
                self.scroll,
                self.table,
                &mut self.items,
                &self.layout,
                [ROW_HEIGHT * 2.0, COLUMN_WIDTH],
                0,
                self.frozen,
                &[],
                |index| index,
                |key| Some(*key),
                |index| index,
                |key| Some(*key),
                |_, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}/{column}")),
            )
            .unwrap();
    }

    fn scroll_to(&mut self, y: f32) {
        self.cx
            .scroll_to(self.scroll, ScrollOffset { x: 0.0, y })
            .unwrap();
    }
}

fn counted(run: impl FnOnce()) -> u64 {
    ALLOCATIONS.with(|count| count.set(0));
    COUNTING.with(|counting| counting.set(true));
    run();
    COUNTING.with(|counting| counting.set(false));
    ALLOCATIONS.with(std::cell::Cell::get)
}

#[test]
fn an_unchanged_table_resyncs_without_allocating() {
    for rows in [10_000, 100_000] {
        for frozen in [[0, 0], [1, 1]] {
            let mut table = Fixture::new(rows, frozen);
            table.scroll_to(4_000.0);
            table.sync();
            table.sync();
            let allocations = counted(|| {
                for _ in 0..200 {
                    table.sync();
                }
            });
            assert_eq!(
                allocations, 0,
                "{rows} rows, frozen {frozen:?}: an unchanged viewport re-synced with allocations"
            );
        }
    }
}

#[test]
fn scrolling_inside_the_window_allocates_nothing() {
    for rows in [10_000, 100_000] {
        let mut table = Fixture::new(rows, [0, 0]);
        table.scroll_to(4_005.0);
        table.sync();
        // Overscan is two rows: moving 0..10 px from 4005 crosses no row
        // boundary at either end of the window, so the window stays.
        for step in 0..4 {
            table.scroll_to(4_005.0 + step as f32);
            table.sync();
        }
        // Moving the scroll offset is the scroll view's business; the
        // count is the table's sync that follows each move.
        let mut allocations = 0;
        for step in 0..200 {
            table.scroll_to(4_005.0 + (step % 10) as f32);
            allocations += counted(|| table.sync());
        }
        assert_eq!(
            allocations, 0,
            "{rows} rows: scrolling inside the window allocated"
        );
    }
}

#[test]
fn projecting_an_unchanged_selection_allocates_nothing() {
    for rows in [10_000, 100_000] {
        let mut table = Fixture::new(rows, [1, 1]);
        let order = (0..rows).collect::<Vec<_>>();
        let mut selection = SelectionState::multiple();
        selection.select_all(&order);
        table
            .cx
            .sync_virtual_table_selection(&table.items, |key| selection.is_selected(key))
            .unwrap();
        let allocations = counted(|| {
            for _ in 0..200 {
                let changed = table
                    .cx
                    .sync_virtual_table_selection(&table.items, |key| selection.is_selected(key))
                    .unwrap();
                assert_eq!(changed, 0);
            }
        });
        assert_eq!(
            allocations, 0,
            "{rows} rows: an unchanged selection projected with allocations"
        );
    }
}

#[test]
fn frozen_panes_cost_the_same_at_any_row_count() {
    // Frozen header and first column follow the scroll by a transform on
    // each of their mounted nodes: work in the window, none in the data.
    let per_step = |rows: usize| {
        let mut table = Fixture::new(rows, [1, 1]);
        table.scroll_to(4_005.0);
        table.sync();
        let mut allocations = 0;
        for step in 1..=50 {
            table.scroll_to(4_005.0 + (step % 10) as f32);
            allocations += counted(|| table.sync());
        }
        allocations
    };
    assert_eq!(per_step(10_000), per_step(100_000));
}
