//! Resources sampled at presentation time (Issue #303).
//!
//! A host-texture node (a [`crate::Video`], or a [`crate::GpuTextureView`]
//! fed by an image sequence or sprite frames) may carry one
//! [`MediaTimeMapping`]. It turns the document's presentation time `now`
//! into the time on the resource's own timeline:
//!
//! ```text
//! media_time = in_point + rate * (now - origin)
//! ```
//!
//! Video frames, image sequences and sprite frames are different producers
//! behind the same mapping; the slot stays an ordinary HostTexture. Reverse
//! playback and speed changes rewrite the mapping and nothing else, so the
//! node is never rebuilt for them. Decoding, audio and demuxing stay with the
//! application, which reads [`MediaSampleRequest`]s before the frame is
//! flushed and publishes the matching frame through the host's frame
//! exchange.

use std::time::Duration;

use crate::{StableNodeId, TextureSlotName};

/// Maps a document's presentation time onto a resource's own timeline.
///
/// Times on the resource's timeline are seconds, as
/// [`crate::MediaTransportBar::position`] is. `origin` is the document time at
/// which the resource is at `in_point`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaTimeMapping {
    /// Document presentation time at which the resource shows `in_point`.
    pub origin: Duration,
    /// Resource time at `origin`, in seconds.
    pub in_point: f64,
    /// Resource seconds per document second. 0 holds at `in_point`; a
    /// negative rate plays backwards.
    pub rate: f64,
    /// When set, the resource time is clamped between `in_point` and this
    /// point and holds on whichever end it reached.
    pub out_point: Option<f64>,
}

impl MediaTimeMapping {
    /// Real-time playback of the resource from `in_point` at document time
    /// `origin`, with no out point.
    pub fn new(origin: Duration, in_point: f64) -> Self {
        Self {
            origin,
            in_point,
            rate: 1.0,
            out_point: None,
        }
    }

    /// A mapping that holds `media_time` whatever the document time is.
    pub fn held(media_time: f64) -> Self {
        Self {
            origin: Duration::ZERO,
            in_point: media_time,
            rate: 0.0,
            out_point: None,
        }
    }

    pub fn with_rate(mut self, rate: f64) -> Self {
        self.rate = rate;
        self
    }

    pub fn with_out_point(mut self, out_point: Option<f64>) -> Self {
        self.out_point = out_point;
        self
    }

    /// The resource time at document time `now`.
    ///
    /// A value that is not finite reads as its neutral one: an `in_point` as
    /// 0, a `rate` as 0 (hold), an `out_point` as none.
    pub fn media_time(&self, now: Duration) -> f64 {
        let in_point = self.in_point();
        let rate = self.rate();
        let raw = if rate == 0.0 {
            in_point
        } else {
            in_point + rate * signed_seconds(now, self.origin)
        };
        match self.out_point() {
            Some(out_point) => raw.clamp(in_point.min(out_point), in_point.max(out_point)),
            None => raw,
        }
    }

    /// The same mapping continuing from `media_time` at document time `now`,
    /// at the same rate: what a committed seek writes. The new in point is
    /// the seek target, so the clamp runs from there to the out point.
    pub fn seeked(self, now: Duration, media_time: f64) -> Self {
        Self {
            origin: now,
            in_point: media_time,
            ..self
        }
    }

    /// The same playback continuing from where it is at `now`, at `rate`:
    /// a speed change or reversal without a jump.
    pub fn retimed(self, now: Duration, rate: f64) -> Self {
        Self {
            origin: now,
            in_point: self.media_time(now),
            rate,
            ..self
        }
    }

    /// The earliest document time, at or after `now`, at which the resource
    /// time moves; `None` when it holds from `now` on (rate 0, or clamped at
    /// the end it is heading for).
    pub fn next_motion(&self, now: Duration) -> Option<Duration> {
        let rate = self.rate();
        if rate == 0.0 {
            return None;
        }
        let Some(out_point) = self.out_point() else {
            return Some(now);
        };
        let in_point = self.in_point();
        let (low, high) = (in_point.min(out_point), in_point.max(out_point));
        if low == high {
            return None;
        }
        let raw = in_point + rate * signed_seconds(now, self.origin);
        let (entry, inside, spent) = if rate > 0.0 {
            (low, raw >= low, raw >= high)
        } else {
            (high, raw <= high, raw <= low)
        };
        if spent {
            return None;
        }
        if inside {
            return Some(now);
        }
        // Before the clamp range: it starts moving once the raw time enters.
        let enters = self.origin.as_secs_f64() + (entry - in_point) / rate;
        Some(Duration::try_from_secs_f64(enters).map_or(now, |at| at.max(now)))
    }

    fn in_point(&self) -> f64 {
        finite_or(self.in_point, 0.0)
    }

    fn rate(&self) -> f64 {
        finite_or(self.rate, 0.0)
    }

    fn out_point(&self) -> Option<f64> {
        self.out_point.filter(|point| point.is_finite())
    }
}

/// How the frame a request stands for is produced, which decides what the
/// host does with a frame that is not aligned to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPacing {
    /// The document plays on the display. A frame that is not aligned is not
    /// shown; the last aligned one repeats.
    Realtime,
    /// The document is held or stepped by a caller (an export, a snapshot).
    /// The frame waits until a frame is aligned, or fails explicitly.
    Stepped,
}

/// One visible time-sampled resource, collected before the frame is flushed.
///
/// The host publishes the frame for `media_time` to the slot's frame
/// exchange with the time recorded on the frame's payload; the binding shows
/// a frame only when its time is within [`Self::tolerance`] of `media_time`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSampleRequest {
    pub node: StableNodeId,
    /// The host texture slot the node samples.
    pub slot: TextureSlotName,
    /// The resource time this frame shows, in seconds.
    pub media_time: f64,
    /// The document's frame interval: the display interval while it plays,
    /// or the step a caller advanced by.
    pub frame_interval: Duration,
    pub pacing: MediaPacing,
}

impl MediaSampleRequest {
    /// Half the document's frame interval, in seconds: how far a frame's time
    /// may be from [`Self::media_time`] and still stand for this frame.
    pub fn tolerance(&self) -> f64 {
        self.frame_interval.as_secs_f64() * 0.5
    }

    /// Whether a frame stamped `frame_time` stands for this request.
    pub fn aligned(&self, frame_time: f64) -> bool {
        frame_time.is_finite() && (frame_time - self.media_time).abs() <= self.tolerance()
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

/// `now - origin` in seconds, negative before the origin.
fn signed_seconds(now: Duration, origin: Duration) -> f64 {
    if now >= origin {
        (now - origin).as_secs_f64()
    } else {
        -(origin - now).as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    #[test]
    fn media_time_is_in_point_plus_rate_times_elapsed() {
        let mapping = MediaTimeMapping::new(secs(2.0), 10.0).with_rate(1.5);
        assert_eq!(mapping.media_time(secs(2.0)), 10.0);
        assert_eq!(mapping.media_time(secs(4.0)), 13.0);
        // Before the origin the formula runs backwards, unclamped.
        assert_eq!(mapping.media_time(secs(1.0)), 8.5);
    }

    #[test]
    fn a_negative_rate_plays_backwards_and_clamps_at_the_out_point() {
        let mapping = MediaTimeMapping::new(secs(0.0), 5.0)
            .with_rate(-2.0)
            .with_out_point(Some(1.0));
        assert_eq!(mapping.media_time(secs(1.0)), 3.0);
        // Holds on the out point's frame, not before it.
        assert_eq!(mapping.media_time(secs(10.0)), 1.0);
        // Before the origin it clamps at the in point.
        assert_eq!(mapping.media_time(secs(0.0)), 5.0);
    }

    #[test]
    fn an_out_point_clamps_forward_playback_at_both_ends() {
        let mapping = MediaTimeMapping::new(secs(1.0), 2.0).with_out_point(Some(4.0));
        assert_eq!(mapping.media_time(secs(0.0)), 2.0);
        assert_eq!(mapping.media_time(secs(2.0)), 3.0);
        assert_eq!(mapping.media_time(secs(9.0)), 4.0);
    }

    #[test]
    fn rate_zero_holds_the_in_point() {
        let mapping = MediaTimeMapping::held(7.25);
        for now in [0.0, 3.0, 1.0e6] {
            assert_eq!(mapping.media_time(secs(now)), 7.25);
        }
        assert_eq!(mapping.next_motion(secs(3.0)), None);
    }

    #[test]
    fn non_finite_fields_read_as_neutral() {
        let mapping = MediaTimeMapping {
            origin: Duration::ZERO,
            in_point: f64::NAN,
            rate: f64::INFINITY,
            out_point: Some(f64::NAN),
        };
        assert_eq!(mapping.media_time(secs(5.0)), 0.0);
        assert_eq!(mapping.next_motion(secs(5.0)), None);
    }

    #[test]
    fn seeking_and_retiming_continue_without_a_jump() {
        let mapping = MediaTimeMapping::new(secs(0.0), 0.0).with_out_point(Some(60.0));
        let seeked = mapping.seeked(secs(3.0), 30.0);
        assert_eq!(seeked.media_time(secs(3.0)), 30.0);
        assert_eq!(seeked.media_time(secs(4.0)), 31.0);
        assert_eq!(seeked.out_point, Some(60.0));
        let reversed = seeked.retimed(secs(4.0), -1.0).with_out_point(Some(0.0));
        assert_eq!(reversed.media_time(secs(4.0)), 31.0);
        assert_eq!(reversed.media_time(secs(6.0)), 29.0);
    }

    #[test]
    fn next_motion_reports_when_the_time_moves() {
        let open = MediaTimeMapping::new(secs(1.0), 0.0);
        assert_eq!(open.next_motion(secs(5.0)), Some(secs(5.0)));
        let clamped = MediaTimeMapping::new(secs(2.0), 0.0).with_out_point(Some(1.0));
        // Before the origin it holds the in point until the origin.
        assert_eq!(clamped.next_motion(secs(0.5)), Some(secs(2.0)));
        assert_eq!(clamped.next_motion(secs(2.5)), Some(secs(2.5)));
        // Holding on the out point, it never moves again.
        assert_eq!(clamped.next_motion(secs(3.0)), None);
        assert_eq!(clamped.next_motion(secs(30.0)), None);
        let reverse = MediaTimeMapping::new(secs(0.0), 2.0)
            .with_rate(-1.0)
            .with_out_point(Some(0.0));
        assert_eq!(reverse.next_motion(secs(1.0)), Some(secs(1.0)));
        assert_eq!(reverse.next_motion(secs(2.0)), None);
    }

    #[test]
    fn alignment_is_within_half_the_frame_interval() {
        let request = MediaSampleRequest {
            node: StableNodeId::new(1).unwrap(),
            slot: "video:1".into(),
            media_time: 1.0,
            frame_interval: Duration::from_millis(40),
            pacing: MediaPacing::Stepped,
        };
        assert_eq!(request.tolerance(), 0.02);
        assert!(request.aligned(1.0));
        assert!(request.aligned(1.019));
        assert!(request.aligned(0.981));
        assert!(!request.aligned(1.021));
        assert!(!request.aligned(0.96));
        assert!(!request.aligned(f64::NAN));
    }
}
