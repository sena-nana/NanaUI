//! Rich spans on the product path: an application-owned `RichText` on a
//! plain text node, priced by the tier each change touches, and carried into
//! the scene's `Text` primitive.
//!
//! Every test runs `RuntimeDocument::flush` against an engine holding only
//! the bundled UI face.

use std::sync::{Arc, Mutex};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{NativeTextEngine, SharedTextEngine, TextWorkCounters};
use nana_ui_core::{
    FlexDirection, LayoutStyle, LengthSpec, PaintColor, RichSpanStyle, RichText, RichTextShadow,
    RichTextStroke,
};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeKind, NodeStyle,
    StableNodeId, TextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

const ROOT: u64 = 1;
const COLUMN: u64 = 2;
const LABEL: u64 = 3;

const RED: PaintColor = PaintColor::srgb([1.0, 0.0, 0.0, 1.0]);
const BLUE: PaintColor = PaintColor::srgb([0.0, 0.0, 1.0, 1.0]);
const BLACK: PaintColor = PaintColor::srgb([0.0, 0.0, 0.0, 1.0]);

fn id(raw: u64) -> StableNodeId {
    StableNodeId::new(raw).unwrap()
}

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

fn document(label_layout: LayoutStyle, rich: RichText) -> RuntimeDocument {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let mut queue = MutationQueue::new();
    queue.create(id(ROOT), document, NodeKind::Document);
    queue.create(
        id(COLUMN),
        document,
        NodeKind::Element { tag: "div".into() },
    );
    queue.insert(id(ROOT), id(COLUMN), None);
    queue.set_style(
        id(COLUMN),
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(400.0)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    queue.create(id(LABEL), document, NodeKind::Text);
    queue.insert(id(COLUMN), id(LABEL), None);
    queue.set_style(
        id(LABEL),
        NodeStyle {
            layout: Arc::new(label_layout),
            ..NodeStyle::default()
        },
    );
    queue.set_rich_text(id(LABEL), rich);
    runtime.context_mut().commit_mutations(queue).unwrap();
    runtime
}

fn label_layout() -> LayoutStyle {
    LayoutStyle {
        font_size: Some(16.0),
        ..LayoutStyle::default()
    }
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(400.0, 600.0)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut impl TextShaper) {
    for _ in 0..5 {
        runtime.flush(viewport(), shaper).unwrap();
    }
}

fn set_rich(runtime: &mut RuntimeDocument, rich: RichText) {
    let mut queue = MutationQueue::new();
    queue.set_rich_text(id(LABEL), rich);
    runtime.context_mut().commit_mutations(queue).unwrap();
}

fn text_work(runtime: &RuntimeDocument) -> TextWorkCounters {
    runtime.context().world().last_text_work_counters()
}

/// The label's scene text primitive, as `(content, spans, rich, layout id)`.
fn label_text(
    runtime: &RuntimeDocument,
) -> (
    String,
    Vec<nana_ui_scene::SceneTextSpan>,
    Option<Arc<nana_ui_scene::SceneRichPaint>>,
    Option<nana_text::TextLayoutId>,
) {
    runtime
        .scene()
        .primitives()
        .find_map(|primitive| match &primitive.kind {
            ScenePrimitiveKind::Text {
                content,
                spans,
                rich,
                layout,
                ..
            } if primitive.node == id(LABEL) => Some((
                content.as_str().to_owned(),
                spans.clone(),
                rich.clone(),
                layout.as_ref().map(|layout| layout.id),
            )),
            _ => None,
        })
        .expect("the label paints as text")
}

fn dialogue(color: PaintColor) -> RichText {
    RichText::builder()
        .plain("Hello ")
        .push("world", RichSpanStyle::new().color(color).underline())
        .plain(", again")
        .build()
}

#[test]
fn a_paint_only_span_change_shapes_and_lays_out_nothing() {
    let mut runtime = document(label_layout(), dialogue(RED));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (handle, _) = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .expect("the label retains a layout");
    let before = text_work(&runtime);

    set_rich(&mut runtime, dialogue(BLUE));
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(!update.is_idle(), "a repaint is a scene change");
    let work = text_work(&runtime);
    if work != before {
        assert_eq!(work.text_nodes_shaped, 0, "a recolour reshapes nothing");
        assert_eq!(work.layouts_created, 0, "a recolour lays nothing out");
    }
    assert_eq!(
        runtime.context().world().text_layout(id(LABEL)).unwrap().0,
        handle,
        "the retained layout survives a paint change"
    );
    let (_, spans, rich, layout) = label_text(&runtime);
    assert_eq!(
        layout,
        Some(handle),
        "the scene still draws the retained layout"
    );
    let world = spans
        .iter()
        .find(|span| span.start == "Hello ".len())
        .expect("the span's fill reaches the scene");
    assert_eq!(world.paint_color, Some(BLUE));
    let rich = rich.expect("an underline is an effect");
    assert!(rich.effects_at("Hello w".len()).decoration.underline);
    assert!(!rich.effects_at(0).decoration.underline);
}

#[test]
fn an_effect_index_alone_costs_no_runtime_work() {
    let mut runtime = document(label_layout(), dialogue(RED));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let mut effected = dialogue(RED);
    effected.apply_span(0..5, &RichSpanStyle::new().effect(3));
    set_rich(&mut runtime, effected.clone());
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(
        update.is_idle(),
        "nothing the Runtime draws read the effect"
    );
    assert_eq!(
        runtime.context().world().rich_text(id(LABEL)),
        Some(&effected),
        "the value is still the application's"
    );
}

#[test]
fn a_larger_span_grows_the_line_box_and_its_run() {
    let plain = RichText::new("Hello world");
    let mut runtime = document(label_layout(), plain);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let small = runtime.context().world().text_metrics(id(LABEL)).unwrap();

    let big = RichText::new("Hello world").with_span(6..11, RichSpanStyle::new().size(32.0));
    set_rich(&mut runtime, big);
    settle(&mut runtime, &mut shaper);
    assert!(
        text_work(&runtime).text_nodes_shaped > 0,
        "a size span reshapes"
    );
    let grown = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        grown.height > small.height * 1.5,
        "the line holds the larger word: {} -> {}",
        small.height,
        grown.height
    );
    assert!(grown.width > small.width);
    let (_, layout) = runtime.context().world().text_layout(id(LABEL)).unwrap();
    assert!(
        layout.runs.iter().any(|run| run.font_size_px == 32.0),
        "the span's run is shaped at its size"
    );
    let laid_out = runtime
        .context()
        .world()
        .layout_box(id(LABEL))
        .expect("the label is laid out");
    assert!(
        laid_out.height >= grown.height - 0.5,
        "layout read the grown metrics"
    );
}

#[test]
fn stroke_and_shadows_reach_the_scene_and_plain_decorations_draw_no_box_strokes() {
    let styled = RichText::new("outlined").with_span(
        0..8,
        RichSpanStyle::new()
            .stroke(RichTextStroke::new(3.0, BLACK))
            .shadow(RichTextShadow::new([2.0, 2.0], 4.0, BLACK))
            .shadow(RichTextShadow::new([0.0, 0.0], 8.0, RED).spread(1.0))
            .line_through(),
    );
    let mut runtime = document(label_layout(), styled);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (content, _, rich, _) = label_text(&runtime);
    assert_eq!(content, "outlined");
    let rich = rich.expect("effects reach the scene");
    let effects = rich.effects_at(0);
    assert_eq!(effects.stroke.map(|stroke| stroke.width_px), Some(3.0));
    assert_eq!(effects.shadows.len(), 2);
    assert_eq!(effects.shadows[1].spread_px, 1.0);
    assert!(effects.decoration.line_through);
    assert!(
        rich.reach >= 8.0 + 1.0,
        "a blurred, spread layer reaches past the ink"
    );
    let strokes = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id(LABEL))
        .filter(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Stroke { .. }))
        .count();
    assert_eq!(
        strokes, 0,
        "decorations are the text painter's, not box strokes"
    );
}

#[test]
fn css_text_effects_are_the_base_a_span_overrides() {
    let mut layout = label_layout();
    layout.text_decoration = Some(nana_ui_core::TextDecorationLine {
        underline: true,
        line_through: false,
    });
    layout.paint.text_shadows = vec![
        nana_ui_core::TextShadowSpec {
            offset_x: 1.0,
            offset_y: 1.0,
            blur_radius: 2.0,
            color: [0.0, 0.0, 0.0, 0.5],
            paint_color: None,
        };
        2
    ];
    layout.paint.text_stroke = Some(nana_ui_core::TextStrokeSpec {
        width: 2.0,
        color: Some([1.0, 1.0, 1.0, 1.0]),
        paint_color: None,
    });
    layout.paint.paint_order_stroke_first = true;
    let rich = RichText::new("base over").with_span(
        5..9,
        RichSpanStyle {
            paint: nana_ui_core::RichPaintStyle {
                decoration: Some(nana_ui_core::TextDecorationLine::default()),
                ..Default::default()
            },
            ..RichSpanStyle::default()
        },
    );
    let mut runtime = document(layout, rich);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (_, _, rich, _) = label_text(&runtime);
    let rich = rich.expect("CSS effects reach the scene");
    let base = rich.effects_at(0);
    assert!(base.decoration.underline);
    assert_eq!(base.shadows.len(), 2);
    let stroke = base.stroke.expect("the CSS stroke");
    assert_eq!(stroke.placement, nana_ui_core::TextStrokePlacement::Under);
    let over = rich.effects_at(6);
    assert!(
        !over.decoration.underline,
        "the span clears the node's underline"
    );
    assert_eq!(over.shadows.len(), 2, "and keeps what it did not set");
}
