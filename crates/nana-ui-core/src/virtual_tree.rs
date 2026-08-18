//! Flattened visible-row geometry for disclosure trees.
//!
//! Collapsed subtrees are absent from the Fenwick index. Expanding a branch
//! inserts descendant extents after the parent; collapsing removes them.
//! Window queries share the logarithmic range lookup of [`VirtualListLayout`].

use crate::{VirtualListLayout, VirtualListWindow};

/// One currently visible (expanded-walk) row in a virtual tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualTreeRow {
    pub extent: f32,
    /// Visible descendants that currently follow this row. Zero when collapsed
    /// or when the row is a leaf.
    pub descendant_count: usize,
}

/// Visible-row Fenwick for a disclosure tree. Logical nodes that are collapsed
/// (and their descendants) are not stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualTreeLayout {
    rows: VirtualListLayout,
    descendant_counts: Vec<usize>,
}

pub type VirtualTreeWindow = VirtualListWindow;

impl VirtualTreeLayout {
    /// `rows` are the currently visible (expanded-walk) extents together with
    /// how many visible descendants follow each row.
    pub fn new(rows: impl IntoIterator<Item = VirtualTreeRow>) -> Self {
        let rows = rows.into_iter().collect::<Vec<_>>();
        Self {
            rows: VirtualListLayout::new(rows.iter().map(|row| row.extent)),
            descendant_counts: rows.iter().map(|row| row.descendant_count).collect(),
        }
    }

    /// Uniform-height flattened walk. `descendant_counts[i]` must describe the
    /// currently visible subtree following row `i`.
    pub fn uniform(row_extent: f32, descendant_counts: impl IntoIterator<Item = usize>) -> Self {
        Self::new(
            descendant_counts
                .into_iter()
                .map(|descendant_count| VirtualTreeRow {
                    extent: row_extent,
                    descendant_count,
                }),
        )
    }

    pub fn visible_len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn total_extent(&self) -> f32 {
        self.rows.total_extent()
    }

    pub fn descendant_count(&self, index: usize) -> Option<usize> {
        self.descendant_counts.get(index).copied()
    }

    pub fn row_layout(&self) -> &VirtualListLayout {
        &self.rows
    }

    pub fn window(
        &self,
        scroll_offset: f32,
        viewport_extent: f32,
        overscan_extent: f32,
    ) -> VirtualTreeWindow {
        self.rows
            .window(scroll_offset, viewport_extent, overscan_extent)
    }

    pub fn update_row_extent(&mut self, index: usize, extent: f32) -> bool {
        self.rows.update_item_extent(index, extent)
    }

    /// Insert a flattened expanded walk immediately after `parent`.
    ///
    /// `parent` must currently have no visible descendants. Nested already-
    /// expanded children are encoded in `descendants` itself.
    pub fn expand(
        &mut self,
        parent: usize,
        descendants: impl IntoIterator<Item = VirtualTreeRow>,
    ) -> bool {
        if parent >= self.descendant_counts.len() || self.descendant_counts[parent] != 0 {
            return false;
        }
        let descendants = descendants.into_iter().collect::<Vec<_>>();
        if descendants.is_empty() {
            return false;
        }
        let inserted = descendants.len();
        self.rows
            .insert_items(parent + 1, descendants.iter().map(|row| row.extent));
        self.descendant_counts.splice(
            parent + 1..parent + 1,
            descendants.iter().map(|row| row.descendant_count),
        );
        self.descendant_counts[parent] = inserted;
        for ancestor in 0..parent {
            if self.contains_visible_index(ancestor, parent) {
                self.descendant_counts[ancestor] += inserted;
            }
        }
        true
    }

    /// Remove the visible descendants currently following `parent`.
    pub fn collapse(&mut self, parent: usize) -> bool {
        let Some(&removed) = self.descendant_counts.get(parent) else {
            return false;
        };
        if removed == 0 {
            return false;
        }
        for ancestor in 0..parent {
            if self.contains_visible_index(ancestor, parent) {
                self.descendant_counts[ancestor] -= removed;
            }
        }
        self.rows.remove_items(parent + 1..parent + 1 + removed);
        self.descendant_counts
            .drain(parent + 1..parent + 1 + removed);
        self.descendant_counts[parent] = 0;
        true
    }

    fn contains_visible_index(&self, ancestor: usize, index: usize) -> bool {
        let end = ancestor + 1 + self.descendant_counts[ancestor];
        ancestor < index && index < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 20.0;
    const VIEWPORT: f32 = 100.0;
    const OVERSCAN: f32 = 20.0;

    fn leaf() -> VirtualTreeRow {
        VirtualTreeRow {
            extent: ROW,
            descendant_count: 0,
        }
    }

    fn geometric_cap() -> usize {
        VirtualListLayout::uniform_window_item_cap(VIEWPORT, OVERSCAN, ROW)
    }

    #[test]
    fn window_is_bounded_by_geometry_not_logical_size() {
        let layout = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, 10_000));
        let cap = geometric_cap();
        assert!(cap < 10_000);
        let window = layout.window(0.0, VIEWPORT, OVERSCAN);
        assert!(window.range.len() <= cap);
        assert!(window.range.len() < layout.visible_len());
        let scrolled = layout.window(80.0, VIEWPORT, OVERSCAN);
        assert!(scrolled.range.start > 0);
        assert!(scrolled.range.len() <= cap);
    }

    #[test]
    fn expand_and_collapse_insert_visible_descendants_into_the_fenwick() {
        let mut layout = VirtualTreeLayout::uniform(ROW, [0, 0, 0]);
        assert_eq!(layout.visible_len(), 3);
        assert_eq!(layout.total_extent(), 60.0);
        assert!(layout.expand(0, [leaf(), leaf()]));
        assert_eq!(layout.visible_len(), 5);
        assert_eq!(layout.descendant_count(0), Some(2));
        assert_eq!(layout.total_extent(), 100.0);
        assert!(!layout.expand(0, [leaf()]));
        assert!(layout.collapse(0));
        assert_eq!(layout.visible_len(), 3);
        assert_eq!(layout.descendant_count(0), Some(0));
        assert_eq!(layout.total_extent(), 60.0);
        assert!(!layout.collapse(0));
        assert!(!layout.expand(99, [leaf()]));
    }

    #[test]
    fn nested_expand_updates_ancestor_descendant_counts() {
        let mut layout = VirtualTreeLayout::uniform(ROW, [2, 0, 0]);
        assert!(layout.expand(1, [leaf(), leaf()]));
        assert_eq!(layout.visible_len(), 5);
        assert_eq!(layout.descendant_count(0), Some(4));
        assert_eq!(layout.descendant_count(1), Some(2));
        assert!(layout.collapse(1));
        assert_eq!(layout.visible_len(), 3);
        assert_eq!(layout.descendant_count(0), Some(2));
        assert_eq!(layout.descendant_count(1), Some(0));
    }

    #[test]
    fn materialization_reuses_overlap_on_scroll_and_expand() {
        let mut keys = (0..10_000).collect::<Vec<_>>();
        let mut layout = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, keys.len()));
        let mut materializer = crate::VirtualListMaterializer::default();
        let first = materializer
            .prepare(layout.row_layout(), 0.0, VIEWPORT, OVERSCAN, |index| {
                keys[index]
            })
            .unwrap();
        let cap = geometric_cap();
        assert!(first.order.len() <= cap);
        assert!(first.unmounts.is_empty());
        assert_eq!(first.mounts.len(), first.order.len());
        assert!(materializer.commit(first).unwrap());

        let scrolled = materializer
            .prepare(layout.row_layout(), 80.0, VIEWPORT, OVERSCAN, |index| {
                keys[index]
            })
            .unwrap();
        assert!(!scrolled.mounts.is_empty());
        assert!(!scrolled.unmounts.is_empty());
        assert!(scrolled.mounts.len() < scrolled.order.len());
        assert!(scrolled.order.len() <= cap);
        let overlap = scrolled.order[0];
        assert!(materializer.commit(scrolled).unwrap());
        assert!(materializer.mounted().contains(&overlap));

        let parent = keys.iter().position(|key| *key == overlap).unwrap();
        let child_keys = [1_000_000usize, 1_000_001];
        assert!(layout.expand(parent, [leaf(), leaf()]));
        keys.splice(parent + 1..parent + 1, child_keys);
        let expanded = materializer
            .prepare(layout.row_layout(), 80.0, VIEWPORT, OVERSCAN, |index| {
                keys[index]
            })
            .unwrap();
        assert!(expanded.order.contains(&overlap));
        assert!(expanded.order.len() <= cap);
        assert!(
            expanded
                .mounts
                .iter()
                .any(|mount| child_keys.contains(&mount.key))
        );
        assert!(materializer.commit(expanded).unwrap());
        assert_eq!(
            materializer.mounted().len(),
            materializer
                .prepare(layout.row_layout(), 80.0, VIEWPORT, OVERSCAN, |index| {
                    keys[index]
                })
                .unwrap()
                .order
                .len()
        );
    }
}
