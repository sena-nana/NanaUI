//! Which way lines run and stack, and the one map from logical coordinates to
//! the page.
//!
//! `writing-mode` and `direction` change what "start", "end", "along" and
//! "across" mean; they never rotate a finished result. Layout measures and
//! places in logical coordinates, and a [`WritingContext`] turns them onto the
//! page once, at the boundary.
//!
//! CSS has two logical frames, and both live here:
//!
//! - **Flow-relative** — `inline-start` / `inline-end` / `block-start` /
//!   `block-end`. Where content *begins*: padding-inline-start, alignment to
//!   start, the first flex item. `direction` moves it: in `horizontal-tb` +
//!   RTL the inline-start is the right edge, and in a vertical mode + RTL it is
//!   the bottom one.
//! - **Line-relative** — line-left / line-right, line-over / line-under. How a
//!   line's content is *ordered*: the bidirectional algorithm lays a line out
//!   from line-left to line-right, whatever the paragraph direction, and a
//!   right-to-left paragraph only starts at line-right. Line-left is the left
//!   edge of a horizontal line and the top of a vertical one; `direction` does
//!   not move it. This is the space `nana-text` lays text out in.
//!
//! Keeping the two apart is what keeps RTL from being "reverse the children"
//! and vertical from being "rotate the box": a vertical RTL column still reads
//! CJK top to bottom (line-relative) and sits against the bottom edge
//! (flow-relative).

use crate::box_layout::{DirSpec, FlexDirection, WritingModeSpec};
use serde::{Deserialize, Serialize};

/// One side of a box on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PhysicalEdge {
    Top,
    Right,
    Bottom,
    Left,
}

impl PhysicalEdge {
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Right => Self::Left,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
        }
    }
}

/// A box's writing mode and direction: an inheritable environment, like its
/// font, that every layout algorithm reads its axes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WritingContext {
    pub mode: WritingModeSpec,
    pub direction: DirSpec,
}

impl WritingContext {
    pub const HORIZONTAL_LTR: Self = Self::new(WritingModeSpec::HorizontalTb, DirSpec::Ltr);

    pub const fn new(mode: WritingModeSpec, direction: DirSpec) -> Self {
        Self { mode, direction }
    }

    pub const fn is_vertical(self) -> bool {
        self.mode.is_vertical()
    }

    // ---- flow-relative ------------------------------------------------------

    /// The page edge content starts from along the inline axis.
    pub const fn inline_start(self) -> PhysicalEdge {
        match (self.mode.is_vertical(), self.direction.is_rtl()) {
            (false, false) => PhysicalEdge::Left,
            (false, true) => PhysicalEdge::Right,
            (true, false) => PhysicalEdge::Top,
            (true, true) => PhysicalEdge::Bottom,
        }
    }

    pub const fn inline_end(self) -> PhysicalEdge {
        self.inline_start().opposite()
    }

    /// The page edge lines and blocks stack from.
    pub const fn block_start(self) -> PhysicalEdge {
        match self.mode {
            WritingModeSpec::HorizontalTb => PhysicalEdge::Top,
            WritingModeSpec::VerticalRl => PhysicalEdge::Right,
            WritingModeSpec::VerticalLr => PhysicalEdge::Left,
        }
    }

    pub const fn block_end(self) -> PhysicalEdge {
        self.block_start().opposite()
    }

    /// The inline axis runs against its page axis: right to left, or bottom
    /// to top.
    pub const fn inline_reversed(self) -> bool {
        matches!(
            self.inline_start(),
            PhysicalEdge::Right | PhysicalEdge::Bottom
        )
    }

    /// The block axis runs against its page axis: columns stacked from the
    /// right (`vertical-rl`).
    pub const fn block_reversed(self) -> bool {
        matches!(self.block_start(), PhysicalEdge::Right)
    }

    /// True when the page axis `physical` (`Row` = horizontal) carries the
    /// inline axis: the horizontal one in `horizontal-tb`, the vertical one in
    /// a vertical mode.
    const fn carries_inline(self, physical: FlexDirection) -> bool {
        matches!(physical, FlexDirection::Row) != self.is_vertical()
    }

    /// True when content along the page axis `physical` starts at its far
    /// end — the right or the bottom — because the logical axis on it is
    /// reversed: an RTL inline axis, a vertical RTL one, or `vertical-rl`'s
    /// block axis.
    pub const fn physical_axis_reversed(self, physical: FlexDirection) -> bool {
        if self.carries_inline(physical) {
            self.inline_reversed()
        } else {
            self.block_reversed()
        }
    }

    /// The page axis a `flex-direction: row` runs along: the inline axis.
    pub const fn inline_flex_direction(self) -> FlexDirection {
        if self.is_vertical() {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        }
    }

    /// CSS `flex-direction` (row = inline, column = block) on page axes.
    pub const fn physical_flex_direction(self, css: FlexDirection) -> FlexDirection {
        match css {
            FlexDirection::Row => self.inline_flex_direction(),
            FlexDirection::Column => {
                if self.is_vertical() {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                }
            }
        }
    }

    /// A page size as `(inline, block)`: the extents along the inline and
    /// block axes.
    pub const fn logical_size(self, width: f32, height: f32) -> (f32, f32) {
        if self.is_vertical() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The inline extent of a page size. For a containing block this is the
    /// base of its children's percentage margins and paddings (CSS Box Model
    /// §5).
    pub const fn inline_size(self, width: f32, height: f32) -> f32 {
        self.logical_size(width, height).0
    }

    /// `(inline, block)` extents as a page `(width, height)`.
    pub const fn physical_size(self, inline: f32, block: f32) -> (f32, f32) {
        if self.is_vertical() {
            (block, inline)
        } else {
            (inline, block)
        }
    }

    /// A flow-relative rectangle — offsets from the inline-start and
    /// block-start edges of a box `(box_inline, box_block)` large, and sizes
    /// along those axes — on the page, relative to the box's top-left, as
    /// `(x, y, width, height)`. A reversed axis measures back from its far
    /// edge, so this is the one place `direction` and `vertical-rl` turn
    /// flow-relative layout onto the page.
    pub fn flow_rect_to_page(
        self,
        (inline, block, inline_size, block_size): (f32, f32, f32, f32),
        (box_inline, box_block): (f32, f32),
    ) -> (f32, f32, f32, f32) {
        let along = if self.inline_reversed() {
            box_inline - inline - inline_size
        } else {
            inline
        };
        let across = if self.block_reversed() {
            box_block - block - block_size
        } else {
            block
        };
        if self.is_vertical() {
            (across, along, block_size, inline_size)
        } else {
            (along, across, inline_size, block_size)
        }
    }

    // ---- line-relative ------------------------------------------------------

    /// Page x of a block-axis coordinate of a vertical layout, inside a box
    /// `box_width` wide: `vertical-rl` counts columns from the right edge, so
    /// it anchors to the box; `vertical-lr` counts from the left and does not.
    ///
    /// Mirroring about the box is its own inverse, so this also turns a page
    /// x back into a block coordinate. A horizontal context has its block
    /// axis on `y` and returns `block` unchanged.
    pub fn block_to_page_x(self, block: f32, box_width: f32) -> f32 {
        if self.is_vertical() && self.block_reversed() {
            box_width - block
        } else {
            block
        }
    }

    /// A line-space rectangle — `x` / `width` along the line from line-left,
    /// `y` / `height` across the lines from block-start — on the page of a box
    /// `box_width` wide, as `(x, y, width, height)`. A horizontal context's
    /// line space is the page.
    pub fn line_rect_to_page(
        self,
        (x, y, width, height): (f32, f32, f32, f32),
        box_width: f32,
    ) -> (f32, f32, f32, f32) {
        if !self.is_vertical() {
            return (x, y, width, height);
        }
        let near = self.block_to_page_x(y, box_width);
        let far = self.block_to_page_x(y + height, box_width);
        (near.min(far), x, height, width)
    }

    /// A page point inside a box `box_width` wide, in line space. The inverse
    /// of [`Self::line_rect_to_page`].
    pub fn page_point_to_line(self, x: f32, y: f32, box_width: f32) -> (f32, f32) {
        if !self.is_vertical() {
            return (x, y);
        }
        (y, self.block_to_page_x(x, box_width))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [WritingModeSpec; 3] = [
        WritingModeSpec::HorizontalTb,
        WritingModeSpec::VerticalRl,
        WritingModeSpec::VerticalLr,
    ];

    #[test]
    fn flow_relative_edges_follow_css() {
        let edges = |mode, direction| {
            let context = WritingContext::new(mode, direction);
            (context.inline_start(), context.block_start())
        };
        use PhysicalEdge::*;
        assert_eq!(
            edges(WritingModeSpec::HorizontalTb, DirSpec::Ltr),
            (Left, Top)
        );
        assert_eq!(
            edges(WritingModeSpec::HorizontalTb, DirSpec::Rtl),
            (Right, Top)
        );
        assert_eq!(
            edges(WritingModeSpec::VerticalRl, DirSpec::Ltr),
            (Top, Right)
        );
        // A vertical RTL line starts at the bottom (CSS Writing Modes §2.1).
        assert_eq!(
            edges(WritingModeSpec::VerticalRl, DirSpec::Rtl),
            (Bottom, Right)
        );
        assert_eq!(
            edges(WritingModeSpec::VerticalLr, DirSpec::Ltr),
            (Top, Left)
        );
        assert_eq!(
            edges(WritingModeSpec::VerticalLr, DirSpec::Rtl),
            (Bottom, Left)
        );
        for mode in MODES {
            for direction in [DirSpec::Ltr, DirSpec::Rtl] {
                let context = WritingContext::new(mode, direction);
                assert_eq!(context.inline_end(), context.inline_start().opposite());
                assert_eq!(context.block_end(), context.block_start().opposite());
                let horizontal = |edge| matches!(edge, Left | Right);
                assert_ne!(
                    horizontal(context.inline_start()),
                    horizontal(context.block_start()),
                    "the two axes are perpendicular"
                );
            }
        }
    }

    #[test]
    fn a_page_axis_is_reversed_by_the_logical_axis_on_it() {
        let reversed = |mode, direction, physical| {
            WritingContext::new(mode, direction).physical_axis_reversed(physical)
        };
        use FlexDirection::{Column, Row};
        // Horizontal: the row is the inline axis, the column the block one.
        assert!(!reversed(WritingModeSpec::HorizontalTb, DirSpec::Ltr, Row));
        assert!(reversed(WritingModeSpec::HorizontalTb, DirSpec::Rtl, Row));
        assert!(!reversed(
            WritingModeSpec::HorizontalTb,
            DirSpec::Rtl,
            Column
        ));
        // `vertical-rl`: the row is the block axis, from the right.
        assert!(reversed(WritingModeSpec::VerticalRl, DirSpec::Ltr, Row));
        assert!(!reversed(WritingModeSpec::VerticalRl, DirSpec::Ltr, Column));
        // …and with RTL the column — the inline axis — runs from the bottom.
        assert!(reversed(WritingModeSpec::VerticalRl, DirSpec::Rtl, Column));
        assert!(reversed(WritingModeSpec::VerticalLr, DirSpec::Rtl, Column));
        assert!(!reversed(WritingModeSpec::VerticalLr, DirSpec::Rtl, Row));
    }

    #[test]
    fn flow_rects_land_from_the_start_edges() {
        let rect = (10.0, 20.0, 30.0, 5.0);
        let page = |mode, direction| {
            WritingContext::new(mode, direction).flow_rect_to_page(rect, (100.0, 50.0))
        };
        assert_eq!(
            page(WritingModeSpec::HorizontalTb, DirSpec::Ltr),
            (10.0, 20.0, 30.0, 5.0)
        );
        // RTL: 10 in from the right edge of a 100-wide box.
        assert_eq!(
            page(WritingModeSpec::HorizontalTb, DirSpec::Rtl),
            (60.0, 20.0, 30.0, 5.0)
        );
        // `vertical-rl`: inline down from the top, block 20 in from the right
        // of a 50-wide box.
        assert_eq!(
            page(WritingModeSpec::VerticalRl, DirSpec::Ltr),
            (25.0, 10.0, 5.0, 30.0)
        );
        // …and with RTL, inline up from the bottom of a 100-tall box.
        assert_eq!(
            page(WritingModeSpec::VerticalRl, DirSpec::Rtl),
            (25.0, 60.0, 5.0, 30.0)
        );
        assert_eq!(
            page(WritingModeSpec::VerticalLr, DirSpec::Ltr),
            (20.0, 10.0, 5.0, 30.0)
        );
        let context = WritingContext::new(WritingModeSpec::VerticalLr, DirSpec::Ltr);
        assert_eq!(context.logical_size(40.0, 70.0), (70.0, 40.0));
        assert_eq!(context.physical_size(70.0, 40.0), (40.0, 70.0));
    }

    #[test]
    fn line_space_ignores_direction_and_round_trips() {
        for mode in MODES {
            let ltr = WritingContext::new(mode, DirSpec::Ltr);
            let rtl = WritingContext::new(mode, DirSpec::Rtl);
            let rect = (10.0, 20.0, 30.0, 5.0);
            assert_eq!(
                ltr.line_rect_to_page(rect, 100.0),
                rtl.line_rect_to_page(rect, 100.0),
                "line-left does not move with direction"
            );
            let (x, y, _, _) = ltr.line_rect_to_page(rect, 100.0);
            let (px, py) = if mode.is_vertical() && ltr.block_reversed() {
                // The rect's page x is its block *end*, mirrored.
                (x + 5.0, y)
            } else {
                (x, y)
            };
            assert_eq!(ltr.page_point_to_line(px, py, 100.0).0, 10.0);
        }
        let rl = WritingContext::new(WritingModeSpec::VerticalRl, DirSpec::Ltr);
        assert_eq!(
            rl.line_rect_to_page((10.0, 20.0, 20.0, 20.0), 100.0),
            (60.0, 10.0, 20.0, 20.0)
        );
        assert_eq!(rl.page_point_to_line(70.0, 15.0, 100.0), (15.0, 30.0));
        let lr = WritingContext::new(WritingModeSpec::VerticalLr, DirSpec::Ltr);
        assert_eq!(
            lr.line_rect_to_page((10.0, 20.0, 20.0, 20.0), 100.0),
            (20.0, 10.0, 20.0, 20.0)
        );
    }
}
