//! The immutable result of laying text out. Retained, comparable, serializable.
//!
//! The IR only. The engine that fills it is [`super::Layouter`].

use crate::constraints::TextConstraints;
use crate::id::{FontGeneration, TextLayoutId, TextRevision};
use crate::metrics::LineMetrics;
use crate::shape::{RunDirection, ShapedRun};
use crate::style::TextKind;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// An axis-aligned rectangle in physical px.
///
/// Not `nana_ui_core::LogicalRect`: that type does not serialize, and its
/// constructor clamps negative extents to zero, which would quietly erase the
/// degenerate-bounds regressions this IR is meant to expose.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TextRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Bounding box of both rects. Does not normalize negative extents.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}

/// Why a line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineBreakCause {
    /// A `\n` in the source.
    Explicit,
    /// The line ran out of width.
    Wrap,
    /// `max_lines` truncated here.
    MaxLines,
    /// `max_height_px` truncated here: the next line did not fit the box.
    MaxHeight,
    /// The text ended.
    EndOfText,
}

/// What a layout could not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct OverflowFlags(pub u8);

impl OverflowFlags {
    pub const NONE: Self = Self(0);
    /// Lines were dropped to satisfy `max_lines` or `max_height_px`.
    pub const TRUNCATED_LINES: Self = Self(1 << 0);
    /// At least one line is wider than `max_width_px`.
    pub const CLIPPED_WIDTH: Self = Self(1 << 1);
    /// An ellipsis was substituted.
    pub const ELLIPSIZED: Self = Self(1 << 2);

    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[must_use]
    pub const fn with(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    pub fn set(&mut self, flag: Self, on: bool) {
        if on {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineBox {
    pub index: u32,
    /// Byte range of the line in the source text.
    pub source: Range<usize>,
    /// Index range into [`TextLayout::runs`], in **visual** order.
    pub runs: Range<u32>,
    pub break_cause: LineBreakCause,
    pub metrics: LineMetrics,
    pub bounds: TextRect,
    pub base_direction: RunDirection,
}

/// The immutable layout result.
///
/// Geometry is **line-relative**: `x` runs along a line and `y` across the
/// stack of lines. For horizontal text that is the page. For a vertical layout
/// ([`Self::is_vertical`]) `x` runs down a column and `y` counts from the
/// block-start column; [`Self::physical_x_of_block`] and
/// [`Self::physical_size`] are the one place that turns it into the page,
/// so line breaking, alignment, truncation and the caches never learn which
/// way the page is turned.
///
/// It carries the [`TextRevision`] and [`FontGeneration`] it was produced
/// under, so checking whether it is stale is an O(1) comparison instead of
/// re-fingerprinting the text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLayout {
    #[serde(default)]
    pub id: TextLayoutId,
    pub kind: TextKind,
    pub revision: TextRevision,
    pub font_generation: FontGeneration,
    pub constraints: TextConstraints,
    pub runs: Vec<ShapedRun>,
    pub lines: Vec<LineBox>,
    /// Union of the line bounds. The superset of today's
    /// `TextMetrics { width, height, ascent }`.
    pub bounds: TextRect,
    #[serde(default)]
    pub overflow: OverflowFlags,
    /// Set when the constraints asked for a vertical writing mode that this
    /// layout did not honour (#59): editable text, which is still laid out
    /// horizontally (see [`TextConstraints::lays_out_vertically`](crate::TextConstraints::lays_out_vertically)).
    ///
    /// The geometry is then horizontal-tb, and says so rather than passing
    /// horizontal metrics off as vertical ones. A consumer that cannot accept
    /// horizontal fallback checks this flag.
    #[serde(default)]
    pub unsupported_writing_mode: bool,
}

impl TextLayout {
    /// True when the lines of this layout are vertical columns.
    pub fn is_vertical(&self) -> bool {
        self.constraints.wants_vertical_writing() && !self.unsupported_writing_mode
    }

    /// The page x of a block-axis coordinate of a vertical layout, inside a
    /// box `box_width_px` wide.
    ///
    /// `vertical-rl` stacks its columns from the box's right edge leftwards,
    /// so it anchors to the box and needs its width; `vertical-lr` stacks from
    /// the left edge and does not. Horizontal layouts return `block` as is —
    /// there the block axis is `y`, and a caller should not be asking.
    pub fn physical_x_of_block(&self, block: f32, box_width_px: f32) -> f32 {
        if self.is_vertical() && self.constraints.writing_mode.block_start_is_right() {
            box_width_px - block
        } else {
            block
        }
    }

    /// A line-space rectangle — what [`Self::selection_rects`] and the line
    /// bounds are in — on the page of a box `box_width_px` wide. Horizontal
    /// layouts return it unchanged.
    pub fn page_rect(&self, rect: TextRect, box_width_px: f32) -> TextRect {
        if !self.is_vertical() {
            return rect;
        }
        let near = self.physical_x_of_block(rect.y, box_width_px);
        let far = self.physical_x_of_block(rect.y + rect.height, box_width_px);
        TextRect::new(near.min(far), rect.x, rect.height, rect.width)
    }

    /// A page point inside a box `box_width_px` wide, in line space: what
    /// [`Self::hit_test`] reads. The inverse of [`Self::page_rect`]; horizontal
    /// layouts return it unchanged.
    pub fn line_space_point(&self, x: f32, y: f32, box_width_px: f32) -> (f32, f32) {
        if !self.is_vertical() {
            return (x, y);
        }
        // Mirroring about the box is its own inverse, so the block→page map
        // turns a page x back into a block coordinate too.
        (y, self.physical_x_of_block(x, box_width_px))
    }

    /// Width and height the text occupies on the page: the longest line and
    /// the summed line boxes, crossed over for a vertical layout.
    ///
    /// Summed line boxes rather than [`Self::bounds`], whose extent along the
    /// line follows alignment: a centred line starts inside the box, and
    /// reporting its right edge as the text's width would size a shrink-wrapped
    /// container by its own previous width.
    pub fn physical_size(&self) -> (f32, f32) {
        let finite = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        let along = self
            .lines
            .iter()
            .map(|line| finite(line.metrics.width_px))
            .fold(0.0, f32::max);
        let across = self
            .lines
            .iter()
            .map(|line| finite(line.metrics.height_px))
            .sum();
        if self.is_vertical() {
            (across, along)
        } else {
            (along, across)
        }
    }

    pub fn glyph_count(&self) -> usize {
        self.runs.iter().map(|run| run.glyphs.len()).sum()
    }

    /// Runs of one line, in visual order.
    pub fn line_runs(&self, line: &LineBox) -> &[ShapedRun] {
        let start = line.runs.start as usize;
        let end = (line.runs.end as usize).min(self.runs.len());
        if start >= end {
            return &[];
        }
        &self.runs[start..end]
    }

    /// True when this layout was produced from a different source revision or
    /// against a different font database than the ones given.
    pub fn is_stale(&self, revision: TextRevision, font_generation: FontGeneration) -> bool {
        self.revision != revision || self.font_generation != font_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_keeps_a_negative_extent_instead_of_clamping_it_away() {
        let degenerate = TextRect::new(10.0, 0.0, -4.0, 8.0);
        assert_eq!(degenerate.right(), 6.0);
        let joined = degenerate.union(TextRect::new(0.0, 0.0, 2.0, 2.0));
        assert_eq!(joined.x, 0.0);
        assert_eq!(joined.width, 6.0);
    }

    fn vertical(mode: nana_ui_core::WritingModeSpec) -> TextLayout {
        TextLayout {
            id: TextLayoutId::default(),
            kind: TextKind::Paragraph,
            revision: TextRevision::default(),
            font_generation: FontGeneration::default(),
            constraints: TextConstraints {
                writing_mode: mode,
                ..TextConstraints::default()
            },
            runs: Vec::new(),
            lines: Vec::new(),
            bounds: TextRect::default(),
            overflow: OverflowFlags::NONE,
            unsupported_writing_mode: false,
        }
    }

    #[test]
    fn line_space_and_page_space_are_each_others_inverse() {
        let rl = vertical(nana_ui_core::WritingModeSpec::VerticalRl);
        // The second column (block 20..40) from 10 to 30 down it, in a box
        // 100 wide: `vertical-rl` puts it 40 in from the right edge.
        let page = rl.page_rect(TextRect::new(10.0, 20.0, 20.0, 20.0), 100.0);
        assert_eq!(page, TextRect::new(60.0, 10.0, 20.0, 20.0));
        assert_eq!(rl.line_space_point(70.0, 15.0, 100.0), (15.0, 30.0));

        let lr = vertical(nana_ui_core::WritingModeSpec::VerticalLr);
        let page = lr.page_rect(TextRect::new(10.0, 20.0, 20.0, 20.0), 100.0);
        assert_eq!(page, TextRect::new(20.0, 10.0, 20.0, 20.0));
        assert_eq!(lr.line_space_point(30.0, 15.0, 100.0), (15.0, 30.0));

        let horizontal = vertical(nana_ui_core::WritingModeSpec::HorizontalTb);
        let rect = TextRect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(horizontal.page_rect(rect, 100.0), rect);
        assert_eq!(horizontal.line_space_point(5.0, 6.0, 100.0), (5.0, 6.0));
    }

    #[test]
    fn overflow_flags_compose_and_read_back_independently() {
        let mut flags = OverflowFlags::NONE.with(OverflowFlags::ELLIPSIZED);
        assert!(flags.contains(OverflowFlags::ELLIPSIZED));
        assert!(!flags.contains(OverflowFlags::TRUNCATED_LINES));
        flags.set(OverflowFlags::TRUNCATED_LINES, true);
        assert!(flags.contains(OverflowFlags::ELLIPSIZED));
        assert!(flags.contains(OverflowFlags::TRUNCATED_LINES));
    }
}
