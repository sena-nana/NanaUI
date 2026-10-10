//! Issue #211: CJK punctuation closing up, kinsoku, strict and loose
//! breaking, autospace, justification and pretty paragraphs, decided in one
//! place: each line weighs keeping a stop by closing punctuation up against
//! breaking earlier and leaving slack.
//!
//! Widths are read from the laid-out text itself (the fixture face's
//! fullwidth advance), never hard-coded, and assertions are relations, so a
//! different fixture build does not break them.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    GlyphFlags, InlineEnvelope, InlineObject, InlineObjectMetrics, NativeTextEngine,
    TextConstraints, TextEngine, TextKind, TextLayout, TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::{
    LineBreakSpec, TextAlignSpec, TextAutospaceSpec, TextJustifySpec, TextSpacingTrimSpec,
    TextWrapBreak, TextWrapStyleSpec, WordBreakSpec,
};
use std::sync::Arc;

use support::corpus::{fixture_bytes, fixture_family};

const SIZE: f32 = 16.0;

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .expect("fixture registers");
    NativeTextEngine::new(fonts)
}

fn style() -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from(format!("\"{}\"", fixture_family("noto-sans-sc")))),
        font_size_px: SIZE,
        ..TextStyle::default()
    }
}

fn wrapped(max_width_px: f32, trim: TextSpacingTrimSpec) -> TextConstraints {
    TextConstraints {
        max_width_px: Some(max_width_px),
        wrap: Some(TextWrapBreak::Word),
        spacing_trim: trim,
        ..TextConstraints::default()
    }
}

fn lay_out(
    engine: &mut NativeTextEngine,
    text: &str,
    constraints: &TextConstraints,
) -> Arc<TextLayout> {
    lay_out_counted(engine, text, constraints).0
}

fn lay_out_counted(
    engine: &mut NativeTextEngine,
    text: &str,
    constraints: &TextConstraints,
) -> (Arc<TextLayout>, TextWorkCounters) {
    let source = TextSource::new(text);
    let mut counters = TextWorkCounters::default();
    let layout = engine.layout(
        TextKind::Paragraph,
        &source,
        &style(),
        constraints,
        &mut counters,
    );
    (layout, counters)
}

fn lines<'t>(layout: &TextLayout, text: &'t str) -> Vec<&'t str> {
    layout
        .lines
        .iter()
        .map(|line| &text[line.source.clone()])
        .collect()
}

/// The advance one fullwidth ideograph takes in the fixture.
fn em(engine: &mut NativeTextEngine) -> f32 {
    let layout = lay_out(engine, "中", &TextConstraints::default());
    layout.lines[0].metrics.width_px
}

/// Every placed glyph's advance adds up to the width its line reports, plus
/// what hangs: what the decision changed is in the glyphs, so caret and
/// hit-testing follow it.
fn assert_lines_add_up(layout: &TextLayout) {
    for line in &layout.lines {
        let runs = &layout.runs[line.runs.start as usize..line.runs.end as usize];
        let glyphs: f32 = runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .map(|glyph| glyph.advance_px)
            .sum();
        let summed: f32 = runs.iter().map(|run| run.advance_px).sum();
        assert!((glyphs - summed).abs() < 0.01, "{glyphs} != {summed}");
        assert!(line.metrics.width_px <= summed + 0.01);
    }
}

#[test]
fn punctuation_is_flagged_by_where_its_ink_sits() {
    let mut engine = engine();
    let text = "中，文「字」";
    let layout = lay_out(&mut engine, text, &TextConstraints::default());
    let flag_of = |byte: usize| {
        layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .find(|glyph| glyph.cluster as usize == byte)
            .map(|glyph| glyph.flags)
            .unwrap()
    };
    let at = |ch: &str| text.find(ch).unwrap();
    assert!(flag_of(at("，")).contains(GlyphFlags::PUNCT_BLANK_AFTER));
    assert!(flag_of(at("「")).contains(GlyphFlags::PUNCT_BLANK_BEFORE));
    assert!(flag_of(at("」")).contains(GlyphFlags::PUNCT_BLANK_AFTER));
    assert!(!flag_of(at("中")).contains(GlyphFlags::PUNCT_BLANK_AFTER));
}

/// With nothing turned on, a layout is exactly what it was; `normal` on
/// text with no punctuation changes nothing either.
#[test]
fn nothing_changes_where_nothing_applies() {
    let mut engine = engine();
    let text = "中文排版测试中文排版测试";
    let off = lay_out(
        &mut engine,
        text,
        &wrapped(100.0, TextSpacingTrimSpec::SpaceAll),
    );
    let on = lay_out(
        &mut engine,
        text,
        &wrapped(100.0, TextSpacingTrimSpec::Normal),
    );
    assert_eq!(lines(&off, text), lines(&on, text));
    for (a, b) in off.lines.iter().zip(&on.lines) {
        assert_eq!(a.metrics.width_px, b.metrics.width_px);
    }
}

/// A closing mark against another mark closes up: one blank half between
/// them, not two.
#[test]
fn adjacent_marks_close_up() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "文。」文";
    let all = lay_out(&mut engine, text, &TextConstraints::default());
    let normal = lay_out(
        &mut engine,
        text,
        &TextConstraints {
            spacing_trim: TextSpacingTrimSpec::Normal,
            ..TextConstraints::default()
        },
    );
    let closed = all.lines[0].metrics.width_px - normal.lines[0].metrics.width_px;
    assert!((closed - em * 0.5).abs() < 0.05, "{closed} vs half of {em}");
    assert_lines_add_up(&normal);
}

/// A closing mark at the end of a line closes up only when the line would
/// not fit otherwise.
#[test]
fn a_line_end_mark_closes_only_when_it_must() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "中文中文，中文";
    let max = em * 4.5 + 0.5;
    let all = lay_out(
        &mut engine,
        text,
        &wrapped(max, TextSpacingTrimSpec::SpaceAll),
    );
    let normal = lay_out(
        &mut engine,
        text,
        &wrapped(max, TextSpacingTrimSpec::Normal),
    );
    assert_eq!(lines(&normal, text)[0], "中文中文，");
    assert_ne!(lines(&all, text)[0], "中文中文，");
    assert!(normal.lines[0].metrics.width_px <= max + 0.01);
    // In a wide box nothing closes.
    let wide = lay_out(
        &mut engine,
        "中文中文，",
        &wrapped(em * 10.0, TextSpacingTrimSpec::Normal),
    );
    assert!((wide.lines[0].metrics.width_px - em * 5.0).abs() < 0.05);
    assert_lines_add_up(&normal);
}

/// A line a little short closes its punctuation up rather than break early
/// and leave a gap; a line far short breaks, because closing that much
/// costs more than the slack a break leaves.
#[test]
fn light_compression_beats_a_ragged_break_and_heavy_compression_loses() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "中文，中文，中文，中文";
    let natural = em * 11.0;
    let light = lay_out(
        &mut engine,
        text,
        &wrapped(natural - em * 0.3, TextSpacingTrimSpec::Auto),
    );
    assert_eq!(light.lines.len(), 1, "{:?}", lines(&light, text));
    assert!(light.lines[0].metrics.width_px <= natural - em * 0.3 + 0.01);
    let normal = lay_out(
        &mut engine,
        text,
        &wrapped(natural - em * 0.3, TextSpacingTrimSpec::Normal),
    );
    assert_eq!(normal.lines.len(), 2);
    let heavy = lay_out(
        &mut engine,
        text,
        &wrapped(natural - em * 1.4, TextSpacingTrimSpec::Auto),
    );
    assert_eq!(heavy.lines.len(), 2, "{:?}", lines(&heavy, text));
    assert_lines_add_up(&light);
}

/// A cluster break (`break-all`) never puts a closing mark at the start of
/// a line, nor leaves an opening one at the end.
#[test]
fn kinsoku_keeps_marks_off_the_wrong_line_edge() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "中文中文。中文「中文」中文";
    for width in [4.0f32, 5.0, 6.0, 7.0] {
        let layout = lay_out(
            &mut engine,
            text,
            &TextConstraints {
                word_break: WordBreakSpec::BreakAll,
                ..wrapped(em * width + 0.5, TextSpacingTrimSpec::Normal)
            },
        );
        for line in lines(&layout, text) {
            assert!(
                !line.starts_with('。') && !line.starts_with('」'),
                "{width}: {line:?}"
            );
            assert!(!line.ends_with('「'), "{width}: {line:?}");
        }
    }
}

/// `loose` allows a break before a small kana after a kana; `normal` and
/// `strict` do not.
#[test]
fn loose_breaks_before_small_kana_and_strict_does_not() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "あいうえおかきくけこっさしすせそ";
    let width = em * 10.0 + 0.5;
    let mut starts_small = |line_break| {
        let layout = lay_out(
            &mut engine,
            text,
            &TextConstraints {
                line_break,
                ..wrapped(width, TextSpacingTrimSpec::SpaceAll)
            },
        );
        lines(&layout, text)
            .iter()
            .any(|line| line.starts_with('っ'))
    };
    assert!(starts_small(LineBreakSpec::Loose));
    assert!(!starts_small(LineBreakSpec::Strict));
    assert!(!starts_small(LineBreakSpec::Normal));
}

/// Autospace puts an eighth of an em between ideographs and Latin letters,
/// and drops it at the end of a line.
#[test]
fn autospace_sits_between_ideographs_and_latin_and_not_at_a_line_end() {
    let mut engine = engine();
    let em = em(&mut engine);
    let constraints = |autospace| TextConstraints {
        autospace,
        ..TextConstraints::default()
    };
    let plain = lay_out(
        &mut engine,
        "中A中",
        &constraints(TextAutospaceSpec::NoAutospace),
    );
    let spaced = lay_out(
        &mut engine,
        "中A中",
        &constraints(TextAutospaceSpec::Normal),
    );
    let gap = spaced.lines[0].metrics.width_px - plain.lines[0].metrics.width_px;
    assert!((gap - em / 4.0).abs() < 0.05, "{gap}");
    let ending = lay_out(&mut engine, "A中", &constraints(TextAutospaceSpec::Normal));
    let ending_plain = lay_out(
        &mut engine,
        "A中",
        &constraints(TextAutospaceSpec::NoAutospace),
    );
    assert!(
        (ending.lines[0].metrics.width_px - ending_plain.lines[0].metrics.width_px - em / 8.0)
            .abs()
            < 0.05
    );
    let trailing = lay_out(&mut engine, "中A", &constraints(TextAutospaceSpec::Normal));
    let trailing_plain = lay_out(
        &mut engine,
        "中A",
        &constraints(TextAutospaceSpec::NoAutospace),
    );
    assert!(
        (trailing.lines[0].metrics.width_px - trailing_plain.lines[0].metrics.width_px - em / 8.0)
            .abs()
            < 0.05
    );
    assert_lines_add_up(&spaced);
}

/// A justified paragraph fills every line but its last.
#[test]
fn justified_lines_fill_the_box_but_the_last() {
    let mut engine = engine();
    for (text, justify) in [
        (
            "one two three four five six seven eight nine ten",
            TextJustifySpec::InterWord,
        ),
        (
            "中文排版测试中文排版测试中文排版测试中文",
            TextJustifySpec::InterCharacter,
        ),
    ] {
        let max = 101.0;
        let layout = lay_out(
            &mut engine,
            text,
            &TextConstraints {
                align: TextAlignSpec::Justify,
                justify,
                ..wrapped(max, TextSpacingTrimSpec::SpaceAll)
            },
        );
        assert!(layout.lines.len() > 1);
        let (last, filled) = layout.lines.split_last().unwrap();
        for line in filled {
            assert!(
                (line.metrics.width_px - max).abs() < 0.05,
                "{text}: {}",
                line.metrics.width_px
            );
        }
        assert!(last.metrics.width_px < max);
        assert_lines_add_up(&layout);
    }
}

/// Growing the box a quarter pixel at a time never brings a line count back
/// up: the decision does not oscillate.
#[test]
fn a_resize_never_oscillates() {
    let mut engine = engine();
    let em = em(&mut engine);
    let text = "中文，排版「测试」中文、排版。测试中文，排版测试。中文排版，测试";
    let mut previous = usize::MAX;
    for step in 0..200 {
        let width = em * 5.0 + step as f32 * 0.25;
        let layout = lay_out(
            &mut engine,
            text,
            &wrapped(width, TextSpacingTrimSpec::Auto),
        );
        assert!(
            layout.lines.len() <= previous,
            "{width}: {} after {previous}",
            layout.lines.len()
        );
        previous = layout.lines.len();
        for line in &layout.lines {
            assert!(line.metrics.width_px <= width + 0.05);
        }
    }
}

/// A pretty paragraph looks ahead within its budget: it never costs more
/// slack than the greedy one, keeps its beam bounded and lays out the same
/// every time.
#[test]
fn a_pretty_paragraph_is_bounded_and_no_raggeder_than_greedy() {
    let mut engine = engine();
    let text = "the quick brown fox jumps over the lazy dog and keeps on running far away";
    let constraints = |wrap_style| TextConstraints {
        wrap_style,
        ..wrapped(120.0, TextSpacingTrimSpec::SpaceAll)
    };
    let (greedy, _) = lay_out_counted(&mut engine, text, &constraints(TextWrapStyleSpec::Auto));
    let (pretty, counters) =
        lay_out_counted(&mut engine, text, &constraints(TextWrapStyleSpec::Pretty));
    let raggedness = |layout: &TextLayout| -> f32 {
        let (_, filled) = layout.lines.split_last().unwrap();
        filled
            .iter()
            .map(|line| (120.0 - line.metrics.width_px).powi(2))
            .sum()
    };
    assert!(raggedness(&pretty) <= raggedness(&greedy) + 0.01);
    assert!(counters.line_beam_states <= 4);
    assert_eq!(counters.line_budget_fallbacks, 0);
    let again = lay_out(
        &mut self::engine(),
        text,
        &constraints(TextWrapStyleSpec::Pretty),
    );
    assert_eq!(lines(&again, text), lines(&pretty, text));
}

const BADGE: &str = "中中中\u{FFFC}中中";
const BADGE_AT: usize = "中中中".len();

/// A badge two ems wide, with or without an envelope keeping `gap` after it,
/// `shrink` of which may close up.
fn lay_out_badge(
    engine: &mut NativeTextEngine,
    em: f32,
    envelope: Option<InlineEnvelope>,
    max_width_px: f32,
) -> Arc<TextLayout> {
    let mut source = TextSource::new(BADGE);
    source.set_objects(vec![InlineObject {
        offset: BADGE_AT,
        id: 1,
        metrics: InlineObjectMetrics {
            width_px: em * 2.0,
            ascent_px: em,
            descent_px: 0.0,
            envelope,
        },
    }]);
    let mut counters = TextWorkCounters::default();
    let constraints = wrapped(max_width_px, TextSpacingTrimSpec::SpaceAll);
    engine.layout(
        TextKind::Paragraph,
        &source,
        &style(),
        &constraints,
        &mut counters,
    )
}

fn gap(em: f32, shrink: f32) -> InlineEnvelope {
    InlineEnvelope {
        gap_px: em / 2.0,
        gap_shrink_px: shrink,
        ..InlineEnvelope::default()
    }
}

/// The badge keeps its own box; its gap takes room on the line after it.
#[test]
fn a_badge_envelope_adds_its_gap_after_the_box() {
    let mut engine = engine();
    let em = em(&mut engine);
    let bare = lay_out_badge(&mut engine, em, None, em * 20.0);
    let spaced = lay_out_badge(&mut engine, em, Some(gap(em, 0.0)), em * 20.0);
    assert_lines_add_up(&spaced);
    let width = |layout: &TextLayout| layout.lines[0].metrics.width_px;
    assert!((width(&spaced) - width(&bare) - em / 2.0).abs() < 0.05);
    assert!((spaced.objects[0].rect.width - em * 2.0).abs() < 0.01);
    assert!((spaced.objects[0].rect.x - bare.objects[0].rect.x).abs() < 0.01);
}

/// A line short by what the badge's gap may give closes the gap up rather
/// than break; one short by more breaks, and a gap that may not close never
/// keeps the line.
#[test]
fn a_short_line_closes_the_badge_gap_before_breaking() {
    let mut engine = engine();
    let em = em(&mut engine);
    // Everything: 3 + 2 + ½ + 2 ems. A quarter em short of it.
    let width = em * 7.25;
    let closing = lay_out_badge(&mut engine, em, Some(gap(em, em / 2.0)), width);
    assert_eq!(lines(&closing, BADGE), vec![BADGE]);
    assert!(closing.lines[0].metrics.width_px <= width + 0.05);
    assert_lines_add_up(&closing);
    let fixed = lay_out_badge(&mut engine, em, Some(gap(em, 0.0)), width);
    assert_eq!(fixed.lines.len(), 2);
    // Short by more than the gap may give: it breaks.
    let tight = lay_out_badge(&mut engine, em, Some(gap(em, em / 2.0)), em * 6.75);
    assert_eq!(tight.lines.len(), 2);
}

/// At a line end the closable part of the gap is dropped, and a break the
/// envelope forbids is never taken.
#[test]
fn a_badge_envelope_drops_its_gap_at_a_line_end_and_keeps_its_breaks() {
    let mut engine = engine();
    let em = em(&mut engine);
    // Room for the text and badge, not the gap and the next character.
    let ends = lay_out_badge(&mut engine, em, Some(gap(em, em / 2.0)), em * 5.25);
    assert_eq!(lines(&ends, BADGE)[0], "中中中\u{FFFC}");
    assert!(ends.lines[0].metrics.width_px <= em * 5.25 + 0.05);
    assert!(ends.objects[0].line == 0);
    // Room for the text only: the badge starts the next line, unless it may
    // not be broken before, when the character before it comes along.
    let free = lay_out_badge(&mut engine, em, Some(gap(em, 0.0)), em * 4.0);
    assert!(lines(&free, BADGE)[1].starts_with('\u{FFFC}'));
    let glued = InlineEnvelope {
        break_before: false,
        ..gap(em, 0.0)
    };
    let kept = lay_out_badge(&mut engine, em, Some(glued), em * 4.0);
    assert!(
        lines(&kept, BADGE)
            .iter()
            .all(|line| !line.starts_with('\u{FFFC}'))
    );
    assert!(
        lines(&kept, BADGE)
            .iter()
            .any(|line| line.starts_with("中\u{FFFC}"))
    );
}
