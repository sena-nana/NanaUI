//! A focused text field through the product frame loop: with nothing in it —
//! no value, no placeholder — it still has a line for its caret and draws an
//! IME preedit, and it says it is focused with its fill as well as its border.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine};
use nana_ui_core::SemanticColorRole;
use nana_ui_runtime::view::widget;
use nana_ui_runtime::{
    ComponentGeometry, DocumentId, ImeComposition, LayoutViewport, MutationQueue,
    NanaTextEngineShaper, SemanticPaint, StableNodeId, TextInput,
};
use nana_ui_scene::RuntimeDocument;

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
    LayoutViewport::new(400.0, 200.0)
}

/// `field` mounted, laid out and focused, then flushed again; with the
/// shaper later frames run through.
fn focused(field: TextInput) -> (RuntimeDocument, StableNodeId, NanaTextEngineShaper) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let input = runtime
        .context_mut()
        .mount_view_root(document, || widget(field))
        .unwrap()
        .root::<TextInput>()
        .unwrap()
        .stable_id();
    let mut shaper = NanaTextEngineShaper::new(engine());
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(runtime.context_mut().focus_node(document, input).unwrap());
    runtime.flush(viewport(), &mut shaper).unwrap();
    (runtime, input, shaper)
}

/// The focused field paints [`SemanticPaint::FOCUS_FIELD`]: the focus
/// border over its fill deepened toward the text.
#[test]
fn a_focused_field_deepens_its_fill_and_takes_the_focus_border() {
    let (runtime, id, _) = focused(TextInput::new(""));
    let world = runtime.context().world();
    let Some(ComponentGeometry::TextInput {
        background, border, ..
    }) = world.component_geometry(id)
    else {
        panic!("the focused field has no geometry");
    };
    let fill = SemanticPaint::FOCUS_FIELD
        .background_mix
        .expect("a deepened fill")
        .resolve(world.style_model())
        .as_rgba_array();
    let model = world.style_model();
    assert_ne!(
        fill,
        model.color(SemanticColorRole::Background).as_rgba_array()
    );
    assert_eq!(background, Some(fill));
    assert_eq!(
        border,
        Some(model.color(SemanticColorRole::FocusBorder).as_rgba_array())
    );
}

#[test]
fn a_focused_field_shows_its_caret_with_or_without_text() {
    for (name, field) in [
        ("empty", TextInput::new("")),
        ("placeholder", TextInput::new("").placeholder("Name")),
        ("value", TextInput::new("Nana")),
    ] {
        let (runtime, id, _) = focused(field);
        let geometry = runtime.context().world().component_geometry(id);
        let Some(ComponentGeometry::TextInput {
            caret: Some(caret), ..
        }) = geometry
        else {
            panic!("{name}: no caret in {geometry:?}");
        };
        assert!(caret.height > 0.0, "{name}: {caret:?}");
    }
}

/// Composing into an empty field: the field has no value yet, and its text
/// work is still done, so the preedit is drawn in the frame that follows.
#[test]
fn an_empty_field_draws_the_ime_preedit() {
    let (mut runtime, id, mut shaper) = focused(TextInput::new(""));
    let mut ime = MutationQueue::new();
    ime.set_ime(
        id,
        Some(ImeComposition {
            text: "nana".into(),
            selection: None,
        }),
    );
    runtime.context_mut().commit_mutations(ime).unwrap();
    runtime.flush(viewport(), &mut shaper).unwrap();
    let geometry = runtime.context().world().component_geometry(id);
    let Some(ComponentGeometry::TextInput { preedit, .. }) = &geometry else {
        panic!("no geometry: {geometry:?}");
    };
    assert!(!preedit.is_empty(), "the preedit is not drawn");
}
