//! Issue #92: the layout engine's own contracts.
//!
//! The corpus test next door proves the native engine lays the migration cases
//! out the way the reference did. This file covers what the corpus cannot: the
//! Label fast path, relayout without reshaping, the strut, ellipsis cutting,
//! alignment, intrinsic widths, the bounded cache and the fail-closed writing
//! mode.
//!
//! Every fixture is hermetic: the fonts are the checked-in fixtures plus the
//! bundled UI face, registered in a fresh [`FontSystem`] per test.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::layout::{LayoutCacheBudget, LayoutRequest, Layouter};
use nana_text::shaping::{ShapeRequest, Shaper};
use nana_text::{
    LineBreakCause, NativeTextEngine, OverflowFlags, TextConstraints, TextEngine, TextKind,
    TextLayout, TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::{DirSpec, LineHeightSpec, TextAlignSpec, TextWrapBreak, WritingModeSpec};
use std::sync::Arc;

use support::corpus::{fixture_bytes, fixture_family};

/// The UI face on its own: Latin, CJK, combining marks, ligatures.
const UI: &[&str] = &["noto-sans-sc"];
/// The UI face with the monochrome emoji face behind it.
const UI_AND_EMOJI: &[&str] = &["noto-sans-sc", "noto-emoji"];
/// The UI face with Arabic behind it, for the BiDi fixtures.
const UI_AND_ARABIC: &[&str] = &["noto-sans-sc", "noto-sans-arabic"];

fn fonts(ids: &[&str]) -> FontSystem {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    for id in ids {
        fonts
            .register_bytes(fixture_bytes(id), &FaceDescriptor::default())
            .expect("fixture registers");
    }
    fonts
}

fn text_engine(ids: &[&str]) -> NativeTextEngine {
    NativeTextEngine::new(fonts(ids))
}

/// A style naming every registered fixture, in order, as its family list.
fn style(ids: &[&str], size_px: f32) -> TextStyle {
    let families: Vec<String> = ids
        .iter()
        .map(|id| format!("\"{}\"", fixture_family(id)))
        .collect();
    TextStyle {
        font_family: Some(Arc::from(families.join(", "))),
        font_size_px: size_px,
        ..TextStyle::default()
    }
}

fn wrapped(max_width_px: f32) -> TextConstraints {
    TextConstraints {
        max_width_px: Some(max_width_px),
        wrap: Some(TextWrapBreak::Word),
        ..TextConstraints::default()
    }
}

/// Lays one source out through the whole engine.
fn lay_out(
    engine: &mut NativeTextEngine,
    kind: TextKind,
    text: &str,
    style: &TextStyle,
    constraints: &TextConstraints,
) -> Arc<TextLayout> {
    let source = TextSource::new(text);
    let mut counters = TextWorkCounters::default();
    engine.layout(kind, &source, style, constraints, &mut counters)
}

/// The source text of each line, as the layout says it is.
fn line_texts(layout: &TextLayout, text: &str) -> Vec<String> {
    layout
        .lines
        .iter()
        .map(|line| text[line.source.clone()].to_string())
        .collect()
}

// ---- the Label fast path ------------------------------------------------

#[test]
fn ten_thousand_labels_never_enter_the_paragraph_path() {
    let mut engine = text_engine(UI);
    let style = style(UI, 14.0);
    // A width the labels do not fit in: a Label does not wrap however narrow
    // the container is, and must not start scanning for a place to break.
    let constraints = TextConstraints {
        max_width_px: Some(40.0),
        ..TextConstraints::default()
    };
    for index in 0..10_000 {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            &format!("Item {index}"),
            &style,
            &constraints,
        );
        assert_eq!(layout.lines.len(), 1);
    }

    let counters = engine.layout_counters();
    assert_eq!(counters.layout_requests, 10_000);
    assert_eq!(
        counters.paragraph_paths, 0,
        "a single-line label must never walk paragraphs"
    );
    assert_eq!(counters.label_fast_paths, 10_000);
    assert_eq!(
        counters.line_break_candidates, 0,
        "the fast path must not look for break opportunities"
    );
    assert_eq!(counters.lines_created, 10_000, "one line each, and no more");
}

#[test]
fn ten_thousand_labels_reading_the_same_string_are_one_layout() {
    let mut engine = text_engine(UI);
    let style = style(UI, 14.0);
    for _ in 0..10_000 {
        lay_out(
            &mut engine,
            TextKind::Label,
            "Save",
            &style,
            &TextConstraints::default(),
        );
    }
    let counters = engine.layout_counters();
    assert_eq!(counters.layout_created, 1);
    assert_eq!(counters.layout_cache_hits, 9_999);
    assert_eq!(
        engine.shape_counters().shape_runs_created,
        1,
        "and one shaping"
    );
}

#[test]
fn a_label_degrades_to_the_paragraph_path_when_it_stops_being_one_line() {
    let style = style(UI, 14.0);
    let newline = TextConstraints {
        preserve_lines: true,
        ..TextConstraints::default()
    };
    let mut engine = text_engine(UI);
    let layout = lay_out(&mut engine, TextKind::Label, "one\ntwo", &style, &newline);
    assert_eq!(layout.lines.len(), 2, "an authored newline is two lines");
    assert_eq!(engine.layout_counters().label_fast_paths, 0);
    assert_eq!(engine.layout_counters().paragraph_paths, 1);

    let mut engine = text_engine(UI);
    lay_out(
        &mut engine,
        TextKind::Label,
        "one two",
        &style,
        &wrapped(20.0),
    );
    assert_eq!(
        engine.layout_counters().label_fast_paths,
        0,
        "a wrapping label is a paragraph"
    );

    let mut engine = text_engine(UI);
    let two_lines = TextConstraints {
        max_lines: Some(2),
        ..TextConstraints::default()
    };
    lay_out(&mut engine, TextKind::Label, "one", &style, &two_lines);
    assert_eq!(engine.layout_counters().label_fast_paths, 0);
}

#[test]
fn a_label_that_is_not_preserving_lines_folds_the_newline_into_a_space() {
    let mut engine = text_engine(UI);
    let style = style(UI, 14.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        "one\ntwo",
        &style,
        &TextConstraints::default(),
    );
    assert_eq!(layout.lines.len(), 1);
    assert_eq!(
        layout.glyph_count(),
        7,
        "the newline became a space and still draws"
    );
}

// ---- relayout without reshaping -----------------------------------------

#[test]
fn changing_only_the_width_relayouts_and_never_reshapes() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(400.0),
    );
    let shaped_once = engine.shape_counters().shape_runs_created;
    assert!(shaped_once > 0, "the first width shaped the text");
    engine.reset_counters();

    for width in [300.0, 200.0, 150.0] {
        lay_out(
            &mut engine,
            TextKind::Paragraph,
            text,
            &style,
            &wrapped(width),
        );
    }

    let shape = engine.shape_counters();
    assert_eq!(shape.shape_requests, 3);
    assert_eq!(
        shape.shape_runs_created, 0,
        "a width change must not create a single shaped run"
    );
    assert_eq!(shape.shape_cache_misses, 0);
    assert_eq!(shape.shape_cache_hits, 3);

    let layout = engine.layout_counters();
    assert_eq!(layout.layout_created, 3);
    assert_eq!(
        layout.constraint_only_relayouts, 3,
        "each was the same shaped text at a new width"
    );
    assert!(
        layout.shape_runs_reused_for_layout >= 3,
        "every layout read the shaped runs instead of making new ones"
    );
}

#[test]
fn a_resize_touches_only_the_paragraph_whose_width_changed() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let texts: Vec<String> = (0..50)
        .map(|index| format!("paragraph number {index}"))
        .collect();
    for text in &texts {
        lay_out(
            &mut engine,
            TextKind::Paragraph,
            text,
            &style,
            &wrapped(400.0),
        );
    }
    engine.reset_counters();

    let resized = lay_out(
        &mut engine,
        TextKind::Paragraph,
        &texts[7],
        &style,
        &wrapped(90.0),
    );
    let counters = engine.layout_counters();
    assert_eq!(counters.layout_created, 1, "one paragraph, one relayout");
    assert_eq!(counters.constraint_only_relayouts, 1);
    assert_eq!(
        counters.lines_created,
        resized.lines.len(),
        "line work is that paragraph's lines, not the node count"
    );
    assert_eq!(engine.shape_counters().shape_cache_misses, 0);
}

#[test]
fn an_unchanged_request_is_answered_from_the_cache_with_the_same_layout() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let first = lay_out(
        &mut engine,
        TextKind::Label,
        "Preferences",
        &style,
        &TextConstraints::default(),
    );
    let second = lay_out(
        &mut engine,
        TextKind::Label,
        "Preferences",
        &style,
        &TextConstraints::default(),
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "an immutable layout is shared, not copied"
    );
    assert_eq!(engine.layout_counters().layout_created, 1);
}

// ---- wrapping ------------------------------------------------------------

#[test]
fn word_wrap_breaks_between_words_and_reports_the_wrap() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(140.0),
    );

    assert!(layout.lines.len() > 1);
    for line in &layout.lines[..layout.lines.len() - 1] {
        assert_eq!(line.break_cause, LineBreakCause::Wrap);
    }
    assert_eq!(
        layout.lines.last().expect("lines").break_cause,
        LineBreakCause::EndOfText
    );
    for line_text in line_texts(&layout, text) {
        assert!(
            !line_text.starts_with(' ') && !line_text.ends_with(' '),
            "a soft wrap hangs its whitespace: {line_text:?}"
        );
        for word in line_text.split(' ') {
            assert!(
                text.split(' ').any(|whole| whole == word),
                "{word:?} is half a word"
            );
        }
    }
    assert!(
        !layout.overflow.contains(OverflowFlags::CLIPPED_WIDTH),
        "every line fits"
    );
    assert!(engine.layout_counters().line_break_candidates > 0);
}

#[test]
fn a_word_wider_than_the_box_overflows_under_word_wrap_and_is_cut_under_break_word() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "Hamburgefonstiv";
    let narrow = wrapped(60.0);
    let overflowing = lay_out(&mut engine, TextKind::Paragraph, text, &style, &narrow);
    assert_eq!(
        overflowing.lines.len(),
        1,
        "`word-break: normal` sticks out"
    );
    assert!(overflowing.overflow.contains(OverflowFlags::CLIPPED_WIDTH));

    let cut = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            wrap: Some(TextWrapBreak::WordOrGlyph),
            ..narrow
        },
    );
    assert!(cut.lines.len() > 1, "word-or-glyph cuts the word up");
    for line in &cut.lines {
        assert!(line.metrics.width_px <= 60.0 + 0.01, "every piece fits");
    }
    let joined: String = line_texts(&cut, text).join("");
    assert_eq!(joined, text, "the cuts lose no bytes");
}

#[test]
fn han_wraps_between_ideographs_with_no_space_to_break_at() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "中文排版测试中文排版测试中文排版测试";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(100.0),
    );
    assert!(layout.lines.len() > 1);
    for line in &layout.lines {
        assert!(line.metrics.width_px <= 100.0 + 0.01);
        assert!(
            text.is_char_boundary(line.source.start) && text.is_char_boundary(line.source.end),
            "a break never lands inside a character"
        );
    }
}

#[test]
fn an_explicit_newline_ends_a_line_even_where_the_text_would_have_fitted() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "one\ntwo\nthree";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    assert_eq!(line_texts(&layout, text), vec!["one", "two", "three"]);
    assert_eq!(layout.lines[0].break_cause, LineBreakCause::Explicit);
    assert_eq!(layout.lines[2].break_cause, LineBreakCause::EndOfText);
    assert!(
        layout.lines[1].metrics.top_y_px > layout.lines[0].metrics.top_y_px,
        "lines stack downwards"
    );
}

#[test]
fn an_empty_paragraph_still_occupies_a_line_a_caret_can_land_on() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "one\n\ntwo";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    assert_eq!(layout.lines.len(), 3);
    let blank = &layout.lines[1];
    assert_eq!(blank.source, 4..4);
    assert!(blank.metrics.height_px > 0.0);
    assert_eq!(layout.line_runs(blank).len(), 0);
}

// ---- BiDi ----------------------------------------------------------------

#[test]
fn a_mixed_bidi_line_is_ordered_visually_and_keeps_its_logical_ranges() {
    let mut engine = text_engine(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let text = "abc عربي def";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints::default(),
    );
    assert_eq!(layout.lines.len(), 1);
    let line = &layout.lines[0];
    let runs = layout.line_runs(line);
    assert!(runs.len() >= 3, "latin, arabic, latin at the very least");

    let mut cursor = f32::NEG_INFINITY;
    for run in runs {
        assert!(
            run.origin_x_px >= cursor,
            "runs are stored left to right: {:?}",
            runs.iter().map(|run| run.origin_x_px).collect::<Vec<_>>()
        );
        cursor = run.origin_x_px;
    }
    assert_eq!(
        runs.first().expect("runs").source.start,
        0,
        "an ltr paragraph puts the first logical bytes on the left"
    );
    assert_eq!(runs.last().expect("runs").source.end, text.len());
    let rtl = runs
        .iter()
        .find(|run| run.direction.is_rtl())
        .expect("the arabic run");
    assert!(
        rtl.glyphs
            .windows(2)
            .all(|pair| pair[0].cluster >= pair[1].cluster),
        "an rtl run's glyphs descend through the source"
    );

    let rtl_base = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            base_direction: DirSpec::Rtl,
            ..TextConstraints::default()
        },
    );
    let runs = rtl_base.line_runs(&rtl_base.lines[0]);
    assert_eq!(
        runs.last().expect("runs").source.start,
        0,
        "an rtl paragraph puts the first logical bytes on the right"
    );
}

#[test]
fn every_wrapped_line_of_a_bidi_paragraph_is_ordered_on_its_own() {
    let mut engine = text_engine(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let text = "abc عربي def عربي ghi";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(120.0),
    );
    assert!(layout.lines.len() > 1);
    let mut covered = 0;
    for line in &layout.lines {
        let runs = layout.line_runs(line);
        let mut cursor = f32::NEG_INFINITY;
        for run in runs {
            assert!(run.origin_x_px >= cursor);
            cursor = run.origin_x_px;
            assert!(
                run.source.start >= line.source.start && run.source.end <= line.source.end,
                "a run belongs to the line that carries it"
            );
        }
        assert!(line.source.start >= covered);
        covered = line.source.end;
    }
}

// ---- line box metrics ----------------------------------------------------

#[test]
fn a_fallback_run_grows_the_line_box_without_moving_the_baseline() {
    // The Arabic face is taller than the UI face, so a line that falls back to
    // it would drag its own baseline up were the line box measured from the
    // tallest run.
    let mut engine = text_engine(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let plain = lay_out(
        &mut engine,
        TextKind::Label,
        "Search",
        &style,
        &TextConstraints::default(),
    );
    let fallback = lay_out(
        &mut engine,
        TextKind::Label,
        "Search عربي",
        &style,
        &TextConstraints::default(),
    );

    let plain = plain.lines[0].metrics;
    let fallback = fallback.lines[0].metrics;
    assert!(
        (plain.baseline_y_px - fallback.baseline_y_px).abs() < 0.001,
        "the strut fixes the baseline: {plain:?} vs {fallback:?}"
    );
    assert!((plain.height_px - fallback.height_px).abs() < 0.001);
    assert!(
        fallback.ascent_px > plain.ascent_px,
        "the taller face is still reported: {plain:?} vs {fallback:?}"
    );
}

#[test]
fn without_a_strut_the_baseline_follows_the_tallest_run_on_each_line() {
    // The reference engine's rule, and the one the corpus goldens record. It is
    // deterministic but it moves, which is exactly why the engine supplies a
    // strut.
    let mut fonts = fonts(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let mut shaper = Shaper::default();
    let mut layouter = Layouter::default();
    let constraints = TextConstraints::default();

    let mut baseline_of = |text: &str| {
        let source = TextSource::new(text);
        let shaped = shaper.shape(
            &mut fonts,
            &ShapeRequest::new(&source, &style, &constraints),
        );
        let layout = layouter.layout(&LayoutRequest::new(
            TextKind::Label,
            &source,
            &shaped,
            &style,
            &constraints,
        ));
        layout.lines[0].metrics.baseline_y_px
    };
    assert_ne!(baseline_of("Search"), baseline_of("Search عربي"));
}

#[test]
fn line_height_sets_the_line_box_and_the_half_leading_around_the_baseline() {
    let mut engine = text_engine(UI);
    let mut style = style(UI, 20.0);
    style.line_height = Some(LineHeightSpec::Absolute(40.0));
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        "Save",
        &style,
        &TextConstraints::default(),
    );
    let metrics = layout.lines[0].metrics;
    assert!((metrics.height_px - 40.0).abs() < 0.001);
    let strut_box = metrics.ascent_px + metrics.descent_px;
    let expected = (metrics.height_px - strut_box) * 0.5 + metrics.ascent_px;
    assert!(
        (metrics.baseline_y_px - expected).abs() < 0.001,
        "half the leading sits above the text: {metrics:?}"
    );
    assert_eq!(metrics.top_y_px, 0.0);
}

#[test]
fn a_fractional_scale_scales_the_line_box_with_the_advances() {
    let mut engine = text_engine(UI);
    let style = style(UI, 20.0);
    let plain = lay_out(
        &mut engine,
        TextKind::Label,
        "Hamburgefonstiv",
        &style,
        &TextConstraints::default(),
    );
    let scaled = lay_out(
        &mut engine,
        TextKind::Label,
        "Hamburgefonstiv",
        &style,
        &TextConstraints {
            scale: nana_text::TextScale {
                px_per_logical: 1.25,
            },
            ..TextConstraints::default()
        },
    );
    let ratio = scaled.lines[0].metrics.width_px / plain.lines[0].metrics.width_px;
    assert!((ratio - 1.25).abs() < 0.01, "advances scale: {ratio}");
    let ratio = scaled.lines[0].metrics.height_px / plain.lines[0].metrics.height_px;
    assert!((ratio - 1.25).abs() < 0.001, "and so does the line box");
    let width = scaled.lines[0].metrics.width_px;
    assert!(
        width.fract() != 0.0,
        "layout keeps fractional precision ({width}); rounding to device pixels is the painter's job"
    );
}

// ---- alignment -----------------------------------------------------------

#[test]
fn alignment_places_the_line_inside_the_container_and_start_follows_direction() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let container = 400.0;
    let mut offsets = Vec::new();
    for (align, direction) in [
        (TextAlignSpec::Start, DirSpec::Ltr),
        (TextAlignSpec::End, DirSpec::Ltr),
        (TextAlignSpec::Center, DirSpec::Ltr),
        (TextAlignSpec::Left, DirSpec::Rtl),
        (TextAlignSpec::Right, DirSpec::Rtl),
        (TextAlignSpec::Start, DirSpec::Rtl),
    ] {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            "Save",
            &style,
            &TextConstraints {
                max_width_px: Some(container),
                align,
                base_direction: direction,
                ..TextConstraints::default()
            },
        );
        let line = &layout.lines[0];
        offsets.push(line.bounds.x);
        assert!(line.bounds.right() <= container + 0.01);
        assert_eq!(
            layout.line_runs(line)[0].origin_x_px,
            line.bounds.x,
            "the runs move with the line box"
        );
    }
    let width = offsets[1];
    assert_eq!(
        offsets[0], 0.0,
        "start is the left edge in an ltr paragraph"
    );
    assert!(width > 0.0, "end is the right edge");
    assert!((offsets[2] - width * 0.5).abs() < 0.01, "centre is half");
    assert_eq!(offsets[3], 0.0, "left is physical and does not flip");
    assert!((offsets[4] - width).abs() < 0.01, "right is physical too");
    assert!(
        (offsets[5] - width).abs() < 0.01,
        "start is the right edge in an rtl paragraph"
    );
}

#[test]
fn alignment_without_a_container_width_has_nothing_to_align_in() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        "Save",
        &style,
        &TextConstraints {
            align: TextAlignSpec::Center,
            ..TextConstraints::default()
        },
    );
    assert_eq!(layout.lines[0].bounds.x, 0.0);
}

// ---- truncation and ellipsis --------------------------------------------

#[test]
fn max_lines_truncates_and_the_last_line_says_why_it_ended() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            max_lines: Some(2),
            ..wrapped(140.0)
        },
    );
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[1].break_cause, LineBreakCause::MaxLines);
    assert!(layout.overflow.contains(OverflowFlags::TRUNCATED_LINES));
    assert!(!layout.overflow.contains(OverflowFlags::ELLIPSIZED));
}

#[test]
fn a_height_budget_truncates_at_the_last_line_that_fits() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    let tall = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(140.0),
    );
    let line_height = tall.lines[0].metrics.height_px;
    let short = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            max_height_px: Some(line_height * 2.0),
            ..wrapped(140.0)
        },
    );
    assert!(tall.lines.len() > 2);
    assert_eq!(short.lines.len(), 2);
    assert!(short.bounds.height <= line_height * 2.0 + 0.01);
    assert!(short.overflow.contains(OverflowFlags::TRUNCATED_LINES));
}

#[test]
fn an_ellipsis_cuts_on_a_cluster_boundary_and_claims_no_source_bytes() {
    let mut engine = text_engine(UI_AND_EMOJI);
    let style = style(UI_AND_EMOJI, 20.0);
    let text = "family 👩‍💻 and more text after it";
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        text,
        &style,
        &TextConstraints {
            max_width_px: Some(150.0),
            ellipsis: true,
            ..TextConstraints::default()
        },
    );
    assert_eq!(layout.lines.len(), 1);
    let line = &layout.lines[0];
    assert!(layout.overflow.contains(OverflowFlags::ELLIPSIZED));
    assert!(line.metrics.width_px <= 150.0 + 0.01, "the line now fits");
    assert!(
        text.is_char_boundary(line.source.end),
        "the cut is on a character boundary"
    );

    let runs = layout.line_runs(line);
    let ellipsis = runs.last().expect("an ellipsis run");
    assert_eq!(
        ellipsis.source,
        line.source.end..line.source.end,
        "the ellipsis owns no bytes of the source"
    );
    for glyph in &ellipsis.glyphs {
        assert_eq!(glyph.cluster, line.source.end as u32);
        assert_eq!(glyph.cluster_end, line.source.end as u32);
    }
    for run in &runs[..runs.len() - 1] {
        for glyph in &run.glyphs {
            assert!(
                glyph.cluster_end as usize <= line.source.end,
                "a surviving cluster is whole"
            );
        }
    }
    assert_eq!(engine.layout_counters().ellipsis_runs_used, 1);
}

#[test]
fn an_ellipsis_never_splits_the_grapheme_cluster_it_lands_in() {
    let mut engine = text_engine(UI_AND_EMOJI);
    let style = style(UI_AND_EMOJI, 20.0);
    // The ZWJ sequence is one cluster of eleven bytes. Every width that cuts
    // near it must either keep all of it or none of it.
    let text = "ab👩‍💻cd";
    let emoji = 2..13;
    for width in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0] {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            text,
            &style,
            &TextConstraints {
                max_width_px: Some(width),
                ellipsis: true,
                ..TextConstraints::default()
            },
        );
        let end = layout.lines[0].source.end;
        assert!(
            !(emoji.start < end && end < emoji.end),
            "width {width} cut the ZWJ sequence at byte {end}"
        );
    }
}

#[test]
fn the_ellipsis_is_shaped_once_however_many_labels_truncate() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let constraints = TextConstraints {
        max_width_px: Some(60.0),
        ellipsis: true,
        ..TextConstraints::default()
    };
    for _ in 0..10_000 {
        lay_out(
            &mut engine,
            TextKind::Label,
            "a long label that will not fit",
            &style,
            &constraints,
        );
    }
    let shape = engine.shape_counters();
    assert_eq!(
        shape.shape_cache_misses, 2,
        "the label and the ellipsis, once each"
    );
    assert_eq!(engine.layout_counters().layout_created, 1);
}

#[test]
fn truncating_with_an_ellipsis_marks_both_the_truncation_and_the_ellipsis() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            max_lines: Some(2),
            ellipsis: true,
            ..wrapped(140.0)
        },
    );
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[1].break_cause, LineBreakCause::MaxLines);
    assert!(layout.overflow.contains(OverflowFlags::TRUNCATED_LINES));
    assert!(layout.overflow.contains(OverflowFlags::ELLIPSIZED));
    assert!(layout.lines[1].metrics.width_px <= 140.0 + 0.01);
    let last = layout.line_runs(&layout.lines[1]).last().expect("runs");
    assert!(last.source.is_empty(), "the last run is the ellipsis");
}

// ---- intrinsic widths ----------------------------------------------------

#[test]
fn intrinsic_widths_report_the_longest_word_and_the_unwrapped_paragraph() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown Hamburgefonstiv";
    let source = TextSource::new(text);
    let widths = engine.intrinsic_widths(&source, &style, &TextConstraints::default());
    assert!(widths.min_px > 0.0 && widths.min_px < widths.max_px);

    let unwrapped = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints::default(),
    );
    assert!(
        (unwrapped.lines[0].metrics.width_px - widths.max_px).abs() < 0.01,
        "max-content is the width it takes when it never wraps"
    );

    let at_min = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(widths.min_px),
    );
    assert!(
        !at_min.overflow.contains(OverflowFlags::CLIPPED_WIDTH),
        "min-content is wide enough for every unbreakable piece"
    );
}

// ---- writing mode --------------------------------------------------------

#[test]
fn a_vertical_writing_mode_is_fail_closed_and_says_so_on_the_layout() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let horizontal = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "one\ntwo",
        &style,
        &TextConstraints {
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    assert!(!horizontal.unsupported_writing_mode);

    let vertical = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "one\ntwo",
        &style,
        &TextConstraints {
            preserve_lines: true,
            writing_mode: WritingModeSpec::VerticalRl,
            ..TextConstraints::default()
        },
    );
    assert!(
        vertical.unsupported_writing_mode,
        "vertical writing is not implemented and must not pretend otherwise"
    );
    assert_eq!(engine.layout_counters().vertical_writing_fallbacks, 1);
    assert!(
        vertical.lines[1].metrics.top_y_px > vertical.lines[0].metrics.top_y_px,
        "the fallback geometry is the horizontal one, plainly"
    );
}

// ---- the cache -----------------------------------------------------------

#[test]
fn the_layout_cache_is_bounded_and_evicts_the_least_recently_used() {
    let mut fonts = fonts(UI);
    let style = style(UI, 16.0);
    let mut shaper = Shaper::default();
    let mut layouter = Layouter::new(LayoutCacheBudget {
        max_entries: 2,
        max_bytes: 1 << 20,
    });
    let source = TextSource::new("the quick brown fox");
    let widths = [400.0, 300.0, 200.0];
    for width in widths {
        let constraints = wrapped(width);
        let shaped = shaper.shape(
            &mut fonts,
            &ShapeRequest::new(&source, &style, &constraints),
        );
        layouter.layout(&LayoutRequest::new(
            TextKind::Paragraph,
            &source,
            &shaped,
            &style,
            &constraints,
        ));
    }
    assert_eq!(layouter.counters().layout_cache_entries, 2);
    assert_eq!(layouter.counters().layout_cache_evictions, 1);
    assert!(layouter.counters().layout_cache_bytes > 0);

    let constraints = wrapped(widths[0]);
    let shaped = shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &style, &constraints),
    );
    layouter.layout(&LayoutRequest::new(
        TextKind::Paragraph,
        &source,
        &shaped,
        &style,
        &constraints,
    ));
    assert_eq!(
        layouter.counters().layout_cache_misses,
        4,
        "the first width was evicted and had to be laid out again"
    );
}

#[test]
fn a_font_change_makes_an_earlier_layout_stale() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        "Save",
        &style,
        &TextConstraints::default(),
    );
    let source = TextSource::new("Save");
    assert!(!layout.is_stale(source.revision(), engine.font_generation()));

    engine
        .fonts_mut()
        .register_bytes(fixture_bytes("noto-emoji"), &FaceDescriptor::default())
        .expect("fixture registers");
    assert!(
        layout.is_stale(source.revision(), engine.font_generation()),
        "a new face is a new generation, and the old layout is stale"
    );
}

// ---- the counters describe the layout they produced ----------------------

#[test]
fn the_counters_agree_with_the_layouts_they_describe() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let mut counters = TextWorkCounters::default();
    let mut lines = 0;
    let mut runs = 0;
    let mut glyphs = 0;
    for index in 0..8 {
        let source = TextSource::new(format!("paragraph number {index} wraps a few times"));
        let layout = engine.layout(
            TextKind::Paragraph,
            &source,
            &style,
            &wrapped(120.0),
            &mut counters,
        );
        lines += layout.lines.len();
        runs += layout.runs.len();
        glyphs += layout.glyph_count();
    }

    let layout = engine.layout_counters();
    assert_eq!(layout.lines_created, lines);
    assert_eq!(layout.runs_placed, runs);
    assert_eq!(layout.layout_created, 8);
    assert_eq!(counters.text_nodes_considered, 8);
    assert_eq!(counters.text_nodes_shaped, 8);
    assert_eq!(counters.layout_cache_misses, Some(8));
    assert_eq!(counters.layout_cache_hits, Some(0));
    assert_eq!(counters.glyphs_resolved, Some(glyphs));
}

// ---- the IR still answers caret questions --------------------------------

#[test]
fn caret_and_hit_test_answers_come_straight_off_a_native_layout() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "the quick brown fox jumps over the lazy dog";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(140.0),
    );
    let second = &layout.lines[1];
    let hit = layout.hit_test(
        second.bounds.x + 1.0,
        second.metrics.top_y_px + second.metrics.height_px * 0.5,
    );
    assert!(hit.inside);
    assert_eq!(hit.caret.line, 1);
    assert!(second.source.contains(&hit.caret.byte));

    let caret = layout
        .caret_geometry(hit.caret)
        .expect("the caret resolves on its own line");
    assert!((caret.top_y_px - second.metrics.top_y_px).abs() < 0.001);
    assert!((caret.height_px - second.metrics.height_px).abs() < 0.001);

    let rects = layout.selection_rects(0..text.len());
    assert!(
        rects.len() >= layout.lines.len(),
        "a selection covering everything touches every line"
    );
}
