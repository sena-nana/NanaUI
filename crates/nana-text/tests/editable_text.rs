//! Issue #96: the editable path — storage, motion, IME composition, and editor
//! geometry read from real layouts.
//!
//! Every fixture is hermetic: the checked-in fonts plus the bundled UI face,
//! registered in a fresh font system per test.

mod support;

use nana_text::editable::{CompositionMarks, EditHit, GeometrySync};
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    Affinity, EditChange, EditSession, EditorGeometry, Motion, NativeTextEngine, TextConstraints,
    TextEngine, TextKind, TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::TextWrapBreak;
use std::sync::Arc;

use support::corpus::{fixture_bytes, fixture_family};

const UI: &[&str] = &["noto-sans-sc"];
const UI_AND_EMOJI: &[&str] = &["noto-sans-sc", "noto-emoji"];
const UI_AND_ARABIC: &[&str] = &["noto-sans-sc", "noto-sans-arabic"];
const UI_AND_KOREAN: &[&str] = &["noto-sans-sc", "noto-sans-kr"];

fn engine(ids: &[&str]) -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    for id in ids {
        fonts
            .register_bytes(fixture_bytes(id), &FaceDescriptor::default())
            .expect("fixture registers");
    }
    NativeTextEngine::new(fonts)
}

fn style(ids: &[&str]) -> TextStyle {
    let families: Vec<String> = ids
        .iter()
        .map(|id| format!("\"{}\"", fixture_family(id)))
        .collect();
    TextStyle {
        font_family: Some(Arc::from(families.join(", "))),
        font_size_px: 16.0,
        ..TextStyle::default()
    }
}

/// A multi-line editor box.
fn editor_constraints(max_width_px: Option<f32>) -> TextConstraints {
    TextConstraints {
        max_width_px,
        wrap: max_width_px.map(|_| TextWrapBreak::Word),
        preserve_lines: true,
        ..TextConstraints::default()
    }
}

struct Editor {
    engine: NativeTextEngine,
    style: TextStyle,
    constraints: TextConstraints,
    session: EditSession,
    geometry: EditorGeometry,
    counters: TextWorkCounters,
}

impl Editor {
    fn new(fonts: &[&str], text: &str, max_width_px: Option<f32>) -> Self {
        let mut editor = Self {
            engine: engine(fonts),
            style: style(fonts),
            constraints: editor_constraints(max_width_px),
            session: EditSession::new(text),
            geometry: EditorGeometry::new(),
            counters: TextWorkCounters::default(),
        };
        editor.sync();
        editor
    }

    fn sync(&mut self) -> GeometrySync {
        self.geometry.sync_session(
            &mut self.engine,
            &self.session,
            &self.style,
            &self.constraints,
            &mut self.counters,
        )
    }

    fn apply(&mut self, change: EditChange) -> GeometrySync {
        assert!(!change.is_none(), "the command should have applied");
        self.sync()
    }

    fn motion(&mut self, motion: Motion, extend: bool) -> EditChange {
        self.session
            .move_caret(motion, extend, Some(&self.geometry))
    }

    fn caret_x(&self) -> f32 {
        let selection = self.session.selection();
        self.geometry
            .caret_rect(selection.focus, selection.affinity)
            .expect("the caret resolves")
            .x_px
    }

    /// Engine shape and layout work so far, as one comparable value.
    fn engine_work(&self) -> (usize, usize, usize) {
        let shape = self.engine.shape_counters();
        let layout = self.engine.layout_counters();
        (
            shape.shape_cache_misses,
            shape.shape_cache_hits,
            layout.layout_created,
        )
    }
}

// ---- storage and typing ----------------------------------------------------

#[test]
fn latin_typing_and_deleting_counts_mutations_and_bytes() {
    let mut editor = Editor::new(UI, "", Some(300.0));
    for character in ["H", "e", "l", "l", "o"] {
        let change = editor.session.insert(character);
        assert!(matches!(change, EditChange::Text(_)));
    }
    editor.session.insert(" world");
    assert_eq!(editor.session.as_str(), "Hello world");
    assert!(matches!(
        editor.session.delete(Motion::WordBackward, None),
        EditChange::Text(_)
    ));
    assert_eq!(editor.session.as_str(), "Hello ");
    editor.session.delete(Motion::GraphemeBackward, None);
    assert_eq!(editor.session.as_str(), "Hello");
    editor
        .session
        .move_caret(Motion::DocumentStart, false, None);
    editor.session.delete(Motion::GraphemeForward, None);
    assert_eq!(editor.session.as_str(), "ello");
    assert_eq!(
        editor.session.delete(Motion::GraphemeBackward, None),
        EditChange::None,
        "nothing before the start"
    );

    let work = editor.session.take_work();
    assert_eq!(work.editable_mutations, 9);
    assert_eq!(work.editable_bytes_inserted, 11);
    assert_eq!(work.editable_bytes_deleted, 7);
    assert_eq!(
        work.caret_only_updates, 1,
        "only the move to the start is a caret-only update; edits move the caret as part of the edit"
    );
}

#[test]
fn a_selection_is_replaced_by_typing_and_collapses_to_its_edge_on_arrow_keys() {
    let mut editor = Editor::new(UI, "one two three", None);
    editor.session.set_selection(4, 7, Affinity::Downstream);
    assert_eq!(editor.session.selected_text(), Some("two"));
    editor.motion(Motion::Left, false);
    assert_eq!(editor.session.selection().focus, 4);
    editor.session.set_selection(4, 7, Affinity::Downstream);
    editor.motion(Motion::Right, false);
    assert_eq!(editor.session.selection().focus, 7);
    editor.session.set_selection(4, 7, Affinity::Downstream);
    editor.session.insert("2");
    assert_eq!(editor.session.as_str(), "one 2 three");
    assert_eq!(editor.session.selection().focus, 5);
}

#[test]
fn clipboard_copy_cut_and_paste_go_through_the_selection() {
    let mut session = EditSession::new("alpha beta");
    assert_eq!(session.selected_text(), None, "a caret copies nothing");
    assert_eq!(session.cut(), None);
    session.select_word_at(7);
    assert_eq!(session.selected_text(), Some("beta"));
    assert_eq!(session.cut().as_deref(), Some("beta"));
    assert_eq!(session.as_str(), "alpha ");
    session.move_caret(Motion::DocumentStart, false, None);
    session.insert("beta ");
    assert_eq!(session.as_str(), "beta alpha ");
    session.select_all();
    assert_eq!(session.selected_text(), Some("beta alpha "));
}

// ---- grapheme safety -------------------------------------------------------

#[test]
fn deleting_an_emoji_removes_the_whole_grapheme_cluster() {
    let family = "👩\u{200d}👩\u{200d}👧";
    let thumbs = "👍🏽";
    let text = format!("a{family}{thumbs}b");
    let mut editor = Editor::new(UI_AND_EMOJI, &text, Some(400.0));
    editor.motion(Motion::DocumentEnd, false);
    editor.motion(Motion::GraphemeBackward, false);
    {
        let change = editor.session.delete(Motion::GraphemeBackward, None);
        editor.apply(change)
    };
    assert_eq!(editor.session.as_str(), format!("a{family}b"));
    {
        let change = editor.session.delete(Motion::GraphemeBackward, None);
        editor.apply(change)
    };
    assert_eq!(editor.session.as_str(), "ab");

    // The caret never lands inside a cluster however it is placed.
    let mut editor = Editor::new(UI_AND_EMOJI, &text, Some(400.0));
    editor.session.set_selection(3, 3, Affinity::Downstream);
    assert_eq!(
        editor.session.selection().focus,
        1,
        "snapped out of the ZWJ sequence"
    );
    let hit = editor.geometry.hit_test(editor.caret_x() + 3.0, 8.0);
    assert!(editor.session.text().is_grapheme_boundary(hit.offset));
}

#[test]
fn combining_marks_move_with_their_base() {
    let text = "e\u{301}\u{323}x";
    let mut editor = Editor::new(UI, text, None);
    editor.motion(Motion::DocumentStart, false);
    editor.motion(Motion::GraphemeForward, false);
    assert_eq!(editor.session.selection().focus, text.len() - 1);
    editor.motion(Motion::Right, false);
    assert_eq!(editor.session.selection().focus, text.len());
    editor.motion(Motion::Left, false);
    editor.motion(Motion::Left, false);
    assert_eq!(editor.session.selection().focus, 0);

    for offset in 0..=text.len() {
        let hit = editor.geometry.hit_test(offset as f32 * 3.0, 8.0);
        assert!(
            hit.offset == 0 || hit.offset == text.len() - 1 || hit.offset == text.len(),
            "hit at {} landed inside the marked cluster",
            hit.offset
        );
    }
}

#[test]
fn a_caret_inside_a_ligature_lands_between_its_graphemes() {
    let text = "office";
    let editor = Editor::new(UI, text, None);
    let layout = {
        let mut engine = engine(UI);
        let mut counters = TextWorkCounters::default();
        engine.layout(
            TextKind::Editable,
            &TextSource::new(text),
            &style(UI),
            &editor.constraints,
            &mut counters,
        )
    };
    let ligature = layout.runs[0]
        .glyphs
        .iter()
        .find(|glyph| glyph.cluster == 1)
        .expect("the ffi cluster");
    assert_eq!(ligature.cluster_end, 4, "f, f and i ligate");

    let xs: Vec<f32> = (1..=4)
        .map(|offset| {
            editor
                .geometry
                .caret_rect(offset, Affinity::Downstream)
                .unwrap()
                .x_px
        })
        .collect();
    assert!(
        xs.windows(2).all(|pair| pair[1] > pair[0]),
        "carets inside the ligature are ordered and distinct: {xs:?}"
    );
    // Clicking at each interior caret resolves back to it.
    for (offset, x) in (1..=4).zip(&xs) {
        let hit = editor.geometry.hit_test(*x + 0.1, 8.0);
        assert_eq!(hit.offset, offset, "a click at the caret for {offset}");
    }
}

// ---- BiDi ------------------------------------------------------------------

#[test]
fn arrow_keys_walk_mixed_bidi_text_in_visual_order() {
    let text = "abc مرحبا def";
    let mut editor = Editor::new(UI_AND_ARABIC, text, None);
    editor.motion(Motion::DocumentStart, false);
    let mut xs = vec![editor.caret_x()];
    let mut visited = vec![editor.session.selection().focus];
    while !editor.motion(Motion::Right, false).is_none() {
        xs.push(editor.caret_x());
        visited.push(editor.session.selection().focus);
        assert!(visited.len() < 64, "right arrow must terminate");
    }
    // Where two positions share an x (a BiDi boundary) one press moves
    // logically without moving on screen; the caret never moves left.
    assert!(
        xs.windows(2).all(|pair| pair[1] >= pair[0]),
        "a right arrow never moves the caret left on screen: {xs:?}"
    );
    let reached: std::collections::BTreeSet<usize> = visited.iter().copied().collect();
    assert_eq!(
        reached.len(),
        text.chars().count() + 1,
        "every caret position is reachable: {visited:?}"
    );
    assert!(
        visited.windows(2).any(|pair| pair[1] < pair[0]),
        "inside the Arabic word a right arrow moves backwards logically: {visited:?}"
    );
    assert_eq!(*visited.last().unwrap(), text.len());

    // And back again.
    let mut back = 1;
    while !editor.motion(Motion::Left, false).is_none() {
        back += 1;
        assert!(back < 64);
    }
    assert_eq!(back, visited.len());
    assert_eq!(editor.session.selection().focus, 0);
}

#[test]
fn affinity_places_a_caret_at_a_bidi_boundary_on_the_side_it_belongs_to() {
    let text = "abcمرحبا";
    let editor = Editor::new(UI_AND_ARABIC, text, None);
    let boundary = 3;
    let upstream = editor
        .geometry
        .caret_rect(boundary, Affinity::Upstream)
        .unwrap();
    let downstream = editor
        .geometry
        .caret_rect(boundary, Affinity::Downstream)
        .unwrap();
    assert!(
        (upstream.x_px - downstream.x_px).abs() > 10.0,
        "after `c` and before the Arabic word are different places: {} vs {}",
        upstream.x_px,
        downstream.x_px
    );
    assert!(
        upstream.x_px < downstream.x_px,
        "`c` ends left of where the RTL word starts"
    );

    // A click on either place resolves to that place.
    for caret in [upstream, downstream] {
        let EditHit {
            offset, affinity, ..
        } = editor.geometry.hit_test(caret.x_px, 8.0);
        let drawn = editor.geometry.caret_rect(offset, affinity).unwrap();
        assert!(
            (drawn.x_px - caret.x_px).abs() < 1.0,
            "clicked at {}, caret drawn at {}",
            caret.x_px,
            drawn.x_px
        );
    }
}

#[test]
fn selecting_across_a_bidi_boundary_yields_disjoint_rectangles() {
    let text = "abc مرحبا def";
    let editor = Editor::new(UI_AND_ARABIC, text, None);
    // `c` plus the first two Arabic letters: logically contiguous, visually
    // split by the rest of the Arabic word.
    let start = 2;
    let end = text.find("مر").unwrap() + "مر".len();
    let rects = editor.geometry.selection_rects(start..end);
    assert!(
        rects.len() >= 2,
        "one logical range, several visual rects: {rects:?}"
    );
    for pair in rects.windows(2) {
        assert!(pair[0].right() <= pair[1].x + 0.5 || pair[1].right() <= pair[0].x + 0.5);
    }
}

// ---- lines -----------------------------------------------------------------

#[test]
fn a_selection_across_wrapped_lines_and_paragraphs_has_one_rect_per_line() {
    let text = "the quick brown fox jumps over the lazy dog\nsecond paragraph";
    let editor = Editor::new(UI, text, Some(120.0));
    assert!(editor.geometry.paragraph_count() == 2);
    assert!(
        editor.geometry.line_count() >= 4,
        "the first paragraph wraps"
    );
    let rects = editor.geometry.selection_rects(4..text.len());
    let mut tops: Vec<f32> = rects.iter().map(|rect| rect.y).collect();
    tops.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    assert_eq!(tops.len(), editor.geometry.line_count(), "{rects:?}");
    assert!(tops.windows(2).all(|pair| pair[1] > pair[0]));
}

#[test]
fn paragraph_geometry_matches_laying_the_whole_text_out() {
    for text in [
        "one\ntwo\n\nfour",
        "trailing\n",
        "\n\nleading",
        "a wrapped first paragraph that is long\nand a second",
        "",
    ] {
        let editor = Editor::new(UI, text, Some(150.0));
        let mut engine = engine(UI);
        let mut counters = TextWorkCounters::default();
        let whole = engine.layout(
            TextKind::Editable,
            &TextSource::new(text),
            &editor.style,
            &editor.constraints,
            &mut counters,
        );
        assert_eq!(editor.geometry.line_count(), whole.lines.len(), "{text:?}");
        let (width, height) = editor.geometry.size();
        let whole_height: f32 = whole.lines.iter().map(|line| line.metrics.height_px).sum();
        assert!(
            (height - whole_height).abs() < 0.01,
            "{text:?}: {height} vs {whole_height}"
        );
        assert!(
            (width
                - whole
                    .lines
                    .iter()
                    .map(|l| l.metrics.width_px)
                    .fold(0.0, f32::max))
            .abs()
                < 0.01
        );
        for line in &whole.lines {
            for byte in [line.source.start, line.source.end] {
                let whole_caret = whole
                    .caret_geometry(nana_text::CaretPosition::new(
                        byte,
                        Affinity::Downstream,
                        line.index,
                    ))
                    .unwrap();
                let affinity = if byte == line.source.end
                    && line.break_cause == nana_text::LineBreakCause::Wrap
                {
                    Affinity::Upstream
                } else {
                    Affinity::Downstream
                };
                let caret = editor.geometry.caret_rect(byte, affinity).unwrap();
                assert!(
                    (caret.x_px - whole_caret.x_px).abs() < 0.01
                        && (caret.y_px - whole_caret.top_y_px).abs() < 0.01,
                    "{text:?} byte {byte}: {caret:?} vs {whole_caret:?}"
                );
            }
        }
    }
}

#[test]
fn up_and_down_keep_the_goal_column_across_short_lines() {
    let text = "a long first line\nab\nanother long line";
    let mut editor = Editor::new(UI, text, None);
    editor.session.set_selection(12, 12, Affinity::Downstream);
    let goal = editor.caret_x();
    editor.motion(Motion::LineDown, false);
    assert_eq!(
        editor.session.selection().focus,
        20,
        "clamped to the end of `ab`"
    );
    editor.motion(Motion::LineDown, false);
    assert!(
        (editor.caret_x() - goal).abs() < 8.0,
        "the goal column survives the short line"
    );
    editor.motion(Motion::LineDown, false);
    assert_eq!(
        editor.session.selection().focus,
        text.len(),
        "past the last line is the end"
    );
    editor.motion(Motion::DocumentStart, false);
    assert!(editor.motion(Motion::LineUp, false).is_none());
}

#[test]
fn home_and_end_follow_visual_lines_in_a_wrapped_paragraph() {
    let text = "the quick brown fox jumps over the lazy dog";
    let mut editor = Editor::new(UI, text, Some(120.0));
    editor.session.set_selection(0, 0, Affinity::Downstream);
    editor.motion(Motion::LineEnd, false);
    let end = editor.session.selection();
    assert!(end.focus < text.len(), "the end of the first visual line");
    assert_eq!(
        end.affinity,
        Affinity::Upstream,
        "the caret stays on the wrapped line"
    );
    let first_line_y = editor
        .geometry
        .caret_rect(0, Affinity::Downstream)
        .unwrap()
        .y_px;
    let end_y = editor
        .geometry
        .caret_rect(end.focus, end.affinity)
        .unwrap()
        .y_px;
    assert_eq!(end_y, first_line_y);
    editor.motion(Motion::GraphemeForward, false);
    editor.motion(Motion::LineStart, false);
    assert!(editor.session.selection().focus > 0);
    editor.motion(Motion::ParagraphEnd, false);
    assert_eq!(editor.session.selection().focus, text.len());
}

// ---- pointer ---------------------------------------------------------------

#[test]
fn a_click_and_a_drag_select_between_hit_positions() {
    let text = "drag across these words\nand onto this line";
    let mut editor = Editor::new(UI, text, None);
    let press = editor.geometry.caret_rect(5, Affinity::Downstream).unwrap();
    let release = editor
        .geometry
        .caret_rect(33, Affinity::Downstream)
        .unwrap();
    let anchor = editor
        .geometry
        .hit_test(press.x_px + 0.5, press.y_px + press.height_px * 0.5);
    assert!(anchor.inside);
    assert_eq!(anchor.offset, 5);
    let focus = editor
        .geometry
        .hit_test(release.x_px + 0.5, release.y_px + release.height_px * 0.5);
    assert_eq!(focus.offset, 33);
    editor
        .session
        .set_selection(anchor.offset, focus.offset, focus.affinity);
    assert_eq!(
        editor.session.selected_text(),
        Some("across these words\nand onto ")
    );

    let (_, height) = editor.geometry.size();
    let below = editor.geometry.hit_test(10_000.0, height + 50.0);
    assert!(!below.inside);
    assert_eq!(
        below.offset,
        text.len(),
        "dragging past the end selects to the end"
    );
    let above = editor.geometry.hit_test(-50.0, -50.0);
    assert_eq!(above.offset, 0);
}

// ---- IME -------------------------------------------------------------------

#[test]
fn pinyin_composition_shows_preedit_without_touching_committed_text() {
    let mut editor = Editor::new(UI, "我说", Some(300.0));
    let committed_revision = editor.session.revisions().text;
    for preedit in ["n", "ni", "nih", "nihao"] {
        let sync = {
            let change = editor.session.set_preedit(preedit, None);
            editor.apply(change)
        };
        assert_eq!(sync.paragraphs_laid_out, 1);
        assert_eq!(
            editor.session.as_str(),
            "我说",
            "the preedit is not committed"
        );
        assert_eq!(editor.session.display_text(), format!("我说{preedit}"));
    }
    assert_eq!(editor.session.revisions().text, committed_revision);
    assert!(
        editor.session.insert("x").is_none(),
        "typing waits for the IME"
    );
    assert!(
        editor
            .session
            .move_caret(Motion::Left, false, Some(&editor.geometry))
            .is_none()
    );

    // The candidate window sits at the end of the preedit.
    let candidate = editor.session.candidate_rect(&editor.geometry).unwrap();
    let end = editor
        .geometry
        .caret_rect("我说nihao".len(), Affinity::Downstream)
        .unwrap();
    assert_eq!(candidate, end);

    // The preedit shapes as its own run, which is what lets it be decorated.
    let (_, _, layout) = editor.geometry.paragraph_layouts().next().unwrap();
    assert!(
        layout
            .runs
            .iter()
            .any(|run| run.source == ("我说".len().."我说nihao".len()))
    );

    {
        let change = editor.session.commit("你好");
        editor.apply(change)
    };
    assert_eq!(editor.session.as_str(), "我说你好");
    assert!(!editor.session.is_composing());
    assert_eq!(editor.session.selection().focus, "我说你好".len());
    let work = editor.session.take_work();
    assert_eq!(work.composition_updates, 5, "four preedits and the commit");
    assert_eq!(
        work.editable_mutations, 1,
        "one commit, not one edit per keystroke"
    );
}

#[test]
fn a_composition_replaces_the_selection_it_started_over_and_cancel_restores_it() {
    let mut editor = Editor::new(UI, "replace THIS word", None);
    editor.session.set_selection(8, 12, Affinity::Downstream);
    {
        let change = editor.session.set_preedit("にほん", Some(0..9));
        editor.apply(change)
    };
    assert_eq!(editor.session.display_text(), "replace にほん word");
    assert_eq!(editor.session.as_str(), "replace THIS word");
    let composition = editor.session.composition().unwrap();
    assert_eq!(composition.display_target(), Some(8..17));
    let (_, _, layout) = editor.geometry.paragraph_layouts().next().unwrap();
    assert!(
        layout.runs.iter().any(|run| run.source == (8..17)),
        "the target segment shapes as its own run"
    );

    // Converting: the IME narrows its target to the first segment.
    {
        let change = editor.session.set_preedit("日本", Some(0..6));
        editor.apply(change)
    };
    {
        let change = editor.session.cancel_composition();
        editor.apply(change)
    };
    assert_eq!(editor.session.display_text(), "replace THIS word");
    assert_eq!(editor.session.selection().range(), 8..12);

    {
        let change = editor.session.set_preedit("にほん", None);
        editor.apply(change)
    };
    {
        let change = editor.session.commit("日本");
        editor.apply(change)
    };
    assert_eq!(editor.session.as_str(), "replace 日本 word");
}

#[test]
fn korean_jamo_composition_commits_the_syllable() {
    let mut editor = Editor::new(UI_AND_KOREAN, "", Some(200.0));
    for preedit in ["ㅎ", "하", "한"] {
        {
            let change = editor.session.set_preedit(preedit, None);
            editor.apply(change)
        };
        let caret = editor.session.candidate_rect(&editor.geometry).unwrap();
        assert!(caret.x_px > 0.0);
    }
    {
        let change = editor.session.commit("한");
        editor.apply(change)
    };
    {
        let change = editor.session.set_preedit("ㄱ", None);
        editor.apply(change)
    };
    {
        let change = editor.session.commit("글");
        editor.apply(change)
    };
    assert_eq!(editor.session.as_str(), "한글");
    {
        let change = editor.session.delete(Motion::GraphemeBackward, None);
        editor.apply(change)
    };
    assert_eq!(
        editor.session.as_str(),
        "한",
        "a precomposed syllable deletes as one"
    );
}

#[test]
fn losing_focus_mid_composition_cancels_it() {
    let mut editor = Editor::new(UI, "text", None);
    {
        let change = editor.session.set_preedit("zhong", None);
        editor.apply(change)
    };
    let change = editor.session.blur();
    assert_eq!(change, EditChange::Composition);
    editor.sync();
    assert_eq!(editor.session.as_str(), "text");
    assert_eq!(editor.session.display_text(), "text");
    assert_eq!(editor.session.blur(), EditChange::None);
    assert!(!editor.session.insert("!").is_none(), "editing resumes");
}

#[test]
fn surrounding_text_and_its_deletion_follow_the_selection_and_keep_the_preedit() {
    let mut session = EditSession::new("hello wide world");
    session.set_selection(6, 10, Affinity::Downstream);
    let around = session.surrounding_text(3, 3);
    assert_eq!(around.text, "lo wide wo");
    assert_eq!(around.offset, 3);
    assert_eq!(around.selection, (3, 7));

    session.set_selection(6, 6, Affinity::Downstream);
    session.set_preedit("ab", None);
    assert!(matches!(
        session.delete_surrounding(1, 0),
        EditChange::Text(_)
    ));
    assert_eq!(session.as_str(), "hellowide world");
    assert_eq!(
        session.display_text(),
        "helloabwide world",
        "the preedit moved with the text"
    );

    // Composing over a selection: the text the preedit stands in for is not
    // surrounding text, and cancelling still gets it back.
    let mut composing = EditSession::new("hello foo bar");
    composing.set_selection(6, 9, Affinity::Downstream);
    composing.set_preedit("x", None);
    assert!(matches!(
        composing.delete_surrounding(1, 1),
        EditChange::Text(_)
    ));
    assert_eq!(composing.as_str(), "hellofoobar");
    assert_eq!(composing.display_text(), "helloxbar");
    composing.cancel_composition();
    assert_eq!(composing.display_text(), "hellofoobar");
    assert_eq!(composing.selected_text(), Some("foo"));
    let work = composing.take_work();
    assert_eq!(work.editable_bytes_deleted, 2);
    assert_eq!(work.editable_bytes_inserted, 0);
    assert_eq!(session.delete_surrounding(100, 0), EditChange::None);
    let text = EditSession::new("中文");
    let cut = text.surrounding_text(1, 0);
    assert!(cut.text.is_char_boundary(0), "cut on a character boundary");
}

// ---- incremental invalidation and structural gates ------------------------

#[test]
fn a_caret_blink_does_no_text_work() {
    let editor = Editor::new(UI, "blink\nblink blink", Some(200.0));
    let before = editor.engine_work();
    // A blink redraws the caret it already has: geometry queries only.
    for _ in 0..120 {
        let selection = editor.session.selection();
        editor
            .geometry
            .caret_rect(selection.focus, selection.affinity);
    }
    assert_eq!(
        editor.engine_work(),
        before,
        "shape_runs_created == 0 and layout_created == 0"
    );
    let (_, carets) = editor.geometry.take_query_counts();
    assert_eq!(carets, 120);
}

#[test]
fn selection_only_updates_do_not_shape_or_lay_out() {
    let mut editor = Editor::new(UI, "select\nsome text here", Some(200.0));
    let before = editor.engine_work();
    for motion in [
        Motion::DocumentStart,
        Motion::WordForward,
        Motion::LineDown,
        Motion::Right,
        Motion::LineEnd,
    ] {
        assert!(!editor.motion(motion, true).is_none(), "{motion:?}");
        let sync = editor.sync();
        assert_eq!(sync.paragraphs_laid_out, 0);
    }
    editor.session.select_all();
    editor.sync();
    editor
        .geometry
        .selection_rects(editor.session.selection().range());
    assert_eq!(
        editor.engine_work(),
        before,
        "selection-only: shape_runs_created == 0"
    );
    let work = editor.session.take_work();
    assert_eq!(
        work.selection_only_updates + work.caret_only_updates,
        6,
        "LineEnd lands back on the anchor, a caret"
    );
    assert_eq!(work.editable_mutations, 0);
}

#[test]
fn an_edit_lays_out_only_its_own_paragraph() {
    let paragraphs: Vec<String> = (0..50)
        .map(|index| format!("paragraph number {index}"))
        .collect();
    let mut editor = Editor::new(UI, &paragraphs.join("\n"), Some(300.0));
    assert_eq!(editor.geometry.paragraph_count(), 50);

    let middle = editor.session.as_str().find("number 25").unwrap();
    editor
        .session
        .set_selection(middle, middle, Affinity::Downstream);
    let sync = {
        let change = editor.session.insert("x");
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 1);
    assert_eq!(sync.paragraphs_reshaped, 1);
    assert_eq!(sync.paragraphs_kept, 49);
    assert!(sync.incremental);
    assert_eq!(editor.counters.paragraphs_relayout_from_edit, 1);
    assert_eq!(editor.counters.paragraphs_reshaped_from_edit, 1);

    // Splitting a paragraph lays out the two halves.
    let sync = {
        let change = editor.session.insert("\n");
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 2);
    assert_eq!(editor.geometry.paragraph_count(), 51);
    // Joining them back lays out one, and finds its shaping cached.
    let sync = {
        let change = editor.session.delete(Motion::GraphemeBackward, None);
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 1);
    assert_eq!(sync.paragraphs_reshaped, 0);
    assert_eq!(editor.geometry.paragraph_count(), 50);

    // Every offset still resolves against the right paragraph.
    let last = editor.session.as_str().len();
    let caret = editor
        .geometry
        .caret_rect(last, Affinity::Downstream)
        .unwrap();
    let (_, height) = editor.geometry.size();
    assert!((caret.y_px + caret.height_px - height).abs() < 0.01);

    // A composition relays out only the paragraph it is in.
    let sync = {
        let change = editor.session.set_preedit("pin", None);
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 1);
    let sync = {
        let change = editor.session.set_preedit("pinyin", None);
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 1);
    let sync = {
        let change = editor.session.cancel_composition();
        editor.apply(change)
    };
    assert_eq!(sync.paragraphs_laid_out, 1);
}

#[test]
fn a_width_change_relays_out_every_paragraph_without_reshaping() {
    let mut editor = Editor::new(UI, "one two three\nfour five six\nseven", Some(300.0));
    editor.constraints = editor_constraints(Some(60.0));
    let before = editor.engine.shape_counters().shape_cache_misses;
    let sync = editor.sync();
    assert_eq!(sync.paragraphs_laid_out, 3);
    assert_eq!(sync.paragraphs_reshaped, 0);
    assert!(!sync.incremental);
    assert_eq!(
        editor.counters.paragraphs_relayout_from_edit, 0,
        "a constraint change is not edit work"
    );
    assert_eq!(editor.engine.shape_counters().shape_cache_misses, before);
}

#[test]
fn stale_geometry_is_not_used_for_motion() {
    let mut editor = Editor::new(UI, "abc", None);
    editor.session.insert("def");
    // Not synced: vertical and visual moves fall back to logical ones rather
    // than reading layouts of text that is gone.
    let change = editor
        .session
        .move_caret(Motion::Left, false, Some(&editor.geometry));
    assert_eq!(change, EditChange::Selection);
    assert_eq!(editor.session.selection().focus, 5);
}

#[test]
fn composition_marks_outside_a_paragraph_do_not_touch_it() {
    let mut engine = engine(UI);
    let style = style(UI);
    let constraints = editor_constraints(None);
    let mut geometry = EditorGeometry::new();
    let mut counters = TextWorkCounters::default();
    geometry.sync(
        &mut engine,
        "one\ntwo",
        None,
        &style,
        &constraints,
        &mut counters,
    );
    let marks = CompositionMarks {
        range: 4..7,
        target: None,
    };
    let sync = geometry.sync(
        &mut engine,
        "one\ntwo",
        Some(&marks),
        &style,
        &constraints,
        &mut counters,
    );
    assert_eq!(sync.paragraphs_laid_out, 1);
    assert_eq!(sync.paragraphs_kept, 1);
}

// ---- review regressions ---------------------------------------------------

#[test]
fn unsplit_text_is_never_reused_as_a_shifted_paragraph() {
    for constraints in [
        TextConstraints::default(),
        TextConstraints {
            preserve_lines: true,
            max_lines: Some(5),
            ..TextConstraints::default()
        },
    ] {
        let mut engine = engine(UI);
        let style = style(UI);
        let mut counters = TextWorkCounters::default();
        let mut incremental = EditorGeometry::new();
        incremental.sync(
            &mut engine,
            "abc",
            None,
            &style,
            &constraints,
            &mut counters,
        );
        incremental.sync(
            &mut engine,
            "x\nabc",
            None,
            &style,
            &constraints,
            &mut counters,
        );
        let mut whole = EditorGeometry::new();
        whole.sync(
            &mut engine,
            "x\nabc",
            None,
            &style,
            &constraints,
            &mut counters,
        );
        assert_eq!(incremental.paragraph_count(), 1, "{constraints:?}");
        assert_eq!(incremental.size(), whole.size(), "{constraints:?}");
        assert_eq!(incremental.line_count(), whole.line_count());
    }
}

#[test]
fn replacing_the_text_while_composing_reports_the_composition_ending() {
    let mut session = EditSession::new("same");
    session.set_preedit("pre", None);
    assert_eq!(session.set_text("same"), EditChange::Composition);
    assert!(!session.is_composing());
}

#[test]
fn a_caret_after_typed_rtl_text_draws_beside_what_was_typed() {
    let mut editor = Editor::new(UI_AND_ARABIC, "abc ", None);
    for letter in ["م", "ر", "ح"] {
        let change = editor.session.insert(letter);
        editor.apply(change);
    }
    let caret = editor.caret_x();
    let rects = editor
        .geometry
        .selection_rects(4..editor.session.as_str().len());
    let rtl = rects.first().expect("the typed word draws");
    assert!(
        (caret - rtl.x).abs() < 1.0,
        "the logical end of the typed RTL word is its left edge {}, caret at {caret}",
        rtl.x
    );
}

/// A host answering probes has no session to name its text with, so it syncs
/// before every probe. Re-syncing to text the layouts already lay out must
/// cost a comparison: the paragraphs keep the very layouts they had, no
/// engine work happens, and the session that put them there still sees the
/// geometry as its own.
#[test]
fn syncing_to_text_the_layouts_already_lay_out_keeps_them() {
    let mut editor = Editor::new(UI, "one\ntwo\nthree", None);
    let layouts = |geometry: &EditorGeometry| -> Vec<usize> {
        geometry
            .paragraph_layouts()
            .map(|(_, _, layout)| Arc::as_ptr(layout) as usize)
            .collect()
    };
    let before = layouts(&editor.geometry);
    assert_eq!(before.len(), 3);
    let work = editor.engine_work();
    let display = editor.session.display_text().into_owned();

    let sync = editor.geometry.sync(
        &mut editor.engine,
        &display,
        None,
        &editor.style,
        &editor.constraints,
        &mut editor.counters,
    );

    assert_eq!(sync.paragraphs_laid_out, 0, "{sync:?}");
    assert_eq!(sync.paragraphs_kept, 3, "{sync:?}");
    assert_eq!(
        layouts(&editor.geometry),
        before,
        "the layouts are the same"
    );
    assert_eq!(editor.engine_work(), work, "no shaping and no layout");
    assert!(
        editor.geometry.synced_from(editor.session.revisions()),
        "a host probe of unchanged text does not make the geometry stale"
    );
}

/// Every incremental sync has to land on exactly the geometry a sync from
/// scratch would produce. The prefix/suffix matching and the in-place splice
/// are what could drift, and drift would only show up after a sequence of
/// edits -- so this walks one geometry through a sequence while comparing it,
/// at every step, against a geometry that has only ever seen the current
/// text: paragraph starts and tops, line count, size, every caret, and a grid
/// of hits.
#[test]
fn incremental_syncs_land_where_syncing_from_scratch_does() {
    #[derive(Debug, PartialEq)]
    struct Snapshot {
        paragraphs: Vec<(usize, f32, usize)>,
        lines: usize,
        size: (f32, f32),
        carets: Vec<Option<(f32, f32, f32)>>,
        hits: Vec<(usize, Affinity, bool)>,
    }
    let snapshot = |geometry: &EditorGeometry, text: &str| Snapshot {
        paragraphs: geometry
            .paragraph_layouts()
            .map(|(start, top, layout)| (start, top, layout.lines.len()))
            .collect(),
        lines: geometry.line_count(),
        size: geometry.size(),
        carets: (0..=text.len())
            .filter(|offset| text.is_char_boundary(*offset))
            .flat_map(|offset| {
                [Affinity::Downstream, Affinity::Upstream].map(move |affinity| (offset, affinity))
            })
            .map(|(offset, affinity)| {
                geometry
                    .caret_rect(offset, affinity)
                    .map(|caret| (caret.x_px, caret.y_px, caret.height_px))
            })
            .collect(),
        hits: (0..6)
            .flat_map(|row| (0..6).map(move |column| (column as f32 * 40.0, row as f32 * 12.0)))
            .map(|(x, y)| {
                let hit = geometry.hit_test(x, y);
                (hit.offset, hit.affinity, hit.inside)
            })
            .collect(),
    };

    // Insert and delete at the head, in the middle and at the tail; split and
    // join paragraphs; grow and lose a trailing line feed; empty the text and
    // fill it again; and change one byte inside a multi-byte character.
    let sequence = [
        "one
two
three",
        "Zone
two
three",
        "Zone
two
three!",
        "Zone
two and more
three!",
        "Zone
two and more
three!
",
        "Zone
two and more

three!
",
        "Zone
two and morethree!
",
        "Zone
two and morethree!",
        "",
        "
",
        "你好
吗",
        "你奿
吗",
        "你奿
吗 and a much longer line that has to wrap somewhere",
        "你奿
吗 and a much longer line that has to wrap elsewhere",
        "one
two
three",
    ];
    // Composition marks are part of what the prefix/suffix matching compares,
    // so every other step carries a preedit over the second line.
    let marks_for = |step: usize, text: &str| -> Option<CompositionMarks> {
        if !step.is_multiple_of(2) {
            return None;
        }
        let start = text.find('\n').map(|index| index + 1)?;
        let end = text[start..]
            .find('\n')
            .map_or(text.len(), |index| start + index);
        if start >= end {
            return None;
        }
        Some(CompositionMarks {
            range: start..end,
            target: Some(start..end),
        })
    };
    for width in [None, Some(150.0)] {
        let mut engine = engine(UI);
        let style = style(UI);
        let constraints = editor_constraints(width);
        let mut counters = TextWorkCounters::default();
        let mut incremental = EditorGeometry::new();
        for (step, text) in sequence.into_iter().enumerate() {
            let marks = marks_for(step, text);
            incremental.sync(
                &mut engine,
                text,
                marks.as_ref(),
                &style,
                &constraints,
                &mut counters,
            );
            let mut fresh = EditorGeometry::new();
            fresh.sync(
                &mut engine,
                text,
                marks.as_ref(),
                &style,
                &constraints,
                &mut counters,
            );
            assert_eq!(
                snapshot(&incremental, text),
                snapshot(&fresh, text),
                "{width:?} step {step} {text:?}"
            );
        }
    }
}

#[test]
fn geometry_synced_from_one_session_is_stale_for_another() {
    let editor = Editor::new(UI, "abc\ndef", None);
    let other = EditSession::new("abc\ndef");
    assert_eq!(other.revisions().text, editor.session.revisions().text);
    assert!(editor.geometry.synced_from(editor.session.revisions()));
    assert!(!editor.geometry.synced_from(other.revisions()));
    let copy = editor.session.clone();
    assert!(!editor.geometry.synced_from(copy.revisions()));
}

#[test]
fn arrow_keys_collapse_an_rtl_selection_onto_its_visual_edge() {
    let text = "مرحبا";
    let mut editor = Editor::new(UI_AND_ARABIC, text, None);
    editor
        .session
        .set_selection(0, text.len(), Affinity::Downstream);
    editor.motion(Motion::Left, false);
    assert_eq!(
        editor.session.selection().focus,
        text.len(),
        "the left edge of RTL text is its logical end"
    );
    editor
        .session
        .set_selection(0, text.len(), Affinity::Downstream);
    editor.motion(Motion::Right, false);
    assert_eq!(editor.session.selection().focus, 0);
}

#[test]
fn deleting_to_the_line_start_follows_the_visual_line() {
    let text = "the quick brown fox jumps over the lazy dog";
    let mut editor = Editor::new(UI, text, Some(120.0));
    editor
        .session
        .set_selection(text.len(), text.len(), Affinity::Downstream);
    let change = editor
        .session
        .delete(Motion::LineStart, Some(&editor.geometry));
    editor.apply(change);
    assert!(
        editor.session.as_str().len() > 10,
        "only the last visual line went: {:?}",
        editor.session.as_str()
    );
}

#[test]
fn caret_geometry_scales_with_a_fractional_device_scale() {
    let text = "scaled text\nsecond";
    let mut editor = Editor::new(UI, text, None);
    let unscaled = editor.geometry.caret_rect(6, Affinity::Downstream).unwrap();
    editor.constraints.scale = nana_text::TextScale {
        px_per_logical: 1.5,
    };
    editor.sync();
    let scaled = editor.geometry.caret_rect(6, Affinity::Downstream).unwrap();
    assert!(
        (scaled.x_px - unscaled.x_px * 1.5).abs() < 0.5,
        "{scaled:?} vs {unscaled:?}"
    );
    assert!((scaled.height_px - unscaled.height_px * 1.5).abs() < 0.5);
    for offset in [0, 3, 6, 11, 12, text.len()] {
        let caret = editor
            .geometry
            .caret_rect(offset, Affinity::Downstream)
            .unwrap();
        let hit = editor
            .geometry
            .hit_test(caret.x_px + 0.2, caret.y_px + caret.height_px * 0.5);
        assert_eq!(hit.offset, offset, "at 1.5x");
    }
}

#[test]
fn hit_tests_and_carets_hold_across_fallback_runs() {
    let text = "a👩\u{200d}💻b中c";
    let editor = Editor::new(UI_AND_EMOJI, text, None);
    let (_, _, layout) = editor.geometry.paragraph_layouts().next().unwrap();
    let fonts: std::collections::HashSet<_> = layout.runs.iter().map(|run| run.font).collect();
    assert!(fonts.len() >= 2, "the emoji comes from a fallback face");
    let mut previous = f32::NEG_INFINITY;
    for offset in [0, 1, 12, 13, 16, text.len()] {
        let caret = editor
            .geometry
            .caret_rect(offset, Affinity::Downstream)
            .unwrap();
        assert!(caret.x_px > previous, "{offset}");
        previous = caret.x_px;
        let hit = editor.geometry.hit_test(caret.x_px + 0.2, 8.0);
        assert_eq!(hit.offset, offset);
    }
}

#[test]
fn a_selection_across_lines_collapses_in_reading_order() {
    let text = "hello world\nab";
    let mut editor = Editor::new(UI, text, None);
    editor.session.set_selection(8, 13, Affinity::Downstream);
    editor.motion(Motion::Right, false);
    assert_eq!(editor.session.selection().focus, 13);
    editor.session.set_selection(8, 13, Affinity::Downstream);
    editor.motion(Motion::Left, false);
    assert_eq!(editor.session.selection().focus, 8);
}

#[test]
fn every_position_of_a_line_ending_in_opposite_direction_text_is_reachable() {
    for (text, from_end) in [("abc مرحبا", false), ("مرحبا\nabc", true)] {
        let mut editor = Editor::new(UI_AND_ARABIC, text, None);
        let (start, motion, back) = if from_end {
            (Motion::DocumentEnd, Motion::Left, Motion::Right)
        } else {
            (Motion::DocumentStart, Motion::Right, Motion::Left)
        };
        editor.motion(start, false);
        let mut visited = std::collections::BTreeSet::from([editor.session.selection().focus]);
        let mut steps = 0;
        while !editor.motion(motion, false).is_none() {
            visited.insert(editor.session.selection().focus);
            steps += 1;
            assert!(steps < 64, "{text:?}: {motion:?} must terminate");
        }
        let boundaries: std::collections::BTreeSet<usize> = (0..=text.len())
            .filter(|offset| editor.session.text().is_grapheme_boundary(*offset))
            .collect();
        assert_eq!(visited, boundaries, "{text:?}: every position one way");
        let mut returned = 0;
        while !editor.motion(back, false).is_none() {
            returned += 1;
            assert!(returned < 64);
        }
        assert_eq!(returned, steps, "{text:?}: and the same way back");
    }
}

#[test]
fn a_selection_across_rtl_lines_collapses_in_rtl_reading_order() {
    let text = "مرحبا\nعالم";
    let mut editor = Editor::new(UI_AND_ARABIC, text, None);
    editor.constraints.base_direction = nana_ui_core::DirSpec::Rtl;
    editor.sync();
    let (start, end) = (2, text.len() - 2);
    editor
        .session
        .set_selection(start, end, Affinity::Downstream);
    editor.motion(Motion::Right, false);
    assert_eq!(
        editor.session.selection().focus,
        start,
        "right reads backwards in RTL"
    );
    editor
        .session
        .set_selection(start, end, Affinity::Downstream);
    editor.motion(Motion::Left, false);
    assert_eq!(editor.session.selection().focus, end);
}

#[test]
fn carets_and_hits_round_trip_on_wrapped_lines_ending_in_opposite_direction_text() {
    for (text, direction) in [
        ("aaaa bbbb مرحبا cccc", nana_ui_core::DirSpec::Ltr),
        ("مرحبا بالعالم الجميل", nana_ui_core::DirSpec::Ltr),
        ("abc مرحبا def", nana_ui_core::DirSpec::Rtl),
    ] {
        let mut editor = Editor::new(UI_AND_ARABIC, text, Some(70.0));
        editor.constraints.wrap = Some(TextWrapBreak::WordOrGlyph);
        editor.constraints.base_direction = direction;
        editor.sync();
        assert!(editor.geometry.line_count() > 1, "{text:?} wraps");
        for offset in
            (0..=text.len()).filter(|offset| editor.session.text().is_grapheme_boundary(*offset))
        {
            for affinity in [Affinity::Downstream, Affinity::Upstream] {
                let Some(caret) = editor.geometry.caret_rect(offset, affinity) else {
                    continue;
                };
                let hit = editor
                    .geometry
                    .hit_test(caret.x_px, caret.y_px + caret.height_px * 0.5);
                let back = editor
                    .geometry
                    .caret_rect(hit.offset, hit.affinity)
                    .unwrap();
                assert!(
                    (back.x_px - caret.x_px).abs() < 1.0 && back.y_px == caret.y_px,
                    "{text:?} {offset} {affinity:?}: drawn at {caret:?}, hit {hit:?} draws at {back:?}"
                );
            }
        }
        // Past either end of every line lands at that end of that line.
        let (width, height) = editor.geometry.size();
        let mut y = 1.0;
        while y < height {
            for x in [-20.0, width + 20.0] {
                let hit = editor.geometry.hit_test(x, y);
                let back = editor
                    .geometry
                    .caret_rect(hit.offset, hit.affinity)
                    .unwrap();
                assert!(
                    back.y_px <= y && y < back.y_px + back.height_px,
                    "{text:?} ({x}, {y})"
                );
            }
            y += 8.0;
        }
    }
}

#[test]
fn a_caret_at_the_logical_end_after_rtl_text_reaches_every_position_and_clicks_append() {
    let text = "abc مرحبا";
    let mut editor = Editor::new(UI_AND_ARABIC, text, None);
    let boundaries: std::collections::BTreeSet<usize> = (0..=text.len())
        .filter(|offset| editor.session.text().is_grapheme_boundary(*offset))
        .collect();

    // From the end, downstream: left all the way.
    editor.motion(Motion::DocumentEnd, false);
    let mut visited = std::collections::BTreeSet::from([editor.session.selection().focus]);
    while !editor.motion(Motion::Left, false).is_none() {
        visited.insert(editor.session.selection().focus);
        assert!(visited.len() <= boundaries.len() + 1);
    }
    assert_eq!(visited, boundaries);

    // From a caret left upstream by typing: right all the way.
    let mut editor = Editor::new(UI_AND_ARABIC, text, None);
    editor.session.set_selection(0, 0, Affinity::Downstream);
    let change = editor.session.insert("x");
    editor.apply(change);
    let typed: std::collections::BTreeSet<usize> = (0..=editor.session.as_str().len())
        .filter(|offset| editor.session.text().is_grapheme_boundary(*offset))
        .collect();
    let mut visited = std::collections::BTreeSet::from([editor.session.selection().focus]);
    let mut steps = 0;
    while !editor.motion(Motion::Right, false).is_none() {
        visited.insert(editor.session.selection().focus);
        steps += 1;
        assert!(steps < 64);
    }
    assert_eq!(
        visited,
        typed.into_iter().filter(|offset| *offset >= 1).collect()
    );

    // A click past the right of the line appends: it is the text's end.
    let editor = Editor::new(UI_AND_ARABIC, text, None);
    let (width, _) = editor.geometry.size();
    assert_eq!(
        editor.geometry.hit_test(width + 50.0, 8.0).offset,
        text.len()
    );
    // In an LTR line that starts with Arabic, the left edge is the Arabic
    // word's logical end, not byte 0 (which draws at the word's right edge).
    let editor = Editor::new(UI_AND_ARABIC, "مرحبا abc", None);
    let hit = editor.geometry.hit_test(-50.0, 8.0);
    let drawn = editor
        .geometry
        .caret_rect(hit.offset, hit.affinity)
        .unwrap();
    assert!(drawn.x_px.abs() < 0.5, "{hit:?} draws at {drawn:?}");
}

#[test]
fn arrow_keys_reach_every_position_inside_whitespace_hung_at_a_wrap() {
    let text = "aaaa   bbbb cccc";
    let mut editor = Editor::new(UI, text, Some(70.0));
    assert!(editor.geometry.line_count() > 1);
    editor.motion(Motion::DocumentStart, false);
    let mut visited = std::collections::BTreeSet::from([0]);
    let mut steps = 0;
    while !editor.motion(Motion::Right, false).is_none() {
        visited.insert(editor.session.selection().focus);
        steps += 1;
        assert!(steps < 64);
    }
    assert_eq!(visited, (0..=text.len()).collect());
    let mut back = std::collections::BTreeSet::from([text.len()]);
    while !editor.motion(Motion::Left, false).is_none() {
        back.insert(editor.session.selection().focus);
    }
    assert_eq!(back, (0..=text.len()).collect());
}
