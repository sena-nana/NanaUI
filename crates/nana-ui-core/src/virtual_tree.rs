//! Flattened visible-row geometry for disclosure trees.
//!
//! Collapsed subtrees are absent from the index. Expanding a branch inserts
//! descendant extents after the parent; collapsing removes them. Window
//! queries share the logarithmic range lookup of [`VirtualListLayout`].
//!
//! Each visible row also keeps how many visible descendants follow it and
//! its depth, in chunks: expanding or collapsing touches the chunk it edits
//! and the rows above the parent, which a search walking back by depth finds
//! while skipping every chunk too deep to hold one -- never every row before
//! the parent.

use crate::{VirtualListLayout, VirtualListWindow};

/// One currently visible (expanded-walk) row in a virtual tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualTreeRow {
    pub extent: f32,
    /// Visible descendants that currently follow this row. Zero when collapsed
    /// or when the row is a leaf.
    pub descendant_count: usize,
}

/// Rows a metadata chunk holds before it splits.
const META_CHUNK_ROWS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
struct RowMeta {
    descendants: usize,
    depth: u32,
}

#[derive(Debug, Clone, Default)]
struct MetaChunk {
    rows: Vec<RowMeta>,
    /// The shallowest row in the chunk: a search for a row shallower than
    /// this skips the chunk whole.
    min_depth: u32,
}

impl MetaChunk {
    fn new(rows: Vec<RowMeta>) -> Self {
        let min_depth = rows.iter().map(|row| row.depth).min().unwrap_or(u32::MAX);
        Self { rows, min_depth }
    }

    fn refresh(&mut self) {
        self.min_depth = self
            .rows
            .iter()
            .map(|row| row.depth)
            .min()
            .unwrap_or(u32::MAX);
    }
}

/// Every visible row's descendant count and depth, chunked, with the row
/// each chunk starts at to find a row's chunk.
#[derive(Debug, Clone, Default)]
struct TreeMeta {
    chunks: Vec<MetaChunk>,
    /// The row each chunk starts at, then the row count: rebuilt by every
    /// edit, which already walks the chunks.
    starts: Vec<usize>,
}

impl PartialEq for TreeMeta {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .chunks
                .iter()
                .flat_map(|chunk| chunk.rows.iter())
                .eq(other.chunks.iter().flat_map(|chunk| chunk.rows.iter()))
    }
}

impl TreeMeta {
    fn new(rows: Vec<RowMeta>) -> Self {
        let mut meta = Self {
            chunks: rows
                .chunks(META_CHUNK_ROWS)
                .map(|rows| MetaChunk::new(rows.to_vec()))
                .collect(),
            ..Self::default()
        };
        meta.rebuild();
        meta
    }

    fn rebuild(&mut self) {
        self.chunks.retain(|chunk| !chunk.rows.is_empty());
        self.starts = std::iter::once(0)
            .chain(self.chunks.iter().scan(0, |end, chunk| {
                *end += chunk.rows.len();
                Some(*end)
            }))
            .collect();
    }

    fn len(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    /// The chunk holding row `index < len`, and the row within it.
    fn locate(&self, index: usize) -> (usize, usize) {
        let chunk = self.starts.partition_point(|start| *start <= index) - 1;
        (chunk, index - self.starts[chunk])
    }

    fn get(&self, index: usize) -> Option<RowMeta> {
        (index < self.len()).then(|| {
            let (chunk, row) = self.locate(index);
            self.chunks[chunk].rows[row]
        })
    }

    fn get_mut(&mut self, index: usize) -> &mut RowMeta {
        let (chunk, row) = self.locate(index);
        &mut self.chunks[chunk].rows[row]
    }

    /// Insert `rows` before row `at` (or at the end).
    fn insert(&mut self, at: usize, rows: Vec<RowMeta>) {
        if self.chunks.is_empty() {
            *self = Self::new(rows);
            return;
        }
        let (chunk, row) = if at >= self.len() {
            let last = self.chunks.len() - 1;
            (last, self.chunks[last].rows.len())
        } else {
            self.locate(at)
        };
        let target = &mut self.chunks[chunk];
        target.rows.splice(row..row, rows);
        if target.rows.len() > 2 * META_CHUNK_ROWS {
            let pieces: Vec<MetaChunk> = target
                .rows
                .chunks(META_CHUNK_ROWS)
                .map(|rows| MetaChunk::new(rows.to_vec()))
                .collect();
            self.chunks.splice(chunk..chunk + 1, pieces);
        } else {
            target.refresh();
        }
        self.rebuild();
    }

    /// Remove `count` rows from row `start` on.
    fn remove(&mut self, start: usize, count: usize) {
        let (mut chunk, mut row) = self.locate(start);
        let mut left = count;
        while left > 0 && chunk < self.chunks.len() {
            let target = &mut self.chunks[chunk];
            let take = left.min(target.rows.len() - row);
            target.rows.drain(row..row + take);
            target.refresh();
            left -= take;
            chunk += 1;
            row = 0;
        }
        self.rebuild();
    }

    /// The nearest row before `index` shallower than `depth`: the parent of a
    /// row at `depth` that follows it. Chunks with no row that shallow are
    /// skipped whole.
    fn shallower_before(&self, index: usize, depth: u32) -> Option<usize> {
        if index == 0 || depth == 0 {
            return None;
        }
        let (mut chunk, mut row) = self.locate(index - 1);
        loop {
            let current = &self.chunks[chunk];
            if current.min_depth < depth {
                for candidate in (0..=row).rev() {
                    if current.rows[candidate].depth < depth {
                        return Some(self.starts[chunk] + candidate);
                    }
                }
            }
            if chunk == 0 {
                return None;
            }
            chunk -= 1;
            row = self.chunks[chunk].rows.len() - 1;
        }
    }

    /// Add `delta` to the descendant count of every row `index` is under.
    fn adjust_ancestors(&mut self, index: usize, delta: isize) {
        let mut depth = self.get(index).map_or(0, |row| row.depth);
        let mut at = index;
        while let Some(ancestor) = self.shallower_before(at, depth) {
            let meta = self.get_mut(ancestor);
            meta.descendants = meta.descendants.wrapping_add_signed(delta);
            depth = meta.depth;
            at = ancestor;
        }
    }
}

/// Depths for a flattened walk whose first row sits at `base`: each row is
/// one deeper than the nearest row before it whose descendants reach it.
fn walk_depths(rows: &[VirtualTreeRow], base: u32) -> Vec<RowMeta> {
    let mut open: Vec<usize> = Vec::new();
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            while open.last().is_some_and(|end| *end <= index) {
                open.pop();
            }
            let depth = base + open.len() as u32;
            if row.descendant_count > 0 {
                open.push(index + 1 + row.descendant_count);
            }
            RowMeta {
                descendants: row.descendant_count,
                depth,
            }
        })
        .collect()
}

/// Visible-row index for a disclosure tree. Logical nodes that are collapsed
/// (and their descendants) are not stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualTreeLayout {
    rows: VirtualListLayout,
    meta: TreeMeta,
}

pub type VirtualTreeWindow = VirtualListWindow;

impl VirtualTreeLayout {
    /// `rows` are the currently visible (expanded-walk) extents together with
    /// how many visible descendants follow each row.
    pub fn new(rows: impl IntoIterator<Item = VirtualTreeRow>) -> Self {
        let rows = rows.into_iter().collect::<Vec<_>>();
        Self {
            rows: VirtualListLayout::new(rows.iter().map(|row| row.extent)),
            meta: TreeMeta::new(walk_depths(&rows, 0)),
        }
    }

    /// Uniform-height flattened walk. `descendant_counts[i]` must describe the
    /// currently visible subtree following row `i`.
    pub fn uniform(row_extent: f32, descendant_counts: impl IntoIterator<Item = usize>) -> Self {
        let rows: Vec<VirtualTreeRow> = descendant_counts
            .into_iter()
            .map(|descendant_count| VirtualTreeRow {
                extent: row_extent,
                descendant_count,
            })
            .collect();
        Self {
            rows: VirtualListLayout::uniform(rows.len(), row_extent),
            meta: TreeMeta::new(walk_depths(&rows, 0)),
        }
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
        self.meta.get(index).map(|row| row.descendants)
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
        let Some(parent_meta) = self.meta.get(parent) else {
            return false;
        };
        if parent_meta.descendants != 0 {
            return false;
        }
        let descendants = descendants.into_iter().collect::<Vec<_>>();
        if descendants.is_empty() {
            return false;
        }
        let inserted = descendants.len();
        self.rows
            .insert_items(parent + 1, descendants.iter().map(|row| row.extent));
        self.meta
            .insert(parent + 1, walk_depths(&descendants, parent_meta.depth + 1));
        self.meta.get_mut(parent).descendants = inserted;
        self.meta.adjust_ancestors(parent, inserted as isize);
        true
    }

    /// Remove the visible descendants currently following `parent`.
    pub fn collapse(&mut self, parent: usize) -> bool {
        let Some(parent_meta) = self.meta.get(parent) else {
            return false;
        };
        let removed = parent_meta.descendants;
        if removed == 0 {
            return false;
        }
        self.meta.adjust_ancestors(parent, -(removed as isize));
        self.rows.remove_items(parent + 1..parent + 1 + removed);
        self.meta.remove(parent + 1, removed);
        self.meta.get_mut(parent).descendants = 0;
        true
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

    /// Descendant counts the way the index kept them before it was chunked:
    /// every row before the parent checked by hand. The parent's visible
    /// descendants become `children` leaves.
    fn naive_set_children(counts: &mut Vec<usize>, parent: usize, children: usize) {
        let removed = counts[parent];
        for (ancestor, count) in counts.iter_mut().enumerate().take(parent) {
            if parent < ancestor + 1 + *count {
                *count = *count + children - removed;
            }
        }
        counts.splice(parent + 1..parent + 1 + removed, vec![0; children]);
        counts[parent] = children;
    }

    /// Expanding and collapsing in any order keeps the counts the plain walk
    /// over every earlier row would.
    #[test]
    fn chunked_expand_and_collapse_agree_with_the_plain_walk() {
        let mut layout = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, 2_000));
        let mut counts = vec![0usize; 2_000];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound.max(1) as u64) as usize
        };
        for step in 0..400 {
            let parent = next(counts.len());
            if counts[parent] == 0 {
                let children = 1 + next(900);
                assert!(layout.expand(parent, std::iter::repeat_n(leaf(), children)));
                naive_set_children(&mut counts, parent, children);
            } else {
                assert!(layout.collapse(parent));
                naive_set_children(&mut counts, parent, 0);
            }
            assert_eq!(layout.visible_len(), counts.len(), "step {step}");
            for (index, count) in counts.iter().enumerate().step_by(37) {
                assert_eq!(
                    layout.descendant_count(index),
                    Some(*count),
                    "step {step} row {index}"
                );
            }
        }
    }

    /// Expanding a row deep in a million-row walk writes its own chunks and
    /// the rows above it, not every row before it.
    #[test]
    fn expanding_deep_in_a_million_rows_touches_one_chunk() {
        // A forest of top rows; the last top row holds a branch two deep.
        let mut counts = vec![0usize; 1_000_000];
        counts[999_000] = 999;
        counts[999_100] = 10;
        let mut layout = VirtualTreeLayout::uniform(ROW, counts);
        let before = layout.row_layout().index_entries_touched();
        let parent = 999_105;
        assert!(layout.expand(parent, std::iter::repeat_n(leaf(), 10)));
        assert!(layout.row_layout().index_entries_touched() - before <= 64);
        assert_eq!(layout.descendant_count(parent), Some(10));
        assert_eq!(layout.descendant_count(999_100), Some(20));
        assert_eq!(layout.descendant_count(999_000), Some(1_009));
        assert_eq!(layout.descendant_count(998_999), Some(0));
        assert!(layout.collapse(parent));
        assert_eq!(layout.descendant_count(999_100), Some(10));
        assert_eq!(layout.descendant_count(999_000), Some(999));
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
    fn expanded_forest_window_at_100k_is_bounded_by_geometry() {
        const LEN: usize = 100_000;
        let layout = VirtualTreeLayout::uniform(
            ROW,
            (0..LEN).map(|index| {
                let remaining = LEN - index;
                if index % 3 == 0 && remaining >= 3 {
                    2
                } else {
                    0
                }
            }),
        );
        assert_eq!(layout.visible_len(), LEN);
        assert_eq!(layout.descendant_count(0), Some(2));
        assert_eq!(layout.descendant_count(1), Some(0));
        let cap = geometric_cap();
        assert!(cap < LEN);
        let window = layout.window(0.0, VIEWPORT, OVERSCAN);
        assert!(window.range.len() <= cap);
        assert!(window.range.len() < layout.visible_len());
        let mut collapsed = layout;
        assert!(collapsed.collapse(0));
        assert_eq!(collapsed.visible_len(), LEN - 2);
        assert_eq!(collapsed.descendant_count(0), Some(0));
        assert!(collapsed.expand(0, [leaf(), leaf()]));
        assert_eq!(collapsed.visible_len(), LEN);
        assert_eq!(collapsed.descendant_count(0), Some(2));
        let restored = collapsed.window(4_000.0, VIEWPORT, OVERSCAN);
        assert!(restored.range.start > 0);
        assert!(restored.range.len() <= cap);
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
