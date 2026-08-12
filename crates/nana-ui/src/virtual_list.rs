//! Geometry model for lists that only build the visible item window.

use std::ops::Range;

/// The item range and spacer extents required to represent one virtual list frame.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListWindow {
    pub range: Range<usize>,
    pub leading_extent: f32,
    pub trailing_extent: f32,
    pub total_extent: f32,
}

/// Retained item geometry for a variable-height virtual list.
///
/// Callers provide measured or estimated extents. NanaUI keeps the prefix geometry
/// reusable across viewport changes, while the application remains responsible for
/// building the items in [`VirtualListWindow::range`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VirtualListLayout {
    item_extents: Vec<f32>,
    prefix_extents: Vec<f32>,
}

impl VirtualListLayout {
    pub fn new(item_extents: impl IntoIterator<Item = f32>) -> Self {
        let mut layout = Self::default();
        layout.set_item_extents(item_extents);
        layout
    }

    pub fn set_item_extents(&mut self, item_extents: impl IntoIterator<Item = f32>) {
        self.item_extents = item_extents.into_iter().map(sanitize_extent).collect();
        self.prefix_extents.clear();
        self.prefix_extents.reserve(self.item_extents.len() + 1);
        self.prefix_extents.push(0.0);
        for extent in &self.item_extents {
            let next = self.prefix_extents.last().copied().unwrap_or_default() + extent;
            self.prefix_extents.push(next);
        }
    }

    pub fn len(&self) -> usize {
        self.item_extents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.item_extents.is_empty()
    }

    pub fn total_extent(&self) -> f32 {
        self.prefix_extents.last().copied().unwrap_or_default()
    }

    pub fn extent(&self, range: Range<usize>) -> f32 {
        let start = range.start.min(self.len());
        let end = range.end.max(start).min(self.len());
        self.prefix_extents[end] - self.prefix_extents[start]
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
        let leading_extent = self.prefix_extents[start];
        let trailing_extent = total_extent - self.prefix_extents[end];

        VirtualListWindow {
            range: start..end,
            leading_extent,
            trailing_extent,
            total_extent,
        }
    }

    fn item_at_offset(&self, offset: f32) -> usize {
        self.prefix_extents
            .partition_point(|extent| *extent <= offset)
            .saturating_sub(1)
            .min(self.len().saturating_sub(1))
    }

    fn item_after_offset(&self, offset: f32) -> usize {
        self.prefix_extents
            .partition_point(|extent| *extent < offset)
            .min(self.len())
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
    fn reports_exact_extent_for_prepend_anchor() {
        let layout = VirtualListLayout::new([18.0, 22.0, 30.0]);

        assert_eq!(layout.extent(0..2), 40.0);
    }
}
