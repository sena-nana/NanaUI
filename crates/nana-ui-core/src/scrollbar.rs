//! Backend-neutral scrollbar geometry.
//!
//! Scroll position stays authoritative in the Runtime scroll state. Nothing
//! here stores an offset: a track is derived from viewport / content / offset
//! on demand, and dragging maps a thumb position back to a content offset.

use serde::{Deserialize, Serialize};

/// Which axis a scrollbar drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

/// How a scroll container presents its scrollbars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScrollbarVisibility {
    /// Overlay bars that appear while the container is hovered or dragged.
    #[default]
    AutoHide,
    /// Overlay bars that stay drawn while the axis can scroll.
    Always,
    /// No bar. Wheel and programmatic scrolling still work.
    Hidden,
}

/// Scrollbar chrome geometry, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrollbarMetrics {
    /// Cross-axis extent of the track.
    pub thickness: f32,
    /// Cross-axis extent of the thumb, inset inside the track.
    pub thumb_thickness: f32,
    /// Shortest thumb the track will produce, so long content stays grabbable.
    pub thumb_min_length: f32,
    /// Gap between the track and the viewport edges.
    pub track_inset: f32,
    /// Distance a track click pages, as a fraction of the viewport.
    pub page_fraction: f32,
}

/// Shared Lilia-style scrollbar geometry.
pub const SCROLLBAR_METRICS: ScrollbarMetrics = ScrollbarMetrics {
    thickness: 12.0,
    thumb_thickness: 6.0,
    thumb_min_length: 24.0,
    track_inset: 2.0,
    page_fraction: 0.9,
};

impl Default for ScrollbarMetrics {
    fn default() -> Self {
        SCROLLBAR_METRICS
    }
}

impl ScrollbarMetrics {
    /// Thumb corner radius: a fully rounded capsule.
    pub fn thumb_radius(self) -> f32 {
        self.thumb_thickness.max(0.0) / 2.0
    }
}

/// One axis of derived scrollbar geometry, along the scrolling axis only.
///
/// `origin` / `length` describe the track; `thumb_origin` / `thumb_length`
/// describe the thumb inside it. The cross axis is the caller's business.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarTrack {
    pub origin: f32,
    pub length: f32,
    pub thumb_origin: f32,
    pub thumb_length: f32,
    /// Largest content offset this axis can reach.
    pub max_offset: f32,
}

impl ScrollbarTrack {
    /// Distance the thumb can travel. Zero when the thumb fills the track.
    pub fn travel(self) -> f32 {
        (self.length - self.thumb_length).max(0.0)
    }

    /// Content offset for a thumb leading edge, clamped to the scroll range.
    pub fn offset_for_thumb_origin(self, thumb_origin: f32) -> f32 {
        let travel = self.travel();
        if travel <= 0.0 {
            return 0.0;
        }
        let ratio = ((thumb_origin - self.origin) / travel).clamp(0.0, 1.0);
        ratio * self.max_offset
    }

    /// Content offset that centres the thumb on a track position.
    pub fn offset_for_position(self, position: f32) -> f32 {
        self.offset_for_thumb_origin(position - self.thumb_length / 2.0)
    }

    /// Whether a track position falls on the thumb.
    pub fn thumb_contains(self, position: f32) -> bool {
        position >= self.thumb_origin && position < self.thumb_origin + self.thumb_length
    }
}

/// Derive one axis of scrollbar geometry.
///
/// Returns `None` when the axis cannot scroll, when the track has no room, or
/// when any input is not finite — callers then draw nothing and hit nothing.
pub fn scrollbar_track(
    viewport: f32,
    content: f32,
    offset: f32,
    track_origin: f32,
    track_length: f32,
    metrics: ScrollbarMetrics,
) -> Option<ScrollbarTrack> {
    if ![viewport, content, offset, track_origin, track_length]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    if viewport <= 0.0 || track_length <= 0.0 {
        return None;
    }
    let max_offset = content - viewport;
    if max_offset <= 0.0 {
        return None;
    }
    let proportional = track_length * (viewport / content);
    let thumb_length = proportional
        .max(metrics.thumb_min_length.max(0.0))
        .min(track_length);
    let travel = (track_length - thumb_length).max(0.0);
    let ratio = (offset / max_offset).clamp(0.0, 1.0);
    Some(ScrollbarTrack {
        origin: track_origin,
        length: track_length,
        thumb_origin: track_origin + travel * ratio,
        thumb_length,
        max_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_shorter_than_its_content_yields_a_proportional_thumb() {
        let track = scrollbar_track(100.0, 400.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_length, 25.0);
        assert_eq!(track.thumb_origin, 0.0);
        assert_eq!(track.max_offset, 300.0);
        assert_eq!(track.travel(), 75.0);
    }

    #[test]
    fn the_thumb_reaches_the_track_end_at_the_maximum_offset() {
        let track = scrollbar_track(100.0, 400.0, 300.0, 10.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_origin + track.thumb_length, 110.0);
    }

    #[test]
    fn very_long_content_keeps_a_grabbable_thumb() {
        let track = scrollbar_track(100.0, 100_000.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_length, SCROLLBAR_METRICS.thumb_min_length);
    }

    #[test]
    fn content_within_the_viewport_has_no_track() {
        assert!(scrollbar_track(100.0, 100.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(100.0, 40.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(0.0, 400.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(100.0, 400.0, 0.0, 0.0, 0.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(f32::NAN, 400.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
    }

    #[test]
    fn thumb_position_round_trips_through_the_content_offset() {
        let track = scrollbar_track(100.0, 400.0, 120.0, 4.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        let offset = track.offset_for_thumb_origin(track.thumb_origin);
        assert!((offset - 120.0).abs() < 0.001, "offset {offset}");
        assert_eq!(track.offset_for_thumb_origin(track.origin), 0.0);
        assert_eq!(
            track.offset_for_thumb_origin(track.origin + track.travel()),
            track.max_offset
        );
    }

    #[test]
    fn hit_testing_covers_the_thumb_but_not_the_bare_track() {
        let track = scrollbar_track(100.0, 400.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert!(track.thumb_contains(0.0));
        assert!(track.thumb_contains(24.0));
        assert!(!track.thumb_contains(25.0));
        assert!(!track.thumb_contains(-1.0));
    }
}
