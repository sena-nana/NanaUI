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
    Affinity, CaretPosition, LineBreakCause, NativeTextEngine, OverflowFlags, RunOrientation,
    TextConstraints, TextEngine, TextKind, TextLayout, TextSource, TextStyle, TextWorkCounters,
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

/// Asserts every character a wrapped layout did not deliberately drop is on
/// some line.
///
/// Trailing whitespace hangs at a soft wrap, so whitespace is exempt; anything
/// else missing is a byte range nothing can draw, select or hit-test, and no
/// flag reports it.
fn assert_every_character_is_on_a_line(layout: &TextLayout, text: &str) {
    for (offset, character) in text.char_indices() {
        if character.is_whitespace() {
            continue;
        }
        assert!(
            layout
                .lines
                .iter()
                .any(|line| line.source.start <= offset && offset < line.source.end),
            "byte {offset} ({character:?}) is on no line: {:?}",
            layout
                .lines
                .iter()
                .map(|line| line.source.clone())
                .collect::<Vec<_>>()
        );
    }
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
fn a_wrapping_paragraph_keeps_every_byte_even_when_a_word_overflows() {
    // An unbreakable word wider than the box is already whole on its own line.
    // Cutting it for an ellipsis would drop the rest of it from every line at
    // once, leaving a hole in the middle of the paragraph that no flag reports.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "start Hamburgefonstivwordthatislong end of story";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            ellipsis: true,
            ..wrapped(90.0)
        },
    );
    assert_every_character_is_on_a_line(&layout, text);
    assert!(
        layout.overflow.contains(OverflowFlags::CLIPPED_WIDTH),
        "the long word sticks out, and says so"
    );
    assert!(
        !layout.overflow.contains(OverflowFlags::ELLIPSIZED),
        "nothing was replaced by an ellipsis"
    );
    assert_eq!(engine.layout_counters().ellipsis_runs_used, 0);
}

#[test]
fn trailing_whitespace_hangs_instead_of_overflowing_the_box() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let exact = lay_out(
        &mut engine,
        TextKind::Label,
        "Save",
        &style,
        &TextConstraints::default(),
    )
    .lines[0]
        .metrics
        .width_px;

    let clipped = lay_out(
        &mut engine,
        TextKind::Label,
        "Save ",
        &style,
        &TextConstraints {
            max_width_px: Some(exact),
            ..TextConstraints::default()
        },
    );
    assert!(
        !clipped.overflow.contains(OverflowFlags::CLIPPED_WIDTH),
        "a hanging trailing space is not an overflow"
    );

    let ellipsized = lay_out(
        &mut engine,
        TextKind::Label,
        "Save ",
        &style,
        &TextConstraints {
            max_width_px: Some(exact),
            ellipsis: true,
            ..TextConstraints::default()
        },
    );
    assert!(
        !ellipsized.overflow.contains(OverflowFlags::ELLIPSIZED),
        "and it must not cost the label its last letter"
    );
    assert_eq!(ellipsized.lines[0].source, 0..5);
}

#[test]
fn empty_text_is_one_line_on_either_path() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let label = lay_out(
        &mut engine,
        TextKind::Label,
        "",
        &style,
        &TextConstraints::default(),
    );
    let paragraph = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "",
        &style,
        &wrapped(200.0),
    );

    for layout in [&label, &paragraph] {
        assert_eq!(layout.lines.len(), 1, "an empty field is still a line box");
        assert_eq!(layout.lines[0].source, 0..0);
        assert!(layout.lines[0].metrics.height_px > 0.0);
        assert!(layout.bounds.height > 0.0);
        let hit = layout.hit_test(0.0, layout.lines[0].metrics.height_px * 0.5);
        assert_eq!(hit.caret.byte, 0);
        assert!(
            layout.caret_geometry(hit.caret).is_some(),
            "a caret has somewhere to go"
        );
    }
    assert_eq!(
        label.lines[0].metrics.height_px, paragraph.lines[0].metrics.height_px,
        "the two paths agree on how tall an empty line is"
    );
}

#[test]
fn a_line_cut_twice_still_reports_one_ellipsis() {
    // The first paragraph overflows and is ellipsized; the second hits the line
    // budget, which re-places that same line with a fresh ellipsis. The
    // counters have to describe the layout that came out, not both attempts.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "a long first line\nand a second one",
        &style,
        &TextConstraints {
            max_width_px: Some(60.0),
            max_lines: Some(1),
            ellipsis: true,
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    let placed = layout
        .runs
        .iter()
        .filter(|run| run.source.is_empty())
        .count();
    assert_eq!(placed, 1, "one ellipsis in the layout");
    assert_eq!(
        engine.layout_counters().ellipsis_runs_used,
        placed,
        "and one in the counter"
    );
    assert!(layout.overflow.contains(OverflowFlags::TRUNCATED_LINES));
    assert!(layout.overflow.contains(OverflowFlags::ELLIPSIZED));
}

#[test]
fn a_unicode_line_separator_ends_a_line_and_draws_nothing() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "one\u{2028}two";
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
    assert_eq!(line_texts(&layout, text), vec!["one", "two"]);
    assert_eq!(layout.lines[0].break_cause, LineBreakCause::Explicit);
    assert_eq!(
        layout.glyph_count(),
        6,
        "the separator itself is not drawn: {:?}",
        layout.runs
    );

    // A label carrying one is no longer a single line, so it takes the
    // paragraph path like an authored newline does.
    let label = lay_out(
        &mut engine,
        TextKind::Label,
        text,
        &style,
        &TextConstraints::default(),
    );
    assert_eq!(label.lines.len(), 2);
    assert_eq!(engine.layout_counters().label_fast_paths, 0);
}

#[test]
fn a_truncated_empty_line_keeps_its_own_byte() {
    // The last line has no cells to read a byte from, so re-placing it with an
    // ellipsis has to use the byte it was placed at. Re-deriving one from the
    // cells lands on byte 0, which puts the line before the one above it.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "a\n\n\n",
        &style,
        &TextConstraints {
            max_lines: Some(2),
            ellipsis: true,
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(
        layout.lines[1].source,
        2..2,
        "the empty line starts where it is"
    );
    assert!(
        layout.lines[0].source.end <= layout.lines[1].source.start,
        "line sources only move forwards: {:?}",
        layout
            .lines
            .iter()
            .map(|line| line.source.clone())
            .collect::<Vec<_>>()
    );
    let hit = layout.hit_test(
        0.0,
        layout.lines[1].metrics.top_y_px + layout.lines[1].metrics.height_px * 0.5,
    );
    assert_eq!(hit.caret.byte, 2, "and hit-testing it lands there too");
}

#[test]
fn a_layout_carries_the_revision_of_the_source_that_asked_for_it() {
    // Layouts are shared by content, but `is_stale` compares the revision the
    // holder is at. A layout handed to a second source still claiming the
    // first one's revision would read as stale on every frame, forever.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let mut counters = TextWorkCounters::default();
    let fresh = TextSource::new("Save");
    let mut edited = TextSource::new("draft");
    edited.set_text("Save");
    edited.set_text("Save");
    assert_ne!(fresh.revision(), edited.revision());

    let constraints = TextConstraints::default();
    let first = engine.layout(TextKind::Label, &fresh, &style, &constraints, &mut counters);
    let second = engine.layout(
        TextKind::Label,
        &edited,
        &style,
        &constraints,
        &mut counters,
    );

    assert_eq!(first.revision, fresh.revision());
    assert_eq!(second.revision, edited.revision());
    for (layout, source) in [(&first, &fresh), (&second, &edited)] {
        assert!(
            !layout.is_stale(source.revision(), engine.font_generation()),
            "a layout just built for a source is not stale for it"
        );
    }
    assert_eq!(
        engine.shape_counters().shape_cache_misses,
        1,
        "the shaping underneath is still shared, which is where the work is"
    );
}

#[test]
fn max_content_is_the_widest_line_the_text_can_produce() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    for (text, constraints) in [
        (
            "aaaa\u{2028}bbbbbbbb",
            TextConstraints {
                preserve_lines: true,
                ..TextConstraints::default()
            },
        ),
        (
            "aaaa\nbbbbbbbb",
            TextConstraints {
                preserve_lines: true,
                ..TextConstraints::default()
            },
        ),
    ] {
        let source = TextSource::new(text);
        let widths = engine.intrinsic_widths(&source, &style, &constraints);
        let layout = lay_out(&mut engine, TextKind::Paragraph, text, &style, &constraints);
        let widest = layout
            .lines
            .iter()
            .map(|line| line.metrics.width_px)
            .fold(0.0_f32, f32::max);
        assert!(layout.lines.len() > 1, "{text:?} is more than one line");
        assert!(
            (widths.max_px - widest).abs() < 0.01,
            "max-content {} is not the widest line {widest} of {text:?}",
            widths.max_px
        );
    }
}

#[test]
fn trailing_whitespace_hangs_out_of_the_alignment_too() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let mut origin_of = |text: &str, align| {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            text,
            &style,
            &TextConstraints {
                max_width_px: Some(400.0),
                align,
                ..TextConstraints::default()
            },
        );
        layout.line_runs(&layout.lines[0])[0].origin_x_px
    };
    for align in [TextAlignSpec::Center, TextAlignSpec::End] {
        let plain = origin_of("Save", align);
        let padded = origin_of("Save   ", align);
        assert!(
            (plain - padded).abs() < 0.01,
            "{align:?}: a hung trailing space moved the text by {} px",
            padded - plain
        );
    }
}

#[test]
fn a_trailing_newline_leaves_a_line_for_the_caret() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "abc\n";
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
    assert_eq!(layout.lines.len(), 2, "pressing Enter leaves a line behind");
    assert_eq!(layout.lines[1].source, 4..4);
    let caret = CaretPosition::new(text.len(), Affinity::Downstream, 1);
    let geometry = layout
        .caret_geometry(caret)
        .expect("the caret after the newline has somewhere to go");
    assert!((geometry.top_y_px - layout.lines[1].metrics.top_y_px).abs() < 0.001);
}

#[test]
fn a_caret_inside_hung_whitespace_sits_at_the_edge_it_hangs_from() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "hello   world";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(50.0),
    );
    assert_eq!(layout.lines.len(), 2);
    let gap = layout.lines[0].source.end..layout.lines[1].source.start;
    assert!(!gap.is_empty(), "the wrap hung some whitespace");

    let first = layout.lines[0].source.end;
    let end_of_line = layout
        .caret_geometry(CaretPosition::new(first, Affinity::Upstream, 0))
        .expect("the line's own end resolves");
    for byte in gap {
        let geometry = layout
            .caret_geometry(CaretPosition::new(byte, Affinity::Upstream, 0))
            .unwrap_or_else(|| panic!("byte {byte} hangs from line 0 and has no caret"));
        assert!(
            (geometry.x_px - end_of_line.x_px).abs() < 0.001,
            "a caret in hung whitespace sits at the end of the line it hangs from"
        );
        assert!(
            layout
                .caret_geometry(CaretPosition::new(byte, Affinity::Downstream, 1))
                .is_some(),
            "and at the start of the next line when the caret names that one"
        );
    }
}

#[test]
fn folding_newlines_costs_one_copy_and_one_hash_per_revision() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let mut counters = TextWorkCounters::default();
    let source = TextSource::new("one\ntwo three four five");
    let constraints = wrapped(200.0);
    for _ in 0..10 {
        engine.layout(
            TextKind::Paragraph,
            &source,
            &style,
            &constraints,
            &mut counters,
        );
    }
    assert_eq!(
        engine.shape_counters().text_bytes_hashed,
        source.text().len(),
        "the folded reading is hashed once, not once per frame"
    );
    assert_eq!(engine.layout_counters().layout_created, 1);
}

#[test]
fn a_span_starting_inside_a_cluster_sizes_the_line_box_it_shaped_into() {
    // The shaper snaps a span boundary back to the start of the grapheme
    // cluster it lands in, because it cannot split one. Layout has to resolve
    // the same way, or the run shapes at the span's size and is measured into a
    // line box sized from the base style.
    let mut engine = text_engine(UI);
    let base = style(UI, 16.0);
    let big = TextStyle {
        font_size_px: 40.0,
        line_height: Some(LineHeightSpec::Absolute(80.0)),
        ..style(UI, 40.0)
    };
    // `e` + combining acute is one cluster of three bytes; the span starts at
    // byte 1, inside it.
    let mut source = TextSource::new("e\u{301}x");
    source.set_spans(vec![nana_text::TextSpan {
        range: 1..4,
        style: big,
        composition: None,
    }]);
    let mut counters = TextWorkCounters::default();
    let layout = engine.layout(
        TextKind::Label,
        &source,
        &base,
        &TextConstraints::default(),
        &mut counters,
    );

    let line = &layout.lines[0];
    let tallest = layout
        .line_runs(line)
        .iter()
        .map(|run| run.metrics.ascent_px + run.metrics.descent_px)
        .fold(0.0_f32, f32::max);
    assert!(
        line.metrics.height_px >= tallest,
        "the line box ({}) is smaller than the run it holds ({tallest})",
        line.metrics.height_px
    );
    assert!(
        (line.metrics.height_px - 80.0).abs() < 0.001,
        "the span's own line height sizes the line: {:?}",
        line.metrics
    );
}

#[test]
fn a_trailing_newline_degrades_a_label_to_the_paragraph_path() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "abc\n";
    let layout = lay_out(
        &mut engine,
        TextKind::Label,
        text,
        &style,
        &TextConstraints {
            preserve_lines: true,
            ..TextConstraints::default()
        },
    );
    assert_eq!(
        layout.lines.len(),
        2,
        "a trailing newline adds no paragraph, but it does add a line"
    );
    assert_eq!(engine.layout_counters().label_fast_paths, 0);
    assert!(
        layout
            .caret_geometry(CaretPosition::new(text.len(), Affinity::Downstream, 1))
            .is_some(),
        "the caret after the newline has somewhere to go on either path"
    );
}

#[test]
fn an_overflowing_line_aligns_past_the_start_edge_instead_of_snapping_back() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let container = 20.0;
    let mut bounds_of = |align| {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            "Hamburgefonstiv",
            &style,
            &TextConstraints {
                max_width_px: Some(container),
                align,
                ..TextConstraints::default()
            },
        );
        layout.lines[0].bounds
    };

    let start = bounds_of(TextAlignSpec::Start);
    assert_eq!(start.x, 0.0, "start still starts at the start edge");

    let end = bounds_of(TextAlignSpec::End);
    assert!(
        end.x < 0.0,
        "an overflowing line hangs off the start edge rather than snapping back"
    );
    assert!(
        (end.right() - container).abs() < 0.01,
        "so a clipped container shows the end the caller aligned to"
    );

    let centre = bounds_of(TextAlignSpec::Center);
    assert!((centre.x - (container - centre.width) * 0.5).abs() < 0.01);
}

#[test]
fn a_separator_too_long_to_fold_stays_a_line_break() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    // NEL is two bytes and U+2028 three, so neither can become a one-byte
    // space without moving every offset after it. They break regardless of
    // `preserve_lines`, and the doc on `with_folded_newlines` says so.
    for text in ["one\u{85}two", "one\u{2028}two"] {
        let layout = lay_out(
            &mut engine,
            TextKind::Paragraph,
            text,
            &style,
            &TextConstraints::default(),
        );
        assert_eq!(layout.lines.len(), 2, "{text:?} is two lines either way");
    }
    let folded = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "one\ntwo",
        &style,
        &TextConstraints::default(),
    );
    assert_eq!(folded.lines.len(), 1, "a one-byte separator does fold");
}

/// The rightmost edge any glyph of the line actually reaches.
fn drawn_right_edge(layout: &TextLayout, line: &nana_text::LineBox) -> f32 {
    layout
        .line_runs(line)
        .iter()
        .map(|run| run.origin_x_px + run.advance_px)
        .fold(f32::NEG_INFINITY, f32::max)
}

#[test]
fn leading_whitespace_does_not_open_a_paragraph_with_a_blank_line() {
    // There is a break opportunity after the leading run of spaces. Taking it
    // would put nothing on the first line and leave the whitespace with nowhere
    // to go.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "  ab cd";
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &wrapped(20.0),
    );
    for line in &layout.lines {
        assert!(
            !layout.line_runs(line).is_empty(),
            "line {} draws nothing: {:?}",
            line.index,
            line_texts(&layout, text)
        );
    }
    assert_every_character_is_on_a_line(&layout, text);

    // The same text truncated to one line must keep what fits rather than
    // ellipsizing a line that was never there. The container is measured, not
    // guessed: room for `"  ab"` and the ellipsis, and less than the whole
    // string needs.
    let mut width_of = |text: &str| {
        lay_out(
            &mut engine,
            TextKind::Label,
            text,
            &style,
            &TextConstraints::default(),
        )
        .lines[0]
            .metrics
            .width_px
    };
    let whole = width_of(text);
    let container = width_of("  ab") + width_of("…") + 1.0;
    assert!(container < whole, "the text still has to wrap");

    let truncated = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            max_lines: Some(1),
            ellipsis: true,
            ..wrapped(container)
        },
    );
    assert_eq!(
        truncated.lines[0].source,
        0..4,
        "the first line kept `  ab` and the ellipsis followed it"
    );
    assert!(
        truncated.glyph_count() > 1,
        "the whole string vanished behind the ellipsis"
    );
}

#[test]
fn an_ellipsis_stays_inside_the_box_when_the_line_ends_in_whitespace() {
    // The cut line ends in whitespace that hangs, so the ellipsis has to start
    // where the last drawn cluster ended. Placing it after the hung glyphs
    // draws it past the width the line reports — outside the container, with
    // nothing saying so.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let container = 200.0;
    for align in [
        TextAlignSpec::Start,
        TextAlignSpec::Center,
        TextAlignSpec::End,
    ] {
        let layout = lay_out(
            &mut engine,
            TextKind::Paragraph,
            "ab   \ncd",
            &style,
            &TextConstraints {
                max_width_px: Some(container),
                max_lines: Some(1),
                ellipsis: true,
                preserve_lines: true,
                align,
                ..TextConstraints::default()
            },
        );
        let line = &layout.lines[0];
        let drawn = drawn_right_edge(&layout, line);
        assert!(
            drawn <= line.bounds.right() + 0.01,
            "{align:?}: glyphs reach {drawn} but the line box ends at {}",
            line.bounds.right()
        );
        assert!(
            drawn <= container + 0.01,
            "{align:?}: glyphs reach {drawn}, outside the {container} px container"
        );
    }
}

#[test]
fn a_record_separator_ends_a_line_and_is_never_drawn() {
    // UBA ends a paragraph at U+001C–U+001E although UAX #14 does not break
    // there. Layout follows the paragraph structure, so shaping has to drop
    // them too — otherwise the separator draws a .notdef box on the line it
    // just ended.
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let plain = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "abcd",
        &style,
        &TextConstraints::default(),
    );
    for separator in ['\u{1c}', '\u{1d}', '\u{1e}'] {
        let text = format!("ab{separator}cd");
        let layout = lay_out(
            &mut engine,
            TextKind::Paragraph,
            &text,
            &style,
            &TextConstraints {
                preserve_lines: true,
                ..TextConstraints::default()
            },
        );
        assert_eq!(
            layout.lines.len(),
            2,
            "U+{:04X} ends a line",
            separator as u32
        );
        assert_eq!(layout.lines[0].source, 0..2, "and is not part of it");
        assert_eq!(
            layout.glyph_count(),
            plain.glyph_count(),
            "U+{:04X} drew a glyph of its own",
            separator as u32
        );

        // Folded, it is one byte and becomes a space like `\n` does.
        let folded = lay_out(
            &mut engine,
            TextKind::Paragraph,
            &text,
            &style,
            &TextConstraints::default(),
        );
        assert_eq!(folded.lines.len(), 1);
        assert_eq!(folded.glyph_count(), plain.glyph_count() + 1);
    }
}

#[test]
fn trailing_whitespace_hangs_off_the_start_edge_of_an_rtl_line() {
    // Rule L1 puts a line's trailing whitespace at the paragraph's end, which
    // is the visual *left* in an RTL paragraph. It is drawn but not measured,
    // so it has to hang outside the aligned box — drawn inside it, it would
    // push every real glyph off the other edge.
    let mut engine = text_engine(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let container = 200.0;
    let mut line_of = |text: &str| {
        let layout = lay_out(
            &mut engine,
            TextKind::Label,
            text,
            &style,
            &TextConstraints {
                max_width_px: Some(container),
                base_direction: DirSpec::Rtl,
                ..TextConstraints::default()
            },
        );
        let line = layout.lines[0].clone();
        (line.bounds, drawn_right_edge(&layout, &line))
    };

    let (plain_bounds, plain_edge) = line_of("عربي");
    let (padded_bounds, padded_edge) = line_of("عربي ");
    assert!(
        (plain_bounds.x - padded_bounds.x).abs() < 0.01,
        "the hung space moved the line box: {plain_bounds:?} vs {padded_bounds:?}"
    );
    assert!(
        padded_edge <= container + 0.01,
        "glyphs reach {padded_edge}, past the {container} px edge they were aligned to"
    );
    assert!(
        (plain_edge - padded_edge).abs() < 0.01,
        "the word itself moved: {plain_edge} vs {padded_edge}"
    );
}

#[test]
fn a_caret_at_the_wrap_point_of_an_rtl_line_stays_on_that_line() {
    // The ambiguous byte is the line's *logical* end — also the next line's
    // start — and on an RTL line that is the visually left edge. Deciding the
    // affinity by which edge was clicked hands "stay on the line above" to the
    // logical start, where there is no line above.
    let mut engine = text_engine(UI_AND_ARABIC);
    let style = style(UI_AND_ARABIC, 20.0);
    let layout = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "عربي عربي عربي عربي",
        &style,
        &TextConstraints {
            base_direction: DirSpec::Rtl,
            ..wrapped(120.0)
        },
    );
    assert!(layout.lines.len() > 1);
    let first = &layout.lines[0];
    assert_eq!(first.break_cause, LineBreakCause::Wrap);
    let middle_y = first.metrics.top_y_px + first.metrics.height_px * 0.5;

    let past_left = layout.hit_test(first.bounds.x - 5.0, middle_y);
    assert_eq!(
        past_left.caret.byte, first.source.end,
        "the visually-left edge of an RTL line is its logical end"
    );
    assert_eq!(
        past_left.caret.affinity,
        Affinity::Upstream,
        "and a caret dropped there stays on the line that wrapped"
    );

    let past_right = layout.hit_test(first.bounds.right() + 5.0, middle_y);
    assert_eq!(past_right.caret.byte, first.source.start);
    assert_eq!(
        past_right.caret.affinity,
        Affinity::Downstream,
        "the logical start of the first line has no line above it"
    );
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
    assert_eq!(
        short.lines[1].break_cause,
        LineBreakCause::MaxHeight,
        "the budget that ran out was the height, and `max_lines` was never set"
    );
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

/// #59: `vertical-rl` lays CJK out upright down a column and Latin sideways,
/// each with an advance along the line.
#[test]
fn a_vertical_line_stands_cjk_upright_and_lays_latin_on_its_side() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    // Horizontal first: the vertical shaping below must not be answered from
    // this entry of the shape cache.
    let horizontal = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "中文ab",
        &style,
        &TextConstraints::default(),
    );
    assert!(!horizontal.is_vertical());
    assert!(
        horizontal
            .runs
            .iter()
            .all(|run| run.orientation == RunOrientation::Horizontal)
    );

    let vertical = lay_out(
        &mut engine,
        TextKind::Paragraph,
        "中文ab",
        &style,
        &TextConstraints {
            writing_mode: WritingModeSpec::VerticalRl,
            ..TextConstraints::default()
        },
    );
    assert!(vertical.is_vertical());
    assert!(!vertical.unsupported_writing_mode);
    assert_eq!(engine.layout_counters().vertical_writing_fallbacks, 0);
    let orientations: Vec<_> = vertical.runs.iter().map(|run| run.orientation).collect();
    assert_eq!(
        orientations,
        [RunOrientation::Upright, RunOrientation::Sideways]
    );
    // An upright ideograph advances one em down the column (the face's
    // `vmtx`), never a negative or zero length a line breaker would misread.
    for glyph in &vertical.runs[0].glyphs {
        assert!(
            (glyph.advance_px - 16.0).abs() < 0.5,
            "upright advance is the vertical one: {glyph:?}"
        );
    }
    // Sideways Latin keeps its horizontal advances.
    assert_eq!(
        vertical.runs[1].advance_px, horizontal.runs[1].advance_px,
        "a sideways run is the horizontal shaping, turned"
    );
    assert_eq!(vertical.runs[1].origin_x_px, vertical.runs[0].advance_px);

    // On the page the column is one line box wide and as tall as its text.
    let (width, height) = vertical.physical_size();
    assert!((width - 16.0 * 1.2).abs() < 0.01, "one column: {width}");
    let length = vertical.runs.iter().map(|run| run.advance_px).sum::<f32>();
    assert!((height - length).abs() < 0.01, "{height} vs {length}");
    // The baseline is the column's centre line.
    let line = &vertical.lines[0];
    assert!(
        (line.metrics.baseline_y_px - (line.metrics.top_y_px + line.metrics.height_px / 2.0)).abs()
            < 0.01
    );
}

/// #59: the box's **height** is the line budget of a vertical paragraph and
/// its width only anchors the columns; `vertical-rl` stacks them from the
/// right edge.
#[test]
fn a_vertical_paragraph_wraps_against_the_box_height_and_stacks_columns_right_to_left() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let text = "中文中文中文";
    let constraints = TextConstraints {
        // Wide enough for all six on one horizontal line: if this were the
        // line budget, nothing would wrap.
        max_width_px: Some(200.0),
        max_height_px: Some(40.0),
        wrap: Some(TextWrapBreak::Word),
        writing_mode: WritingModeSpec::VerticalRl,
        ..TextConstraints::default()
    };
    let laid = lay_out(&mut engine, TextKind::Paragraph, text, &style, &constraints);
    assert_eq!(
        line_texts(&laid, text),
        ["中文", "中文", "中文"],
        "two 16px ideographs fit a 40px column, three do not"
    );
    assert_every_character_is_on_a_line(&laid, text);
    let (width, height) = laid.physical_size();
    assert!(
        (width - 3.0 * 16.0 * 1.2).abs() < 0.01,
        "three columns: {width}"
    );
    assert!(
        height <= 40.0 + 0.01,
        "no column is longer than the box: {height}"
    );

    // Columns stack from the right edge of the box, first column rightmost.
    let centres: Vec<f32> = laid
        .lines
        .iter()
        .map(|line| laid.physical_x_of_block(line.metrics.baseline_y_px, 200.0))
        .collect();
    assert!(
        (centres[0] - (200.0 - 16.0 * 1.2 / 2.0)).abs() < 0.01,
        "{centres:?}"
    );
    assert!(
        centres[0] > centres[1] && centres[1] > centres[2],
        "{centres:?}"
    );

    // `vertical-lr` stacks the same columns from the left edge.
    let lr = lay_out(
        &mut engine,
        TextKind::Paragraph,
        text,
        &style,
        &TextConstraints {
            writing_mode: WritingModeSpec::VerticalLr,
            ..constraints
        },
    );
    let lr_centres: Vec<f32> = lr
        .lines
        .iter()
        .map(|line| lr.physical_x_of_block(line.metrics.baseline_y_px, 200.0))
        .collect();
    assert!(
        (lr_centres[0] - 16.0 * 1.2 / 2.0).abs() < 0.01,
        "{lr_centres:?}"
    );
    assert!(lr_centres[0] < lr_centres[1], "{lr_centres:?}");
}

/// #59: an upright run is shaped top-to-bottom, so the face's `vert` feature
/// swaps in the vertical forms of punctuation. A corner bracket drawn with its
/// horizontal glyph in a column points the wrong way.
#[test]
fn upright_punctuation_takes_its_vertical_form() {
    let mut engine = text_engine(UI);
    let style = style(UI, 16.0);
    let glyph_of = |layout: &TextLayout| layout.runs[0].glyphs[0].glyph_id;
    let horizontal = lay_out(
        &mut engine,
        TextKind::Label,
        "「",
        &style,
        &TextConstraints::default(),
    );
    let vertical = lay_out(
        &mut engine,
        TextKind::Label,
        "「",
        &style,
        &TextConstraints {
            writing_mode: WritingModeSpec::VerticalRl,
            ..TextConstraints::default()
        },
    );
    assert_eq!(vertical.runs[0].orientation, RunOrientation::Upright);
    assert_ne!(
        glyph_of(&horizontal),
        glyph_of(&vertical),
        "the vertical corner bracket is a different glyph"
    );
    assert!(
        engine.layout_counters().label_fast_paths >= 2,
        "a one-line vertical label still takes the fast path"
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

#[test]
fn a_language_change_moves_the_engine_epoch_and_a_repeat_does_not() {
    // `locl` shaping and fallback read the language, so a layout retained under
    // one hint is not current under another.
    let mut engine = text_engine(UI);
    let start = engine.epoch();
    let hans = nana_text::font::LanguageTag::new("zh-Hans").unwrap();
    engine.set_language(Some(hans.clone()));
    let changed = engine.epoch();
    assert_ne!(changed, start);
    assert_eq!(changed.font_generation, start.font_generation);
    engine.set_language(Some(hans));
    assert_eq!(
        engine.epoch(),
        changed,
        "the same hint again changes nothing"
    );
}

/// #59: editable text asked for a vertical writing mode is still laid out
/// horizontally — an editor does not yet move a caret across columns — and
/// **says so** all the way out to the pass counters, where a frame, a devtools
/// panel or a gate can notice it. Every other kind honours the request.
#[test]
fn a_vertical_editor_falls_back_horizontally_and_is_counted_out_to_the_pass() {
    let mut engine = text_engine(&["nana-test-vf"]);
    let source = TextSource::new("AB");
    let style = style(&["nana-test-vf"], 16.0);
    let vertical = TextConstraints {
        max_width_px: Some(200.0),
        writing_mode: WritingModeSpec::VerticalRl,
        ..TextConstraints::default()
    };
    let mut counters = TextWorkCounters::default();
    let laid = engine.layout(
        TextKind::Editable,
        &source,
        &style,
        &vertical,
        &mut counters,
    );
    assert!(
        laid.unsupported_writing_mode,
        "the layout says it could not do what was asked"
    );
    assert!(!laid.is_vertical());
    assert!(
        laid.runs
            .iter()
            .all(|run| run.orientation == RunOrientation::Horizontal),
        "and its geometry is plainly the horizontal one"
    );
    let label = engine.layout(TextKind::Label, &source, &style, &vertical, &mut counters);
    assert!(label.is_vertical(), "a label is not an editor");
    assert_eq!(
        counters.vertical_writing_fallbacks, 1,
        "and the pass carries that out where somebody can see it: {counters:?}"
    );

    // Horizontal asks for nothing it cannot do, so it reports nothing.
    let mut horizontal_counters = TextWorkCounters::default();
    let horizontal = engine.layout(
        TextKind::Label,
        &source,
        &style,
        &TextConstraints {
            max_width_px: Some(200.0),
            ..TextConstraints::default()
        },
        &mut horizontal_counters,
    );
    assert!(!horizontal.unsupported_writing_mode);
    assert_eq!(horizontal_counters.vertical_writing_fallbacks, 0);
}
