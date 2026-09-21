//! Hand-built layouts for tests that must not depend on any engine.
//!
//! Every number here is chosen, not measured, so a failure means the code under
//! test changed rather than that a font did.

#![allow(dead_code)]

pub mod corpus;

use nana_text::{
    FontGeneration, FontId, GlyphFlags, LineBox, LineBreakCause, LineMetrics, OverflowFlags,
    RunDirection, RunMetrics, RunOrientation, ScriptTag, ShapeRunId, ShapedGlyph, ShapedRun,
    TextConstraints, TextKind, TextLayout, TextLayoutId, TextRect, TextRevision,
};

pub const GLYPH_ADVANCE: f32 = 10.0;
pub const LINE_HEIGHT: f32 = 20.0;

pub fn glyph(glyph_id: u32, cluster: u32, cluster_end: u32) -> ShapedGlyph {
    ShapedGlyph {
        glyph_id,
        cluster,
        cluster_end,
        advance_px: GLYPH_ADVANCE,
        advance_y_px: 0.0,
        offset_x_px: 0.0,
        offset_y_px: 0.0,
        flags: GlyphFlags::NONE,
    }
}

/// A glyph with an explicit advance, for zero-width combining marks.
pub fn glyph_with_advance(
    glyph_id: u32,
    cluster: u32,
    cluster_end: u32,
    advance_px: f32,
) -> ShapedGlyph {
    ShapedGlyph {
        advance_px,
        ..glyph(glyph_id, cluster, cluster_end)
    }
}

pub fn run(
    font: u32,
    direction: RunDirection,
    bidi_level: u8,
    source: std::ops::Range<usize>,
    origin_x_px: f32,
    glyphs: Vec<ShapedGlyph>,
) -> ShapedRun {
    let advance_px = glyphs.iter().map(|g| g.advance_px).sum();
    ShapedRun {
        id: ShapeRunId::from_parts(0, 1),
        source,
        direction,
        bidi_level,
        script: ScriptTag::LATIN,
        orientation: RunOrientation::Horizontal,
        font: FontId::from_parts(font, 1),
        font_size_px: 16.0,
        glyphs,
        advance_px,
        origin_x_px,
        metrics: RunMetrics {
            ascent_px: 16.0,
            descent_px: 4.0,
            line_gap_px: 0.0,
        },
        instance: None,
    }
}

pub fn line(
    index: u32,
    source: std::ops::Range<usize>,
    runs: std::ops::Range<u32>,
    break_cause: LineBreakCause,
    base_direction: RunDirection,
    width_px: f32,
) -> LineBox {
    let top = index as f32 * LINE_HEIGHT;
    LineBox {
        index,
        source,
        runs,
        break_cause,
        base_direction,
        metrics: LineMetrics {
            baseline_y_px: top + 16.0,
            top_y_px: top,
            height_px: LINE_HEIGHT,
            ascent_px: 16.0,
            descent_px: 4.0,
            width_px,
        },
        bounds: TextRect::new(0.0, top, width_px, LINE_HEIGHT),
    }
}

pub fn layout(runs: Vec<ShapedRun>, lines: Vec<LineBox>) -> TextLayout {
    let bounds = lines
        .iter()
        .map(|line| line.bounds)
        .reduce(TextRect::union)
        .unwrap_or_default();
    TextLayout {
        id: TextLayoutId::from_parts(0, 1),
        kind: TextKind::Paragraph,
        revision: TextRevision::INITIAL,
        font_generation: FontGeneration::new(1),
        constraints: TextConstraints::default(),
        runs,
        lines,
        bounds,
        overflow: OverflowFlags::NONE,
        unsupported_writing_mode: false,
    }
}

/// `abc` on one LTR line, glyph cells at x = 0, 10, 20.
pub fn latin_single_line() -> TextLayout {
    let runs = vec![run(
        0,
        RunDirection::Ltr,
        0,
        0..3,
        0.0,
        vec![glyph(1, 0, 1), glyph(2, 1, 2), glyph(3, 2, 3)],
    )];
    let lines = vec![line(
        0,
        0..3,
        0..1,
        LineBreakCause::EndOfText,
        RunDirection::Ltr,
        30.0,
    )];
    layout(runs, lines)
}

/// `ab` + a two-byte-per-char RTL word + `cd`, on one line.
///
/// Logical bytes: `ab` = 0..2, RTL = 2..6 (two chars, two bytes each), `cd` =
/// 6..8. Visually the RTL run's glyphs run right-to-left, so its first glyph in
/// visual order carries the *later* cluster.
pub fn mixed_bidi_single_line() -> TextLayout {
    let runs = vec![
        run(
            0,
            RunDirection::Ltr,
            0,
            0..2,
            0.0,
            vec![glyph(1, 0, 1), glyph(2, 1, 2)],
        ),
        run(
            1,
            RunDirection::Rtl,
            1,
            2..6,
            20.0,
            vec![glyph(10, 4, 6), glyph(11, 2, 4)],
        ),
        run(
            0,
            RunDirection::Ltr,
            0,
            6..8,
            40.0,
            vec![glyph(3, 6, 7), glyph(4, 7, 8)],
        ),
    ];
    let lines = vec![line(
        0,
        0..8,
        0..3,
        LineBreakCause::EndOfText,
        RunDirection::Ltr,
        60.0,
    )];
    layout(runs, lines)
}

/// One RTL run of two marked clusters, each a zero-advance mark followed by
/// its base.
///
/// That is the order HarfBuzz emits for RTL: the mark is visually first, so a
/// caret resolved against whichever cell comes first lands on a zero-width
/// cell. Cells are [0,0), [0,10), [10,10), [10,20); clusters run 2..4 then
/// 0..2 because visual order reverses logical order.
pub fn rtl_with_combining_marks() -> TextLayout {
    let runs = vec![run(
        0,
        RunDirection::Rtl,
        1,
        0..4,
        0.0,
        vec![
            glyph_with_advance(90, 2, 4, 0.0),
            glyph_with_advance(20, 2, 4, GLYPH_ADVANCE),
            glyph_with_advance(91, 0, 2, 0.0),
            glyph_with_advance(21, 0, 2, GLYPH_ADVANCE),
        ],
    )];
    let lines = vec![line(
        0,
        0..4,
        0..1,
        LineBreakCause::EndOfText,
        RunDirection::Rtl,
        GLYPH_ADVANCE * 2.0,
    )];
    layout(runs, lines)
}

/// Three RTL glyphs on a line whose text starts well inside the line box, as a
/// centred or indented line does: box x = 0 width 100, run origin x = 40.
///
/// RTL because that is what reaches the line-edge fallback in
/// `caret_geometry`: the visually-left edge of an RTL line is `source.end`,
/// which no cluster covers. On an LTR line the leading byte is inside the first
/// cluster and never gets that far.
pub fn rtl_indented_single_line() -> TextLayout {
    // Visual order reverses logical order, so the leftmost cell is the last
    // byte.
    let runs = vec![run(
        0,
        RunDirection::Rtl,
        1,
        0..3,
        40.0,
        vec![glyph(3, 2, 3), glyph(2, 1, 2), glyph(1, 0, 1)],
    )];
    let lines = vec![line(
        0,
        0..3,
        0..1,
        LineBreakCause::EndOfText,
        RunDirection::Rtl,
        100.0,
    )];
    layout(runs, lines)
}

/// Two soft-wrapped LTR lines: `abc` then `de`.
pub fn wrapped_two_lines() -> TextLayout {
    let runs = vec![
        run(
            0,
            RunDirection::Ltr,
            0,
            0..3,
            0.0,
            vec![glyph(1, 0, 1), glyph(2, 1, 2), glyph(3, 2, 3)],
        ),
        run(
            0,
            RunDirection::Ltr,
            0,
            3..5,
            0.0,
            vec![glyph(4, 3, 4), glyph(5, 4, 5)],
        ),
    ];
    let lines = vec![
        line(0, 0..3, 0..1, LineBreakCause::Wrap, RunDirection::Ltr, 30.0),
        line(
            1,
            3..5,
            1..2,
            LineBreakCause::EndOfText,
            RunDirection::Ltr,
            20.0,
        ),
    ];
    layout(runs, lines)
}
