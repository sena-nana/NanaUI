//! Backend-neutral geometry for lists that materialize only a visible window.

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListWindow {
    pub range: Range<usize>,
    pub leading_extent: f32,
    pub trailing_extent: f32,
    pub total_extent: f32,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualListMaterializationError {
    DuplicateKey,
    StalePlan,
}

/// Retains only visible item identity; application data and item views remain
/// owned by the consumer.
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
        let order = window.range.clone().map(&mut key_at).collect::<Vec<_>>();
        let desired = order.iter().cloned().collect::<HashSet<_>>();
        if desired.len() != order.len() {
            return Err(VirtualListMaterializationError::DuplicateKey);
        }
        let current = self.mounted.iter().cloned().collect::<HashSet<_>>();
        let mounts = window
            .range
            .clone()
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
    pub fn new(item_extents: impl IntoIterator<Item = f32>) -> Self {
        let mut layout = Self::default();
        layout.set_item_extents(item_extents);
        layout
    }

    pub fn set_item_extents(&mut self, item_extents: impl IntoIterator<Item = f32>) {
        self.item_extents = item_extents.into_iter().map(sanitize_extent).collect();
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
}
