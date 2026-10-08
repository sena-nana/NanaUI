//! Ruby annotations: small text set above a base range. The base lays out as
//! one unit, is spaced out to a wider annotation, and its line grows to hold
//! it. Horizontal only.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    NativeTextEngine, RubySpan, TextConstraints, TextEngine, TextKind, TextLayout, TextSource,
    TextStyle, TextWorkCounters,
};
use nana_ui_core::{TextWrapBreak, WritingModeSpec};
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

fn source(text: &str, rubies: &[(std::ops::Range<usize>, &str)]) -> TextSource {
    let mut source = TextSource::new(text);
    source.set_rubies(
        rubies
            .iter()
            .map(|(range, text)| RubySpan {
                range: range.clone(),
                text: Arc::from(*text),
            })
            .collect(),
    );
    source
}

fn lay_out(
    engine: &mut NativeTextEngine,
    source: &TextSource,
    constraints: TextConstraints,
) -> Arc<TextLayout> {
    let mut counters = TextWorkCounters::default();
    let kind = if constraints.max_width_px.is_some() {
        TextKind::Paragraph
    } else {
        TextKind::Label
    };
    engine.layout(kind, source, &style(), &constraints, &mut counters)
}

fn label() -> TextConstraints {
    TextConstraints::default()
}

/// Left edge and right edge of the glyphs whose cluster is in `range`.
fn base_extent(layout: &TextLayout, range: std::ops::Range<usize>) -> (f32, f32) {
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

// "漢字です": 漢 0..3, 字 3..6.
const TEXT: &str = "漢字です";

#[test]
fn an_annotation_sits_centred_above_its_base_and_grows_the_line() {
    let mut engine = engine();
    let plain = lay_out(&mut engine, &TextSource::new(TEXT), label());
    let annotated = lay_out(&mut engine, &source(TEXT, &[(0..6, "かんじ")]), label());
    assert_eq!(annotated.rubies.len(), 1);
    assert!(!annotated.rubies_dropped);
    let ruby = &annotated.rubies[0];
    assert_eq!(ruby.range, 0..6);
    assert_eq!(ruby.line, 0);
    assert!(!ruby.runs.is_empty());
    let line = &annotated.lines[0];
    assert!(
        line.bounds.height > plain.lines[0].bounds.height,
        "the line grows to hold the annotation"
    );
    assert!(
        ruby.rect.y >= line.bounds.y - 0.01,
        "the annotation stays inside its line"
    );
    assert!(
        ruby.baseline_y_px < line.metrics.baseline_y_px - 10.0,
        "it is set above the base"
    );
    let (left, right) = base_extent(&annotated, 0..6);
    let centre = ruby.rect.x + ruby.rect.width * 0.5;
    assert!(
        (centre - (left + right) * 0.5).abs() < 0.01,
        "centred over the base"
    );
    assert!(ruby.runs[0].metrics.ascent_px < annotated.runs[0].metrics.ascent_px);
}

#[test]
fn a_wider_annotation_spaces_its_base_out_and_moves_the_rest_on() {
    let mut engine = engine();
    let plain = lay_out(&mut engine, &TextSource::new(TEXT), label());
    // Six kana at half size are wider than one ideograph at full size.
    let annotated = lay_out(
        &mut engine,
        &source(TEXT, &[(0..3, "かんかんかん")]),
        label(),
    );
    let ruby = &annotated.rubies[0];
    let (plain_left, plain_right) = base_extent(&plain, 0..3);
    let (left, right) = base_extent(&annotated, 0..3);
    assert!(ruby.rect.width > plain_right - plain_left);
    assert!(
        (right - left - ruby.rect.width).abs() < 0.01,
        "the base takes the annotation's width"
    );
    assert!((left - plain_left).abs() < 0.01);
    let extra = ruby.rect.width - (plain_right - plain_left);
    let after_plain = base_extent(&plain, 3..6).0;
    let after = base_extent(&annotated, 3..6).0;
    assert!(
        (after - after_plain - extra).abs() < 0.01,
        "what follows moves on by the difference"
    );
    assert!((ruby.rect.x - left).abs() < 0.01);
}

#[test]
fn a_line_never_breaks_inside_a_base() {
    let mut engine = engine();
    let text = "漢字漢字漢字漢字";
    let narrow = TextConstraints {
        max_width_px: Some(50.0),
        wrap: Some(TextWrapBreak::Word),
        ..TextConstraints::default()
    };
    let plain = lay_out(&mut engine, &TextSource::new(text), narrow);
    assert!(
        plain.lines.iter().any(|line| line.source.start == 6),
        "unannotated, ideographs break anywhere"
    );
    let annotated = lay_out(&mut engine, &source(text, &[(3..9, "かんじ")]), narrow);
    for line in &annotated.lines {
        assert!(
            !(3 < line.source.start && line.source.start < 9),
            "no line starts inside the base: {:?}",
            line.source
        );
    }
}

#[test]
fn annotations_follow_edits_and_change_the_layout() {
    let mut engine = engine();
    let mut annotated = source(TEXT, &[(0..6, "かんじ")]);
    annotated.replace_range(0..0, "「");
    assert_eq!(annotated.rubies()[0].range, 3..9);
    annotated.replace_range(3..6, "");
    assert!(
        annotated.rubies().is_empty(),
        "an edit inside a base drops it"
    );
    let a = lay_out(&mut engine, &source(TEXT, &[(0..6, "かんじ")]), label());
    let b = lay_out(&mut engine, &source(TEXT, &[(0..6, "かな")]), label());
    assert_ne!(
        a.rubies[0].rect.width, b.rubies[0].rect.width,
        "a new annotation lays out anew"
    );
}

#[test]
fn a_vertical_layout_drops_annotations_and_says_so() {
    let mut engine = engine();
    let vertical = TextConstraints {
        writing_mode: WritingModeSpec::VerticalRl,
        ..TextConstraints::default()
    };
    let layout = lay_out(&mut engine, &source(TEXT, &[(0..6, "かんじ")]), vertical);
    assert!(layout.rubies.is_empty());
    assert!(layout.rubies_dropped);
}
