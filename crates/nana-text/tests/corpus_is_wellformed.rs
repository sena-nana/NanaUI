//! The corpus has to be well formed before it can mean anything, and it has to
//! stay complete: a requirement that quietly loses its last case looks exactly
//! like a requirement that passes.

use nana_text::parity::{
    CATEGORIES, CaseStatus, DEFAULT_TOLERANCES, GOLDEN_SCHEMA_VERSION, KNOWN_FONT_FIXTURES,
    compare, corpus_case_dir, golden_ids, load_cases, load_golden,
};
use std::collections::BTreeSet;

#[test]
fn every_corpus_case_has_a_golden_and_every_golden_has_a_case() {
    let cases: BTreeSet<String> = load_cases()
        .expect("the corpus loads")
        .into_iter()
        .filter(|case| case.status == CaseStatus::Pass)
        .map(|case| case.id)
        .collect();
    let goldens: BTreeSet<String> = golden_ids()
        .expect("the goldens load")
        .into_iter()
        .collect();

    let missing: Vec<&String> = cases.difference(&goldens).collect();
    assert!(missing.is_empty(), "cases without a golden: {missing:?}");
    let orphans: Vec<&String> = goldens.difference(&cases).collect();
    assert!(
        orphans.is_empty(),
        "goldens without a passing case: {orphans:?}"
    );
}

#[test]
fn case_ids_are_unique_and_match_their_file_names() {
    let cases = load_cases().expect("the corpus loads");
    let mut seen = BTreeSet::new();
    for case in &cases {
        assert!(
            seen.insert(case.id.clone()),
            "duplicate case id {}",
            case.id
        );
        let path = corpus_case_dir().join(format!("{}.json", case.id));
        assert!(
            path.exists(),
            "case {} does not live in {}",
            case.id,
            path.display()
        );
    }
}

#[test]
fn ignored_cases_document_the_gap_that_keeps_them_ignored() {
    for case in load_cases().expect("the corpus loads") {
        if case.status == CaseStatus::Ignore {
            assert!(
                case.gap.is_some() && case.gap_note.is_some(),
                "{} is ignored without naming the gap and how to close it",
                case.id
            );
        } else {
            assert!(
                case.gap.is_none(),
                "{} passes but still carries a gap note",
                case.id
            );
        }
    }
}

#[test]
fn the_corpus_covers_every_required_category() {
    let cases = load_cases().expect("the corpus loads");
    let covered: BTreeSet<&str> = cases
        .iter()
        .filter(|case| case.status == CaseStatus::Pass)
        .map(|case| case.category.as_str())
        .collect();
    let missing: Vec<&&str> = CATEGORIES
        .iter()
        .filter(|category| !covered.contains(*category))
        .collect();
    assert!(
        missing.is_empty(),
        "these required corpus categories have no passing case: {missing:?}"
    );

    let unknown: Vec<&str> = covered
        .iter()
        .filter(|category| !CATEGORIES.contains(category))
        .copied()
        .collect();
    assert!(
        unknown.is_empty(),
        "these cases claim a category that is not declared: {unknown:?}"
    );
}

#[test]
fn every_case_font_id_is_a_known_fixture() {
    for case in load_cases().expect("the corpus loads") {
        assert!(!case.fonts.is_empty(), "{} declares no font", case.id);
        for font in &case.fonts {
            assert!(
                KNOWN_FONT_FIXTURES.contains(&font.as_str()),
                "{} names font fixture {font:?}, which is not declared in KNOWN_FONT_FIXTURES",
                case.id
            );
        }
    }
}

#[test]
fn goldens_round_trip_through_serde_without_drift() {
    for id in golden_ids().expect("the goldens load") {
        let golden = load_golden(&id).expect("golden parses");
        assert_eq!(
            golden.schema_version, GOLDEN_SCHEMA_VERSION,
            "{id} is recorded under an older golden schema"
        );
        assert_eq!(golden.case, id, "{id} disagrees with the case it names");

        let text = serde_json::to_string(&golden).expect("golden serializes");
        let reparsed: nana_text::parity::Golden =
            serde_json::from_str(&text).expect("golden re-parses");
        let deltas = compare(&golden.layout, &reparsed.layout, &DEFAULT_TOLERANCES);
        assert!(
            deltas.is_empty(),
            "{id} does not survive a serde round trip; the IR's serde is lossy"
        );
        assert_eq!(reparsed.carets, golden.carets, "{id} caret answers drifted");
        assert_eq!(
            reparsed.hit_tests, golden.hit_tests,
            "{id} hit-test answers drifted"
        );
    }
}

#[test]
fn every_golden_lists_each_lines_runs_in_visual_order() {
    // `LineBox::runs` is documented as visual order and `TextLayout::cells`
    // flattens it assuming the result is sorted by x. Nothing in the type
    // system enforces that, so assert it on every recorded baseline: a golden
    // that violates it would silently break hit-testing, caret placement and
    // selection rectangles for whatever engine reproduces it.
    for id in golden_ids().expect("the goldens load") {
        let golden = load_golden(&id).expect("golden parses");
        for line in &golden.layout.lines {
            let origins: Vec<f32> = golden
                .layout
                .line_runs(line)
                .iter()
                .map(|run| run.origin_x_px)
                .collect();
            assert!(
                origins.windows(2).all(|pair| pair[0] <= pair[1]),
                "{id} line {} lists its runs out of visual order: {origins:?}",
                line.index
            );
        }
    }
}

#[test]
fn every_golden_records_a_layout_with_content() {
    for id in golden_ids().expect("the goldens load") {
        let golden = load_golden(&id).expect("golden parses");
        assert!(
            !golden.layout.lines.is_empty(),
            "{id} recorded an empty layout, which asserts nothing"
        );
        assert!(
            golden.layout.glyph_count() > 0,
            "{id} recorded no glyphs, which asserts nothing"
        );
    }
}
