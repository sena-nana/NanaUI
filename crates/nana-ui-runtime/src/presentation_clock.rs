//! The document's presentation time (Issue #300).
//!
//! A document has one presentation time, `animation_now`. Every track,
//! compositor overlay, reveal and chart transition samples it. The host does
//! not own a second clock: its monotonic time only *drives* the document's
//! while the document plays. Setting the time (a seek), pausing and stepping
//! all write this one value, and input, network and debounce timers keep
//! running on the host's clock.
//!
//! The mapping is affine: while playing at `rate`, the document time is
//! `anchor_document + (host - anchor_host) * rate`; while paused it is
//! `anchor_document`. The default (playing at rate 1 from anchors 0/0) is the
//! identity, so a document nobody pauses advances exactly as it did when the
//! host's time was written straight into `animation_now`.

use std::time::Duration;

/// How a document's presentation time moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PresentationPlayback {
    /// The host clock drives the time at `rate` (1.0 is real time).
    Playing { rate: f64 },
    /// The time holds; host frames and input do not move it.
    Paused,
}

impl PresentationPlayback {
    /// Real-time playback, the default.
    pub const REAL_TIME: Self = Self::Playing { rate: 1.0 };

    /// A rate that is not finite or not positive cannot drive time forward:
    /// it pauses instead.
    fn sanitized(self) -> Self {
        match self {
            Self::Playing { rate } if rate.is_finite() && rate > 0.0 => self,
            Self::Playing { .. } | Self::Paused => Self::Paused,
        }
    }

    /// Whether host time moves the document's time.
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Playing { .. })
    }
}

impl Default for PresentationPlayback {
    fn default() -> Self {
        Self::REAL_TIME
    }
}

/// The interval a frame stands for before the host reports its display's.
pub(crate) const DEFAULT_PRESENTATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Maps host time onto one document's presentation time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PresentationClock {
    playback: PresentationPlayback,
    anchor_host: Duration,
    anchor_document: Duration,
    /// Set when the document time was re-anchored (play, seek) without a host
    /// time to pair it with: the next host time observed becomes the anchor,
    /// so time idle before it is not counted as played.
    anchor_pending: bool,
    /// The host time that last moved (or would have moved) the document.
    host_now: Duration,
    /// Display interval while playing, or the step a caller advanced by.
    frame_interval: Duration,
    display_interval: Duration,
}

impl Default for PresentationClock {
    fn default() -> Self {
        Self {
            playback: PresentationPlayback::REAL_TIME,
            anchor_host: Duration::ZERO,
            anchor_document: Duration::ZERO,
            anchor_pending: false,
            host_now: Duration::ZERO,
            frame_interval: DEFAULT_PRESENTATION_FRAME_INTERVAL,
            display_interval: DEFAULT_PRESENTATION_FRAME_INTERVAL,
        }
    }
}

impl PresentationClock {
    pub(crate) fn playback(&self) -> PresentationPlayback {
        self.playback
    }

    pub(crate) fn host_now(&self) -> Duration {
        self.host_now
    }

    pub(crate) fn frame_interval(&self) -> Duration {
        self.frame_interval
    }

    /// The document time `host` maps to, recording `host` as the latest host
    /// time. The first host time after a re-anchor pairs with it.
    pub(crate) fn observe(&mut self, host: Duration) -> Duration {
        self.host_now = host;
        if self.anchor_pending {
            self.anchor_pending = false;
            self.anchor_host = host;
        }
        self.document_time(host)
    }

    fn document_time(&self, host: Duration) -> Duration {
        match self.playback {
            PresentationPlayback::Paused => self.anchor_document,
            PresentationPlayback::Playing { rate } => {
                let elapsed = host.saturating_sub(self.anchor_host);
                let scaled = if rate == 1.0 {
                    elapsed
                } else {
                    Duration::try_from_secs_f64(elapsed.as_secs_f64() * rate)
                        .unwrap_or(Duration::MAX)
                };
                self.anchor_document.saturating_add(scaled)
            }
        }
    }

    /// Hold or drive the time from `document_now`, the time currently
    /// presented. Driving pairs it with the next host time observed.
    pub(crate) fn set_playback(&mut self, playback: PresentationPlayback, document_now: Duration) {
        let playback = playback.sanitized();
        self.anchor_document = document_now;
        self.anchor_pending = playback.is_playing();
        self.playback = playback;
        if playback.is_playing() {
            self.frame_interval = self.display_interval;
        }
    }

    /// Set the time. A playing document continues from it at the next host
    /// time observed; a paused one holds it.
    pub(crate) fn seek(&mut self, document_now: Duration) {
        self.anchor_document = document_now;
        self.anchor_pending = self.playback.is_playing();
    }

    /// A caller advanced the time by `interval` itself: that is the frame
    /// interval until the document plays again.
    pub(crate) fn note_step(&mut self, interval: Duration) {
        self.frame_interval = interval;
    }

    pub(crate) fn set_display_interval(&mut self, interval: Duration) {
        if interval.is_zero() {
            return;
        }
        self.display_interval = interval;
        if self.playback.is_playing() {
            self.frame_interval = interval;
        }
    }

    /// The host time a document-time deadline falls due at. A paused
    /// document never reaches a future deadline; one already reached is due
    /// at once.
    pub(crate) fn host_deadline(
        &self,
        deadline: Duration,
        document_now: Duration,
    ) -> Option<Duration> {
        if deadline <= document_now {
            // Already reached (or reported as now, for pending events).
            return Some(self.host_now.min(self.host_time_unchecked(deadline)));
        }
        match self.playback {
            PresentationPlayback::Paused => None,
            PresentationPlayback::Playing { .. } => Some(self.host_time_unchecked(deadline)),
        }
    }

    /// Inverse of [`Self::document_time`] while playing; `host_now` when
    /// paused or before the anchor.
    fn host_time_unchecked(&self, deadline: Duration) -> Duration {
        let PresentationPlayback::Playing { rate } = self.playback else {
            return self.host_now;
        };
        let base = if self.anchor_pending {
            self.host_now
        } else {
            self.anchor_host
        };
        let Some(ahead) = deadline.checked_sub(self.anchor_document) else {
            return self.host_now.min(base);
        };
        let host_ahead = if rate == 1.0 {
            ahead
        } else {
            Duration::try_from_secs_f64(ahead.as_secs_f64() / rate).unwrap_or(Duration::MAX)
        };
        base.saturating_add(host_ahead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_default_clock_is_the_identity() {
        let mut clock = PresentationClock::default();
        for host in [0, 5, 16, 1_000, 999_999] {
            assert_eq!(clock.observe(ms(host)), ms(host));
            assert_eq!(
                clock.host_deadline(ms(host + 7), ms(host)),
                Some(ms(host + 7))
            );
        }
        // An older host time (an input stamp) still maps to itself.
        assert_eq!(clock.observe(ms(3)), ms(3));
    }

    #[test]
    fn a_paused_clock_holds_and_reports_no_future_deadline() {
        let mut clock = PresentationClock::default();
        let now = clock.observe(ms(100));
        clock.set_playback(PresentationPlayback::Paused, now);
        assert_eq!(clock.observe(ms(500)), ms(100));
        assert_eq!(clock.host_deadline(ms(116), ms(100)), None);
        // A deadline already reached (pending events) is due at once.
        assert_eq!(clock.host_deadline(ms(100), ms(100)), Some(ms(500)));
    }

    #[test]
    fn playing_again_does_not_count_the_idle_gap() {
        let mut clock = PresentationClock::default();
        let now = clock.observe(ms(100));
        clock.set_playback(PresentationPlayback::Paused, now);
        clock.observe(ms(400));
        clock.set_playback(PresentationPlayback::REAL_TIME, ms(100));
        // Idle until the host's next frame at 2 s: that frame pairs with 100.
        assert_eq!(clock.observe(ms(2_000)), ms(100));
        assert_eq!(clock.observe(ms(2_016)), ms(116));
        assert_eq!(clock.host_deadline(ms(150), ms(116)), Some(ms(2_050)));
    }

    #[test]
    fn a_rate_scales_elapsed_host_time_both_ways() {
        let mut clock = PresentationClock::default();
        clock.observe(ms(1_000));
        clock.set_playback(PresentationPlayback::Playing { rate: 0.5 }, ms(1_000));
        assert_eq!(clock.observe(ms(1_000)), ms(1_000));
        assert_eq!(clock.observe(ms(1_100)), ms(1_050));
        assert_eq!(clock.host_deadline(ms(1_060), ms(1_050)), Some(ms(1_120)));
    }

    #[test]
    fn an_unusable_rate_pauses() {
        let mut clock = PresentationClock::default();
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            clock.set_playback(PresentationPlayback::Playing { rate }, ms(10));
            assert_eq!(clock.playback(), PresentationPlayback::Paused);
        }
    }

    #[test]
    fn a_seek_while_playing_continues_from_the_new_time() {
        let mut clock = PresentationClock::default();
        clock.observe(ms(500));
        clock.seek(ms(40));
        assert_eq!(clock.observe(ms(520)), ms(40));
        assert_eq!(clock.observe(ms(536)), ms(56));
    }

    #[test]
    fn the_frame_interval_is_the_display_interval_or_the_step() {
        let mut clock = PresentationClock::default();
        clock.set_display_interval(ms(8));
        assert_eq!(clock.frame_interval(), ms(8));
        clock.set_playback(PresentationPlayback::Paused, ms(0));
        clock.note_step(ms(40));
        assert_eq!(clock.frame_interval(), ms(40));
        clock.set_display_interval(ms(7));
        assert_eq!(clock.frame_interval(), ms(40));
        clock.set_playback(PresentationPlayback::REAL_TIME, ms(40));
        assert_eq!(clock.frame_interval(), ms(7));
    }
}
