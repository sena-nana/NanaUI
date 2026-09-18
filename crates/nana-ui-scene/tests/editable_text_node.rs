//! Issue #96: Runtime editors on the `nana-text` editable path, driven through
//! the product frame loop with an engine host.
//!
//! Caret, selection and pointer questions are answered from the editor's
//! retained per-paragraph geometry; an edit lays out its own paragraph; caret
//! and selection moves do no text layout at all.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine, TextWorkCounters};
use nana_ui_runtime::{
    DocumentId, Entity, LayoutBox, LayoutViewport, NanaTextEngineShaper, TextAffinity, TextArea,
    TextCaretIntent, TextSelection,
};
use nana_ui_scene::RuntimeDocument;

const DOCUMENT: u64 = 1;

fn engine() -> SharedTextEngine {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 800.0)
}

struct Fixture {
    runtime: RuntimeDocument,
    shaper: NanaTextEngineShaper,
    engine: SharedTextEngine,
    area: Entity<TextArea>,
    document: DocumentId,
}

impl Fixture {
    fn new(text: &str) -> Self {
        let document = DocumentId::new(DOCUMENT).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let area = runtime
            .context_mut()
            .build(document, |ui| ui.child("editor", TextArea::new(text)))
            .unwrap();
        let engine = engine();
        let shaper = NanaTextEngineShaper::new(Arc::clone(&engine));
        let mut fixture = Self {
            runtime,
            shaper,
            engine,
            area,
            document,
        };
        assert!(
            fixture
                .runtime
                .context_mut()
                .focus_node(document, area.stable_id())
                .unwrap()
        );
        fixture.settle();
        fixture
    }

    fn flush(&mut self) -> TextWorkCounters {
        self.runtime.flush(viewport(), &mut self.shaper).unwrap();
        self.runtime.context().world().last_text_work_counters()
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.flush();
        }
    }

    fn layouts_created(&self) -> usize {
        nana_text::lock_text_engine(&self.engine)
            .layout_counters()
            .layout_created
    }

    fn shapes(&self) -> usize {
        nana_text::lock_text_engine(&self.engine)
            .shape_counters()
            .shape_cache_misses
    }

    fn value(&self) -> String {
        self.runtime
            .context()
            .world()
            .text_input(self.area.stable_id())
            .unwrap()
            .value
            .clone()
    }

    fn caret(&self) -> (f32, f32) {
        let presentation = self
            .runtime
            .context()
            .world()
            .text_input_presentation(self.area.stable_id())
            .unwrap();
        (presentation.caret_x, presentation.caret_y)
    }

    fn selection(&self) -> TextSelection {
        self.runtime
            .context()
            .world()
            .text_input(self.area.stable_id())
            .unwrap()
            .selection
    }

    fn content(&self) -> LayoutBox {
        self.runtime
            .context()
            .world()
            .text_input_pointer_context(self.area.stable_id())
            .unwrap()
            .0
    }

    fn line_height(&self) -> f32 {
        self.runtime
            .context()
            .world()
            .text_input_presentation(self.area.stable_id())
            .unwrap()
            .line_height
    }

    /// One click at a document point, settled.
    fn click(&mut self, x: f32, y: f32) {
        let document = self.document;
        let node = self.area.stable_id();
        let Fixture {
            runtime, shaper, ..
        } = self;
        runtime
            .context_mut()
            .text_editor_pointer_press(
                document,
                node,
                1,
                x,
                y,
                false,
                false,
                std::time::Duration::from_secs(10),
                shaper,
            )
            .unwrap();
        runtime.context_mut().text_editor_pointer_release(1);
        self.flush();
    }
}

fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|index| format!("paragraph {index} of the document"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn caret_and_selection_moves_do_no_text_layout() {
    let mut fixture = Fixture::new(&paragraphs(20));
    let (layouts, shapes) = (fixture.layouts_created(), fixture.shapes());
    let start = fixture.caret();
    let document = fixture.document;
    for (intent, extend) in [
        (TextCaretIntent::DocStart, false),
        (TextCaretIntent::Down, false),
        (TextCaretIntent::Right, true),
        (TextCaretIntent::WordRight, true),
        (TextCaretIntent::LineEnd, false),
    ] {
        let Fixture {
            runtime, shaper, ..
        } = &mut fixture;
        assert!(
            runtime
                .context_mut()
                .move_focused_text_caret(document, intent, extend, Some(shaper))
                .unwrap(),
            "{intent:?}"
        );
        let work = fixture.flush();
        assert_eq!(work.layouts_created, 0, "{intent:?}: no layout");
        assert_eq!(work.editable_mutations, 0);
        assert_eq!(
            work.caret_only_updates + work.selection_only_updates,
            1,
            "{intent:?} is one caret or selection update"
        );
    }
    assert_ne!(fixture.caret(), start, "the caret moved on screen");
    assert_eq!(fixture.layouts_created(), layouts, "layout_created == 0");
    assert_eq!(fixture.shapes(), shapes, "shape_runs_created == 0");
}

#[test]
fn typing_lays_out_only_the_paragraph_it_edits() {
    let mut fixture = Fixture::new(&paragraphs(30));
    let value = fixture.value();
    let middle = value.find("paragraph 15").unwrap() + "paragraph".len();
    let document = fixture.document;
    assert!(
        fixture
            .runtime
            .context_mut()
            .select_focused_text_range(document, middle, middle)
            .unwrap()
    );
    fixture.flush();
    let layouts = fixture.layouts_created();

    assert!(
        fixture
            .runtime
            .context_mut()
            .replace_focused_text(document, "!")
            .unwrap()
    );
    let work = fixture.flush();
    assert_eq!(work.editable_mutations, 1);
    assert_eq!(work.editable_bytes_inserted, 1);
    assert_eq!(work.paragraphs_relayout_from_edit, 1, "{work:?}");
    assert_eq!(
        fixture.layouts_created() - layouts,
        1,
        "one character does not lay out the other 29 paragraphs"
    );
    assert!(fixture.value().contains("paragraph! 15"));
}

#[test]
fn composition_updates_lay_out_only_the_composing_paragraph() {
    let mut fixture = Fixture::new(&paragraphs(10));
    let document = fixture.document;
    fixture
        .runtime
        .context_mut()
        .select_focused_text_range(document, 5, 5)
        .unwrap();
    fixture.flush();
    for preedit in ["n", "ni", "nih"] {
        let layouts = fixture.layouts_created();
        assert!(
            fixture
                .runtime
                .context_mut()
                .set_ime_preedit(document, preedit.into(), None)
                .unwrap()
        );
        let work = fixture.flush();
        assert_eq!(work.composition_updates, 1);
        assert_eq!(work.editable_mutations, 0, "a preedit is not an edit");
        assert_eq!(fixture.layouts_created() - layouts, 1, "{preedit}");
    }
    assert!(
        fixture
            .runtime
            .context_mut()
            .commit_ime(document, "你")
            .unwrap()
    );
    let work = fixture.flush();
    assert_eq!(work.editable_mutations, 1);
    assert_eq!(work.composition_updates, 1);
    assert!(fixture.value().starts_with("parag你raph 0"));
}

#[test]
fn a_click_resolves_through_the_retained_geometry() {
    let mut fixture = Fixture::new(&paragraphs(5));
    let world = fixture.runtime.context().world();
    let (content, _) = world
        .text_input_pointer_context(fixture.area.stable_id())
        .unwrap();
    let presentation = world
        .text_input_presentation(fixture.area.stable_id())
        .unwrap();
    let line_height = presentation.line_height;
    let layouts = fixture.layouts_created();
    let document = fixture.document;
    let node = fixture.area.stable_id();
    // Third line, a little way in.
    let (x, y) = (content.x + 40.0, content.y + line_height * 2.5);
    let Fixture {
        runtime, shaper, ..
    } = &mut fixture;
    runtime
        .context_mut()
        .text_editor_pointer_press(
            document,
            node,
            1,
            x,
            y,
            false,
            false,
            std::time::Duration::from_secs(10),
            shaper,
        )
        .unwrap();
    runtime.context_mut().text_editor_pointer_release(1);
    let work = fixture.flush();
    assert!(work.hit_test_queries >= 1, "{work:?}");
    let selection = fixture
        .runtime
        .context()
        .world()
        .text_input(node)
        .unwrap()
        .selection;
    let value = fixture.value();
    let third = value.find("paragraph 2").unwrap();
    assert!(
        selection.focus > third && selection.focus < third + "paragraph 2".len(),
        "clicked into the third paragraph, got {}",
        selection.focus
    );
    assert_eq!(
        fixture.layouts_created(),
        layouts,
        "a click lays nothing out"
    );
}

#[test]
fn focus_moving_away_mid_composition_stops_drawing_the_preedit() {
    let document = DocumentId::new(DOCUMENT).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let (first, second) = runtime
        .context_mut()
        .build(document, |ui| {
            (
                ui.child("first", TextArea::new("first")),
                ui.child("second", TextArea::new("second")),
            )
        })
        .unwrap();
    let mut shaper = NanaTextEngineShaper::new(engine());
    let context = runtime.context_mut();
    assert!(context.focus_node(document, first.stable_id()).unwrap());
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(
        runtime
            .context_mut()
            .set_ime_preedit(document, "zhong".into(), None)
            .unwrap()
    );
    runtime.flush(viewport(), &mut shaper).unwrap();
    let presentation = |runtime: &RuntimeDocument| {
        runtime
            .context()
            .world()
            .text_input_presentation(first.stable_id())
            .unwrap()
            .clone()
    };
    assert!(presentation(&runtime).preedit.is_some());
    assert_eq!(presentation(&runtime).display_value, "firstzhong");

    assert!(
        runtime
            .context_mut()
            .focus_node(document, second.stable_id())
            .unwrap()
    );
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(runtime.context().world().ime(first.stable_id()).is_none());
    assert!(
        presentation(&runtime).preedit.is_none(),
        "the cancelled preedit is not drawn any more"
    );
    assert_eq!(presentation(&runtime).display_value, "first");
}

#[test]
fn up_and_clicks_at_the_end_of_a_wrap_without_whitespace_stay_on_that_line() {
    // CJK wraps between any two characters: a line's end is the next line's
    // start, with no hung whitespace in between.
    let mut fixture = Fixture::new(&"中".repeat(120));
    let node = fixture.area.stable_id();
    let document = fixture.document;
    let tops = |fixture: &Fixture| fixture.caret().1;
    let (content, _) = fixture
        .runtime
        .context()
        .world()
        .text_input_pointer_context(node)
        .unwrap();
    let line_height = fixture
        .runtime
        .context()
        .world()
        .text_input_presentation(node)
        .unwrap()
        .line_height;

    // A click past the right end of the first line.
    let Fixture {
        runtime, shaper, ..
    } = &mut fixture;
    runtime
        .context_mut()
        .text_editor_pointer_press(
            document,
            node,
            1,
            content.x + content.width - 1.0,
            content.y + line_height * 0.5,
            false,
            false,
            std::time::Duration::from_secs(10),
            shaper,
        )
        .unwrap();
    runtime.context_mut().text_editor_pointer_release(1);
    fixture.flush();
    assert_eq!(tops(&fixture), 0.0, "the caret stays on the clicked line");

    // Down twice, then up: every move changes line.
    let mut previous = tops(&fixture);
    for intent in [
        TextCaretIntent::Down,
        TextCaretIntent::Down,
        TextCaretIntent::Up,
        TextCaretIntent::Up,
    ] {
        let Fixture {
            runtime, shaper, ..
        } = &mut fixture;
        assert!(
            runtime
                .context_mut()
                .move_focused_text_caret(document, intent, false, Some(shaper))
                .unwrap()
        );
        fixture.flush();
        let top = tops(&fixture);
        match intent {
            TextCaretIntent::Down => assert!(top > previous, "{intent:?}: {previous} -> {top}"),
            _ => assert!(top < previous, "{intent:?}: {previous} -> {top}"),
        }
        previous = top;
    }
    assert_eq!(previous, 0.0);
}

#[test]
fn a_click_past_a_wrapped_cjk_line_end_keeps_the_caret_after_that_line() {
    // One offset, two places: the end of a line that wrapped with no hanging
    // whitespace is also the next line's start. The click decides which, and
    // the affinity it resolves is what the editor stores and draws.
    let mut fixture = Fixture::new(&"中".repeat(120));
    let content = fixture.content();
    let line_height = fixture.line_height();
    let layouts = fixture.layouts_created();

    // The second line's start, hit from its own line.
    fixture.click(content.x + 0.5, content.y + line_height * 1.5);
    let downstream = fixture.selection();
    let (start_x, start_y) = fixture.caret();
    assert_eq!(downstream.affinity, TextAffinity::Downstream);
    assert_eq!(start_y, line_height, "hit on the second line");
    assert!(
        start_x < line_height,
        "at the second line's start: {start_x}"
    );
    assert!(downstream.focus > 0, "past the first line");

    // Past the right end of the first line: the same offset, drawn as that
    // line's end rather than the next line's start.
    fixture.click(
        content.x + content.width - 1.0,
        content.y + line_height * 0.5,
    );
    let upstream = fixture.selection();
    let (end_x, end_y) = fixture.caret();
    assert_eq!(
        upstream.focus, downstream.focus,
        "the line's end is the next line's start"
    );
    assert_eq!(upstream.affinity, TextAffinity::Upstream);
    assert_eq!(end_y, 0.0, "the caret stays on the clicked line");
    assert!(
        end_x > content.width - line_height,
        "after the last character of the first line, not before it: {end_x}"
    );
    assert_eq!(fixture.layouts_created(), layouts, "clicks lay nothing out");
}

/// The Runtime's layout cache is content addressed: keying it copies the whole
/// text into the key and hashes it. An editor is measured by summing the
/// paragraphs it already holds, so no frame of it may touch that cache --
/// otherwise every caret move on a large document pays a hash of the document.
#[test]
fn an_editors_frames_never_key_the_runtime_layout_cache() {
    let mut fixture = Fixture::new(&paragraphs(30));
    let document = fixture.document;
    let cache = |fixture: &Fixture| {
        let counters = fixture.runtime.context().last_work_counters();
        (
            counters.text_layout_cache_hits,
            counters.text_layout_cache_misses,
        )
    };
    // The mount is exempt: the first measurement happens before the node has
    // any geometry to be measured from (the measure pass never creates it --
    // it would be under the layout pass's constraints, not the probes').
    assert_eq!(cache(&fixture).0, 0, "nothing is answered from the cache");

    // A caret move: no text work is owed at all.
    let Fixture {
        runtime, shaper, ..
    } = &mut fixture;
    assert!(
        runtime
            .context_mut()
            .move_focused_text_caret(document, TextCaretIntent::Up, false, Some(shaper))
            .unwrap()
    );
    fixture.flush();
    assert_eq!(cache(&fixture), (0, 0), "a caret move");

    // An edit: the paragraph it changed is laid out again, still without the
    // Runtime cache in front of the editor.
    assert!(
        fixture
            .runtime
            .context_mut()
            .replace_focused_text(document, "!")
            .unwrap()
    );
    let work = fixture.flush();
    assert_eq!(work.editable_mutations, 1);
    assert_eq!(cache(&fixture), (0, 0), "an edit");
}
