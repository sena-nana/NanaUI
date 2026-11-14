//! Emphasis marks (CSS `text-emphasis`): a small mark centred over or under
//! each character of a range that takes one. The line grows by the mark's
//! height on its side; the characters are neither reshaped nor moved.
//! Horizontal only.

use crate::support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    EMPHASIS_SCALE, EmphasisSpan, NativeTextEngine, RubySpan, TextConstraints, TextEngine,
    TextKind, TextLayout, TextSource, TextSpan, TextStyle, TextWorkCounters,
};
use nana_ui_core::{TextEmphasisPosition, TextEmphasisShape, TextEmphasisStyle, WritingModeSpec};
use std::sync::Arc;

use support::corpus::fixture_bytes;

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .expect("fixture registers");
    NativeTextEngine::new(fonts)
}

fn style() -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from("Noto Sans SC")),
        font_size_px: 20.0,
        ..TextStyle::default()
    }
}

fn dot() -> Arc<str> {
    TextEmphasisStyle::Mark {
        shape: Some(TextEmphasisShape::Dot),
        open: false,
    }
    .mark_text(false)
    .expect("a dot sets a mark")
}

fn source(
    text: &str,
    range: std::ops::Range<usize>,
    mark: Arc<str>,
    position: TextEmphasisPosition,
) -> TextSource {
    let mut source = TextSource::new(text);
    source.set_emphasis(vec![EmphasisSpan {
        range,
        mark,
        position,
    }]);
    source
}

fn lay_out(engine: &mut NativeTextEngine, source: &TextSource) -> Arc<TextLayout> {
    lay_out_with(engine, source, TextConstraints::default())
}

fn lay_out_with(
    engine: &mut NativeTextEngine,
    source: &TextSource,
    constraints: TextConstraints,
) -> Arc<TextLayout> {
    let mut counters = TextWorkCounters::default();
    engine.layout(
        TextKind::Label,
        source,
        &style(),
        &constraints,
        &mut counters,
    )
}

/// Left edge and right edge of the glyphs whose cluster is in `range`.
fn extent(layout: &TextLayout, range: std::ops::Range<usize>) -> (f32, f32) {
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for run in &layout.runs {
        let mut pen = run.origin_x_px;
        for glyph in &run.glyphs {
            if range.contains(&(glyph.cluster as usize)) {
                left = left.min(pen);
                right = right.max(pen + glyph.advance_px);
            }
            pen += glyph.advance_px;
        }
    }
    (left, right)
}

// 重 0..3, 点 3..6, 在 6..9, 此 9..12.
const TEXT: &str = "重点在此";

#[test]
fn marks_over_the_text_grow_the_line_by_their_own_height() {
    let mut engine = engine();
    let plain = lay_out(&mut engine, &TextSource::new(TEXT));
    let marked = lay_out(
        &mut engine,
        &source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::OVER),
    );
    assert!(!marked.emphasis_dropped);
    assert_eq!(
        marked
            .emphasis
            .iter()
            .map(|mark| mark.offset)
            .collect::<Vec<_>>(),
        vec![0, 3, 6, 9],
        "one mark per character"
    );
    assert_eq!(marked.emphasis_marks.len(), 1, "one distinct mark");

    // The line keeps its half-leading and grows by what the mark reaches
    // past it: not a fixed multiple of the font size.
    let line = &plain.lines[0].metrics;
    let half_leading = (line.height_px - (line.ascent_px + line.descent_px)) * 0.5;
    let mark_height = marked.emphasis[0].rect.height;
    let expected = (mark_height - half_leading).max(0.0);
    let grown = marked.lines[0].metrics.height_px - line.height_px;
    assert!(expected > 0.0);
    assert!(
        (grown - expected).abs() < 0.01,
        "grew {grown}, the mark asks for {expected}"
    );
    assert!(
        grown < style().font_size_px,
        "not a fixed multiple of the font size"
    );
    assert!(
        (marked.lines[0].metrics.baseline_y_px - line.baseline_y_px - grown).abs() < 0.01,
        "the growth is above the text: the baseline moves down by it"
    );

    for mark in &marked.emphasis {
        let (left, right) = extent(&marked, mark.offset..mark.offset + 3);
        let centre = mark.rect.x + mark.rect.width * 0.5;
        assert!(
            (centre - (left + right) * 0.5).abs() < 0.01,
            "centred on its character"
        );
        assert!(mark.rect.y >= marked.lines[0].bounds.y - 0.01);
        assert!(
            mark.rect.y + mark.rect.height
                <= marked.lines[0].metrics.baseline_y_px - line.ascent_px + 0.01,
            "standing on the character's ascent"
        );
    }
    // The characters keep their advances.
    assert_eq!(extent(&plain, 0..12), extent(&marked, 0..12));
}

#[test]
fn marks_under_the_text_grow_the_line_below_it() {
    let mut engine = engine();
    let plain = lay_out(&mut engine, &TextSource::new(TEXT));
    let marked = lay_out(
        &mut engine,
        &source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::UNDER),
    );
    let line = &plain.lines[0].metrics;
    let half_leading = (line.height_px - (line.ascent_px + line.descent_px)) * 0.5;
    let mark_height = marked.emphasis[0].rect.height;
    let grown = marked.lines[0].metrics.height_px - line.height_px;
    assert!(
        (grown - (mark_height - half_leading).max(0.0)).abs() < 0.01,
        "grew {grown} for a mark {mark_height} tall"
    );
    assert_eq!(
        marked.lines[0].metrics.baseline_y_px, line.baseline_y_px,
        "nothing grows above the text"
    );
    let bottom = marked.lines[0].bounds.y + marked.lines[0].bounds.height;
    for mark in &marked.emphasis {
        assert!(
            mark.rect.y >= marked.lines[0].metrics.baseline_y_px + line.descent_px - 0.01,
            "hanging from the character's descent"
        );
        assert!(mark.rect.y + mark.rect.height <= bottom + 0.01);
    }
}

#[test]
fn separators_and_punctuation_take_no_mark() {
    let mut engine = engine();
    // 重 0..3, ， 3..6, 点 6..9, space 9, a 10, 。 11..14, \t 14, b 15.
    let text = "重，点 a。\tb";
    let marked = lay_out(
        &mut engine,
        &source(text, 0..text.len(), dot(), TextEmphasisPosition::OVER),
    );
    assert_eq!(
        marked
            .emphasis
            .iter()
            .map(|mark| mark.offset)
            .collect::<Vec<_>>(),
        vec![0, 6, 10, 15]
    );
    // A line of nothing but punctuation inside the range does not grow.
    let plain = lay_out(&mut engine, &TextSource::new("，。"));
    let punctuation = lay_out(
        &mut engine,
        &source("，。", 0..6, dot(), TextEmphasisPosition::OVER),
    );
    assert!(punctuation.emphasis.is_empty());
    assert_eq!(punctuation.lines[0].metrics, plain.lines[0].metrics);
}

#[test]
fn a_string_mark_is_its_first_grapheme_at_half_the_size() {
    let mut engine = engine();
    let marked = lay_out(
        &mut engine,
        &source(
            TEXT,
            3..9,
            Arc::from("※ignored"),
            TextEmphasisPosition::OVER,
        ),
    );
    assert_eq!(
        marked
            .emphasis
            .iter()
            .map(|mark| mark.offset)
            .collect::<Vec<_>>(),
        vec![3, 6]
    );
    let runs = &marked.emphasis_marks[0];
    let glyphs: Vec<u32> = runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cluster))
        .collect();
    assert_eq!(glyphs, vec![0], "only the first grapheme is set");
    assert!(
        runs.iter()
            .all(|run| run.font_size_px == style().font_size_px * EMPHASIS_SCALE)
    );
    // A keyword mark is a different character, so a different layout.
    let dotted = lay_out(
        &mut engine,
        &source(TEXT, 3..9, dot(), TextEmphasisPosition::OVER),
    );
    assert_ne!(
        dotted.emphasis_marks[0][0].glyphs[0].glyph_id,
        runs[0].glyphs[0].glyph_id
    );
}

#[test]
fn a_larger_span_gets_larger_marks() {
    let mut engine = engine();
    let mut big = source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::OVER);
    big.set_spans(vec![TextSpan {
        range: 6..12,
        style: TextStyle {
            font_size_px: 40.0,
            ..style()
        },
        composition: None,
    }]);
    let marked = lay_out(&mut engine, &big);
    assert_eq!(marked.emphasis.len(), 4);
    assert_eq!(marked.emphasis_marks.len(), 2);
    let size = |offset: usize| {
        let mark = marked
            .emphasis
            .iter()
            .find(|mark| mark.offset == offset)
            .expect("marked");
        marked.emphasis_marks[mark.mark as usize][0].font_size_px
    };
    assert_eq!(size(0), 10.0);
    assert_eq!(size(9), 20.0);
}

#[test]
fn changing_the_marks_lays_out_again_without_reshaping_the_text() {
    let mut engine = engine();
    let over = source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::OVER);
    let under = source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::UNDER);
    let a = lay_out(&mut engine, &over);
    let before = engine.shape_counters();
    let b = lay_out(&mut engine, &under);
    let after = engine.shape_counters();
    assert_eq!(
        after.shape_cache_misses, before.shape_cache_misses,
        "the text and the mark are both shaped already"
    );
    assert!(!Arc::ptr_eq(&a, &b), "another position is another layout");
    assert!(a.emphasis[0].rect.y < b.emphasis[0].rect.y);
    assert!(Arc::ptr_eq(&a, &lay_out(&mut engine, &over)));
}

#[test]
fn a_mark_over_a_ruby_base_stands_on_the_ruby() {
    let mut engine = engine();
    let mut both = source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::OVER);
    both.set_rubies(vec![RubySpan {
        range: 0..6,
        text: Arc::from("zhòngdiǎn"),
    }]);
    let marked = lay_out(&mut engine, &both);
    let ruby = &marked.rubies[0];
    let over_ruby = marked
        .emphasis
        .iter()
        .find(|mark| mark.offset == 0)
        .unwrap();
    let beside = marked
        .emphasis
        .iter()
        .find(|mark| mark.offset == 6)
        .unwrap();
    assert!(over_ruby.rect.y + over_ruby.rect.height <= ruby.rect.y + 0.01);
    assert!(beside.rect.y > over_ruby.rect.y);
    assert!(over_ruby.rect.y >= marked.lines[0].bounds.y - 0.01);
}

#[test]
fn emphasis_follows_edits() {
    let mut marked = source(TEXT, 3..9, dot(), TextEmphasisPosition::OVER);
    marked.replace_range(0..0, "「");
    assert_eq!(marked.emphasis()[0].range, 6..12);
    marked.replace_range(9..9, "的");
    assert_eq!(
        marked.emphasis()[0].range,
        6..15,
        "it takes in what is typed"
    );
    marked.replace_range(0..9, "");
    assert_eq!(marked.emphasis()[0].range, 0..6, "and loses what is cut");
}

#[test]
fn a_vertical_layout_drops_marks_and_says_so() {
    let mut engine = engine();
    let vertical = TextConstraints {
        writing_mode: WritingModeSpec::VerticalRl,
        ..TextConstraints::default()
    };
    let layout = lay_out_with(
        &mut engine,
        &source(TEXT, 0..TEXT.len(), dot(), TextEmphasisPosition::OVER),
        vertical,
    );
    assert!(layout.emphasis.is_empty());
    assert!(layout.emphasis_dropped);
}
