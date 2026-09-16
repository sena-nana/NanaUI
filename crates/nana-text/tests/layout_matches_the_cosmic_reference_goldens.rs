//! Issue #92: the native layout engine against the Phase 0 cosmic-text
//! goldens.
//!
//! Phase 2 compared shaping per logical cluster, deliberately independent of
//! where either engine cut its runs or its lines. This is the other half: the
//! same 26 corpus cases, laid out natively and run through
//! [`compare_golden`] — the one structural diff, the same goldens, the same
//! tolerances, and the caret and hit-test probes each case carries.
//!
//! Two classes of delta are allowed, both documented Phase 0 gaps rather than
//! layout behaviour:
//!
//! - **`script`**: the reference does not export a per-run script and records
//!   `Zzzz`. The native shaper fills it in (#91).
//! - **`run_count`**: the two engines cut runs at different places — the native
//!   shaper at every script and style boundary, cosmic at every face, level and
//!   size change. The glyphs are the same; only the grouping differs.
//! - **`overflow`**: only its `ELLIPSIZED` bit, and only where the case asked
//!   for an ellipsis. cosmic cannot draw one, so the reference sets the bit for
//!   a truncation it did not mark; this run supplies no shaped ellipsis either,
//!   so the native layout reports the truncation without claiming a `…` was
//!   substituted. [`overflow_deltas`] compares every other bit exactly.
//!
//! `compare` stops descending once a count disagrees, so a `run_count` delta
//! would leave that line's geometry uncompared. The second pass here closes
//! that hole: it walks each line's glyph cells in visual order and compares
//! their ids, clusters and x positions, which is grouping-independent.
//!
//! Run through [`Layouter`] directly rather than through
//! [`NativeTextEngine`](nana_text::NativeTextEngine), with **no strut and no
//! ellipsis**, because those are the two places the native engine deliberately
//! does something the reference cannot:
//!
//! - a strut pins the baseline to the base style's own face, so one taller
//!   fallback run cannot move it; cosmic centres each line on its own tallest
//!   run. Both are deterministic, only one keeps `Save` and `Save 🔥` on the
//!   same baseline. `layout_engine.rs` owns that fixture.
//! - cosmic 0.19 has no ellipsis at all, so a golden records the truncation
//!   without the `…`. `layout_engine.rs` owns that one too.

mod support;

use nana_text::layout::{LayoutRequest, Layouter};
use nana_text::parity::{
    CaseStatus, CorpusCase, DEFAULT_TOLERANCES, LayoutDelta, ParityReport, compare_golden,
    format_report, load_cases, load_golden,
};
use nana_text::shaping::{ShapeRequest, Shaper};
use nana_text::{CaretPosition, LineBox, OverflowFlags, TextLayout};
use support::corpus::{case_source, hermetic_fonts, with_chain};

/// Deltas that say the two engines describe the same layout differently, not
/// that they laid it out differently. See the module comment.
fn is_a_documented_gap(delta: &LayoutDelta) -> bool {
    matches!(delta.field, "script" | "run_count" | "overflow")
}

/// The overflow flags, compared bit by bit with the one documented exception.
fn overflow_deltas(case: &CorpusCase, actual: &TextLayout) -> Vec<String> {
    let golden = load_golden(&case.id).expect("golden loads");
    let mut expected = golden.layout.overflow;
    if case.constraints.ellipsis {
        expected.set(OverflowFlags::ELLIPSIZED, false);
    }
    if expected == actual.overflow {
        return Vec::new();
    }
    vec![format!(
        "overflow: expected {expected:?} got {:?}",
        actual.overflow
    )]
}

/// One glyph cell of a line, in visual order.
#[derive(Debug, Clone, PartialEq)]
struct Cell {
    glyph_id: u32,
    cluster: u32,
    cluster_end: u32,
    left_px: f32,
}

fn cells(layout: &TextLayout, line: &LineBox) -> Vec<Cell> {
    let mut cells = Vec::new();
    for run in layout.line_runs(line) {
        for (left_px, glyph) in run.glyph_cells() {
            cells.push(Cell {
                glyph_id: glyph.glyph_id,
                cluster: glyph.cluster,
                cluster_end: glyph.cluster_end,
                left_px,
            });
        }
    }
    cells
}

fn near(deltas: &mut Vec<String>, what: &str, expected: f32, actual: f32, tolerance: f32) {
    if (expected - actual).abs() > tolerance {
        deltas.push(format!(
            "{what}: expected {expected:.3} got {actual:.3} (tol {tolerance})"
        ));
    }
}

/// Lays a case out natively, with no strut and no ellipsis.
fn lay_out(case: &CorpusCase) -> TextLayout {
    let mut fonts = hermetic_fonts(case);
    let source = case_source(case);
    let style = with_chain(&case.style, case);
    let mut shaper = Shaper::default();
    let shaped = shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &style, &case.constraints),
    );
    let mut layouter = Layouter::default();
    let layout = layouter.layout(&LayoutRequest::new(
        case.kind,
        &source,
        &shaped,
        &style,
        &case.constraints,
    ));
    TextLayout::clone(&layout)
}

/// The structural diff, minus the two documented gaps.
fn structural_report(case: &CorpusCase, actual: &TextLayout) -> ParityReport {
    let golden = load_golden(&case.id).expect("golden loads");
    let hit_tests = case
        .hit_tests
        .iter()
        .map(|probe| actual.hit_test(probe.x_px, probe.y_px))
        .collect::<Vec<_>>();
    let carets = case
        .carets
        .iter()
        .map(|caret| actual.caret_geometry(CaretPosition::from(*caret)))
        .collect::<Vec<_>>();
    let tol = case.tolerances.unwrap_or(DEFAULT_TOLERANCES);
    let deltas = compare_golden(&golden, actual, &hit_tests, &carets, &tol)
        .into_iter()
        .filter(|delta| !is_a_documented_gap(delta))
        .collect();
    ParityReport {
        case_id: case.id.clone(),
        deltas,
    }
}

/// The per-line, per-cell geometry diff that survives a different run grouping.
fn cell_report(case: &CorpusCase, actual: &TextLayout) -> Vec<String> {
    let golden = load_golden(&case.id).expect("golden loads");
    let expected = &golden.layout;
    let tol = case.tolerances.unwrap_or(DEFAULT_TOLERANCES);
    let mut deltas = Vec::new();
    if expected.lines.len() != actual.lines.len() {
        return vec![format!(
            "line count: expected {} got {}",
            expected.lines.len(),
            actual.lines.len()
        )];
    }
    for (index, (want, got)) in expected.lines.iter().zip(actual.lines.iter()).enumerate() {
        if want.source != got.source {
            deltas.push(format!(
                "line {index} source: expected {:?} got {:?}",
                want.source, got.source
            ));
        }
        if want.break_cause != got.break_cause {
            deltas.push(format!(
                "line {index} break_cause: expected {:?} got {:?}",
                want.break_cause, got.break_cause
            ));
        }
        near(
            &mut deltas,
            &format!("line {index} baseline_y_px"),
            want.metrics.baseline_y_px,
            got.metrics.baseline_y_px,
            tol.baseline_px,
        );
        near(
            &mut deltas,
            &format!("line {index} top_y_px"),
            want.metrics.top_y_px,
            got.metrics.top_y_px,
            tol.baseline_px,
        );
        near(
            &mut deltas,
            &format!("line {index} height_px"),
            want.metrics.height_px,
            got.metrics.height_px,
            tol.line_height_px,
        );
        near(
            &mut deltas,
            &format!("line {index} ascent_px"),
            want.metrics.ascent_px,
            got.metrics.ascent_px,
            tol.line_height_px,
        );
        near(
            &mut deltas,
            &format!("line {index} descent_px"),
            want.metrics.descent_px,
            got.metrics.descent_px,
            tol.line_height_px,
        );
        near(
            &mut deltas,
            &format!("line {index} width_px"),
            want.metrics.width_px,
            got.metrics.width_px,
            tol.advance_px,
        );

        let want_cells = cells(expected, want);
        let got_cells = cells(actual, got);
        if want_cells.len() != got_cells.len() {
            deltas.push(format!(
                "line {index} cells: expected {} got {}",
                want_cells.len(),
                got_cells.len()
            ));
            continue;
        }
        for (at, (want, got)) in want_cells.iter().zip(got_cells.iter()).enumerate() {
            if (want.glyph_id, want.cluster, want.cluster_end)
                != (got.glyph_id, got.cluster, got.cluster_end)
            {
                deltas.push(format!(
                    "line {index} cell {at}: expected {:?} got {:?}",
                    (want.glyph_id, want.cluster, want.cluster_end),
                    (got.glyph_id, got.cluster, got.cluster_end)
                ));
                continue;
            }
            near(
                &mut deltas,
                &format!("line {index} cell {at} x"),
                want.left_px,
                got.left_px,
                tol.advance_px,
            );
        }
    }
    deltas
}

#[test]
fn every_passing_corpus_case_lays_out_as_the_reference_did() {
    let cases = load_cases().expect("corpus loads");
    let mut failures = Vec::new();
    let mut compared = 0;
    for case in cases.iter().filter(|case| case.status == CaseStatus::Pass) {
        compared += 1;
        let actual = lay_out(case);
        let report = structural_report(case, &actual);
        if !report.ok() {
            failures.push(format_report(&report));
        }
        let mut deltas = cell_report(case, &actual);
        deltas.extend(overflow_deltas(case, &actual));
        if !deltas.is_empty() {
            failures.push(format!("{} cells\n  {}", case.id, deltas.join("\n  ")));
        }
    }
    assert!(
        compared >= 26,
        "the corpus lost cases: only {compared} were compared"
    );
    assert!(
        failures.is_empty(),
        "native layout differs from the recorded reference layout:\n{}",
        failures.join("\n")
    );
}

/// The gap filter must not be able to hide a real difference: strip it and the
/// only fields left are the three the module comment names.
#[test]
fn the_only_unfiltered_deltas_are_the_three_documented_gaps() {
    let mut fields: Vec<&'static str> = Vec::new();
    for case in load_cases()
        .expect("corpus loads")
        .iter()
        .filter(|case| case.status == CaseStatus::Pass)
    {
        let actual = lay_out(case);
        let golden = load_golden(&case.id).expect("golden loads");
        let tol = case.tolerances.unwrap_or(DEFAULT_TOLERANCES);
        for delta in nana_text::parity::compare(&golden.layout, &actual, &tol) {
            if !fields.contains(&delta.field) {
                fields.push(delta.field);
            }
        }
    }
    fields.sort_unstable();
    assert_eq!(fields, vec!["overflow", "run_count", "script"]);
}
