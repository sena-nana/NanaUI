//! Content aspect ratio held through a user frame resize.
//!
//! The ratio applies to the client area, not the outer frame, so the
//! non-client border and caption never skew it. The math here is pure; the
//! Win32 size-move hook feeds it `WM_SIZING` rectangles and the custom-chrome
//! [`crate::LiveFrameResize`] feeds it the frames it is about to apply.

use crate::FrameResizeEdge;

/// The ratio a window's client area keeps while the user resizes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AspectLock {
    /// Client width over client height, finite and positive.
    pub ratio: f64,
    /// Smallest client size, in the same units as the frames it constrains.
    pub minimum: (f64, f64),
}

// Reached from the Windows size-move hook and the custom-chrome resize; other
// targets only test it.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
impl AspectLock {
    /// The outer frame `[left, top, right, bottom]` (y grows down) closest to
    /// `proposed` whose client area has this ratio.
    ///
    /// `frame` is the non-client size per axis (outer minus client) and
    /// `reference` the client size when the gesture began. The dragged edge
    /// picks the axis that follows the pointer: a top or bottom edge keeps
    /// the proposed height and derives the width, a left or right edge keeps
    /// the proposed width and derives the height, and a corner follows the
    /// axis that changed more relative to `reference`. The result never goes
    /// below `minimum` on either edge. The edges opposite the dragged ones
    /// stay put; a top or bottom drag keeps the left edge, a left or right
    /// drag keeps the top edge. Sizes come out whole.
    pub(crate) fn constrain_frame(
        self,
        proposed: [f64; 4],
        edge: FrameResizeEdge,
        frame: (f64, f64),
        reference: (f64, f64),
    ) -> [f64; 4] {
        let [left, top, right, bottom] = proposed;
        let client = (
            (right - left - frame.0).max(0.0),
            (bottom - top - frame.1).max(0.0),
        );
        let ratio = self.ratio;
        let width_drives = match edge {
            FrameResizeEdge::East | FrameResizeEdge::West => true,
            FrameResizeEdge::North | FrameResizeEdge::South => false,
            _ => relative_change(client.0, reference.0) >= relative_change(client.1, reference.1),
        };
        // The smallest ratio-exact size that breaks neither minimum edge.
        let min_width = self.minimum.0.max(self.minimum.1 * ratio).max(1.0).ceil();
        let (width, height) = if width_drives {
            let width = client.0.round().max(min_width);
            (width, (width / ratio).round().max(1.0))
        } else {
            let height = client.1.round().max((min_width / ratio).ceil());
            ((height * ratio).round().max(min_width), height)
        };
        let (outer_width, outer_height) = (width + frame.0, height + frame.1);
        let (left, right) = if matches!(
            edge,
            FrameResizeEdge::West | FrameResizeEdge::NorthWest | FrameResizeEdge::SouthWest
        ) {
            (right - outer_width, right)
        } else {
            (left, left + outer_width)
        };
        let (top, bottom) = if matches!(
            edge,
            FrameResizeEdge::North | FrameResizeEdge::NorthWest | FrameResizeEdge::NorthEast
        ) {
            (bottom - outer_height, bottom)
        } else {
            (top, top + outer_height)
        };
        [left, top, right, bottom]
    }
}

fn relative_change(value: f64, reference: f64) -> f64 {
    if reference > 0.0 {
        ((value - reference) / reference).abs()
    } else {
        0.0
    }
}

/// The edge a `WM_SIZING` `wParam` (`WMSZ_*`) names.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn sizing_edge(wmsz: usize) -> Option<FrameResizeEdge> {
    Some(match wmsz {
        1 => FrameResizeEdge::West,
        2 => FrameResizeEdge::East,
        3 => FrameResizeEdge::North,
        4 => FrameResizeEdge::NorthWest,
        5 => FrameResizeEdge::NorthEast,
        6 => FrameResizeEdge::South,
        7 => FrameResizeEdge::SouthWest,
        8 => FrameResizeEdge::SouthEast,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: AspectLock = AspectLock {
        ratio: 16.0 / 9.0,
        minimum: (320.0, 180.0),
    };
    /// 8 px side borders, 31 px caption plus bottom border.
    const FRAME: (f64, f64) = (16.0, 39.0);

    fn client(frame: [f64; 4]) -> (f64, f64) {
        (frame[2] - frame[0] - FRAME.0, frame[3] - frame[1] - FRAME.1)
    }

    #[test]
    fn a_bottom_drag_keeps_its_height_and_widens_to_the_right() {
        // 960 x 540 client at (100, 100); the bottom edge pulled down 90 px.
        let proposed = [100.0, 100.0, 1076.0, 769.0];
        let next = WIDE.constrain_frame(proposed, FrameResizeEdge::South, FRAME, (960.0, 540.0));
        assert_eq!(client(next), (1120.0, 630.0));
        assert_eq!((next[0], next[1]), (100.0, 100.0), "left and top stay");
    }

    #[test]
    fn a_top_drag_keeps_the_bottom_edge() {
        // The top edge pushed down 90 px: the client shrinks to 450 tall.
        let proposed = [100.0, 190.0, 1076.0, 679.0];
        let next = WIDE.constrain_frame(proposed, FrameResizeEdge::North, FRAME, (960.0, 540.0));
        assert_eq!(client(next), (800.0, 450.0));
        assert_eq!((next[0], next[3]), (100.0, 679.0));
    }

    #[test]
    fn a_left_drag_keeps_its_width_and_the_right_and_top_edges() {
        let proposed = [260.0, 100.0, 1076.0, 679.0];
        let next = WIDE.constrain_frame(proposed, FrameResizeEdge::West, FRAME, (960.0, 540.0));
        assert_eq!(client(next), (800.0, 450.0));
        assert_eq!((next[1], next[2]), (100.0, 1076.0));
    }

    #[test]
    fn a_corner_follows_the_axis_that_moved_more() {
        // Mostly sideways: width 960 -> 1280 (+33 %), height 540 -> 560 (+4 %).
        let sideways = [100.0, 100.0, 1396.0, 699.0];
        let next =
            WIDE.constrain_frame(sideways, FrameResizeEdge::SouthEast, FRAME, (960.0, 540.0));
        assert_eq!(client(next), (1280.0, 720.0));
        // Mostly down: height 540 -> 720 (+33 %), width 960 -> 980 (+2 %).
        let down = [100.0, 100.0, 1096.0, 859.0];
        let next = WIDE.constrain_frame(down, FrameResizeEdge::SouthEast, FRAME, (960.0, 540.0));
        assert_eq!(client(next), (1280.0, 720.0));
        // A north-west corner keeps the bottom-right corner.
        let next = WIDE.constrain_frame(
            [0.0, 0.0, 1076.0, 679.0],
            FrameResizeEdge::NorthWest,
            FRAME,
            (960.0, 540.0),
        );
        assert_eq!((next[2], next[3]), (1076.0, 679.0));
    }

    #[test]
    fn the_minimum_holds_on_both_edges_for_portrait_content() {
        // A 9:16 lock with a square 320 minimum: width may not go under 320,
        // so height may not go under 569.
        let tall = AspectLock {
            ratio: 9.0 / 16.0,
            minimum: (320.0, 320.0),
        };
        let squeezed = [0.0, 0.0, 16.0 + 100.0, 39.0 + 300.0];
        let next = tall.constrain_frame(squeezed, FrameResizeEdge::South, FRAME, (360.0, 640.0));
        let (width, height) = client(next);
        assert!(width >= 320.0 && height >= 569.0, "{width} x {height}");
        let next = tall.constrain_frame(squeezed, FrameResizeEdge::East, FRAME, (360.0, 640.0));
        let (width, height) = client(next);
        assert!(width >= 320.0 && height >= 569.0, "{width} x {height}");
    }
}
