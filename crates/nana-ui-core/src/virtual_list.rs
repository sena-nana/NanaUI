//! Backend-neutral geometry for lists that materialize only a visible window.

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Range;

/// Shared viewport contract for both table axes, lists and disclosure trees.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VirtualViewport {
    pub offset: [f32; 2],
    pub extent: [f32; 2],
    pub overscan: [f32; 2],
}

impl VirtualViewport {
    pub fn vertical(offset: f32, extent: f32, overscan: f32) -> Self {
        Self {
            offset: [0.0, offset],
            extent: [0.0, extent],
            overscan: [0.0, overscan],
        }
    }
}

/// Pixel offset within an item. The application retains the corresponding
/// business key and resolves its current index after a data reorder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualScrollAnchor {
    pub index: usize,
    pub inset: f32,
}

/// Alignment when locating an item before materializing and focusing it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VirtualAlignment {
    #[default]
    Nearest,
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListWindow {
    pub range: Range<usize>,
    pub leading_extent: f32,
    pub trailing_extent: f32,
    pub total_extent: f32,
}

/// A scrollable body plus the visible part of a frozen prefix. The ranges
/// never overlap, including when the frozen prefix fills the viewport.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualFrozenWindow {
    pub body: VirtualListWindow,
    pub frozen: Range<usize>,
    pub frozen_extent: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualListMount<K> {
    pub index: usize,
    pub key: K,
}

/// Two-phase visible-item reconciliation. Consumers apply the mount/unmount
/// plan to their retained tree before publishing it with `commit`.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListMaterialization<K> {
    pub base_revision: u64,
    pub window: VirtualListWindow,
    pub mounts: Vec<VirtualListMount<K>>,
    pub unmounts: Vec<K>,
    pub order: Vec<K>,
    /// Logical index for every key in `order`, including sparse retained items.
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualListMaterializationError {
    DuplicateKey,
    StalePlan,
}

/// Retains visible and explicitly protected item identity; application data
/// and item views remain owned by the consumer.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListMaterializer<K> {
    revision: u64,
    mounted: Vec<K>,
}

impl<K> Default for VirtualListMaterializer<K> {
    fn default() -> Self {
        Self {
            revision: 0,
            mounted: Vec::new(),
        }
    }
}

impl<K> VirtualListMaterializer<K>
where
    K: Clone + Eq + Hash,
{
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mounted(&self) -> &[K] {
        &self.mounted
    }

    pub fn prepare(
        &self,
        layout: &VirtualListLayout,
        scroll_offset: f32,
        viewport_extent: f32,
        overscan_extent: f32,
        mut key_at: impl FnMut(usize) -> K,
    ) -> Result<VirtualListMaterialization<K>, VirtualListMaterializationError> {
        let window = layout.window(scroll_offset, viewport_extent, overscan_extent);
        self.prepare_window(window, &mut key_at)
    }

    pub fn prepare_window(
        &self,
        window: VirtualListWindow,
        mut key_at: impl FnMut(usize) -> K,
    ) -> Result<VirtualListMaterialization<K>, VirtualListMaterializationError> {
        let indices = window.range.clone();
        self.prepare_indices(window, indices, &mut key_at)
    }

    /// Reconcile the visible window plus active item indices. `window` in the
    /// returned plan still describes the viewport; `order` includes retained
    /// items in data order. Use `layout.retained_ranges` for sparse placement.
    pub fn prepare_retained(
        &self,
        layout: &VirtualListLayout,
        viewport: VirtualViewport,
        retained_indices: impl IntoIterator<Item = usize>,
        key_at: impl FnMut(usize) -> K,
    ) -> Result<VirtualListMaterialization<K>, VirtualListMaterializationError> {
        self.prepare_retained_window(
            layout,
            layout.window_for(viewport),
            retained_indices,
            key_at,
        )
    }

    /// Reconcile a caller-selected body window plus frozen/active indices.
    pub fn prepare_retained_window(
        &self,
        layout: &VirtualListLayout,
        window: VirtualListWindow,
        retained_indices: impl IntoIterator<Item = usize>,
        key_at: impl FnMut(usize) -> K,
    ) -> Result<VirtualListMaterialization<K>, VirtualListMaterializationError> {
        let indices = layout
            .retained_ranges(&window, retained_indices)
            .into_iter()
            .flatten();
        self.prepare_indices(window, indices, key_at)
    }

    fn prepare_indices(
        &self,
        window: VirtualListWindow,
        indices: impl Iterator<Item = usize> + Clone,
        mut key_at: impl FnMut(usize) -> K,
    ) -> Result<VirtualListMaterialization<K>, VirtualListMaterializationError> {
        let order = indices.clone().map(&mut key_at).collect::<Vec<_>>();
        let desired = order.iter().cloned().collect::<HashSet<_>>();
        if desired.len() != order.len() {
            return Err(VirtualListMaterializationError::DuplicateKey);
        }
        let current = self.mounted.iter().cloned().collect::<HashSet<_>>();
        let mounted_indices = indices.clone().collect();
        let mounts = indices
            .zip(order.iter().cloned())
            .filter(|(_, key)| !current.contains(key))
            .map(|(index, key)| VirtualListMount { index, key })
            .collect();
        let unmounts = self
            .mounted
            .iter()
            .filter(|key| !desired.contains(*key))
            .cloned()
            .collect();
        Ok(VirtualListMaterialization {
            base_revision: self.revision,
            window,
            mounts,
            unmounts,
            order,
            indices: mounted_indices,
        })
    }

    pub fn commit(
        &mut self,
        plan: VirtualListMaterialization<K>,
    ) -> Result<bool, VirtualListMaterializationError> {
        if plan.base_revision != self.revision {
            return Err(VirtualListMaterializationError::StalePlan);
        }
        if self.mounted == plan.order {
            return Ok(false);
        }
        self.mounted = plan.order;
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }
}

/// Retained variable-height item geometry with logarithmic range queries and
/// single-item measurement updates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualListLayout {
    item_extents: Vec<f32>,
    fenwick: Vec<f32>,
}

impl VirtualListLayout {
    pub fn window_with_frozen(
        &self,
        offset: f32,
        extent: f32,
        overscan: f32,
        count: usize,
    ) -> VirtualFrozenWindow {
        let count = count.min(self.len());
        let frozen_extent = self.prefix_extent(count);
        let available = (sanitize_extent(extent) - frozen_extent).max(0.0);
        let mut body = if available > 0.0 {
            self.window(sanitize_extent(offset) + frozen_extent, available, overscan)
        } else {
            VirtualListWindow {
                range: count..count,
                leading_extent: frozen_extent,
                trailing_extent: self.total_extent() - frozen_extent,
                total_extent: self.total_extent(),
            }
        };
        body.range.start = body.range.start.max(count);
        body.range.end = body.range.end.max(body.range.start);
        body.leading_extent = self.prefix_extent(body.range.start);
        body.trailing_extent = body.total_extent - self.prefix_extent(body.range.end);
        let frozen = if count > 0 && extent > 0.0 {
            let visible = self.window(0.0, extent.min(frozen_extent), 0.0).range;
            visible.start..visible.end.min(count)
        } else {
            0..0
        };
        VirtualFrozenWindow {
            body,
            frozen,
            frozen_extent,
        }
    }

    pub fn offset_for_frozen_index(
        &self,
        index: usize,
        offset: f32,
        extent: f32,
        count: usize,
        alignment: VirtualAlignment,
    ) -> Option<f32> {
        if index >= self.len() {
            return None;
        }
        let count = count.min(self.len());
        if index < count {
            if self.prefix_extent(index) >= sanitize_extent(extent) {
                return None;
            }
            return Some(
                sanitize_extent(offset)
                    .min((self.total_extent() - sanitize_extent(extent)).max(0.0)),
            );
        }
        let frozen = self.prefix_extent(count);
        if frozen >= sanitize_extent(extent) {
            return None;
        }
        self.offset_for_index(
            index,
            sanitize_extent(offset) + frozen,
            extent - frozen,
            alignment,
        )
        .map(|offset| (offset - frozen).max(0.0))
    }

    /// Ordered, disjoint mounted ranges. A distant active editor contributes
    /// one item, never the entire gap between it and the viewport. Callers
    /// resolve retained business keys to current indices before this query.
    pub fn retained_ranges(
        &self,
        window: &VirtualListWindow,
        retained_indices: impl IntoIterator<Item = usize>,
    ) -> Vec<Range<usize>> {
        let mut ranges = retained_indices
            .into_iter()
            .filter(|index| *index < self.len())
            .map(|index| index..index + 1)
            .collect::<Vec<_>>();
        let start = window.range.start.min(self.len());
        let end = window.range.end.min(self.len());
        if start < end {
            ranges.push(start..end);
        }
        ranges.sort_unstable_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = merged.last_mut()
                && range.start <= previous.end
            {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }

    /// Resolve a data index in O(log N), without materializing intervening rows.
    /// Oversized items use the closest edge; an item covering the viewport
    /// does not move under `Nearest`. Invalid indices leave the viewport alone.
    pub fn offset_for_index(
        &self,
        index: usize,
        offset: f32,
        viewport_extent: f32,
        alignment: VirtualAlignment,
    ) -> Option<f32> {
        let extent = *self.item_extents.get(index)?;
        let viewport = sanitize_extent(viewport_extent);
        let max_offset = (self.total_extent() - viewport).max(0.0);
        let offset = sanitize_extent(offset).min(max_offset);
        let start = self.prefix_extent(index);
        let end = start + extent;
        let next = match alignment {
            VirtualAlignment::Start => start,
            VirtualAlignment::Center => start + (extent - viewport) / 2.0,
            VirtualAlignment::End => end - viewport,
            VirtualAlignment::Nearest => {
                if (start >= offset && end <= offset + viewport)
                    || (start <= offset && end >= offset + viewport)
                {
                    offset
                } else if (start - offset).abs() <= (end - viewport - offset).abs() {
                    start
                } else {
                    end - viewport
                }
            }
        };
        Some(next.clamp(0.0, max_offset))
    }

    pub fn reveal_index(
        &self,
        index: usize,
        viewport: &mut VirtualViewport,
        alignment: VirtualAlignment,
    ) -> bool {
        let Some(offset) =
            self.offset_for_index(index, viewport.offset[1], viewport.extent[1], alignment)
        else {
            return false;
        };
        viewport.offset[1] = offset;
        true
    }

    pub fn window_for(&self, viewport: VirtualViewport) -> VirtualListWindow {
        self.window(viewport.offset[1], viewport.extent[1], viewport.overscan[1])
    }

    pub fn scroll_anchor(&self, offset: f32) -> Option<VirtualScrollAnchor> {
        if self.is_empty() {
            return None;
        }
        let offset = sanitize_extent(offset).min(self.total_extent());
        let index = self.item_at_offset(offset);
        Some(VirtualScrollAnchor {
            index,
            inset: offset - self.prefix_extent(index),
        })
    }

    pub fn restore_scroll_anchor(&self, anchor: VirtualScrollAnchor, viewport_extent: f32) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let index = anchor.index.min(self.len() - 1);
        (self.prefix_extent(index) + sanitize_extent(anchor.inset).min(self.item_extents[index]))
            .min((self.total_extent() - sanitize_extent(viewport_extent)).max(0.0))
    }

    /// Measure one item and preserve the visible anchor in O(log N).
    pub fn measure_anchored(
        &mut self,
        index: usize,
        extent: f32,
        viewport: &mut VirtualViewport,
    ) -> bool {
        let anchor = self.scroll_anchor(viewport.offset[1]);
        let changed = self.update_item_extent(index, extent);
        if changed && let Some(anchor) = anchor {
            viewport.offset[1] = self.restore_scroll_anchor(anchor, viewport.extent[1]);
        }
        changed
    }
    pub fn new(item_extents: impl IntoIterator<Item = f32>) -> Self {
        let mut layout = Self::default();
        layout.set_item_extents(item_extents);
        layout
    }

    pub fn set_item_extents(&mut self, item_extents: impl IntoIterator<Item = f32>) {
        self.item_extents = item_extents.into_iter().map(sanitize_extent).collect();
        self.rebuild_fenwick();
    }

    /// Insert measured rows at `at` without requiring callers to rebuild the
    /// rest of the data set. Used when a disclosure tree expands.
    pub fn insert_items(&mut self, at: usize, extents: impl IntoIterator<Item = f32>) {
        let at = at.min(self.len());
        let inserted = extents.into_iter().map(sanitize_extent).collect::<Vec<_>>();
        if inserted.is_empty() {
            return;
        }
        self.item_extents.splice(at..at, inserted);
        self.rebuild_fenwick();
    }

    /// Remove a contiguous measured range. Used when a disclosure tree collapses.
    pub fn remove_items(&mut self, range: Range<usize>) {
        let start = range.start.min(self.len());
        let end = range.end.max(start).min(self.len());
        if start == end {
            return;
        }
        self.item_extents.drain(start..end);
        self.rebuild_fenwick();
    }

    /// Update one measured row without rebuilding all following prefix sums.
    pub fn update_item_extent(&mut self, index: usize, extent: f32) -> bool {
        let Some(previous) = self.item_extents.get_mut(index) else {
            return false;
        };
        let extent = sanitize_extent(extent);
        if *previous == extent {
            return false;
        }
        let delta = extent - *previous;
        *previous = extent;
        let mut tree_index = index + 1;
        while tree_index < self.fenwick.len() {
            self.fenwick[tree_index] += delta;
            tree_index += low_bit(tree_index);
        }
        true
    }

    pub fn len(&self) -> usize {
        self.item_extents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.item_extents.is_empty()
    }

    pub fn total_extent(&self) -> f32 {
        self.prefix_extent(self.len())
    }

    pub fn extent(&self, range: Range<usize>) -> f32 {
        let start = range.start.min(self.len());
        let end = range.end.max(start).min(self.len());
        self.prefix_extent(end) - self.prefix_extent(start)
    }

    /// Geometric upper bound on a uniform-row window, including two-sided
    /// overscan and one partial item on each edge. Independent of `self.len()`,
    /// so a cap cannot be the tautology `range.len()`.
    pub fn uniform_window_item_cap(
        viewport_extent: f32,
        overscan_extent: f32,
        item_extent: f32,
    ) -> usize {
        let item_extent = sanitize_extent(item_extent);
        if item_extent == 0.0 {
            return 0;
        }
        ((sanitize_extent(viewport_extent) + 2.0 * sanitize_extent(overscan_extent)) / item_extent)
            .ceil() as usize
            + 2
    }

    pub fn window(
        &self,
        scroll_offset: f32,
        viewport_extent: f32,
        overscan_extent: f32,
    ) -> VirtualListWindow {
        let total_extent = self.total_extent();
        if self.is_empty() {
            return VirtualListWindow {
                range: 0..0,
                leading_extent: 0.0,
                trailing_extent: 0.0,
                total_extent,
            };
        }

        let scroll_offset = sanitize_extent(scroll_offset).min(total_extent);
        let viewport_extent = sanitize_extent(viewport_extent);
        let overscan_extent = sanitize_extent(overscan_extent);
        let start_offset = (scroll_offset - overscan_extent).max(0.0);
        let end_offset = (scroll_offset + viewport_extent + overscan_extent).min(total_extent);
        let start = self.item_at_offset(start_offset);
        let end = self
            .item_after_offset(end_offset)
            .max(start + 1)
            .min(self.len());
        let leading_extent = self.prefix_extent(start);
        let trailing_extent = total_extent - self.prefix_extent(end);

        VirtualListWindow {
            range: start..end,
            leading_extent,
            trailing_extent,
            total_extent,
        }
    }

    fn rebuild_fenwick(&mut self) {
        self.fenwick.clear();
        self.fenwick.resize(self.item_extents.len() + 1, 0.0);
        for (index, extent) in self.item_extents.iter().copied().enumerate() {
            let tree_index = index + 1;
            self.fenwick[tree_index] += extent;
            let parent = tree_index + low_bit(tree_index);
            if parent < self.fenwick.len() {
                self.fenwick[parent] += self.fenwick[tree_index];
            }
        }
    }

    fn prefix_extent(&self, end: usize) -> f32 {
        let mut tree_index = end.min(self.len());
        let mut sum = 0.0;
        while tree_index > 0 {
            sum += self.fenwick[tree_index];
            tree_index -= low_bit(tree_index);
        }
        sum
    }

    fn prefix_partition_point(&self, offset: f32, inclusive: bool) -> usize {
        let before = |prefix: f32| {
            if inclusive {
                prefix <= offset
            } else {
                prefix < offset
            }
        };
        if !before(0.0) {
            return 0;
        }
        let mut index = 0;
        let mut prefix = 0.0;
        let mut step = 1;
        while step <= self.len() / 2 {
            step <<= 1;
        }
        while step > 0 {
            let next = index + step;
            if next <= self.len() {
                let candidate = prefix + self.fenwick[next];
                if before(candidate) {
                    index = next;
                    prefix = candidate;
                }
            }
            step >>= 1;
        }
        // Include prefix[0], which is known to satisfy the predicate here.
        index + 1
    }

    fn item_at_offset(&self, offset: f32) -> usize {
        self.prefix_partition_point(offset, true)
            .saturating_sub(1)
            .min(self.len().saturating_sub(1))
    }

    fn item_after_offset(&self, offset: f32) -> usize {
        self.prefix_partition_point(offset, false).min(self.len())
    }
}

fn low_bit(value: usize) -> usize {
    value & value.wrapping_neg()
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
    fn frozen_prefix_reserves_space_without_materializing_all_frozen_data() {
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 1_000_000));
        assert_eq!(
            layout.offset_for_frozen_index(500_000, 0.0, 100.0, 1, VirtualAlignment::Start),
            Some(9_999_980.0)
        );
        let pane = layout.window_with_frozen(9_999_980.0, 100.0, 0.0, 1);
        assert_eq!(pane.frozen, 0..1);
        assert_eq!(pane.body.range, 500_000..500_004);
        let covered = layout.window_with_frozen(0.0, 100.0, 0.0, 1_000_000);
        assert_eq!(covered.frozen.len(), 5);
        assert_eq!(
            layout.offset_for_frozen_index(
                999_999,
                0.0,
                100.0,
                1_000_000,
                VirtualAlignment::Nearest
            ),
            None
        );
        assert!(covered.body.range.is_empty());
        assert_eq!(
            layout.offset_for_frozen_index(10, 0.0, 100.0, 8, VirtualAlignment::Start),
            None
        );
        assert_eq!(
            layout
                .window_with_frozen(0.0, 100.0, 200.0, 2)
                .body
                .range
                .start,
            2
        );
    }

    #[test]
    fn navigation_uses_closest_edge_and_clamps_without_scanning_data() {
        let layout = VirtualListLayout::new([20.0, 200.0, 30.0, 40.0]);
        assert_eq!(
            layout.offset_for_index(1, 50.0, 60.0, VirtualAlignment::Nearest),
            Some(50.0)
        );
        assert_eq!(
            layout.offset_for_index(2, 0.0, 60.0, VirtualAlignment::Nearest),
            Some(190.0)
        );
        assert_eq!(
            layout.offset_for_index(0, 190.0, 60.0, VirtualAlignment::Nearest),
            Some(0.0)
        );
        assert_eq!(
            layout.offset_for_index(3, 0.0, 60.0, VirtualAlignment::Start),
            Some(230.0)
        );
        assert_eq!(
            layout.offset_for_index(1, 0.0, 60.0, VirtualAlignment::Center),
            Some(90.0)
        );
        assert_eq!(
            layout.offset_for_index(4, 10.0, 60.0, VirtualAlignment::End),
            None
        );
    }

    #[test]
    fn retained_editor_stays_keyed_without_materializing_the_gap() {
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 1_000_000));
        let mut materializer = VirtualListMaterializer::default();
        let first = materializer
            .prepare(&layout, 0.0, 100.0, 0.0, |i| i)
            .unwrap();
        materializer.commit(first).unwrap();
        let mut reads = 0;
        let plan = materializer
            .prepare_retained(
                &layout,
                VirtualViewport::vertical(10_000_000.0, 100.0, 0.0),
                [2, 2, usize::MAX],
                |i| {
                    reads += 1;
                    i
                },
            )
            .unwrap();
        assert_eq!(reads, 6);
        assert_eq!(plan.order, [2, 500_000, 500_001, 500_002, 500_003, 500_004]);
        assert!(!plan.unmounts.contains(&2));
        assert!(!plan.mounts.iter().any(|mount| mount.key == 2));
        materializer.commit(plan).unwrap();
        let released = materializer
            .prepare(&layout, 10_000_000.0, 100.0, 0.0, |i| i)
            .unwrap();
        assert_eq!(released.unmounts, [2]);
    }

    #[test]
    fn million_items_keep_a_bounded_window_and_measurement_anchor() {
        let mut layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 1_000_000));
        let mut viewport = VirtualViewport::vertical(10_000_003.0, 400.0, 80.0);
        let anchor = layout.scroll_anchor(viewport.offset[1]).unwrap();
        assert_eq!(anchor.index, 500_000);
        assert!(layout.window_for(viewport).range.len() <= 30);
        assert!(layout.measure_anchored(1, 40.0, &mut viewport));
        assert_eq!(viewport.offset[1], 10_000_023.0);
        assert_eq!(layout.scroll_anchor(viewport.offset[1]), Some(anchor));
    }

    #[test]
    fn returns_visible_range_with_overscan_and_spacers() {
        let layout = VirtualListLayout::new([10.0, 20.0, 30.0, 40.0, 50.0]);
        let window = layout.window(35.0, 35.0, 10.0);
        assert_eq!(window.range, 1..4);
        assert_eq!(window.leading_extent, 10.0);
        assert_eq!(window.trailing_extent, 50.0);
        assert_eq!(window.total_extent, 150.0);
    }

    #[test]
    fn clamps_invalid_geometry_and_keeps_one_item_visible() {
        let layout = VirtualListLayout::new([f32::NAN, -5.0, 24.0]);
        assert_eq!(layout.total_extent(), 24.0);
        assert_eq!(layout.extent(0..2), 0.0);
        assert_eq!(layout.window(f32::INFINITY, 0.0, 0.0).range, 2..3);
    }

    #[test]
    fn updates_one_measurement_without_changing_unrelated_ranges() {
        let mut layout = VirtualListLayout::new([18.0, 22.0, 30.0, 40.0]);
        assert_eq!(layout.extent(0..2), 40.0);
        assert!(layout.update_item_extent(2, 50.0));
        assert_eq!(layout.extent(0..2), 40.0);
        assert_eq!(layout.extent(2..4), 90.0);
        assert_eq!(layout.total_extent(), 130.0);
        assert!(!layout.update_item_extent(2, 50.0));
        assert!(!layout.update_item_extent(99, 10.0));
    }

    #[test]
    fn materialization_reuses_overlap_and_rejects_stale_or_duplicate_plans() {
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
        let mut materializer = VirtualListMaterializer::default();
        let first = materializer
            .prepare(&layout, 0.0, 100.0, 20.0, |index| index)
            .unwrap();
        assert!(first.unmounts.is_empty());
        assert_eq!(first.mounts.len(), first.order.len());
        let stale = first.clone();
        assert!(materializer.commit(first).unwrap());

        let next = materializer
            .prepare(&layout, 80.0, 100.0, 20.0, |index| index)
            .unwrap();
        assert!(!next.mounts.is_empty());
        assert!(!next.unmounts.is_empty());
        assert!(next.mounts.len() < next.order.len());
        assert_eq!(
            materializer.commit(stale),
            Err(VirtualListMaterializationError::StalePlan)
        );
        assert!(materializer.commit(next).unwrap());

        let revision = materializer.revision();
        assert_eq!(
            materializer.prepare(&layout, 0.0, 100.0, 0.0, |_| 1),
            Err(VirtualListMaterializationError::DuplicateKey)
        );
        assert_eq!(materializer.revision(), revision);
    }

    #[test]
    fn insert_and_remove_items_keep_unrelated_prefix_extents() {
        let mut layout = VirtualListLayout::new([10.0, 20.0, 30.0]);
        layout.insert_items(1, [40.0, 50.0]);
        assert_eq!(layout.len(), 5);
        assert_eq!(layout.extent(0..1), 10.0);
        assert_eq!(layout.extent(1..3), 90.0);
        assert_eq!(layout.extent(3..5), 50.0);
        layout.remove_items(1..3);
        assert_eq!(layout.len(), 3);
        assert_eq!(layout.total_extent(), 60.0);
        assert_eq!(layout.extent(0..2), 30.0);
    }

    #[test]
    fn uniform_window_item_cap_is_geometric_not_data_len() {
        let cap = VirtualListLayout::uniform_window_item_cap(100.0, 20.0, 20.0);
        assert!(cap < 10_000);
        assert_eq!(cap, 9);
        let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
        let window = layout.window(0.0, 100.0, 20.0);
        assert!(window.range.len() <= cap);
        assert!(window.range.len() < layout.len());
    }
}
