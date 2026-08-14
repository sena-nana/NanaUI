//! Backend-neutral geometry and keyboard navigation for virtualized tables.

use std::hash::Hash;

use crate::{
    VirtualListLayout, VirtualListMaterialization, VirtualListMaterializationError,
    VirtualListMaterializer, VirtualListWindow,
};

#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub key: String,
    pub extent: f32,
    pub min_extent: f32,
    pub max_extent: f32,
    pub resizable: bool,
}

impl TableColumn {
    pub fn new(key: impl Into<String>, extent: f32) -> Self {
        let extent = sanitize_extent(extent);
        Self {
            key: key.into(),
            extent,
            min_extent: 0.0,
            max_extent: f32::INFINITY,
            resizable: true,
        }
    }

    pub fn limits(mut self, min_extent: f32, max_extent: f32) -> Self {
        self.min_extent = sanitize_extent(min_extent);
        self.max_extent = if max_extent.is_finite() {
            sanitize_extent(max_extent).max(self.min_extent)
        } else {
            f32::INFINITY
        };
        self.extent = self.extent.clamp(self.min_extent, self.max_extent);
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualTableWindow {
    pub rows: VirtualListWindow,
    pub columns: VirtualListWindow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualTableMaterialization<R, C> {
    pub window: VirtualTableWindow,
    pub rows: VirtualListMaterialization<R>,
    pub columns: VirtualListMaterialization<C>,
}

/// Shares the keyed, revision-fenced reconciliation contract between both
/// table axes. It owns visible identity only, never application data or views.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualTableMaterializer<R, C> {
    rows: VirtualListMaterializer<R>,
    columns: VirtualListMaterializer<C>,
}

impl<R, C> Default for VirtualTableMaterializer<R, C> {
    fn default() -> Self {
        Self {
            rows: VirtualListMaterializer::default(),
            columns: VirtualListMaterializer::default(),
        }
    }
}

impl<R, C> VirtualTableMaterializer<R, C>
where
    R: Clone + Eq + Hash,
    C: Clone + Eq + Hash,
{
    pub fn mounted_rows(&self) -> &[R] {
        self.rows.mounted()
    }

    pub fn mounted_columns(&self) -> &[C] {
        self.columns.mounted()
    }

    pub fn prepare(
        &self,
        layout: &VirtualTableLayout,
        scroll: (f32, f32),
        viewport: (f32, f32),
        overscan: (f32, f32),
        row_key_at: impl FnMut(usize) -> R,
        column_key_at: impl FnMut(usize) -> C,
    ) -> Result<VirtualTableMaterialization<R, C>, VirtualListMaterializationError> {
        let window = layout.window(scroll, viewport, overscan);
        let rows = self.rows.prepare_window(window.rows.clone(), row_key_at)?;
        let columns = self
            .columns
            .prepare_window(window.columns.clone(), column_key_at)?;
        Ok(VirtualTableMaterialization {
            window,
            rows,
            columns,
        })
    }

    pub fn commit(
        &mut self,
        plan: VirtualTableMaterialization<R, C>,
    ) -> Result<bool, VirtualListMaterializationError> {
        if plan.rows.base_revision != self.rows.revision()
            || plan.columns.base_revision != self.columns.revision()
        {
            return Err(VirtualListMaterializationError::StalePlan);
        }
        let rows_changed = self
            .rows
            .commit(plan.rows)
            .expect("table row plan revision was checked");
        let columns_changed = self
            .columns
            .commit(plan.columns)
            .expect("table column plan revision was checked");
        Ok(rows_changed || columns_changed)
    }
}

/// Retained two-dimensional table geometry. Rows and columns use the same
/// logarithmic Fenwick index, so scrolling and one measurement update never
/// require rebuilding the full data set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualTableLayout {
    rows: VirtualListLayout,
    columns: Vec<TableColumn>,
    column_extents: VirtualListLayout,
}

impl VirtualTableLayout {
    pub fn new(
        row_extents: impl IntoIterator<Item = f32>,
        columns: impl IntoIterator<Item = TableColumn>,
    ) -> Self {
        let columns = columns.into_iter().collect::<Vec<_>>();
        let column_extents = VirtualListLayout::new(columns.iter().map(|column| column.extent));
        Self {
            rows: VirtualListLayout::new(row_extents),
            columns,
            column_extents,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }

    pub fn update_row_extent(&mut self, row: usize, extent: f32) -> bool {
        self.rows.update_item_extent(row, extent)
    }

    pub fn resize_column(&mut self, column: usize, extent: f32) -> bool {
        let Some(model) = self.columns.get_mut(column) else {
            return false;
        };
        if !model.resizable {
            return false;
        }
        let extent = sanitize_extent(extent).clamp(model.min_extent, model.max_extent);
        if model.extent == extent {
            return false;
        }
        model.extent = extent;
        self.column_extents.update_item_extent(column, extent)
    }

    pub fn window(
        &self,
        scroll: (f32, f32),
        viewport: (f32, f32),
        overscan: (f32, f32),
    ) -> VirtualTableWindow {
        VirtualTableWindow {
            rows: self.rows.window(scroll.1, viewport.1, overscan.1),
            columns: self.column_extents.window(scroll.0, viewport.0, overscan.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCursor {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableNavigation {
    PreviousRow,
    NextRow,
    PreviousColumn,
    NextColumn,
    RowStart,
    RowEnd,
    FirstRow,
    LastRow,
    PageUp,
    PageDown,
}

impl TableCursor {
    pub fn navigate(
        &mut self,
        navigation: TableNavigation,
        row_count: usize,
        column_count: usize,
        page_rows: usize,
    ) -> bool {
        if row_count == 0 || column_count == 0 {
            return false;
        }
        self.row = self.row.min(row_count - 1);
        self.column = self.column.min(column_count - 1);
        let previous = *self;
        match navigation {
            TableNavigation::PreviousRow => self.row = self.row.saturating_sub(1),
            TableNavigation::NextRow => self.row = (self.row + 1).min(row_count - 1),
            TableNavigation::PreviousColumn => self.column = self.column.saturating_sub(1),
            TableNavigation::NextColumn => {
                self.column = (self.column + 1).min(column_count - 1);
            }
            TableNavigation::RowStart => self.column = 0,
            TableNavigation::RowEnd => self.column = column_count - 1,
            TableNavigation::FirstRow => self.row = 0,
            TableNavigation::LastRow => self.row = row_count - 1,
            TableNavigation::PageUp => self.row = self.row.saturating_sub(page_rows.max(1)),
            TableNavigation::PageDown => {
                self.row = self.row.saturating_add(page_rows.max(1)).min(row_count - 1);
            }
        }
        *self != previous
    }
}

fn sanitize_extent(extent: f32) -> f32 {
    if extent.is_finite() && extent > 0.0 {
        extent
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_dimensional_window_and_column_resize_are_incremental() {
        let mut table = VirtualTableLayout::new(
            std::iter::repeat_n(20.0, 10_000),
            (0..100).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
        );
        let window = table.window((4_000.0, 100_000.0), (640.0, 800.0), (80.0, 200.0));
        assert!(window.rows.range.start > 0);
        assert!(window.rows.range.end < 10_000);
        assert!(window.columns.range.start > 0);
        assert!(window.columns.range.end < 100);

        assert!(table.resize_column(50, 120.0));
        assert_eq!(table.columns()[50].extent, 120.0);
        assert!(!table.resize_column(50, 120.0));
    }

    #[test]
    fn navigation_clamps_to_the_available_grid() {
        let mut cursor = TableCursor { row: 5, column: 2 };
        assert!(cursor.navigate(TableNavigation::PageDown, 10, 3, 4));
        assert_eq!(cursor, TableCursor { row: 9, column: 2 });
        assert!(cursor.navigate(TableNavigation::RowStart, 10, 3, 4));
        assert_eq!(cursor, TableCursor { row: 9, column: 0 });
        assert!(!cursor.navigate(TableNavigation::PreviousColumn, 10, 3, 4));
        assert!(cursor.navigate(TableNavigation::FirstRow, 10, 3, 4));
        assert_eq!(cursor, TableCursor { row: 0, column: 0 });
    }

    #[test]
    fn table_materialization_reuses_both_visible_axes_and_rejects_stale_plan() {
        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(20.0, 10_000),
            (0..100).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
        );
        let mut materializer = VirtualTableMaterializer::default();
        let first = materializer
            .prepare(
                &layout,
                (0.0, 0.0),
                (160.0, 100.0),
                (0.0, 0.0),
                |index| index,
                |index| index,
            )
            .unwrap();
        let stale = first.clone();
        assert_eq!(first.rows.mounts.len(), first.window.rows.range.len());
        assert_eq!(first.columns.mounts.len(), first.window.columns.range.len());
        assert!(materializer.commit(first).unwrap());

        let shifted = materializer
            .prepare(
                &layout,
                (80.0, 40.0),
                (160.0, 100.0),
                (0.0, 0.0),
                |index| index,
                |index| index,
            )
            .unwrap();
        assert!(shifted.rows.mounts.len() < shifted.rows.order.len());
        assert!(shifted.columns.mounts.len() < shifted.columns.order.len());
        assert!(materializer.commit(shifted).unwrap());
        assert_eq!(
            materializer.commit(stale),
            Err(VirtualListMaterializationError::StalePlan)
        );
    }
}
