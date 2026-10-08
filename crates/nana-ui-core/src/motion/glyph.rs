//! Per-glyph presentation of rich text: looping effects and a typewriter
//! reveal, sampled on the motion clock.
//!
//! These are *presentation*. Nothing here reaches shaping, layout or the
//! glyph instances a paragraph was built into: the text vertex shader
//! evaluates them per glyph from the motion clock, and this CPU evaluator —
//! the same arithmetic, written once more — answers for what the shader does
//! not draw (inline objects) and for tests. Shake and flicker are driven by an
//! integer hash, so a frame at a given time always looks the same.
//!
//! Times are in seconds. An effect's phase runs on the motion clock modulo an
//! hour (so a long session keeps f32 precision; the loop restarts on the
//! hour); a reveal's on whole seconds since its start, subtracted as
//! integers.

use std::sync::Arc;
use std::time::Duration;

/// What a looping effect does to each glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphEffectKind {
    /// Jitter by up to `amplitude` px each way, a new position `frequency`
    /// times a second.
    Shake,
    /// Bob up and down by `amplitude` px.
    Wave,
    /// Hop up by `amplitude` px and land.
    Jump,
    /// Cycle the hue, `frequency` turns a second.
    Rainbow,
    /// Breathe in scale by `amplitude` (0.2 is ±20%).
    Pulse,
    /// Dip to dim at random, `frequency` times a second.
    Flicker,
}

/// A looping effect a span plays (the span's `effect` index names one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphEffect {
    pub kind: GlyphEffectKind,
    pub amplitude: f32,
    pub frequency_hz: f32,
    /// How far behind its predecessor each glyph runs, in cycles: what makes
    /// a wave travel along the text.
    pub stagger: f32,
}

impl GlyphEffect {
    pub fn new(kind: GlyphEffectKind, amplitude: f32, frequency_hz: f32) -> Self {
        Self {
            kind,
            amplitude,
            frequency_hz,
            stagger: 0.12,
        }
    }

    pub fn shake(amplitude_px: f32) -> Self {
        Self::new(GlyphEffectKind::Shake, amplitude_px, 20.0)
    }

    pub fn wave(amplitude_px: f32) -> Self {
        Self::new(GlyphEffectKind::Wave, amplitude_px, 1.2)
    }

    pub fn jump(amplitude_px: f32) -> Self {
        Self::new(GlyphEffectKind::Jump, amplitude_px, 1.5)
    }

    pub fn rainbow() -> Self {
        Self::new(GlyphEffectKind::Rainbow, 1.0, 0.5)
    }

    pub fn pulse(amount: f32) -> Self {
        Self::new(GlyphEffectKind::Pulse, amount, 1.0)
    }

    pub fn flicker() -> Self {
        Self::new(GlyphEffectKind::Flicker, 1.0, 8.0)
    }

    pub fn stagger(mut self, cycles: f32) -> Self {
        self.stagger = cycles;
        self
    }

    pub fn frequency(mut self, hz: f32) -> Self {
        self.frequency_hz = hz;
        self
    }
}

/// How a revealed glyph's entrance eases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GlyphEasing {
    Linear,
    #[default]
    EaseOut,
    /// Overshoots a little before settling: a pop.
    BackOut,
}

impl GlyphEasing {
    pub fn sample(self, progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t),
            Self::BackOut => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                let u = t - 1.0;
                1.0 + c3 * u * u * u + c1 * u * u
            }
        }
    }
}

/// How a glyph enters when the reveal reaches it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphIntro {
    pub fade: bool,
    /// Grow from half size.
    pub pop: bool,
    /// Rise from this many px below.
    pub rise_px: f32,
    pub duration_s: f32,
    pub easing: GlyphEasing,
}

impl Default for GlyphIntro {
    /// Appear at once.
    fn default() -> Self {
        Self {
            fade: false,
            pop: false,
            rise_px: 0.0,
            duration_s: 0.0,
            easing: GlyphEasing::EaseOut,
        }
    }
}

impl GlyphIntro {
    pub fn fade(duration_s: f32) -> Self {
        Self {
            fade: true,
            duration_s,
            ..Self::default()
        }
    }

    pub fn pop(duration_s: f32) -> Self {
        Self {
            fade: true,
            pop: true,
            duration_s,
            easing: GlyphEasing::BackOut,
            ..Self::default()
        }
    }

    pub fn rise(rise_px: f32, duration_s: f32) -> Self {
        Self {
            fade: true,
            rise_px,
            duration_s,
            ..Self::default()
        }
    }
}

/// A typewriter reveal: when each grapheme of the text appears.
///
/// `at_s[i]` is when grapheme `i` (of the text the node shows, counted in
/// extended grapheme clusters) starts its intro, in seconds after `start`. A
/// grapheme past the end of `at_s`, or at or past `limit`, is not shown yet.
/// `start` is on the document's animation clock
/// ([`AppContext::animation_now`](../../nana_ui_runtime/struct.AppContext.html#method.animation_now)).
#[derive(Debug, Clone, PartialEq)]
pub struct RevealSchedule {
    pub start: Duration,
    pub at_s: Arc<[f32]>,
    pub limit: Option<u32>,
    pub intro: GlyphIntro,
}

impl RevealSchedule {
    pub fn new(start: Duration, at_s: impl Into<Arc<[f32]>>) -> Self {
        Self {
            start,
            at_s: at_s.into(),
            limit: None,
            intro: GlyphIntro::default(),
        }
    }

    /// One grapheme every `interval_s`, the first at once.
    pub fn uniform(start: Duration, graphemes: usize, interval_s: f32) -> Self {
        Self::new(
            start,
            (0..graphemes)
                .map(|index| index as f32 * interval_s)
                .collect::<Vec<_>>(),
        )
    }

    pub fn intro(mut self, intro: GlyphIntro) -> Self {
        self.intro = intro;
        self
    }

    /// Show at most `limit` graphemes (a reveal paused at a marker).
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// When the last scheduled grapheme has finished entering.
    pub fn end(&self) -> Duration {
        let last = self.at_s.iter().copied().fold(0.0f32, f32::max);
        self.start + Duration::from_secs_f32((last + self.intro.duration_s.max(0.0)).max(0.0))
    }
}

/// What a rich text node presents per glyph: the effect table its spans'
/// `effect` indices name, and the reveal it plays. Set with
/// `AppContext::set_rich_presentation`; changing it costs no text work.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphPresentation {
    pub effects: Arc<[GlyphEffect]>,
    pub reveal: Option<RevealSchedule>,
}

impl GlyphPresentation {
    pub fn new(effects: impl Into<Arc<[GlyphEffect]>>, reveal: Option<RevealSchedule>) -> Self {
        Self {
            effects: effects.into(),
            reveal,
        }
    }

    /// Until when it changes what is drawn: `None` for a looping effect,
    /// which never stops.
    pub fn live_until(&self, uses_effects: bool) -> Option<Option<Duration>> {
        if uses_effects && !self.effects.is_empty() {
            return Some(None);
        }
        self.reveal.as_ref().map(|reveal| Some(reveal.end()))
    }
}

/// What a glyph looks like at one instant, relative to where it was laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSample {
    /// Logical px, positive right and down.
    pub offset: [f32; 2],
    /// About the glyph's own centre.
    pub scale: f32,
    pub alpha: f32,
    /// A colour replacing the glyph's fill (rainbow), sRGB.
    pub color: Option<[f32; 3]>,
}

impl GlyphSample {
    pub const IDENTITY: Self = Self {
        offset: [0.0; 2],
        scale: 1.0,
        alpha: 1.0,
        color: None,
    };
}

/// The integer hash both evaluators share (lowbias32).
pub fn glyph_hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn unit(hash: u32) -> f32 {
    (hash >> 8) as f32 / 16_777_216.0
}

/// Split a clock reading into whole seconds and the fraction, as the GPU
/// receives it.
pub fn split_seconds(time: Duration) -> (u32, f32) {
    (time.as_secs() as u32, time.subsec_nanos() as f32 * 1e-9)
}

/// The effect's phase clock: the motion clock modulo an hour.
pub fn effect_clock(now: Duration) -> f32 {
    let (secs, fraction) = split_seconds(now);
    (secs % 3600) as f32 + fraction
}

/// `hsv(h, 0.8, 1)` as sRGB: the rainbow's colour at hue `h` (turns).
pub fn rainbow_color(hue: f32) -> [f32; 3] {
    let h = (hue - hue.floor()) * 6.0;
    let (s, v) = (0.8f32, 1.0f32);
    let channel = |n: f32| {
        let k = (n + h) % 6.0;
        v - v * s * (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

/// Glyph `ordinal`'s look at `now` under `effect` and `reveal`.
pub fn evaluate_glyph(
    effect: Option<&GlyphEffect>,
    reveal: Option<&RevealSchedule>,
    ordinal: u32,
    now: Duration,
) -> GlyphSample {
    let mut sample = GlyphSample::IDENTITY;
    if let Some(effect) = effect {
        apply_effect(&mut sample, effect, ordinal, effect_clock(now));
    }
    if let Some(reveal) = reveal {
        let (now_secs, now_fraction) = split_seconds(now);
        let (start_secs, start_fraction) = split_seconds(reveal.start);
        let since =
            now_secs.wrapping_sub(start_secs) as i32 as f32 + (now_fraction - start_fraction);
        apply_reveal(&mut sample, reveal, ordinal, since);
    }
    sample
}

fn apply_effect(sample: &mut GlyphSample, effect: &GlyphEffect, ordinal: u32, clock: f32) {
    let phase = clock * effect.frequency_hz - ordinal as f32 * effect.stagger;
    let tau = std::f32::consts::TAU;
    match effect.kind {
        GlyphEffectKind::Shake => {
            let step = (clock * effect.frequency_hz).floor() as i32 as u32;
            let seed = glyph_hash(ordinal.wrapping_mul(0x9e37_79b9) ^ step);
            sample.offset[0] += (unit(seed) * 2.0 - 1.0) * effect.amplitude;
            sample.offset[1] += (unit(glyph_hash(seed)) * 2.0 - 1.0) * effect.amplitude;
        }
        GlyphEffectKind::Wave => {
            sample.offset[1] -= effect.amplitude * (tau * phase).sin();
        }
        GlyphEffectKind::Jump => {
            sample.offset[1] -= effect.amplitude * (std::f32::consts::PI * phase).sin().abs();
        }
        GlyphEffectKind::Rainbow => {
            sample.color = Some(rainbow_color(phase));
        }
        GlyphEffectKind::Pulse => {
            sample.scale *= 1.0 + effect.amplitude * (0.5 + 0.5 * (tau * phase).sin());
        }
        GlyphEffectKind::Flicker => {
            let step = (clock * effect.frequency_hz).floor() as i32 as u32;
            if unit(glyph_hash(ordinal.wrapping_mul(0x85eb_ca6b) ^ step)) < 0.2 {
                sample.alpha *= 0.35;
            }
        }
    }
}

fn apply_reveal(sample: &mut GlyphSample, reveal: &RevealSchedule, ordinal: u32, since: f32) {
    let scheduled = reveal.at_s.get(ordinal as usize).copied();
    let limited = reveal.limit.is_some_and(|limit| ordinal >= limit);
    let Some(at) = scheduled.filter(|_| !limited) else {
        sample.alpha = 0.0;
        return;
    };
    let local = since - at;
    if local < 0.0 {
        sample.alpha = 0.0;
        return;
    }
    let intro = &reveal.intro;
    if intro.duration_s <= 0.0 {
        return;
    }
    let linear = (local / intro.duration_s).clamp(0.0, 1.0);
    let eased = intro.easing.sample(linear);
    if intro.fade {
        sample.alpha *= linear;
    }
    if intro.pop {
        sample.scale *= 0.5 + 0.5 * eased;
    }
    sample.offset[1] += intro.rise_px * (1.0 - eased);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reveal_hides_what_it_has_not_reached_and_enters_the_rest() {
        let reveal = RevealSchedule::uniform(Duration::from_secs(10), 3, 0.5)
            .intro(GlyphIntro::rise(8.0, 0.2));
        let at = |seconds: f32| Duration::from_secs(10) + Duration::from_secs_f32(seconds);
        assert_eq!(evaluate_glyph(None, Some(&reveal), 1, at(0.4)).alpha, 0.0);
        let entering = evaluate_glyph(None, Some(&reveal), 1, at(0.6));
        assert!(entering.alpha > 0.0 && entering.alpha < 1.0);
        assert!(entering.offset[1] > 0.0, "still rising: {entering:?}");
        let settled = evaluate_glyph(None, Some(&reveal), 1, at(2.0));
        assert_eq!(settled, GlyphSample::IDENTITY);
        assert_eq!(
            evaluate_glyph(None, Some(&reveal), 3, at(9.0)).alpha,
            0.0,
            "not scheduled yet"
        );
        assert_eq!(
            evaluate_glyph(None, Some(&reveal.clone().limit(1)), 1, at(9.0)).alpha,
            0.0,
            "held back by the limit"
        );
        assert_eq!(reveal.end(), at(1.2));
    }

    #[test]
    fn effects_are_deterministic_functions_of_the_clock() {
        let shake = GlyphEffect::shake(3.0);
        let now = Duration::from_millis(12_345);
        let first = evaluate_glyph(Some(&shake), None, 4, now);
        assert_eq!(first, evaluate_glyph(Some(&shake), None, 4, now));
        assert!(first.offset[0].abs() <= 3.0 && first.offset[1].abs() <= 3.0);
        assert_ne!(
            first,
            evaluate_glyph(Some(&shake), None, 5, now),
            "per glyph"
        );
        let wave = GlyphEffect::wave(4.0);
        let crest = evaluate_glyph(Some(&wave), None, 0, Duration::from_secs_f32(0.25 / 1.2));
        assert!((crest.offset[1] + 4.0).abs() < 1e-3, "{crest:?}");
        let rainbow = evaluate_glyph(Some(&GlyphEffect::rainbow()), None, 0, Duration::ZERO);
        assert!(rainbow.color.is_some());
        assert!((GlyphEasing::BackOut.sample(1.0) - 1.0).abs() < 1e-5);
    }
}
