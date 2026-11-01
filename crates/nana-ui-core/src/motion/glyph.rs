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
//!
//! "Up" is line-relative: the line-over side of the text's line, which is the
//! page's top in `horizontal-tb` and its right in both vertical modes (CSS
//! Writing Modes §6.4; a sideways run of a vertical line is turned clockwise,
//! so its glyphs' tops face right too). Wave, jump and a rise move across the
//! line, never along it, so a vertical column keeps its spacing. Shake,
//! rainbow, pulse and flicker have no direction.

use std::sync::Arc;
use std::time::Duration;

use crate::box_layout::WritingModeSpec;

/// What a looping effect does to each glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphEffectKind {
    /// Jitter by up to `amplitude` px each way, a new position `frequency`
    /// times a second.
    Shake,
    /// Bob up and down by `amplitude` px: across the line, so right and left
    /// on a vertical one.
    Wave,
    /// Hop up by `amplitude` px and land: toward the line-over side, the
    /// right of a vertical line.
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

    /// Whether this descriptor has a visible effect, including a frozen pose.
    pub fn is_effective(&self) -> bool {
        self.amplitude.is_finite()
            && self.amplitude != 0.0
            && (self.kind != GlyphEffectKind::Flicker || self.amplitude > 0.0)
            && self.frequency_hz.is_finite()
            && self.stagger.is_finite()
    }

    /// Frozen effects still paint, but never keep a window awake.
    pub fn is_animated(&self) -> bool {
        self.is_effective() && self.frequency_hz != 0.0
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
    /// Rise from this many px below: from the line-under side, the left of a
    /// vertical line.
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
        let count = self.limit.map_or(self.at_s.len(), |limit| limit as usize);
        let last = self
            .at_s
            .iter()
            .take(count)
            .copied()
            .filter(|at| at.is_finite())
            .reduce(f32::max);
        self.start
            + Duration::from_secs_f32(
                last.map_or(0.0, |last| (last + self.intro.duration_s.max(0.0)).max(0.0)),
            )
    }
}

/// Fragment masks multiply the normal glyph/object paint, never recolor it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextTransitions {
    pub entrance: Option<crate::PresentationMask>,
    pub exit: Option<crate::PresentationMask>,
    /// Freeze reveal at retirement while the independent exit clock runs.
    pub freeze_reveal: Option<Duration>,
    /// Text-local visible viewport, including its current scroll offset.
    pub viewport: Option<crate::LogicalRect>,
}
impl TextTransitions {
    pub fn reveal_end(&self, reveal: &RevealSchedule) -> Duration {
        let mut end = reveal.end();
        if let Some(mask) = &self.entrance {
            let count = reveal.limit.map_or(reveal.at_s.len(), |n| n as usize);
            let at = if mask.space == crate::PresentationMaskSpace::Target {
                reveal.at_s.first().copied()
            } else {
                reveal
                    .at_s
                    .iter()
                    .take(count)
                    .copied()
                    .filter(|t| t.is_finite())
                    .reduce(f32::max)
            };
            if let Some(at) = at.filter(|at| at.is_finite()) {
                end = end.max(reveal.start.max(mask.start).saturating_add(
                    Duration::from_secs_f64((f64::from(at) + mask.duration.as_secs_f64()).max(0.)),
                ));
            }
        }
        end
    }
    pub fn end(&self) -> Duration {
        self.exit
            .as_ref()
            .map_or(Duration::ZERO, |e| e.start.saturating_add(e.duration))
    }
}

/// What a rich text node presents per glyph: the effect table its spans'
/// `effect` indices name, and the reveal it plays. Set with
/// `AppContext::set_rich_presentation`; changing it costs no text work.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphPresentation {
    pub transitions: TextTransitions,
    pub effects: Arc<[GlyphEffect]>,
    /// Optional effect chains indexed by RichSpanStyle::effect. Empty keeps
    /// the legacy single-effect table. Empty chains are explicitly static.
    pub effect_chains: Arc<[GlyphEffectChain]>,
    pub reveal: Option<RevealSchedule>,
}

impl GlyphPresentation {
    pub fn new(effects: impl Into<Arc<[GlyphEffect]>>, reveal: Option<RevealSchedule>) -> Self {
        Self {
            transitions: TextTransitions::default(),
            effects: effects.into(),
            effect_chains: Arc::from([]),
            reveal,
        }
    }

    pub fn chains(
        chains: impl Into<Arc<[GlyphEffectChain]>>,
        reveal: Option<RevealSchedule>,
    ) -> Self {
        Self {
            transitions: TextTransitions::default(),
            effects: Arc::from([]),
            effect_chains: chains.into(),
            reveal,
        }
    }

    pub fn effect_chain(&self, index: usize) -> &[GlyphEffect] {
        if self.effect_chains.is_empty() {
            self.effects
                .get(index)
                .map(std::slice::from_ref)
                .unwrap_or(&[])
        } else {
            self.effect_chains
                .get(index)
                .map(|chain| chain.0.as_ref())
                .unwrap_or(&[])
        }
    }

    /// Until when it changes what is drawn: `None` for a looping effect,
    /// which never stops.
    pub fn live_until(&self, uses_effects: bool) -> Option<Option<Duration>> {
        if self.transitions.exit.is_some() {
            return Some(Some(self.transitions.end()));
        }
        if uses_effects
            && (self.effects.iter().any(GlyphEffect::is_animated)
                || self.effect_chains.iter().any(GlyphEffectChain::is_animated))
        {
            return Some(None);
        }
        self.reveal
            .as_ref()
            .map(|reveal| Some(self.transitions.reveal_end(reveal)))
    }
}

/// An ordered combination of looping effects. Offsets add, scale and alpha
/// multiply, and the last colour-producing effect supplies the colour.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphEffectChain(pub Arc<[GlyphEffect]>);

impl GlyphEffectChain {
    pub fn new(effects: impl Into<Arc<[GlyphEffect]>>) -> Self {
        Self(effects.into())
    }
    pub fn is_animated(&self) -> bool {
        self.0.iter().any(GlyphEffect::is_animated)
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

/// Glyph `ordinal`'s look at `now` under `effect` and `reveal`, on a
/// horizontal line.
pub fn evaluate_glyph(
    effect: Option<&GlyphEffect>,
    reveal: Option<&RevealSchedule>,
    ordinal: u32,
    now: Duration,
) -> GlyphSample {
    evaluate_glyph_chain(
        effect.map(std::slice::from_ref).unwrap_or(&[]),
        reveal,
        ordinal,
        now,
    )
}

/// CPU counterpart of the text shader, on a horizontal line. Text in another
/// writing mode goes through [`evaluate_glyph_chain_in`].
pub fn evaluate_glyph_chain(
    effects: &[GlyphEffect],
    reveal: Option<&RevealSchedule>,
    ordinal: u32,
    now: Duration,
) -> GlyphSample {
    evaluate_glyph_chain_in(effects, reveal, ordinal, now, WritingModeSpec::HorizontalTb)
}

/// CPU counterpart of the text shader for text set in `writing_mode`, also
/// used for inline objects: a vertical line takes the wave, the jump and the
/// rise across itself, `(0, dy)` on the page becoming `(-dy, 0)`.
pub fn evaluate_glyph_chain_in(
    effects: &[GlyphEffect],
    reveal: Option<&RevealSchedule>,
    ordinal: u32,
    now: Duration,
    writing_mode: WritingModeSpec,
) -> GlyphSample {
    let vertical = writing_mode.is_vertical();
    let mut sample = GlyphSample::IDENTITY;
    for effect in effects.iter().filter(|effect| effect.is_effective()) {
        apply_effect(&mut sample, effect, ordinal, effect_clock(now), vertical);
    }
    if let Some(reveal) = reveal {
        let (now_secs, now_fraction) = split_seconds(now);
        let (start_secs, start_fraction) = split_seconds(reveal.start);
        let since =
            now_secs.wrapping_sub(start_secs) as i32 as f32 + (now_fraction - start_fraction);
        apply_reveal(&mut sample, reveal, ordinal, since, vertical);
    }
    sample
}

/// Move `sample` `px` toward the line-over side: up on the page, or right on
/// a vertical line. `glyph_over` in `text_atlas.wgsl` is the same step.
fn lift(sample: &mut GlyphSample, vertical: bool, px: f32) {
    if vertical {
        sample.offset[0] += px;
    } else {
        sample.offset[1] -= px;
    }
}

fn apply_effect(
    sample: &mut GlyphSample,
    effect: &GlyphEffect,
    ordinal: u32,
    clock: f32,
    vertical: bool,
) {
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
            lift(sample, vertical, effect.amplitude * (tau * phase).sin());
        }
        GlyphEffectKind::Jump => {
            lift(
                sample,
                vertical,
                effect.amplitude * (std::f32::consts::PI * phase).sin().abs(),
            );
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
                sample.alpha *= 1.0 - 0.65 * effect.amplitude.clamp(0.0, 1.0);
            }
        }
    }
}

fn apply_reveal(
    sample: &mut GlyphSample,
    reveal: &RevealSchedule,
    ordinal: u32,
    since: f32,
    vertical: bool,
) {
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
    lift(sample, vertical, -(intro.rise_px * (1.0 - eased)));
}

#[cfg(test)]
mod tests {
    #[test]
    fn effect_chains_compose_and_empty_effects_do_not_loop() {
        use super::*;
        let fx = [
            GlyphEffect::wave(3.0),
            GlyphEffect::rainbow(),
            GlyphEffect::pulse(0.2),
        ];
        let now = Duration::from_millis(210);
        let combined = evaluate_glyph_chain(&fx, None, 0, now);
        assert_eq!(
            combined.offset,
            evaluate_glyph(Some(&fx[0]), None, 0, now).offset
        );
        assert_eq!(
            combined.color,
            evaluate_glyph(Some(&fx[1]), None, 0, now).color
        );
        assert_eq!(
            combined.scale,
            evaluate_glyph(Some(&fx[2]), None, 0, now).scale
        );
        let reveal = RevealSchedule::uniform(Duration::ZERO, 2, 0.1).intro(GlyphIntro::fade(0.2));
        let expected = reveal.end();
        let static_fx = GlyphPresentation::new(vec![GlyphEffect::shake(0.0)], Some(reveal));
        assert_eq!(static_fx.live_until(true), Some(Some(expected)));
        assert_eq!(
            evaluate_glyph_chain(&[GlyphEffect::shake(0.0)], None, 0, now),
            GlyphSample::IDENTITY
        );
        assert!(!GlyphEffect::wave(1.0).frequency(0.0).is_animated());
        assert!(!GlyphEffect::wave(f32::NAN).is_animated());
        assert!(!GlyphEffect::new(GlyphEffectKind::Flicker, -1.0, 8.0).is_effective());
        let frozen = GlyphEffect::wave(3.0).frequency(0.0);
        assert!(frozen.is_effective());
        assert_eq!(
            evaluate_glyph(Some(&frozen), None, 2, Duration::ZERO),
            evaluate_glyph(Some(&frozen), None, 2, Duration::from_secs(10))
        );
        assert_ne!(
            evaluate_glyph(Some(&frozen), None, 2, Duration::ZERO),
            GlyphSample::IDENTITY
        );
    }

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
    fn paused_reveal_deadline_excludes_the_hidden_suffix() {
        let reveal = RevealSchedule::new(Duration::from_secs(2), vec![0.1, 1.0e6])
            .intro(GlyphIntro::fade(0.2));
        assert_eq!(
            reveal.clone().limit(1).end(),
            Duration::from_secs(2) + Duration::from_secs_f32(0.3)
        );
        assert_eq!(reveal.limit(0).end(), Duration::from_secs(2));
        let shown =
            RevealSchedule::new(Duration::from_secs(2), vec![-60.0]).intro(GlyphIntro::fade(0.2));
        assert_eq!(shown.end(), Duration::from_secs(2));
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

    #[test]
    fn a_vertical_line_takes_wave_jump_and_rise_across_itself() {
        let crest = Duration::from_secs_f32(0.25 / 1.2);
        let wave = [GlyphEffect::wave(4.0)];
        let jump = [GlyphEffect::jump(5.0)];
        let shake = [GlyphEffect::shake(3.0)];
        let reveal = RevealSchedule::new(Duration::ZERO, vec![0.0; 4]).intro(GlyphIntro {
            pop: true,
            ..GlyphIntro::rise(8.0, 1.0)
        });
        let half = Duration::from_millis(500);
        let jitter = Duration::from_millis(12_345);
        let horizontal = |effects: &[GlyphEffect], reveal, now| {
            evaluate_glyph_chain_in(effects, reveal, 3, now, WritingModeSpec::HorizontalTb)
        };
        for mode in [WritingModeSpec::VerticalRl, WritingModeSpec::VerticalLr] {
            let vertical = |effects: &[GlyphEffect], reveal, now| {
                evaluate_glyph_chain_in(effects, reveal, 3, now, mode)
            };
            for (effects, reveal, now) in [
                (&wave[..], None, crest),
                (&jump[..], None, crest),
                (&[][..], Some(&reveal), half),
                (&[wave[0], jump[0]][..], Some(&reveal), half),
            ] {
                let across = horizontal(effects, reveal, now);
                let down = vertical(effects, reveal, now);
                // `(0, dy)` on a horizontal line is `(-dy, 0)` down a column:
                // line-over is the right in both vertical modes.
                assert_eq!(across.offset[0], 0.0, "{mode:?}");
                assert_eq!(down.offset, [-across.offset[1], 0.0], "{mode:?}");
                assert_eq!(
                    (down.scale, down.alpha, down.color),
                    (across.scale, across.alpha, across.color),
                    "{mode:?}: only the direction turns"
                );
            }
            // The first glyph's crest leans toward the line-over side.
            let peak = evaluate_glyph_chain_in(&wave, None, 0, crest, mode);
            assert!((peak.offset[0] - 4.0).abs() < 1e-3, "{mode:?}: {peak:?}");
            let entering = vertical(&[], Some(&reveal), half);
            assert!(entering.offset[0] < 0.0, "{mode:?}: enters from the left");
            assert_eq!(
                vertical(&shake, None, jitter),
                horizontal(&shake, None, jitter),
                "{mode:?}: a shake has no direction to turn"
            );
        }
        assert_eq!(
            horizontal(&wave, Some(&reveal), half),
            evaluate_glyph_chain(&wave, Some(&reveal), 3, half)
        );
    }
}
