//! Issue #182: `EditSession` as the only storage for an editor — shared
//! snapshots, stamps, several selections, and the batch edits a host that
//! computes its own transforms hands in.

mod support;

use nana_text::editable::{
    EditSelection, collapse_edge, normalize_selections, remap_offset, remap_selection,
};
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    Affinity, EditChange, EditSession, EditorGeometry, Motion, NativeTextEngine, SharedText,
    TextConstraints, TextStyle, TextWorkCounters,
};
use std::sync::Arc;

use support::corpus::{fixture_bytes, fixture_family};

fn caret(offset: usize) -> EditSelection {
    EditSelection::caret(offset)
}

fn span(anchor: usize, focus: usize) -> EditSelection {
    EditSelection::new(anchor, focus)
}

fn session(text: &str, primary: EditSelection, additional: &[EditSelection]) -> EditSession {
    EditSession::with_selections(SharedText::from(text), primary, additional.iter().copied())
}

fn offsets(session: &EditSession) -> Vec<(usize, usize)> {
    session
        .selections()
        .iter()
        .map(|selection| (selection.anchor, selection.focus))
        .collect()
}

// ---- shared storage --------------------------------------------------------

#[test]
fn a_snapshot_survives_the_next_edit_unchanged() {
    let mut session = EditSession::new("hello");
    let snapshot = session.snapshot();
    assert_eq!(snapshot.stamp(), Some(session.text().stamp()));
    assert!(!session.insert("!").is_none());
    assert_eq!(session.as_str(), "hello!");
    assert_eq!(
        snapshot, "hello",
        "the holder of a snapshot keeps its bytes"
    );
    assert_ne!(snapshot.stamp(), Some(session.text().stamp()));
}

#[test]
fn moving_the_caret_keeps_the_text_stamp_and_the_session() {
    let mut session = EditSession::new("hello world");
    let stamp = session.text().stamp();
    let id = session.revisions().session;
    for _ in 0..100 {
        session.move_caret(Motion::GraphemeBackward, false, None);
    }
    session.move_caret(Motion::DocumentEnd, false, None);
    assert_eq!(session.text().stamp(), stamp, "no byte moved");
    assert_eq!(session.revisions().session, id);
}

#[test]
fn assigning_the_same_snapshot_changes_nothing_and_compares_nothing() {
    let mut session = EditSession::new("a long enough text");
    let snapshot = session.snapshot();
    let revisions = session.revisions();
    assert_eq!(session.assign(&snapshot, None), EditChange::None);
    assert_eq!(session.revisions(), revisions);
}

#[test]
fn assigning_new_text_reports_only_what_changed() {
    let mut session = EditSession::new("one two three");
    session.set_selection(8, 13, Affinity::Downstream);
    let EditChange::Text(edit) = session.assign(&SharedText::from("one 2 three"), None) else {
        panic!("the text changed");
    };
    assert_eq!(edit.range, 4..7);
    assert_eq!(edit.inserted_len, 1);
    assert_eq!(session.as_str(), "one 2 three");
    assert_eq!(
        (session.selection().anchor, session.selection().focus),
        (6, 11),
        "the selection after the edit moved with it"
    );
}

// ---- several selections ----------------------------------------------------

/// The Runtime rule this replaces, kept verbatim as the reference.
fn reference_normalize(
    primary: EditSelection,
    others: &[EditSelection],
) -> (EditSelection, Vec<EditSelection>) {
    let mut flagged: Vec<(EditSelection, bool)> = std::iter::once((primary, true))
        .chain(others.iter().map(|&selection| (selection, false)))
        .collect();
    flagged.sort_by_key(|(selection, _)| (selection.range().start, selection.range().end));
    let mut merged: Vec<(EditSelection, bool)> = Vec::new();
    for (next, is_primary) in flagged {
        match merged.last_mut() {
            Some((last, last_is_primary)) if last.range().end >= next.range().start => {
                let start = last.range().start;
                let end = last.range().end.max(next.range().end);
                *last = EditSelection::new(start, end);
                *last_is_primary |= is_primary;
            }
            _ => merged.push((next, is_primary)),
        }
    }
    let mut result = None;
    let mut rest = Vec::new();
    for (selection, is_primary) in merged {
        if is_primary && result.is_none() {
            result = Some(selection);
        } else {
            rest.push(selection);
        }
    }
    (result.unwrap_or_default(), rest)
}

#[test]
fn normalization_matches_the_runtime_rule_it_replaces() {
    // A small linear congruential generator: deterministic, no dependency.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as usize) % bound
    };
    for _ in 0..2_000 {
        let pick = |next: &mut dyn FnMut(usize) -> usize| {
            let anchor = next(40);
            let focus = if next(3) == 0 { anchor } else { next(40) };
            EditSelection::new(anchor, focus)
        };
        let primary = pick(&mut next);
        let count = next(6);
        let others: Vec<EditSelection> = (0..count).map(|_| pick(&mut next)).collect();
        assert_eq!(
            normalize_selections(primary, others.iter().copied()),
            reference_normalize(primary, &others),
            "primary {primary:?}, others {others:?}"
        );
    }
}

#[test]
fn typing_with_several_cursors_is_one_edit_with_a_caret_after_each_insertion() {
    let mut session = session("ab cd ef", caret(5), &[caret(2), caret(8)]);
    let revision = session.revisions().text;
    let _ = session.take_work();
    let EditChange::Text(edit) = session.insert("X") else {
        panic!("typing edits");
    };
    assert_eq!(session.as_str(), "abX cdX efX");
    assert_eq!(edit.range, 2..8, "one edit spanning every cursor");
    assert_eq!(session.revisions().text, revision.next(), "one revision");
    assert_eq!(offsets(&session), vec![(3, 3), (7, 7), (11, 11)]);
    assert_eq!(
        session.selection().focus,
        7,
        "the primary stays the primary"
    );
    assert_eq!(session.primary_index(), 1);
    let work = session.take_work();
    assert_eq!(work.editable_mutations, 1);
    assert_eq!(work.editable_bytes_inserted, 3);
}

#[test]
fn deleting_into_each_other_fuses_the_cursors() {
    let mut session = session("abcd", caret(2), &[caret(1)]);
    assert!(!session.delete(Motion::GraphemeBackward, None).is_none());
    assert_eq!(session.as_str(), "cd");
    assert_eq!(
        offsets(&session),
        vec![(0, 0)],
        "both carets land at 0 and fuse"
    );
    assert!(!session.has_additional_selections());
}

#[test]
fn every_cursor_moves_and_cursors_that_meet_fuse() {
    let mut session = session("abc", caret(1), &[caret(2)]);
    assert!(
        !session
            .move_caret(Motion::GraphemeForward, false, None)
            .is_none()
    );
    assert_eq!(offsets(&session), vec![(2, 2), (3, 3)]);
    assert!(
        !session
            .move_caret(Motion::DocumentEnd, false, None)
            .is_none()
    );
    assert_eq!(offsets(&session), vec![(3, 3)]);
}

#[test]
fn a_caret_update_is_only_counted_when_every_cursor_is_a_caret() {
    let mut session = session("abcdef", caret(1), &[span(3, 5)]);
    let _ = session.take_work();
    session.move_caret(Motion::DocumentStart, true, None);
    let work = session.take_work();
    assert_eq!(work.caret_only_updates, 0);
    assert_eq!(work.selection_only_updates, 1);
}

#[test]
fn setting_one_selection_drops_the_others_and_adding_fuses() {
    let mut session = session("abcdef", caret(1), &[caret(4)]);
    assert!(!session.add_selections([span(3, 4)]).is_none());
    assert_eq!(
        offsets(&session),
        vec![(1, 1), (3, 4)],
        "a caret touching a span fuses into it"
    );
    session.set_selection(2, 2, Affinity::Downstream);
    assert_eq!(offsets(&session), vec![(2, 2)]);
    assert!(session.collapse_selections().is_none());
}

#[test]
fn cutting_several_selections_joins_their_text() {
    let mut session = session("one two three", span(0, 3), &[span(8, 13)]);
    assert_eq!(session.cut().as_deref(), Some("one\nthree"));
    assert_eq!(session.as_str(), " two ");
}

#[test]
fn a_host_computed_splice_lands_with_the_selections_it_names() {
    let mut session = session("aa bb", caret(0), &[caret(3)]);
    let change = session.splice(&[(0..0, "// "), (3..3, "// ")], caret(3), [caret(9)]);
    assert!(matches!(change, EditChange::Text(_)));
    assert_eq!(session.as_str(), "// aa // bb");
    assert_eq!(offsets(&session), vec![(3, 3), (9, 9)]);
}

// ---- composition -----------------------------------------------------------

#[test]
fn committing_moves_the_other_cursors_through_the_commit() {
    let mut session = session("ab cd", caret(2), &[caret(5)]);
    session.set_preedit("ni", None);
    assert!(
        session
            .move_caret(Motion::DocumentStart, false, None)
            .is_none()
    );
    session.commit("你");
    assert_eq!(session.as_str(), "ab你 cd");
    assert_eq!(offsets(&session), vec![(5, 5), (8, 8)]);
}

#[test]
fn an_edit_clear_of_the_preedit_keeps_it_and_one_reaching_it_cancels_it() {
    let mut session = EditSession::new("hello world");
    session.set_selection(6, 11, Affinity::Downstream);
    session.set_preedit("x", None);
    session.splice(&[(0..5, "HELLO")], span(6, 11), []);
    assert!(
        session.is_composing(),
        "the edit stayed clear of the preedit"
    );
    assert_eq!(session.display_text(), "HELLO x");
    session.splice(&[(0..0, ">> ")], span(9, 14), []);
    assert_eq!(
        session.composition().unwrap().replaced,
        9..14,
        "the preedit moved with it"
    );
    session.splice(&[(8..9, ""), (14..14, "!")], span(8, 13), []);
    assert!(
        session.is_composing(),
        "an insertion at its end lands after it, a deletion next to it leaves it"
    );
    assert_eq!(session.composition().unwrap().replaced, 8..13);
    assert_eq!(session.display_text(), ">> HELLOx!");
    let change = session.splice(&[(10..11, "")], caret(10), []);
    assert!(matches!(change, EditChange::Text(_)));
    assert!(
        !session.is_composing(),
        "an edit inside the preedit's range cancels it"
    );
    assert_eq!(session.as_str(), ">> HELLOwold!");
}

#[test]
fn a_refused_or_empty_splice_leaves_the_preedit_and_reports_what_changed() {
    let mut session = EditSession::new("hello world");
    session.set_selection(6, 11, Affinity::Downstream);
    session.set_preedit("x", None);
    let revisions = session.revisions();
    assert_eq!(
        session.splice(&[(6..11, "z"), (0..1, "q")], caret(0), []),
        EditChange::None,
        "unsorted edits are refused"
    );
    assert_eq!(session.revisions(), revisions, "and change nothing");
    assert!(session.is_composing());
    assert_eq!(
        session.splice(&[(6..11, "world")], caret(0), []),
        EditChange::None,
        "rewriting the replaced bytes as they are reaches nothing"
    );
    assert!(session.is_composing());
}

#[test]
fn an_insertion_at_the_preedit_start_goes_before_it() {
    let mut session = EditSession::new("hello world");
    session.set_selection(6, 11, Affinity::Downstream);
    session.set_preedit("x", None);
    // The common prefix puts the inserted space at 6, the preedit's start.
    session.assign(&SharedText::from("hello  world"), None);
    assert!(session.is_composing());
    assert_eq!(session.display_text(), "hello  x");
}

#[test]
fn edits_that_cancel_out_change_nothing() {
    let mut text = nana_text::EditableText::new("aa");
    let revision = text.revision();
    let stamp = text.stamp();
    assert_eq!(text.splice(&[(0..1, ""), (1..1, "a")]), Ok(None));
    assert_eq!((text.revision(), text.stamp()), (revision, stamp));
    assert!(
        nana_text::EditableText::default()
            .snapshot()
            .stamp()
            .is_some(),
        "a default editable text is stamped too"
    );
}

#[test]
fn unchanged_edits_in_a_batch_are_not_counted() {
    let mut session = session("ab ab", span(0, 1), &[span(3, 5)]);
    let _ = session.take_work();
    session.insert("a");
    let work = session.take_work();
    assert_eq!(session.as_str(), "ab a");
    assert_eq!(
        (work.editable_bytes_inserted, work.editable_bytes_deleted),
        (1, 2)
    );
}

#[test]
fn a_value_write_clear_of_the_preedit_keeps_it() {
    let mut session = EditSession::new("abc def");
    session.set_selection(7, 7, Affinity::Downstream);
    session.set_preedit("ni", None);
    session.assign(&SharedText::from("ABC def"), None);
    assert!(session.is_composing());
    assert_eq!(session.display_text(), "ABC defni");
}

#[test]
fn surrounding_deletion_moves_the_other_cursors() {
    let mut session = session("hello foo bar", span(6, 9), &[caret(13)]);
    assert!(!session.delete_surrounding(1, 1).is_none());
    assert_eq!(session.as_str(), "hellobar");
    assert_eq!(offsets(&session), vec![(5, 5), (8, 8)]);
}

// ---- rules shared with hosts ----------------------------------------------

#[test]
fn offsets_move_through_an_edit_the_way_cursors_do() {
    assert_eq!(remap_offset(2, 2, 3, 1), 2, "at the start stays");
    assert_eq!(
        remap_offset(3, 2, 3, 1),
        3,
        "inside lands after the insertion"
    );
    assert_eq!(remap_offset(6, 2, 3, 1), 4, "after shifts");
    let moved = remap_selection(caret(6).with_affinity(Affinity::Upstream), 2, 3, 1);
    assert_eq!(
        moved.affinity,
        Affinity::Downstream,
        "a moved focus drops its side"
    );
}

#[test]
fn a_selection_collapses_onto_its_edge_on_screen() {
    // LTR, one line: start is left.
    assert_eq!(
        collapse_edge(2..5, false, Some((10.0, 0.0)), Some((40.0, 0.0)), false),
        2
    );
    assert_eq!(
        collapse_edge(2..5, true, Some((10.0, 0.0)), Some((40.0, 0.0)), false),
        5
    );
    // RTL, one line: the logical end draws on the left.
    assert_eq!(
        collapse_edge(2..5, false, Some((40.0, 0.0)), Some((10.0, 0.0)), true),
        5
    );
    assert_eq!(
        collapse_edge(2..5, true, Some((40.0, 0.0)), Some((10.0, 0.0)), true),
        2
    );
    // Across lines or without geometry: reading order.
    assert_eq!(
        collapse_edge(2..5, true, Some((40.0, 0.0)), Some((10.0, 20.0)), false),
        5
    );
    assert_eq!(collapse_edge(2..5, true, None, None, true), 2);
    assert_eq!(collapse_edge(2..5, false, None, None, false), 2);
}

// ---- geometry named by stamp -----------------------------------------------

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .expect("fixture registers");
    NativeTextEngine::new(fonts)
}

fn style() -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from(format!("\"{}\"", fixture_family("noto-sans-sc")))),
        font_size_px: 16.0,
        ..TextStyle::default()
    }
}

#[test]
fn stamped_text_already_laid_out_is_recognised_without_comparing_a_byte() {
    let mut engine = engine();
    let style = style();
    let constraints = TextConstraints {
        preserve_lines: true,
        ..TextConstraints::default()
    };
    let text = SharedText::stamped("first line\nsecond line\nthird line");
    let mut geometry = EditorGeometry::new();
    let mut counters = TextWorkCounters::default();
    let first = geometry.sync_stamped(
        &mut engine,
        text.stamp(),
        &text,
        None,
        &style,
        &constraints,
        &mut counters,
    );
    assert_eq!(first.paragraphs_laid_out, 3);

    let mut probe = TextWorkCounters::default();
    let again = geometry.sync_stamped(
        &mut engine,
        text.clone().stamp(),
        &text,
        None,
        &style,
        &constraints,
        &mut probe,
    );
    assert!(again.unchanged);
    assert_eq!(probe.editor_text_bytes_compared, 0, "named, not compared");
    assert_eq!(probe.layouts_created, 0);

    // The same bytes without a name are compared once, and still kept.
    let anonymous = SharedText::from(text.as_str());
    let mut compared = TextWorkCounters::default();
    let by_content = geometry.sync_stamped(
        &mut engine,
        anonymous.stamp(),
        &anonymous,
        None,
        &style,
        &constraints,
        &mut compared,
    );
    assert!(by_content.unchanged);
    assert!(compared.editor_text_bytes_compared > 0);
}
