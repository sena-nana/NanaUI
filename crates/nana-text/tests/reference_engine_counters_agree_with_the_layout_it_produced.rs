//! Counters have to describe the artifact they claim to describe, and the IR's
//! caret derivations have to agree with the engine that produced the layout.
//!
//! Both are cross-checks rather than self-assertions: a counter compared only
//! against itself, and a caret compared only against its own golden, would both
//! stay green while being wrong.

mod reference;

use nana_text::parity::{CaseStatus, load_case, load_cases};
use nana_text::{CompositionSegment, TextSource, TextSpan};

#[test]
fn the_reference_engine_reports_the_glyph_count_it_actually_produced() {
    for case in load_cases().expect("the corpus loads") {
        if case.status == CaseStatus::Ignore {
            continue;
        }
        let run = reference::run_case(&case);
        assert_eq!(
            run.counters.glyphs_resolved,
            Some(run.layout.glyph_count()),
            "{} reports a glyph count that disagrees with its own layout",
            case.id
        );
        assert_eq!(run.counters.text_nodes_considered, 1, "{}", case.id);
        assert_eq!(run.counters.text_nodes_shaped, 1, "{}", case.id);
    }
}

#[test]
fn the_reference_engine_leaves_cache_counters_unobserved_because_it_has_no_cache() {
    let case = load_case("TX-L01").expect("TX-L01 loads");
    let run = reference::run_case(&case);
    // Not `Some(0)`: nothing consulted a cache, and a fake zero would read as
    // "the cache answered nothing" instead of "there is no cache".
    assert_eq!(run.counters.shape_cache_hits, None);
    assert_eq!(run.counters.shape_cache_misses, None);
    assert_eq!(run.counters.layout_cache_hits, None);
    assert_eq!(run.counters.layout_cache_misses, None);
}

#[test]
fn nana_text_hit_testing_agrees_with_the_reference_engines_own_cursor() {
    // Restricted to single-line, single-direction cases: at a BiDi boundary the
    // two engines are entitled to disagree on affinity, and cosmic's answer is
    // recorded as a golden rather than asserted as a standard.
    for id in ["TX-L01", "TX-C01", "TX-K01", "TX-W03"] {
        let case = load_case(id).unwrap_or_else(|error| panic!("{error}"));
        let run = reference::run_case(&case);
        for probe in &case.hit_tests {
            let ours = run.layout.hit_test(probe.x_px, probe.y_px);
            let Some(theirs) = run.buffer.hit(probe.x_px, probe.y_px) else {
                continue;
            };
            let Some(theirs) = run.source_byte(theirs) else {
                continue;
            };
            assert_eq!(
                ours.caret.byte, theirs,
                "{id} at ({}, {}): the IR's own hit test disagrees with the engine's",
                probe.x_px, probe.y_px
            );
        }
    }
}

#[test]
fn a_composition_span_survives_on_the_source_that_the_layout_was_taken_from() {
    // The layout carries geometry; composition state stays on the source. This
    // is the boundary the IME fixture exists to pin, so that a later phase does
    // not quietly start reading preedit state off the glyphs.
    let case = load_case("TX-E01").expect("TX-E01 loads");
    let span = case
        .spans
        .iter()
        .find(|span| span.composition == Some(CompositionSegment::Preedit))
        .expect("the IME fixture declares a preedit span");

    let mut source = TextSource::new(case.text.clone());
    assert!(!source.has_composition());
    source.set_composition(vec![TextSpan {
        range: span.range.clone(),
        style: span.style.clone(),
        composition: span.composition,
    }]);
    assert!(source.has_composition());
    assert_eq!(source.spans()[0].range, span.range);

    // Committing the composition clears it without touching the text.
    let text = source.text().to_string();
    source.set_composition(Vec::new());
    assert!(!source.has_composition());
    assert_eq!(source.text(), text);

    // And the span really did reach the layout: the preedit run shapes at its
    // own size, so it is a separate run.
    let run = reference::run_case(&case);
    assert_eq!(
        run.layout.runs.len(),
        2,
        "the preedit span must lay out as its own run, or the fixture asserts nothing"
    );
    assert_ne!(
        run.layout.runs[0].font_size_px, run.layout.runs[1].font_size_px,
        "the two runs must differ in the way the span declared"
    );
}
