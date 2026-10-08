//! Per-glyph presentation on the product path: it costs no text work, asks
//! for frames only while something still moves, and moves inline objects the
//! way the shader moves the glyphs around them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine};
use nana_ui_core::{
    GlyphEffect, GlyphIntro, LayoutStyle, LengthSpec, RevealSchedule, RichObject, RichSpanStyle,
    RichText,
};
use nana_ui_runtime::{
    DocumentId, Entity, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeKind, NodeStyle,
    RichTextView, StableNodeId, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

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
    LayoutViewport::new(400.0, 300.0)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut impl TextShaper) {
    for _ in 0..4 {
        runtime.flush(viewport(), shaper).unwrap();
    }
}

fn document(value: RichText) -> (RuntimeDocument, Entity<RichTextView>) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    let root = StableNodeId::new(1).unwrap();
    queue.create(root, document, NodeKind::Document);
    let column = StableNodeId::new(2).unwrap();
    queue.create(column, document, NodeKind::Element { tag: "div".into() });
    queue.insert(root, column, None);
    queue.set_style(
        column,
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    runtime.context_mut().commit_mutations(queue).unwrap();
    let view = runtime
        .context_mut()
        .create_component(document, RichTextView::new(value).font_size(20.0))
        .unwrap();
    let mut queue = MutationQueue::new();
    queue.insert(column, view.stable_id(), None);
    runtime.context_mut().commit_mutations(queue).unwrap();
    (runtime, view)
}

fn at(runtime: &mut RuntimeDocument, shaper: &mut impl TextShaper, now: Duration) {
    runtime.sync_presentation_clock(now);
    runtime.flush(viewport(), shaper).unwrap();
}

#[test]
fn a_reveal_costs_no_text_work_and_asks_for_frames_only_until_it_ends() {
    let (mut runtime, view) = document(RichText::new("你好呀"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    assert!(!runtime.compositor_needs_tick());
    let before = runtime.context().world().last_text_work_counters();
    let reveal =
        RevealSchedule::uniform(Duration::from_secs(2), 3, 0.1).intro(GlyphIntro::fade(0.1));
    runtime
        .context_mut()
        .set_rich_presentation(view, Vec::new(), Some(reveal))
        .unwrap();
    at(&mut runtime, &mut shaper, Duration::from_secs(2));
    let work = runtime.context().world().last_text_work_counters();
    if work != before {
        assert_eq!(work.text_nodes_shaped, 0, "a reveal shapes nothing");
        assert_eq!(work.layouts_created, 0, "nor lays anything out");
    }
    let presentation = runtime
        .scene()
        .primitives()
        .find_map(|primitive| match &primitive.kind {
            ScenePrimitiveKind::Text { presentation, .. } if primitive.node == view.stable_id() => {
                presentation.clone()
            }
            _ => None,
        })
        .expect("the text primitive carries the presentation");
    assert!(presentation.reveal.is_some());
    assert!(
        runtime.compositor_needs_tick(),
        "a reveal in progress keeps frames coming"
    );
    at(&mut runtime, &mut shaper, Duration::from_millis(2350));
    assert!(
        !runtime.compositor_needs_tick(),
        "a finished reveal stops them"
    );
}

#[test]
fn a_looping_effect_keeps_presenting_and_clearing_it_stops() {
    let value = RichText::new("wavy text").with_span(0..4, RichSpanStyle::new().effect(0));
    let (mut runtime, view) = document(value);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    runtime
        .context_mut()
        .set_rich_presentation(view, vec![GlyphEffect::wave(3.0)], None)
        .unwrap();
    at(&mut runtime, &mut shaper, Duration::from_secs(100));
    assert!(runtime.compositor_needs_tick());
    let effects = runtime
        .scene()
        .primitives()
        .find_map(|primitive| match &primitive.kind {
            ScenePrimitiveKind::Text { presentation, .. } if primitive.node == view.stable_id() => {
                presentation.clone()
            }
            _ => None,
        })
        .expect("presented");
    assert_eq!(effects.cluster_effects.len(), "wavy text".len());
    assert_eq!(effects.cluster_effects[0], 0);
    assert_eq!(
        effects.cluster_effects[5],
        u16::MAX,
        "only the span plays it"
    );
    runtime.context_mut().clear_rich_presentation(view).unwrap();
    at(&mut runtime, &mut shaper, Duration::from_secs(101));
    assert!(!runtime.compositor_needs_tick());
}

#[test]
fn an_inline_sticker_reveals_with_its_place_in_the_text() {
    let value = RichText::builder()
        .plain("ab")
        .object(RichObject::image(3, "file:///s/a.png", 20.0, 20.0))
        .build();
    let (mut runtime, view) = document(value);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let reveal = RevealSchedule::uniform(Duration::from_secs(5), 3, 0.5);
    runtime
        .context_mut()
        .set_rich_presentation(view, Vec::new(), Some(reveal))
        .unwrap();
    let sticker_opacity = |runtime: &RuntimeDocument| {
        runtime
            .scene()
            .primitives()
            .find(|primitive| {
                primitive.node == view.stable_id()
                    && matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
            })
            .map(|primitive| primitive.opacity)
            .expect("the sticker paints")
    };
    at(&mut runtime, &mut shaper, Duration::from_millis(5600));
    assert_eq!(
        sticker_opacity(&runtime),
        0.0,
        "its turn is the third grapheme, at 1.0s"
    );
    at(&mut runtime, &mut shaper, Duration::from_millis(6100));
    assert_eq!(sticker_opacity(&runtime), 1.0);
}
