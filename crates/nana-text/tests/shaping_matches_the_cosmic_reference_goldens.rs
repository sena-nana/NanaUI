//! Issue #91: the native shaper against the Phase 0 cosmic-text goldens.
//!
//! The goldens are laid-out lines, and Phase 2 produces unwrapped runs, so the
//! comparison is made where shaping is authoritative and line layout is not
//! involved: per **logical cluster**. Glyphs are grouped by the cluster they
//! belong to (visual order kept inside a cluster), clusters are ordered by
//! source byte, and each cluster is compared field by field. That is
//! independent of where either engine cut its runs or wrapped its lines.
//!
//! Compared exactly: cluster start and end, glyph count, glyph id, `MISSING`,
//! `FALLBACK_FONT`, bidi level, font size, and font identity as a bijection
//! between the two engines' faces. Compared with [`DEFAULT_TOLERANCES`]:
//! advance and offsets.
//!
//! One documented difference is allowed, and only for wrapping cases: the
//! golden is laid-out lines, so it lacks the whitespace glyph at each soft wrap
//! and everything after a `max_lines` truncation. That is line layout (Phase
//! 3), not shaping; see `diff_case`.

mod support;

use nana_text::parity::{CaseStatus, CorpusCase, DEFAULT_TOLERANCES, load_cases, load_golden};
use nana_text::shaping::{ShapeRequest, Shaper};
use nana_text::{FontId, GlyphFlags, ShapedRun};
use std::collections::{BTreeMap, HashMap};
use support::corpus::{case_source, hermetic_fonts, with_chain};

#[derive(Debug, Clone, PartialEq)]
struct Cell {
    glyph_id: u32,
    cluster_end: u32,
    advance: f32,
    offset_x: f32,
    offset_y: f32,
    missing: bool,
    fallback: bool,
    level: u8,
    size: f32,
    font: FontId,
}

/// cluster start -> glyphs of that cluster, visual order within it.
fn clusters<'a>(runs: impl Iterator<Item = &'a ShapedRun>) -> BTreeMap<u32, Vec<Cell>> {
    let mut map: BTreeMap<u32, Vec<Cell>> = BTreeMap::new();
    for run in runs {
        for glyph in &run.glyphs {
            map.entry(glyph.cluster).or_default().push(Cell {
                glyph_id: glyph.glyph_id,
                cluster_end: glyph.cluster_end,
                advance: glyph.advance_px,
                offset_x: glyph.offset_x_px,
                offset_y: glyph.offset_y_px,
                missing: glyph.flags.contains(GlyphFlags::MISSING),
                fallback: glyph.flags.contains(GlyphFlags::FALLBACK_FONT),
                level: run.bidi_level,
                size: run.font_size_px,
                font: run.font,
            });
        }
    }
    map
}

fn diff_case(case: &CorpusCase) -> Vec<String> {
    let mut fonts = hermetic_fonts(case);
    let source = case_source(case);
    let style = with_chain(&case.style, case);
    let mut shaper = Shaper::default();
    let shaped = shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &style, &case.constraints),
    );

    let golden = load_golden(&case.id).unwrap();
    let expected = clusters(golden.layout.runs.iter());
    let actual = clusters(shaped.runs.iter());

    let mut deltas = Vec::new();
    let mut push = |cluster: u32, glyph: Option<usize>, field: &str, e: String, a: String| {
        let at = match glyph {
            Some(index) => format!("cluster {cluster} glyph {index}"),
            None => format!("cluster {cluster}"),
        };
        deltas.push(format!("{} {at} {field}: expected {e} got {a}", case.id));
    };

    // Documented difference: a wrapped golden is laid-out lines. Line layout
    // drops the whitespace glyph at each soft wrap and everything after a
    // `max_lines` truncation; unwrapped shaping keeps both. Those clusters, and
    // only those, may be absent from the golden.
    let expected_starts: Vec<u32> = expected.keys().copied().collect();
    let text = source.text();
    let last_kept = expected_starts.last().copied().unwrap_or(0);
    let actual_starts: Vec<u32> = actual
        .keys()
        .copied()
        .filter(|start| {
            expected.contains_key(start)
                || case.constraints.wrap.is_none()
                || !(*start > last_kept
                    || text[*start as usize..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace))
        })
        .collect();
    if expected_starts != actual_starts {
        push(
            0,
            None,
            "cluster_starts",
            format!("{expected_starts:?}"),
            format!("{actual_starts:?}"),
        );
        return deltas;
    }

    // Faces are compared as a bijection, never by raw id.
    let mut forward: HashMap<FontId, FontId> = HashMap::new();
    let mut backward: HashMap<FontId, FontId> = HashMap::new();
    let tol = DEFAULT_TOLERANCES;
    for (start, e_cells) in &expected {
        let a_cells = &actual[start];
        if e_cells.len() != a_cells.len() {
            push(
                *start,
                None,
                "glyph_count",
                e_cells.len().to_string(),
                a_cells.len().to_string(),
            );
            continue;
        }
        for (index, (e, a)) in e_cells.iter().zip(a_cells).enumerate() {
            let glyph = Some(index);
            if e.glyph_id != a.glyph_id {
                push(
                    *start,
                    glyph,
                    "glyph_id",
                    e.glyph_id.to_string(),
                    a.glyph_id.to_string(),
                );
            }
            if e.cluster_end != a.cluster_end {
                push(
                    *start,
                    glyph,
                    "cluster_end",
                    e.cluster_end.to_string(),
                    a.cluster_end.to_string(),
                );
            }
            if e.missing != a.missing {
                push(
                    *start,
                    glyph,
                    "MISSING",
                    e.missing.to_string(),
                    a.missing.to_string(),
                );
            }
            if e.fallback != a.fallback {
                push(
                    *start,
                    glyph,
                    "FALLBACK_FONT",
                    e.fallback.to_string(),
                    a.fallback.to_string(),
                );
            }
            if e.level != a.level {
                push(
                    *start,
                    glyph,
                    "bidi_level",
                    e.level.to_string(),
                    a.level.to_string(),
                );
            }
            if (e.size - a.size).abs() > 1e-3 {
                push(
                    *start,
                    glyph,
                    "font_size_px",
                    e.size.to_string(),
                    a.size.to_string(),
                );
            }
            for (field, ev, av, limit) in [
                ("advance_px", e.advance, a.advance, tol.advance_px),
                ("offset_x_px", e.offset_x, a.offset_x, tol.offset_px),
                ("offset_y_px", e.offset_y, a.offset_y, tol.offset_px),
            ] {
                if (ev - av).abs() > limit {
                    push(
                        *start,
                        glyph,
                        field,
                        format!("{ev:.3}"),
                        format!("{av:.3} (tol {limit})"),
                    );
                }
            }
            let consistent = forward.get(&e.font).is_none_or(|mapped| *mapped == a.font)
                && backward.get(&a.font).is_none_or(|mapped| *mapped == e.font);
            if !consistent {
                push(
                    *start,
                    glyph,
                    "font",
                    format!("{:?}", e.font),
                    format!("{:?}", a.font),
                );
            }
            forward.insert(e.font, a.font);
            backward.insert(a.font, e.font);
        }
    }
    deltas
}

#[test]
fn every_corpus_case_shapes_like_its_golden_cluster_by_cluster() {
    let mut report = Vec::new();
    let mut compared = 0;
    for case in load_cases().unwrap() {
        if case.status == CaseStatus::Ignore {
            continue;
        }
        compared += 1;
        report.extend(diff_case(&case));
    }
    assert!(compared >= 26);
    assert!(
        report.is_empty(),
        "{} deltas:\n{}",
        report.len(),
        report.join("\n")
    );
}
