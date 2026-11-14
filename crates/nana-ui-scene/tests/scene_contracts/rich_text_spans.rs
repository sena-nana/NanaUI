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
    FlexDirection, LayoutStyle, LengthSpec, PaintColor, RichObject, RichObjectFlow, RichSpanStyle,
    RichText, RichTextShadow, RichTextStroke,
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
    let column = LayoutStyle {
        width: Some(LengthSpec::Px(400.0)),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    };
    document_in(column, label_layout, rich)
}

/// The label alone in a container laid out as `container`.
fn document_in(
    container: LayoutStyle,
    label_layout: LayoutStyle,
    rich: RichText,
) -> RuntimeDocument {
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
            layout: Arc::new(container),
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
    assert_eq!(world.color, BLUE);
    let rich = rich.expect("an underline is an effect");
    assert!(rich.effects_at("Hello w".len()).decoration.underline);
    assert!(!rich.effects_at(0).decoration.underline);
}

#[test]
fn an_effect_index_alone_costs_no_runtime_work() {
    let mut runtime = document(label_layout(), dialogue(RED));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (handle, _) = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .expect("laid out");
    let before = text_work(&runtime);
    let mut effected = dialogue(RED);
    effected.apply_span(0..5, &RichSpanStyle::new().effect(3));
    set_rich(&mut runtime, effected.clone());
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    if work != before {
        assert_eq!(work.text_nodes_shaped, 0, "an effect index shapes nothing");
        assert_eq!(work.layouts_created, 0, "nor lays anything out");
    }
    assert_eq!(
        runtime.context().world().text_layout(id(LABEL)).unwrap().0,
        handle,
        "the layout stands"
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

fn sticker_line(width: f32, height: f32) -> RichText {
    RichText::builder()
        .plain("look ")
        .object(nana_ui_core::RichObject::image(
            9,
            "file:///stickers/cat.png",
            width,
            height,
        ))
        .plain(" here")
        .build()
}

fn object_quads(runtime: &RuntimeDocument) -> Vec<nana_ui_scene::SceneRect> {
    runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id(LABEL))
        .filter(|primitive| {
            matches!(
                &primitive.kind,
                ScenePrimitiveKind::Quad { surface, .. } if surface.content_image.is_some()
            )
        })
        .map(|primitive| primitive.bounds)
        .collect()
}

#[test]
fn an_inline_sticker_takes_room_in_the_line_and_paints_as_an_image() {
    let mut runtime = document(label_layout(), RichText::new("look  here"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let plain = runtime.context().world().text_metrics(id(LABEL)).unwrap();

    set_rich(&mut runtime, sticker_line(32.0, 32.0));
    settle(&mut runtime, &mut shaper);
    let with = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        with.width > plain.width + 30.0,
        "{} -> {}",
        plain.width,
        with.width
    );
    assert!(
        with.height >= 32.0,
        "the line holds the sticker: {}",
        with.height
    );
    let quads = object_quads(&runtime);
    assert_eq!(quads.len(), 1, "one image primitive for the sticker");
    let quad = quads[0];
    assert!((quad.width - 32.0).abs() < 0.01 && (quad.height - 32.0).abs() < 0.01);
    let label = runtime
        .context()
        .world()
        .layout_box(id(LABEL))
        .expect("laid out");
    assert!(
        quad.x > label.x && quad.y >= label.y - 0.01,
        "{quad:?} in {label:?}"
    );
}

#[test]
fn resizing_a_sticker_relays_out_without_reshaping() {
    let mut runtime = document(label_layout(), sticker_line(24.0, 24.0));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let small = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    set_rich(&mut runtime, sticker_line(48.0, 24.0));
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    assert!(
        work.layouts_created > 0,
        "a new box is a new layout: {work:?}"
    );
    assert_eq!(
        work.shape_cache_misses,
        Some(0),
        "but the runs are the ones already shaped: {work:?}"
    );
    settle(&mut runtime, &mut shaper);
    let large = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!((large.width - small.width - 24.0).abs() < 0.5);
    assert!((object_quads(&runtime)[0].width - 48.0).abs() < 0.01);
}

/// A 600 × 300 picture between two words, placed as `placed` says.
fn picture_line(placed: impl Fn(RichObject) -> RichObject) -> RichText {
    RichText::builder()
        .plain("look")
        .object(placed(RichObject::image(
            9,
            "file:///photos/beach.png",
            600.0,
            300.0,
        )))
        .plain("here")
        .build()
}

#[test]
fn a_fitted_block_picture_takes_its_own_line_at_the_column_width() {
    let rich = picture_line(|picture| picture.flow(RichObjectFlow::Block).fit_width(true));
    let mut runtime = document(label_layout(), rich);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let label = runtime
        .context()
        .world()
        .layout_box(id(LABEL))
        .expect("laid out");
    let quads = object_quads(&runtime);
    assert_eq!(quads.len(), 1);
    let quad = quads[0];
    assert!(
        (quad.width - label.width).abs() < 0.5 && label.width <= 400.0 + 0.01,
        "scaled to the column: {quad:?} in {label:?}"
    );
    assert!(
        (quad.height - quad.width * 0.5).abs() < 0.5,
        "keeping its aspect: {quad:?}"
    );
    assert!(
        (quad.x - label.x).abs() < 0.5 && quad.y > label.y + 10.0,
        "at the start of a line below the first word: {quad:?} in {label:?}"
    );
    let (_, layout) = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .expect("the label keeps its layout");
    assert_eq!(layout.lines.len(), 3, "{:?}", layout.lines);
    assert!(
        label.height >= quad.height + 2.0 * 16.0,
        "two text lines and the picture: {label:?}"
    );
    assert_eq!(
        runtime
            .context()
            .world()
            .rich_text(id(LABEL))
            .unwrap()
            .objects()[0]
            .1
            .width_px,
        600.0,
        "the application's value keeps the authored size"
    );
}

#[test]
fn a_fitted_picture_lets_a_flex_item_shrink_below_it() {
    let row = LayoutStyle {
        width: Some(LengthSpec::Px(200.0)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    };
    let label_width = |fit: bool| {
        let rich = picture_line(|picture| picture.flow(RichObjectFlow::Block).fit_width(fit));
        let mut runtime = document_in(row.clone(), label_layout(), rich);
        let mut shaper = NanaTextEngineShaper::new(engine());
        settle(&mut runtime, &mut shaper);
        let label = runtime
            .context()
            .world()
            .layout_box(id(LABEL))
            .expect("laid out");
        (label.width, object_quads(&runtime)[0])
    };
    let (fixed, quad) = label_width(false);
    assert!(
        fixed >= 600.0 - 0.5 && quad.width >= 600.0 - 0.5,
        "a picture that cannot shrink holds its item at its width: {fixed} {quad:?}"
    );
    let (fitted, quad) = label_width(true);
    assert!(
        fitted <= 200.0 + 0.5,
        "its min-content is the words', so the item shrinks to the row: {fitted}"
    );
    assert!(
        (quad.width - fitted).abs() < 0.5 && (quad.height - quad.width * 0.5).abs() < 0.5,
        "and the picture with it: {quad:?}"
    );
}

#[test]
fn an_editor_chip_takes_no_room_and_a_display_does_not_draw_it() {
    let mut runtime = document(label_layout(), RichText::new("abcd"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let plain = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    let chipped = RichText::builder()
        .plain("ab")
        .object(nana_ui_core::RichObject::chip(4, "wait", 1))
        .plain("cd")
        .build();
    set_rich(&mut runtime, chipped);
    settle(&mut runtime, &mut shaper);
    let with = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        (with.width - plain.width).abs() < 0.01,
        "{} vs {}",
        plain.width,
        with.width
    );
    let drawn = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id(LABEL))
        .filter(|primitive| !matches!(primitive.kind, ScenePrimitiveKind::Text { .. }))
        .count();
    assert_eq!(drawn, 0, "a display shows no marker");
}

#[test]
fn a_texture_object_is_drawn_by_the_host_texture_renderer() {
    let animated = RichText::builder()
        .plain("gif ")
        .object(nana_ui_core::RichObject::texture(
            5,
            "sticker:dance",
            28.0,
            28.0,
        ))
        .build();
    let mut runtime = document(label_layout(), animated);
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let custom: Vec<_> = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id(LABEL))
        .filter_map(|primitive| match &primitive.kind {
            ScenePrimitiveKind::Custom { node, .. } => Some((node.clone(), primitive.bounds)),
            _ => None,
        })
        .collect();
    assert_eq!(custom.len(), 1);
    assert_eq!(
        custom[0].0.renderer.as_ref(),
        nana_ui_runtime::HOST_TEXTURE_RENDERER
    );
    assert_eq!(custom[0].0.resource.as_ref(), "sticker:dance");
    assert!((custom[0].1.width - 28.0).abs() < 0.01);
}

fn mention_line(name: &str, fill: [f32; 4]) -> RichText {
    RichText::builder()
        .plain("hi ")
        .object(nana_ui_core::RichObject::label(
            6,
            name,
            nana_ui_core::RichLabelStyle {
                fill,
                text: [0.1, 0.1, 0.1, 1.0],
                scale: 0.9,
                icon: Some(Arc::from("file:///avatars/42.png")),
                ..nana_ui_core::RichLabelStyle::default()
            },
        ))
        .plain(" ok")
        .build()
}

#[test]
fn a_styled_label_takes_room_and_paints_its_pill_and_picture() {
    let mut runtime = document(label_layout(), RichText::new("hi  ok"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let plain = runtime.context().world().text_metrics(id(LABEL)).unwrap();

    set_rich(&mut runtime, mention_line("奈奈", [0.9, 0.4, 0.6, 1.0]));
    settle(&mut runtime, &mut shaper);
    let with = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        with.width > plain.width + 20.0,
        "{} -> {}",
        plain.width,
        with.width
    );

    let pictures = object_quads(&runtime);
    assert_eq!(pictures.len(), 1, "the avatar is an image quad");
    let picture = pictures[0];
    assert!(
        (picture.width - picture.height).abs() < 0.01,
        "square: {picture:?}"
    );
    let pills: Vec<_> = runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id(LABEL))
        .filter(|primitive| {
            matches!(
                &primitive.kind,
                ScenePrimitiveKind::Quad { surface, .. } if surface.content_image.is_none()
            )
        })
        .map(|primitive| primitive.bounds)
        .collect();
    assert_eq!(pills.len(), 1, "one pill");
    let pill = pills[0];
    assert!(picture.x > pill.x && picture.x + picture.width < pill.x + pill.width);
    assert!(
        picture.y >= pill.y - 0.01 && picture.y + picture.height <= pill.y + pill.height + 0.01
    );

    // A longer name is a new label: the line is laid out again.
    set_rich(&mut runtime, mention_line("奈奈奈奈", [0.9, 0.4, 0.6, 1.0]));
    settle(&mut runtime, &mut shaper);
    let longer = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        longer.width > with.width + 10.0,
        "{} -> {}",
        with.width,
        longer.width
    );
}

const YELLOW: PaintColor = PaintColor::srgb([1.0, 0.9, 0.2, 1.0]);
const MARKED: &str = "a highlighted run long enough to cross onto the next line of this label";
const BEFORE_MARKED: &str = "Plain words before ";

fn marked(background: PaintColor) -> RichText {
    RichText::builder()
        .plain(BEFORE_MARKED)
        .push(MARKED, RichSpanStyle::new().background_color(background))
        .plain(" and after.")
        .build()
}

fn marked_range() -> std::ops::Range<usize> {
    BEFORE_MARKED.len()..BEFORE_MARKED.len() + MARKED.len()
}

/// The label's quad batches in paint order, with their scene order index.
fn label_fills(
    runtime: &RuntimeDocument,
) -> Vec<(usize, Vec<nana_ui_scene::SceneRect>, Option<PaintColor>)> {
    runtime
        .scene()
        .primitives()
        .enumerate()
        .filter(|(_, primitive)| primitive.node == id(LABEL))
        .filter_map(|(at, primitive)| match &primitive.kind {
            ScenePrimitiveKind::QuadBatch {
                bounds, background, ..
            } => Some((at, bounds.clone(), *background)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_span_background_fills_its_run_on_every_line_it_crosses() {
    let mut runtime = document(label_layout(), marked(YELLOW));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (_, layout) = runtime.context().world().text_layout(id(LABEL)).unwrap();
    let expected = layout.selection_rects(marked_range());
    assert!(expected.len() >= 2, "the span wraps: {expected:?}");
    let fills = label_fills(&runtime);
    assert_eq!(fills.len(), 1, "one fill for the one span: {fills:?}");
    let (_, rects, background) = &fills[0];
    assert_eq!(
        *background,
        Some(YELLOW),
        "the authored colour reaches the painter"
    );
    assert_eq!(rects.len(), expected.len());
    let origin = (rects[0].x - expected[0].x, rects[0].y - expected[0].y);
    for (rect, line) in rects.iter().zip(&expected) {
        assert_eq!(
            (
                rect.x - origin.0,
                rect.y - origin.1,
                rect.width,
                rect.height
            ),
            (line.x, line.y, line.width, line.height),
            "each line's fill is that line's run of the span"
        );
    }
    assert!(rects[1].y > rects[0].y);
    assert!(
        label_text(&runtime).2.is_none(),
        "a background alone adds no glyph effect"
    );
}

#[test]
fn a_span_background_paints_under_the_selection_and_the_glyphs() {
    let selectable = LayoutStyle {
        user_select: Some(nana_ui_core::UserSelectSpec::Text),
        ..label_layout()
    };
    let mut runtime = document(selectable, marked(YELLOW));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let lines = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .unwrap()
        .1
        .selection_rects(0..BEFORE_MARKED.len() + 4)
        .into_iter()
        .map(|rect| nana_ui_runtime::LayoutBox {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        })
        .collect();
    runtime
        .context_mut()
        .compat_world_mut()
        .set_document_text_selection(
            DocumentId::new(1).unwrap(),
            Some(nana_ui_runtime::DocumentTextSelection {
                node: id(LABEL),
                start: 0,
                end: BEFORE_MARKED.len() + 4,
                lines,
            }),
        );
    settle(&mut runtime, &mut shaper);
    let fills = label_fills(&runtime);
    let background = fills
        .iter()
        .find(|(_, _, color)| *color == Some(YELLOW))
        .expect("the span background")
        .0;
    let selection = fills
        .iter()
        .find(|(_, _, color)| *color != Some(YELLOW))
        .expect("the document selection")
        .0;
    let glyphs = runtime
        .scene()
        .primitives()
        .position(|primitive| {
            primitive.node == id(LABEL) && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
        })
        .unwrap();
    assert!(
        background < selection && selection < glyphs,
        "background {background} < selection {selection} < glyphs {glyphs}"
    );
}

#[test]
fn recolouring_a_span_background_shapes_and_lays_out_nothing() {
    let mut runtime = document(label_layout(), marked(YELLOW));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (handle, _) = runtime.context().world().text_layout(id(LABEL)).unwrap();
    let before = text_work(&runtime);

    set_rich(&mut runtime, marked(BLUE));
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
        "the retained layout survives a background change"
    );
    let fills = label_fills(&runtime);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].2, Some(BLUE));

    // Clearing it removes the fill, still without shaping.
    set_rich(&mut runtime, RichText::new(marked(BLUE).text()));
    runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(label_fills(&runtime).is_empty());
    assert_eq!(
        runtime.context().world().text_layout(id(LABEL)).unwrap().0,
        handle
    );
}

#[test]
fn a_stretched_span_shapes_at_the_variable_faces_width() {
    let engine = || {
        let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
        fonts
            .register_bytes(
                font_blob(include_bytes!("../../../nana-text/fonts/nana-test-vf.ttf")),
                &FaceDescriptor::default(),
            )
            .expect("the width fixture registers");
        Arc::new(Mutex::new(NativeTextEngine::new(fonts))) as SharedTextEngine
    };
    let laid_out = |style: RichSpanStyle| {
        let mut runtime = document(
            label_layout(),
            RichText::new("AAAA").with_span(0..4, style.family("NanaTestVF")),
        );
        let mut shaper = NanaTextEngineShaper::new(engine());
        settle(&mut runtime, &mut shaper);
        let layout = runtime.context().world().text_layout(id(LABEL)).unwrap().1;
        let run = &layout.runs[0];
        (
            layout.lines[0].metrics.width_px,
            run.instance.as_ref().and_then(|instance| {
                instance
                    .coords
                    .iter()
                    .find(|coord| coord.tag == *b"wdth")
                    .map(|coord| coord.value)
            }),
        )
    };
    let (normal, normal_wdth) = laid_out(RichSpanStyle::new());
    let (condensed, condensed_wdth) = laid_out(RichSpanStyle::new().stretch(75.0));
    assert_eq!(normal_wdth, None, "normal width is the axis default");
    assert_eq!(condensed_wdth, Some(75.0));
    assert!(condensed < normal, "{condensed} < {normal}");
}

fn dot() -> nana_ui_core::TextEmphasisStyle {
    nana_ui_core::TextEmphasisStyle::Mark {
        shape: Some(nana_ui_core::TextEmphasisShape::Dot),
        open: false,
    }
}

fn emphasised(color: PaintColor) -> RichText {
    RichText::builder()
        .plain("Hello ")
        .push(
            "world",
            RichSpanStyle::new().emphasis(dot()).emphasis_color(color),
        )
        .build()
}

#[test]
fn an_emphasis_colour_change_shapes_and_lays_out_nothing() {
    let mut runtime = document(label_layout(), emphasised(RED));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let (handle, layout) = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .expect("the label retains a layout");
    assert_eq!(
        layout
            .emphasis
            .iter()
            .map(|mark| mark.offset)
            .collect::<Vec<_>>(),
        (6..11).collect::<Vec<_>>(),
        "one mark per letter of the span"
    );
    let before = text_work(&runtime);

    set_rich(&mut runtime, emphasised(BLUE));
    let update = runtime.flush(viewport(), &mut shaper).unwrap();
    assert!(!update.is_idle(), "a repaint is a scene change");
    let work = text_work(&runtime);
    if work != before {
        assert_eq!(work.text_nodes_shaped, 0, "a mark colour reshapes nothing");
        assert_eq!(work.layouts_created, 0, "and lays nothing out");
    }
    assert_eq!(
        runtime.context().world().text_layout(id(LABEL)).unwrap().0,
        handle,
        "the retained layout survives"
    );
    let (_, _, rich, layout) = label_text(&runtime);
    assert_eq!(layout, Some(handle));
    let rich = rich.expect("a mark colour is an effect");
    assert_eq!(rich.effects_at("Hello w".len()).emphasis_color, Some(BLUE));
    assert_eq!(rich.effects_at(0).emphasis_color, None);
}

#[test]
fn css_emphasis_grows_the_line_and_a_new_position_relays_out_without_reshaping() {
    let mut runtime = document(label_layout(), RichText::new("Hello world"));
    let mut shaper = NanaTextEngineShaper::new(engine());
    settle(&mut runtime, &mut shaper);
    let plain = runtime.context().world().text_metrics(id(LABEL)).unwrap();

    let restyle = |runtime: &mut RuntimeDocument, position| {
        let mut queue = MutationQueue::new();
        queue.set_style(
            id(LABEL),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    text_emphasis_style: Some(dot()),
                    text_emphasis_position: Some(position),
                    ..label_layout()
                }),
                ..NodeStyle::default()
            },
        );
        runtime.context_mut().commit_mutations(queue).unwrap();
    };
    restyle(&mut runtime, nana_ui_core::TextEmphasisPosition::OVER);
    settle(&mut runtime, &mut shaper);
    let marked = runtime.context().world().text_metrics(id(LABEL)).unwrap();
    assert!(
        marked.height > plain.height,
        "the marks take room: {} -> {}",
        plain.height,
        marked.height
    );
    let over = runtime
        .context()
        .world()
        .text_layout(id(LABEL))
        .unwrap()
        .1
        .clone();
    assert_eq!(
        over.emphasis.len(),
        "Helloworld".len(),
        "no mark on the space"
    );
    let laid_out = runtime.context().world().layout_box(id(LABEL)).unwrap();
    assert!(laid_out.height >= marked.height - 0.5, "layout read it");

    restyle(&mut runtime, nana_ui_core::TextEmphasisPosition::UNDER);
    runtime.flush(viewport(), &mut shaper).unwrap();
    let work = text_work(&runtime);
    assert!(
        work.layouts_created > 0,
        "a new side is a new layout: {work:?}"
    );
    assert_eq!(
        work.shape_cache_misses,
        Some(0),
        "from the runs already shaped: {work:?}"
    );
    settle(&mut runtime, &mut shaper);
    let (_, under) = runtime.context().world().text_layout(id(LABEL)).unwrap();
    assert!(under.emphasis[0].rect.y > over.emphasis[0].rect.y);
}
