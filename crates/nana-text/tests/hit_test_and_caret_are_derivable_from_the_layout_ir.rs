//! The IR's own sufficiency check.
//!
//! Caret placement, hit-testing and selection geometry must be derivable from a
//! `TextLayout` alone, with no font access and no engine. If one of these
//! cannot be written, the IR is missing a field.

mod support;

use nana_text::{Affinity, CaretPosition, LineBreakCause, RunDirection, TextRect};
use support::{GLYPH_ADVANCE, LINE_HEIGHT};

#[test]
fn hitting_the_left_half_of_a_glyph_puts_the_caret_before_it() {
    let layout = support::latin_single_line();
    let hit = layout.hit_test(GLYPH_ADVANCE + 1.0, 5.0);
    assert!(hit.inside, "the point is inside the laid-out bounds");
    assert_eq!(
        hit.caret.byte, 1,
        "left half of glyph 1 is the caret before it"
    );

    let hit = layout.hit_test(GLYPH_ADVANCE * 2.0 - 1.0, 5.0);
    assert_eq!(hit.caret.byte, 2, "right half is the caret after it");
}

#[test]
fn a_point_past_the_end_of_a_line_clamps_and_reports_itself_outside() {
    let layout = support::latin_single_line();
    let hit = layout.hit_test(1_000.0, 5.0);
    assert_eq!(hit.caret.byte, 3, "clamped to the end of the text");
    assert!(!hit.inside, "the point was past the laid-out bounds");

    let hit = layout.hit_test(-50.0, 5.0);
    assert_eq!(hit.caret.byte, 0);
    assert!(!hit.inside);
}

#[test]
fn a_caret_dropped_at_a_soft_wrap_stays_on_the_line_it_was_dropped_on() {
    let layout = support::wrapped_two_lines();
    assert_eq!(layout.lines[0].break_cause, LineBreakCause::Wrap);

    let hit = layout.hit_test(1_000.0, 5.0);
    assert_eq!(hit.caret.line, 0);
    assert_eq!(hit.caret.byte, 3);
    assert_eq!(
        hit.caret.affinity,
        Affinity::Upstream,
        "the end of a soft-wrapped line is upstream, or the caret jumps a line"
    );

    let hit = layout.hit_test(0.0, LINE_HEIGHT + 5.0);
    assert_eq!(hit.caret.line, 1);
    assert_eq!(hit.caret.byte, 3);
    assert_eq!(hit.caret.affinity, Affinity::Downstream);
}

#[test]
fn carets_inside_an_rtl_run_advance_leftwards_with_the_logical_offset() {
    let layout = support::mixed_bidi_single_line();

    // The RTL run occupies x = 20..40. Logical bytes 2..4 are drawn rightmost
    // and 4..6 to their left, so a caret moves left as its byte offset grows.
    let at_rtl_start = layout
        .caret_geometry(CaretPosition::new(2, Affinity::Downstream, 0))
        .expect("byte 2 is the logical start of the RTL run");
    assert_eq!(at_rtl_start.direction, RunDirection::Rtl);
    assert_eq!(
        at_rtl_start.x_px, 40.0,
        "the logical start of RTL text is its right edge"
    );

    let between = layout
        .caret_geometry(CaretPosition::new(4, Affinity::Downstream, 0))
        .expect("byte 4 starts the second RTL cluster");
    assert_eq!(
        between.x_px, 30.0,
        "one cluster further into RTL text is one cell further left"
    );

    let at_rtl_end = layout
        .caret_geometry(CaretPosition::new(6, Affinity::Downstream, 0))
        .expect("byte 6 ends the RTL run and starts the trailing LTR run");
    assert_eq!(
        at_rtl_end.x_px, 40.0,
        "byte 6 also starts the trailing LTR run, whose left edge is x = 40"
    );
}

#[test]
fn a_caret_resolves_against_the_glyph_that_draws_its_cluster_not_a_zero_width_mark() {
    let layout = support::rtl_with_combining_marks();

    // Byte 2 opens the cluster drawn at [0, 10); in RTL its logical start is
    // the right edge, 10. Resolving against the mark's [0, 0) cell that
    // precedes it would answer 0 and put the caret a whole base glyph left.
    let marked = layout
        .caret_geometry(CaretPosition::new(2, Affinity::Downstream, 0))
        .expect("byte 2 starts a marked cluster");
    assert_eq!(marked.x_px, GLYPH_ADVANCE);
    assert_eq!(marked.direction, RunDirection::Rtl);

    let first = layout
        .caret_geometry(CaretPosition::new(0, Affinity::Downstream, 0))
        .expect("byte 0 starts the other marked cluster");
    assert_eq!(first.x_px, GLYPH_ADVANCE * 2.0);

    // The end of the text is the visually-left edge, and must stay distinct
    // from byte 2.
    let end = layout
        .caret_geometry(CaretPosition::new(4, Affinity::Downstream, 0))
        .expect("byte 4 is the line's leading edge under an RTL base");
    assert_eq!(end.x_px, 0.0);
    assert!(
        first.x_px > marked.x_px && marked.x_px > end.x_px,
        "RTL carets must move left as the byte offset grows, got {} {} {}",
        first.x_px,
        marked.x_px,
        end.x_px
    );
}

#[test]
fn a_caret_at_either_end_of_a_line_resolves_without_a_glyph_to_land_on() {
    let layout = support::latin_single_line();
    let start = layout
        .caret_geometry(CaretPosition::new(0, Affinity::Downstream, 0))
        .expect("line start resolves");
    assert_eq!(start.x_px, 0.0);
    let end = layout
        .caret_geometry(CaretPosition::new(3, Affinity::Downstream, 0))
        .expect("line end resolves even though no cluster contains byte 3");
    assert_eq!(end.x_px, GLYPH_ADVANCE * 3.0);
    assert_eq!(end.height_px, LINE_HEIGHT);
}

#[test]
fn clicking_left_of_an_indented_line_draws_the_caret_at_the_first_glyph() {
    // `hit_test` decides "past the left end of the line" from the first cell,
    // so `caret_geometry` has to answer with that same edge. Reading the line
    // box's x instead would drop the caret 40 px away from the click that
    // placed it.
    let layout = support::rtl_indented_single_line();
    assert_eq!(layout.lines[0].bounds.x, 0.0, "the line box starts at 0");

    let hit = layout.hit_test(10.0, 5.0);
    assert!(
        hit.inside,
        "x = 10 is inside the line box, left of the text"
    );
    assert_eq!(
        hit.caret.byte, 3,
        "the visually-left edge of an RTL line is the end of its text"
    );

    let caret = layout
        .caret_geometry(hit.caret)
        .expect("the caret hit_test just produced must resolve");
    assert_eq!(
        caret.x_px, 40.0,
        "the caret belongs at the first glyph, not at the line box edge"
    );

    let logical_start = layout
        .caret_geometry(CaretPosition::new(0, Affinity::Downstream, 0))
        .expect("byte 0 sits in the rightmost cell under RTL");
    assert_eq!(logical_start.x_px, 40.0 + GLYPH_ADVANCE * 3.0);
}

#[test]
fn a_caret_naming_a_line_the_layout_does_not_have_resolves_to_nothing() {
    let layout = support::latin_single_line();
    assert!(
        layout
            .caret_geometry(CaretPosition::new(0, Affinity::Downstream, 7))
            .is_none(),
        "an out-of-range line must not be silently clamped onto line 0"
    );
}

#[test]
fn selecting_across_a_bidi_boundary_yields_disjoint_rectangles() {
    let layout = support::mixed_bidi_single_line();

    // Bytes 1..7 cover the tail of the first LTR run, the whole RTL run, and
    // the head of the last LTR run — visually contiguous, so one rect.
    let rects = layout.selection_rects(1..7);
    assert_eq!(rects.len(), 1, "visually contiguous selection is one rect");
    assert_eq!(rects[0], TextRect::new(10.0, 0.0, 40.0, LINE_HEIGHT));

    // Bytes 0..1 and 6..8 are visually separated by the RTL run.
    let split = layout.selection_rects(6..8);
    assert_eq!(split.len(), 1);
    assert_eq!(split[0], TextRect::new(40.0, 0.0, 20.0, LINE_HEIGHT));
}

#[test]
fn selecting_across_a_wrap_yields_one_rectangle_per_line() {
    let layout = support::wrapped_two_lines();
    let rects = layout.selection_rects(1..5);
    assert_eq!(rects.len(), 2, "one rect per line");
    assert_eq!(rects[0], TextRect::new(10.0, 0.0, 20.0, LINE_HEIGHT));
    assert_eq!(rects[1], TextRect::new(0.0, LINE_HEIGHT, 20.0, LINE_HEIGHT));
}

#[test]
fn an_empty_selection_range_selects_nothing() {
    let layout = support::latin_single_line();
    assert!(layout.selection_rects(2..2).is_empty());
    // A reversed range is a caller bug, not a selection; it must not be read as
    // if the ends had been swapped.
    #[allow(clippy::reversed_empty_ranges)]
    let reversed = 3..1;
    assert!(layout.selection_rects(reversed).is_empty());
}
