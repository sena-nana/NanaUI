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
    /// Set when the constraints asked for a vertical writing mode (#59).
    ///
    /// The geometry in this layout is then horizontal-tb: the engine does not
    /// own glyph orientation or vertical font metrics, so it says so rather
    /// than reporting horizontal metrics as if they were vertical ones. A
    /// consumer that cannot accept horizontal fallback checks this flag.
    #[serde(default)]
    pub unsupported_writing_mode: bool,
}

impl TextLayout {
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
