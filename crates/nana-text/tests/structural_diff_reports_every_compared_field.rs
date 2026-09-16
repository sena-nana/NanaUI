//! The structural diff has to be able to *see* every field it claims to
//! compare. A field that is silently skipped is worse than one that is
//! documented as out of scope, so this walks the comparable surface and
//! asserts each perturbation produces a delta.

mod support;

use nana_text::parity::{DEFAULT_TOLERANCES, DeltaScope, ParityReport, compare, format_report};
use nana_text::{
    FontId, GlyphFlags, LineBreakCause, OverflowFlags, RunDirection, ScriptTag, TextKind,
    TextLayout, TextRect,
};

fn deltas_for(mutate: impl FnOnce(&mut TextLayout)) -> Vec<String> {
    let expected = support::mixed_bidi_single_line();
    let mut actual = expected.clone();
    mutate(&mut actual);
    compare(&expected, &actual, &DEFAULT_TOLERANCES)
        .into_iter()
        .map(|delta| delta.field.to_string())
        .collect()
}

fn assert_reports(field: &str, mutate: impl FnOnce(&mut TextLayout)) {
    let fields = deltas_for(mutate);
    assert!(
        fields.iter().any(|name| name == field),
        "changing {field} must produce a `{field}` delta, got {fields:?}"
    );
}

#[test]
fn an_identical_layout_produces_no_deltas() {
    let layout = support::mixed_bidi_single_line();
    assert!(compare(&layout, &layout.clone(), &DEFAULT_TOLERANCES).is_empty());
}

#[test]
fn engine_bookkeeping_is_deliberately_not_compared() {
    // Two engines have no reason to agree on a layout id, a source revision or
    // a font generation: those say where a layout came from, not what it says.
    let expected = support::mixed_bidi_single_line();
    let mut actual = expected.clone();
    actual.id = nana_text::TextLayoutId::from_parts(9, 9);
    actual.revision = nana_text::TextRevision::INITIAL.next().next();
    actual.font_generation = nana_text::FontGeneration::new(42);
    assert!(
        compare(&expected, &actual, &DEFAULT_TOLERANCES).is_empty(),
        "identity fields must not be part of the behavioural contract"
    );
}

#[test]
fn every_layout_level_field_is_compared() {
    assert_reports("kind", |layout| layout.kind = TextKind::Label);
    assert_reports("overflow", |layout| {
        layout.overflow = OverflowFlags::NONE.with(OverflowFlags::ELLIPSIZED)
    });
    assert_reports("bounds.width", |layout| {
        layout.bounds = TextRect::new(0.0, 0.0, 999.0, 20.0)
    });
    assert_reports("line_count", |layout| {
        layout.lines.pop();
    });
    assert_reports("run_count", |layout| {
        layout.runs.pop();
    });
}

#[test]
fn every_line_level_field_is_compared() {
    assert_reports("index", |layout| layout.lines[0].index = 7);
    assert_reports("source", |layout| layout.lines[0].source = 0..4);
    assert_reports("break_cause", |layout| {
        layout.lines[0].break_cause = LineBreakCause::Wrap
    });
    assert_reports("base_direction", |layout| {
        layout.lines[0].base_direction = RunDirection::Rtl
    });
    assert_reports("metrics.baseline_y_px", |layout| {
        layout.lines[0].metrics.baseline_y_px += 1.0
    });
    assert_reports("metrics.top_y_px", |layout| {
        layout.lines[0].metrics.top_y_px += 1.0
    });
    assert_reports("metrics.height_px", |layout| {
        layout.lines[0].metrics.height_px += 1.0
    });
    assert_reports("metrics.ascent_px", |layout| {
        layout.lines[0].metrics.ascent_px += 1.0
    });
    assert_reports("metrics.descent_px", |layout| {
        layout.lines[0].metrics.descent_px += 1.0
    });
    assert_reports("metrics.width_px", |layout| {
        layout.lines[0].metrics.width_px += 1.0
    });
    assert_reports("bounds.x", |layout| layout.lines[0].bounds.x += 1.0);
}

#[test]
fn every_run_level_field_is_compared() {
    assert_reports("source", |layout| layout.runs[0].source = 0..1);
    assert_reports("direction", |layout| {
        layout.runs[0].direction = RunDirection::Rtl
    });
    assert_reports("bidi_level", |layout| layout.runs[0].bidi_level = 2);
    assert_reports("script", |layout| layout.runs[0].script = ScriptTag::ARABIC);
    assert_reports("font_size_px", |layout| layout.runs[0].font_size_px = 20.0);
    assert_reports("advance_px", |layout| layout.runs[0].advance_px += 1.0);
    assert_reports("origin_x_px", |layout| layout.runs[0].origin_x_px += 1.0);
    assert_reports("metrics.ascent_px", |layout| {
        layout.runs[0].metrics.ascent_px += 1.0
    });
    assert_reports("metrics.descent_px", |layout| {
        layout.runs[0].metrics.descent_px += 1.0
    });
    assert_reports("metrics.line_gap_px", |layout| {
        layout.runs[0].metrics.line_gap_px += 1.0
    });
    assert_reports("glyph_count", |layout| {
        layout.runs[0].glyphs.pop();
    });
}

#[test]
fn every_glyph_level_field_is_compared() {
    assert_reports("glyph_id", |layout| layout.runs[0].glyphs[0].glyph_id = 999);
    assert_reports("cluster", |layout| layout.runs[0].glyphs[0].cluster = 9);
    assert_reports("cluster_end", |layout| {
        layout.runs[0].glyphs[0].cluster_end = 9
    });
    assert_reports("flags", |layout| {
        layout.runs[0].glyphs[0].flags = GlyphFlags::NONE.with(GlyphFlags::MISSING)
    });
    assert_reports("advance_px", |layout| {
        layout.runs[0].glyphs[0].advance_px += 1.0
    });
    assert_reports("advance_y_px", |layout| {
        layout.runs[0].glyphs[0].advance_y_px += 1.0
    });
    assert_reports("offset_x_px", |layout| {
        layout.runs[0].glyphs[0].offset_x_px += 1.0
    });
    assert_reports("offset_y_px", |layout| {
        layout.runs[0].glyphs[0].offset_y_px += 1.0
    });
}

#[test]
fn fonts_are_compared_as_a_partition_not_as_raw_handles() {
    // Renumbering every face consistently is a registration-order change, not a
    // text behaviour, and must not fail the diff.
    let fields = deltas_for(|layout| {
        for run in &mut layout.runs {
            run.font = FontId::from_parts(run.font.index() + 100, 4);
        }
    });
    assert!(
        fields.is_empty(),
        "a consistent renumbering is not a finding, got {fields:?}"
    );

    // Merging two faces into one *is* a behaviour change.
    assert_reports("font_class", |layout| {
        layout.runs[1].font = layout.runs[0].font
    });
}

#[test]
fn a_count_mismatch_stops_the_diff_from_descending() {
    let expected = support::latin_single_line();
    let mut actual = expected.clone();
    actual.runs[0].glyphs.remove(0);
    actual.runs[0].advance_px = actual.runs[0].glyph_advance_sum_px();

    let deltas = compare(&expected, &actual, &DEFAULT_TOLERANCES);
    assert!(
        deltas.iter().any(|delta| delta.field == "glyph_count"),
        "the count itself is reported"
    );
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta.scope, DeltaScope::Glyph { .. })),
        "a shifted glyph list must not produce a page of misaligned per-glyph deltas"
    );
}

#[test]
fn a_value_inside_tolerance_is_not_a_delta_and_one_outside_it_is() {
    let expected = support::latin_single_line();

    let mut near = expected.clone();
    near.runs[0].glyphs[0].advance_px += DEFAULT_TOLERANCES.advance_px * 0.5;
    assert!(compare(&expected, &near, &DEFAULT_TOLERANCES).is_empty());

    let mut far = expected.clone();
    far.runs[0].glyphs[0].advance_px += DEFAULT_TOLERANCES.advance_px * 2.0;
    assert_eq!(compare(&expected, &far, &DEFAULT_TOLERANCES).len(), 1);
}

#[test]
fn a_nan_never_passes_a_tolerance_check() {
    let expected = support::latin_single_line();
    let mut actual = expected.clone();
    actual.runs[0].glyphs[0].advance_px = f32::NAN;
    assert!(
        !compare(&expected, &actual, &DEFAULT_TOLERANCES).is_empty(),
        "NaN compares false against every bound, so it must be caught explicitly"
    );
}

#[test]
fn the_report_names_the_line_the_run_the_glyph_and_the_field() {
    let expected = support::latin_single_line();
    let mut actual = expected.clone();
    actual.runs[0].glyphs[1].glyph_id = 4242;
    actual.lines[0].metrics.baseline_y_px += 1.0;

    let report = ParityReport {
        case_id: "TX-TEST".to_string(),
        deltas: compare(&expected, &actual, &DEFAULT_TOLERANCES),
    };
    assert!(!report.ok());
    let text = format_report(&report);
    assert!(text.starts_with("TX-TEST FAIL (2 deltas)"), "got {text}");
    assert!(
        text.contains("line 0 run 0 glyph 1 glyph_id: expected 2 got 4242"),
        "got {text}"
    );
    assert!(text.contains("line 0 metrics.baseline_y_px"), "got {text}");
    assert!(text.contains("tol=0.05"), "got {text}");

    let clean = ParityReport {
        case_id: "TX-TEST".to_string(),
        deltas: Vec::new(),
    };
    assert_eq!(format_report(&clean), "TX-TEST OK");
}
