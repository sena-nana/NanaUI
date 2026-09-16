//! The migration parity gate: every corpus case must still produce the layout
//! its golden records.
//!
//! In Phase 0 the only engine is the cosmic reference, so this compares the
//! reference against the committed goldens. When a native engine lands it plugs
//! into the same `compare`, against the same goldens, and additionally against
//! the reference — no part of this file's shape changes.
//!
//! Re-record with:
//!
//! ```text
//! NANA_TEXT_BLESS=1 cargo test -p nana-text --test text_parity_corpus
//! ```
//!
//! Blessing rewrites the goldens in place so the change arrives as a reviewable
//! diff. It is never automatic.

mod reference;

use nana_text::parity::{
    CaseStatus, CorpusCase, DEFAULT_TOLERANCES, GOLDEN_SCHEMA_VERSION, Golden, ParityReport,
    compare_golden, format_report, load_case, load_cases, load_golden, write_golden,
};
use nana_text::{CaretGeometry, CaretPosition, HitTestResult, TextLayout, TextWorkCounters};

fn blessing() -> bool {
    std::env::var_os("NANA_TEXT_BLESS").is_some_and(|value| value != "0")
}

struct Produced {
    layout: TextLayout,
    hit_tests: Vec<HitTestResult>,
    carets: Vec<Option<CaretGeometry>>,
    counters: TextWorkCounters,
}

/// Runs a case through the reference engine and replays the case's probes
/// against **nana-text's own** derivations, not the engine's.
///
/// That is deliberate: it is the IR that has to answer caret questions after
/// the migration, so the golden records what the IR says.
fn produce(case: &CorpusCase) -> Produced {
    let run = reference::run_case(case);
    let hit_tests = case
        .hit_tests
        .iter()
        .map(|probe| run.layout.hit_test(probe.x_px, probe.y_px))
        .collect();
    let carets = case
        .carets
        .iter()
        .map(|caret| run.layout.caret_geometry(CaretPosition::from(*caret)))
        .collect();
    Produced {
        layout: run.layout,
        hit_tests,
        carets,
        counters: run.counters,
    }
}

fn golden_of(case: &CorpusCase, produced: &Produced) -> Golden {
    Golden {
        schema_version: GOLDEN_SCHEMA_VERSION,
        case: case.id.clone(),
        layout: produced.layout.clone(),
        hit_tests: produced.hit_tests.clone(),
        carets: produced.carets.clone(),
        counters: produced.counters,
    }
}

/// Checks one case, or re-records it when blessing.
fn report_for(case: &CorpusCase) -> ParityReport {
    let produced = produce(case);
    if blessing() {
        // Blessing records; it does not gate. Comparing here as well would race
        // the other tests writing the same files, and a "pass" that only means
        // "I just wrote this" is not worth reporting.
        write_golden(&golden_of(case, &produced)).expect("golden is writable");
        return ParityReport {
            case_id: case.id.clone(),
            deltas: Vec::new(),
        };
    }
    let golden = match load_golden(&case.id) {
        Ok(golden) => golden,
        Err(error) => {
            return ParityReport {
                case_id: case.id.clone(),
                deltas: vec![missing_golden_delta(&error.to_string())],
            };
        }
    };
    assert_eq!(
        golden.schema_version, GOLDEN_SCHEMA_VERSION,
        "{} was recorded under golden schema {} but this build speaks {}; re-bless the corpus",
        case.id, golden.schema_version, GOLDEN_SCHEMA_VERSION
    );
    let tolerances = case.tolerances.unwrap_or(DEFAULT_TOLERANCES);
    ParityReport {
        case_id: case.id.clone(),
        deltas: compare_golden(
            &golden,
            &produced.layout,
            &produced.hit_tests,
            &produced.carets,
            &tolerances,
        ),
    }
}

fn missing_golden_delta(message: &str) -> nana_text::parity::LayoutDelta {
    use nana_text::parity::{DeltaScope, DeltaValue, LayoutDelta};
    LayoutDelta {
        scope: DeltaScope::Layout,
        field: "golden",
        expected: DeltaValue::Text(message.to_string()),
        actual: DeltaValue::Missing,
        delta: None,
        tolerance: None,
    }
}

fn check(id: &str) {
    let case = load_case(id).unwrap_or_else(|error| panic!("{error}"));
    if case.status == CaseStatus::Ignore {
        // An ignored case still has to load and still has to name its gap; it
        // just is not held to its golden yet.
        assert!(case.gap.is_some(), "{id} is ignored without naming a gap");
        return;
    }
    let report = report_for(&case);
    assert!(report.ok(), "{}", format_report(&report));
}

macro_rules! corpus_tests {
    ($($name:ident => $id:literal,)*) => {
        $(
            #[test]
            fn $name() {
                check($id);
            }
        )*

        /// Every case id named above, so the coverage check below cannot drift
        /// from the tests it is checking.
        const NAMED_CASES: &[&str] = &[$($id),*];
    };
}

corpus_tests! {
    latin_shapes_one_glyph_per_character => "TX-L01",
    latin_kerning_pairs_match_the_golden => "TX-L02",
    han_ideographs_match_the_golden => "TX-C01",
    kana_and_han_mixed_match_the_golden => "TX-C02",
    hangul_syllables_match_the_golden => "TX-K01",
    combining_marks_match_the_golden => "TX-M01",
    f_ligatures_match_the_golden => "TX-G01",
    disabled_ligatures_match_the_golden => "TX-G02",
    arabic_joining_matches_the_golden => "TX-B01",
    mixed_bidi_ltr_base_matches_the_golden => "TX-B02",
    mixed_bidi_rtl_base_matches_the_golden => "TX-B03",
    emoji_zwj_sequences_match_the_golden => "TX-Z01",
    emoji_variation_selectors_match_the_golden => "TX-Z02",
    font_fallback_matches_the_golden => "TX-F01",
    missing_glyphs_match_the_golden => "TX-F02",
    variable_font_narrow_width_matches_the_golden => "TX-V01",
    variable_font_wide_width_matches_the_golden => "TX-V02",
    variable_font_custom_axis_matches_the_golden => "TX-V03",
    a_label_never_wraps_and_matches_the_golden => "TX-W01",
    explicit_newlines_match_the_golden => "TX-W02",
    word_wrap_matches_the_golden => "TX-W03",
    glyph_wrap_matches_the_golden => "TX-W04",
    max_lines_truncation_matches_the_golden => "TX-W05",
    fractional_scale_125_matches_the_golden => "TX-D01",
    fractional_scale_150_matches_the_golden => "TX-D02",
    ime_preedit_matches_the_golden => "TX-E01",
}

#[test]
fn all_pass_cases_match_their_goldens() {
    let cases = load_cases().expect("the corpus loads");
    let mut failures = Vec::new();
    for case in &cases {
        if case.status == CaseStatus::Ignore {
            continue;
        }
        let report = report_for(case);
        if !report.ok() {
            failures.push(format_report(&report));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus cases drifted:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn every_corpus_case_has_a_test_of_its_own() {
    let declared: Vec<String> = load_cases()
        .expect("the corpus loads")
        .into_iter()
        .map(|case| case.id)
        .collect();
    let missing: Vec<&String> = declared
        .iter()
        .filter(|id| !NAMED_CASES.contains(&id.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these cases have no named test: {missing:?}"
    );
    let stale: Vec<&&str> = NAMED_CASES
        .iter()
        .filter(|id| !declared.iter().any(|declared| declared == *id))
        .collect();
    assert!(stale.is_empty(), "these tests name no case: {stale:?}");
}
