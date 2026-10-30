//! Backend-neutral scrollbar geometry.
//!
//! Scroll position stays authoritative in the Runtime scroll state. Nothing
//! here stores an offset: a track is derived from viewport / content / offset
//! on demand, and dragging maps a thumb position back to a content offset.
//!
//! A bar never takes layout space. It runs in a *lane* `track_inset` inside
//! its container's padding edge and paints above the container's content; a
//! container that wants the bar clear of its content leaves at least
//! `track_inset + thickness` of padding on that edge. The thumb is centred in
//! the lane and grows from its resting thickness to its active one while the
//! lane is hot or the thumb is dragged.

use serde::{Deserialize, Serialize};

/// Which axis a scrollbar drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

/// How a scroll container presents its scrollbars. Every mode overlays: a
/// bar never changes layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScrollbarVisibility {
    /// Shown while the pointer is inside the container, after the user
    /// scrolls it and while its bar is hot or grabbed; fades out a moment
    /// after none of those hold.
    #[default]
    AutoHide,
    /// Drawn whenever the axis can scroll, over the resident track.
    Always,
    /// No bar. Wheel and programmatic scrolling still work.
    Hidden,
}

/// Scrollbar chrome geometry, in logical pixels.
///
/// `serde(default)` so a metrics blob written before a field existed still
/// loads; a missing field takes its [`SCROLLBAR_METRICS`] value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrollbarMetrics {
    /// Cross-axis extent of the lane: the band the bar is grabbed in, which
    /// the thumb and the track are centred in.
    pub thickness: f32,
    /// Cross-axis extent of the thumb at rest.
    pub thumb_thickness: f32,
    /// Cross-axis extent of the thumb while the lane is hot or the thumb is
    /// dragged, and of the track behind it.
    pub thumb_active_thickness: f32,
    /// Shortest thumb the track will produce, so long content stays grabbable.
    pub thumb_min_length: f32,
    /// Gap between the lane and the container's padding edge, and between the
    /// lane's ends and the container's edges.
    pub track_inset: f32,
    /// Reserved. A press on bare track centres the thumb on the press rather
    /// than paging by this fraction of the viewport.
    pub page_fraction: f32,
}

/// Shared Lilia-style scrollbar geometry.
pub const SCROLLBAR_METRICS: ScrollbarMetrics = ScrollbarMetrics {
    thickness: 12.0,
    thumb_thickness: 6.0,
    thumb_active_thickness: 8.0,
    thumb_min_length: 24.0,
    track_inset: 2.0,
    page_fraction: 0.9,
};

impl Default for ScrollbarMetrics {
    fn default() -> Self {
        SCROLLBAR_METRICS
    }
}

/// CSS `::-webkit-scrollbar` / `::-webkit-scrollbar-thumb` overlay on the
/// installed [`ScrollbarMetrics`], or a Rust caller's per-node sizes. Does
/// not invent a second thumb geometry engine: [`scrollbar_layout`] still
/// derives the thumb.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ScrollbarSkin {
    /// Lane extent. `None` keeps the theme's.
    pub thickness: Option<f32>,
    /// Resting thumb extent. `None` scales the theme's thumb with the lane.
    pub thumb_thickness: Option<f32>,
    /// Active thumb extent. `None` scales the theme's with the lane, or keeps
    /// [`Self::thumb_thickness`] when that is named, so a CSS-sized thumb
    /// never grows.
    #[serde(default)]
    pub thumb_active_thickness: Option<f32>,
    /// Track fill. `None` keeps the theme's.
    pub track_color: Option<[f32; 4]>,
    /// Thumb fill. `None` keeps the theme's.
    pub thumb_color: Option<[f32; 4]>,
    /// The margin the bar runs in, when that is narrower than the theme's
    /// lane and its inset on both sides: the lane narrows to fit it, and the
    /// thumbs stay no wider than the lane. Resolved against the installed
    /// metrics when painted, so a theme with a narrower lane still wins.
    /// A room that is a theme value is written to the node's
    /// [`crate::ThemeLayout::scrollbar_room`] instead, which resolves onto
    /// this field with the rest of the node's layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<f32>,
}

impl ScrollbarSkin {
    pub fn is_empty(self) -> bool {
        self.thickness.is_none()
            && self.thumb_thickness.is_none()
            && self.thumb_active_thickness.is_none()
            && self.track_color.is_none()
            && self.thumb_color.is_none()
            && self.room.is_none()
    }

    /// Apply this skin onto the installed metrics without replacing track
    /// math. The thumbs stay inside the lane, and the active thumb is never
    /// thinner than the resting one.
    pub fn metrics(self, base: ScrollbarMetrics) -> ScrollbarMetrics {
        let positive = |value: Option<f32>| value.filter(|value| value.is_finite() && *value > 0.0);
        let thickness = positive(self.thickness).unwrap_or(base.thickness);
        let scale = if base.thickness > 0.0 {
            thickness / base.thickness
        } else {
            1.0
        };
        let named_thumb = positive(self.thumb_thickness);
        let thumb_thickness = named_thumb
            .unwrap_or(base.thumb_thickness * scale)
            .min(thickness)
            .max(0.0);
        let thumb_active_thickness = positive(self.thumb_active_thickness)
            .unwrap_or(match named_thumb {
                Some(_) => thumb_thickness,
                None => base.thumb_active_thickness * scale,
            })
            .min(thickness)
            .max(thumb_thickness);
        let fitted = positive(self.room)
            .map(|room| (room - 2.0 * base.track_inset).max(1.0))
            .filter(|lane| *lane < thickness);
        let (thickness, thumb_thickness, thumb_active_thickness) = match fitted {
            Some(lane) => (
                lane,
                thumb_thickness.min(lane),
                thumb_active_thickness.min(lane),
            ),
            None => (thickness, thumb_thickness, thumb_active_thickness),
        };
        ScrollbarMetrics {
            thickness,
            thumb_thickness,
            thumb_active_thickness,
            ..base
        }
    }
}

impl ScrollbarMetrics {
    /// Resting thumb corner radius: a fully rounded capsule.
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
    /// Smallest content offset this axis can reach: `0`, or negative for an
    /// axis that starts at the right / bottom and overflows the other way.
    pub min_offset: f32,
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
        self.min_offset + ratio * (self.max_offset - self.min_offset)
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
/// `min_offset` is the axis's scroll origin: `0`, or negative when the axis
/// starts at the right / bottom, so `offset` runs from it to `0`. The thumb
/// sits at the track start at `min_offset` either way — the track is physical.
///
/// Returns `None` when the axis cannot scroll, when the track has no room, or
/// when any input is not finite — callers then draw nothing and hit nothing.
pub fn scrollbar_track(
    viewport: f32,
    content: f32,
    offset: f32,
    min_offset: f32,
    track_origin: f32,
    track_length: f32,
    metrics: ScrollbarMetrics,
) -> Option<ScrollbarTrack> {
    if ![
        viewport,
        content,
        offset,
        min_offset,
        track_origin,
        track_length,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return None;
    }
    if viewport <= 0.0 || track_length <= 0.0 {
        return None;
    }
    let range = content - viewport;
    if range <= 0.0 {
        return None;
    }
    let proportional = track_length * (viewport / content);
    let thumb_length = proportional
        .max(metrics.thumb_min_length.max(0.0))
        .min(track_length);
    let travel = (track_length - thumb_length).max(0.0);
    let ratio = ((offset - min_offset) / range).clamp(0.0, 1.0);
    Some(ScrollbarTrack {
        origin: track_origin,
        length: track_length,
        thumb_origin: track_origin + travel * ratio,
        thumb_length,
        min_offset,
        max_offset: min_offset + range,
    })
}

/// A physical rectangle in the container's coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollbarRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ScrollbarRect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Where one axis of a bar sits and how big its parts are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarLayout {
    /// The band the bar is grabbed in; also the hover target.
    pub lane: ScrollbarRect,
    /// The track capsule: the lane's length, the active thumb's thickness,
    /// centred in the lane.
    pub track: ScrollbarRect,
    /// The thumb, centred in the lane, between its resting and active
    /// thickness.
    pub thumb: ScrollbarRect,
    pub track_radius: f32,
    pub thumb_radius: f32,
    /// The thumb's position along the lane, for drag maths.
    pub axis: ScrollbarTrack,
}

/// What [`scrollbar_layout`] needs to know about the content on one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarExtent {
    pub viewport: f32,
    pub content: f32,
    pub offset: f32,
    /// See [`scrollbar_track`].
    pub min_offset: f32,
}

/// Lay out one axis of a bar inside a container's padding box.
///
/// The vertical lane runs down the physical right edge and the horizontal
/// lane along the bottom, each `track_inset` inside the padding edge and
/// inset by the same amount at both ends. `corner` says the other axis has a
/// bar too: the lane then stops where the other lane starts, so the two never
/// overlap. `expand` (`0..=1`) moves the thumb from its resting thickness to
/// its active one, growing evenly about the lane's centre line.
///
/// Returns `None` when the axis cannot scroll or the lane has no room.
pub fn scrollbar_layout(
    axis: ScrollbarAxis,
    padding_box: ScrollbarRect,
    corner: bool,
    extent: ScrollbarExtent,
    expand: f32,
    metrics: ScrollbarMetrics,
) -> Option<ScrollbarLayout> {
    let inset = finite_non_negative(metrics.track_inset);
    let thickness = finite_non_negative(metrics.thickness);
    if thickness <= 0.0 {
        return None;
    }
    let rest = finite_non_negative(metrics.thumb_thickness).min(thickness);
    let active = finite_non_negative(metrics.thumb_active_thickness).clamp(rest, thickness);
    let expand = if expand.is_finite() {
        expand.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cross = rest + (active - rest) * expand;
    let corner = if corner { thickness } else { 0.0 };
    let horizontal = matches!(axis, ScrollbarAxis::Horizontal);
    let (start, span) = if horizontal {
        (padding_box.x, padding_box.width)
    } else {
        (padding_box.y, padding_box.height)
    };
    let lane_length = span - 2.0 * inset - corner;
    let track = scrollbar_track(
        extent.viewport,
        extent.content,
        extent.offset,
        extent.min_offset,
        start + inset,
        lane_length,
        metrics,
    )?;
    // The lane's leading cross-axis edge, and the centre line the thumb and
    // the track share.
    let lane_cross = if horizontal {
        padding_box.y + padding_box.height - inset - thickness
    } else {
        padding_box.x + padding_box.width - inset - thickness
    };
    let centre = lane_cross + thickness / 2.0;
    let place = |along: f32, length: f32, girth: f32| {
        if horizontal {
            ScrollbarRect {
                x: along,
                y: centre - girth / 2.0,
                width: length,
                height: girth,
            }
        } else {
            ScrollbarRect {
                x: centre - girth / 2.0,
                y: along,
                width: girth,
                height: length,
            }
        }
    };
    Some(ScrollbarLayout {
        lane: place(track.origin, track.length, thickness),
        track: place(track.origin, track.length, active),
        thumb: place(track.thumb_origin, track.thumb_length, cross),
        track_radius: active / 2.0,
        thumb_radius: cross / 2.0,
        axis: track,
    })
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_shorter_than_its_content_yields_a_proportional_thumb() {
        let track = scrollbar_track(100.0, 400.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_length, 25.0);
        assert_eq!(track.thumb_origin, 0.0);
        assert_eq!(track.max_offset, 300.0);
        assert_eq!(track.travel(), 75.0);
    }

    #[test]
    fn the_thumb_reaches_the_track_end_at_the_maximum_offset() {
        let track = scrollbar_track(100.0, 400.0, 300.0, 0.0, 10.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_origin + track.thumb_length, 110.0);
    }

    #[test]
    fn very_long_content_keeps_a_grabbable_thumb() {
        let track = scrollbar_track(100.0, 100_000.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(track.thumb_length, SCROLLBAR_METRICS.thumb_min_length);
    }

    #[test]
    fn content_within_the_viewport_has_no_track() {
        assert!(scrollbar_track(100.0, 100.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(100.0, 40.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(0.0, 400.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none());
        assert!(scrollbar_track(100.0, 400.0, 0.0, 0.0, 0.0, 0.0, SCROLLBAR_METRICS).is_none());
        assert!(
            scrollbar_track(f32::NAN, 400.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS).is_none()
        );
    }

    #[test]
    fn thumb_position_round_trips_through_the_content_offset() {
        let track = scrollbar_track(100.0, 400.0, 120.0, 0.0, 4.0, 100.0, SCROLLBAR_METRICS)
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
    fn a_reversed_axis_puts_its_origin_at_the_track_start() {
        // An RTL axis: 300px overflow to the left, offsets -300..=0, the
        // initial offset 0 showing the start (right) edge.
        let start = scrollbar_track(100.0, 400.0, 0.0, -300.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(start.thumb_origin + start.thumb_length, 100.0);
        assert_eq!(start.max_offset, 0.0);
        let far = scrollbar_track(100.0, 400.0, -300.0, -300.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert_eq!(far.thumb_origin, 0.0);
        assert_eq!(far.offset_for_thumb_origin(far.origin), -300.0);
        assert_eq!(far.offset_for_thumb_origin(far.origin + far.travel()), 0.0);
    }

    #[test]
    fn hit_testing_covers_the_thumb_but_not_the_bare_track() {
        let track = scrollbar_track(100.0, 400.0, 0.0, 0.0, 0.0, 100.0, SCROLLBAR_METRICS)
            .expect("scrollable axis");
        assert!(track.thumb_contains(0.0));
        assert!(track.thumb_contains(24.0));
        assert!(!track.thumb_contains(25.0));
        assert!(!track.thumb_contains(-1.0));
    }

    #[test]
    fn scrollbar_skin_overrides_thickness_and_scales_the_thumb() {
        let skin = ScrollbarSkin {
            thickness: Some(8.0),
            ..ScrollbarSkin::default()
        };
        let metrics = skin.metrics(SCROLLBAR_METRICS);
        assert_eq!(metrics.thickness, 8.0);
        assert!((metrics.thumb_thickness - 4.0).abs() < f32::EPSILON);
        // The active thumb scales with the lane too.
        assert!((metrics.thumb_active_thickness - 8.0 * 8.0 / 12.0).abs() < 1e-5);
        assert_eq!(metrics.thumb_min_length, SCROLLBAR_METRICS.thumb_min_length);
        let thick_thumb = ScrollbarSkin {
            thickness: Some(16.0),
            thumb_thickness: Some(10.0),
            ..ScrollbarSkin::default()
        }
        .metrics(SCROLLBAR_METRICS);
        assert_eq!(thick_thumb.thumb_thickness, 10.0);
        let clamped = ScrollbarSkin {
            thickness: Some(6.0),
            thumb_thickness: Some(20.0),
            ..ScrollbarSkin::default()
        }
        .metrics(SCROLLBAR_METRICS);
        assert_eq!(clamped.thumb_thickness, 6.0);
        assert_eq!(clamped.thumb_active_thickness, 6.0);
    }

    /// A bar in a margin narrower than the theme's lane and inset narrows its
    /// lane to fit and keeps its thumbs inside it; a wide enough margin
    /// changes nothing.
    #[test]
    fn a_skin_room_fits_the_lane_into_the_margin() {
        let base = SCROLLBAR_METRICS;
        let fit = |room: f32| {
            ScrollbarSkin {
                room: Some(room),
                ..ScrollbarSkin::default()
            }
            .metrics(base)
        };
        let wide = fit(base.thickness + 2.0 * base.track_inset);
        assert_eq!(wide, base);
        let narrow = fit(10.0);
        let lane = 10.0 - 2.0 * base.track_inset;
        assert_eq!(narrow.thickness, lane);
        assert_eq!(narrow.thumb_thickness, base.thumb_thickness.min(lane));
        assert_eq!(
            narrow.thumb_active_thickness,
            base.thumb_active_thickness.min(lane)
        );
        assert!(
            !ScrollbarSkin {
                room: Some(10.0),
                ..ScrollbarSkin::default()
            }
            .is_empty()
        );
    }

    /// A CSS thumb width has no hover size: the thumb it names stays that
    /// size when the lane is hot. A Rust caller can still name both.
    #[test]
    fn a_named_thumb_keeps_its_size_unless_the_active_size_is_named() {
        let css = ScrollbarSkin {
            thumb_thickness: Some(5.0),
            ..ScrollbarSkin::default()
        }
        .metrics(SCROLLBAR_METRICS);
        assert_eq!(
            (css.thumb_thickness, css.thumb_active_thickness),
            (5.0, 5.0)
        );
        let both = ScrollbarSkin {
            thickness: Some(6.0),
            thumb_thickness: Some(4.0),
            thumb_active_thickness: Some(9.0),
            ..ScrollbarSkin::default()
        }
        .metrics(SCROLLBAR_METRICS);
        // The active thumb stays inside the lane.
        assert_eq!(
            (both.thumb_thickness, both.thumb_active_thickness),
            (4.0, 6.0)
        );
        let thinner = ScrollbarSkin {
            thumb_thickness: Some(6.0),
            thumb_active_thickness: Some(2.0),
            ..ScrollbarSkin::default()
        }
        .metrics(SCROLLBAR_METRICS);
        assert_eq!(thinner.thumb_active_thickness, 6.0);
    }

    /// Metrics saved before the active thickness existed still load, with the
    /// field they lack at its default.
    #[test]
    fn metrics_written_before_the_active_thickness_still_load() {
        let old = r#"{"thickness":10.0,"thumb_thickness":4.0,"thumb_min_length":20.0,"track_inset":1.0,"page_fraction":0.9}"#;
        let metrics: ScrollbarMetrics = serde_json::from_str(old).expect("old metrics load");
        assert_eq!(metrics.thickness, 10.0);
        assert_eq!(metrics.thumb_thickness, 4.0);
        assert_eq!(
            metrics.thumb_active_thickness,
            SCROLLBAR_METRICS.thumb_active_thickness
        );
        let skin: ScrollbarSkin = serde_json::from_str(
            r#"{"thickness":8.0,"thumb_thickness":null,"track_color":null,"thumb_color":null}"#,
        )
        .expect("old skin loads");
        assert_eq!(skin.thumb_active_thickness, None);
    }

    fn extent(viewport: f32, content: f32, offset: f32) -> ScrollbarExtent {
        ScrollbarExtent {
            viewport,
            content,
            offset,
            min_offset: 0.0,
        }
    }

    const FRAME: ScrollbarRect = ScrollbarRect {
        x: 10.0,
        y: 20.0,
        width: 200.0,
        height: 120.0,
    };

    /// The lane sits `track_inset` inside the padding edge and is inset at
    /// both ends; the thumb and the track share its centre line.
    #[test]
    fn the_lane_sits_inset_inside_the_padding_edge() {
        let bar = scrollbar_layout(
            ScrollbarAxis::Vertical,
            FRAME,
            false,
            extent(120.0, 480.0, 0.0),
            0.0,
            SCROLLBAR_METRICS,
        )
        .expect("scrolls");
        let right = FRAME.x + FRAME.width;
        assert_eq!(bar.lane.x + bar.lane.width, right - 2.0);
        assert_eq!(bar.lane.width, 12.0);
        assert_eq!((bar.lane.y, bar.lane.height), (22.0, 116.0));
        let centre = bar.lane.x + 6.0;
        assert_eq!(bar.thumb.x + bar.thumb.width / 2.0, centre);
        assert_eq!(bar.track.x + bar.track.width / 2.0, centre);
        assert_eq!(bar.thumb.width, 6.0);
        assert_eq!(bar.track.width, 8.0);
        assert_eq!((bar.thumb.y, bar.thumb.height), (22.0, 29.0));
        assert_eq!((bar.thumb_radius, bar.track_radius), (3.0, 4.0));
        assert_eq!(bar.axis.origin, bar.lane.y);
        assert_eq!(bar.axis.length, bar.lane.height);
        let horizontal = scrollbar_layout(
            ScrollbarAxis::Horizontal,
            FRAME,
            false,
            extent(200.0, 400.0, 0.0),
            0.0,
            SCROLLBAR_METRICS,
        )
        .expect("scrolls");
        assert_eq!(
            horizontal.lane.y + horizontal.lane.height,
            FRAME.y + FRAME.height - 2.0
        );
        assert_eq!((horizontal.lane.x, horizontal.lane.width), (12.0, 196.0));
    }

    /// With both bars, each lane stops where the other starts.
    #[test]
    fn two_lanes_share_the_corner_without_overlapping() {
        let vertical = scrollbar_layout(
            ScrollbarAxis::Vertical,
            FRAME,
            true,
            extent(120.0, 480.0, 0.0),
            0.0,
            SCROLLBAR_METRICS,
        )
        .expect("scrolls");
        let horizontal = scrollbar_layout(
            ScrollbarAxis::Horizontal,
            FRAME,
            true,
            extent(200.0, 400.0, 0.0),
            0.0,
            SCROLLBAR_METRICS,
        )
        .expect("scrolls");
        assert_eq!(vertical.lane.y + vertical.lane.height, horizontal.lane.y);
        assert_eq!(horizontal.lane.x + horizontal.lane.width, vertical.lane.x);
    }

    /// Expanding grows the thumb evenly about the lane's centre, up to the
    /// active thickness, and never past the lane.
    #[test]
    fn expanding_grows_the_thumb_about_the_lane_centre() {
        let at = |expand: f32| {
            scrollbar_layout(
                ScrollbarAxis::Vertical,
                FRAME,
                false,
                extent(120.0, 480.0, 0.0),
                expand,
                SCROLLBAR_METRICS,
            )
            .expect("scrolls")
        };
        let rest = at(0.0);
        let half = at(0.5);
        let full = at(1.0);
        let centre = |bar: ScrollbarLayout| bar.thumb.x + bar.thumb.width / 2.0;
        assert_eq!(centre(rest), centre(full));
        assert_eq!(centre(half), centre(full));
        assert_eq!(
            (rest.thumb.width, half.thumb.width, full.thumb.width),
            (6.0, 7.0, 8.0)
        );
        assert_eq!(full.thumb_radius, 4.0);
        assert_eq!(at(f32::NAN).thumb.width, 6.0);
        let wide = ScrollbarMetrics {
            thumb_active_thickness: 40.0,
            ..SCROLLBAR_METRICS
        };
        let clamped = scrollbar_layout(
            ScrollbarAxis::Vertical,
            FRAME,
            false,
            extent(120.0, 480.0, 0.0),
            1.0,
            wide,
        )
        .expect("scrolls");
        assert_eq!(clamped.thumb.width, 12.0);
    }

    #[test]
    fn a_lane_with_no_room_or_nothing_to_scroll_lays_out_nothing() {
        let tiny = ScrollbarRect {
            height: 4.0,
            ..FRAME
        };
        assert!(
            scrollbar_layout(
                ScrollbarAxis::Vertical,
                tiny,
                false,
                extent(4.0, 400.0, 0.0),
                0.0,
                SCROLLBAR_METRICS
            )
            .is_none()
        );
        assert!(
            scrollbar_layout(
                ScrollbarAxis::Vertical,
                FRAME,
                false,
                extent(120.0, 100.0, 0.0),
                0.0,
                SCROLLBAR_METRICS
            )
            .is_none()
        );
    }
}
