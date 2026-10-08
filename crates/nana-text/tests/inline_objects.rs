//! Inline objects: a U+FFFC in the text plus a side-table entry with its box.
//! Shaping never reads the box; layout does.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    InlineObject, InlineObjectMetrics, NativeTextEngine, TextConstraints, TextEngine, TextKind,
    TextLayout, TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::TextWrapBreak;
use std::sync::Arc;

use support::corpus::fixture_bytes;

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .expect("fixture registers");
    NativeTextEngine::new(fonts)
}

fn style() -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from("Noto Sans SC")),
        font_size_px: 16.0,
        ..TextStyle::default()
    }
}

fn sticker(offset: usize, width: f32, height: f32) -> InlineObject {
    InlineObject {
        offset,
        id: 7,
        metrics: InlineObjectMetrics {
            width_px: width,
            ascent_px: height,
            descent_px: 0.0,
        },
    }
}

fn source_with(text: &str, objects: Vec<InlineObject>) -> TextSource {
    let mut source = TextSource::new(text);
    source.set_objects(objects);
    source
}

fn lay_out(
    engine: &mut NativeTextEngine,
    source: &TextSource,
    max_width: Option<f32>,
    counters: &mut TextWorkCounters,
) -> Arc<TextLayout> {
    let constraints = TextConstraints {
        max_width_px: max_width,
        wrap: max_width.map(|_| TextWrapBreak::Word),
        ..TextConstraints::default()
    };
    let kind = if max_width.is_some() {
        TextKind::Paragraph
    } else {
        TextKind::Label
    };
    engine.layout(kind, source, &style(), &constraints, counters)
}

const TEXT: &str = "hi \u{FFFC} there";
const AT: usize = 3;

#[test]
fn an_object_takes_its_width_on_the_line_and_reports_its_box() {
    let mut engine = engine();
    let mut counters = TextWorkCounters::default();
    let without = lay_out(
        &mut engine,
        &source_with(TEXT, vec![sticker(AT, 0.0, 0.0)]),
        None,
        &mut counters,
    );
    let with = lay_out(
        &mut engine,
        &source_with(TEXT, vec![sticker(AT, 40.0, 12.0)]),
        None,
        &mut counters,
    );
    let grown = with.lines[0].metrics.width_px - without.lines[0].metrics.width_px;
    assert!(
        (grown - 40.0).abs() < 0.01,
        "the object adds its width: {grown}"
    );
    assert_eq!(with.objects.len(), 1);
    let placed = with.objects[0];
    assert_eq!((placed.id, placed.offset, placed.line), (7, AT, 0));
    assert!((placed.rect.width - 40.0).abs() < 0.01);
    assert!((placed.rect.height - 12.0).abs() < 0.01);
    let baseline = with.lines[0].metrics.baseline_y_px;
    assert!(
        (placed.rect.y + placed.rect.height - baseline).abs() < 0.01,
        "it stands on the baseline"
    );
    assert!(
        without
            .runs
            .iter()
            .filter(|run| run.is_object())
            .all(|run| run.instance.is_none()),
        "a placeholder has no face to draw"
    );
}

#[test]
fn resizing_an_object_lays_out_again_without_shaping_again() {
    let mut engine = engine();
    let mut counters = TextWorkCounters::default();
    let mut source = source_with(TEXT, vec![sticker(AT, 20.0, 12.0)]);
    let _ = lay_out(&mut engine, &source, None, &mut counters);
    let mut resized = TextWorkCounters::default();
    source.set_objects(vec![sticker(AT, 60.0, 12.0)]);
    let layout = lay_out(&mut engine, &source, None, &mut resized);
    assert_eq!(resized.shape_cache_misses, Some(0), "{resized:?}");
    assert_eq!(resized.layouts_created, 1, "{resized:?}");
    assert!((layout.objects[0].rect.width - 60.0).abs() < 0.01);
}

#[test]
fn a_tall_object_grows_its_line_and_keeps_inside_it() {
    let mut engine = engine();
    let mut counters = TextWorkCounters::default();
    let short = lay_out(
        &mut engine,
        &source_with(TEXT, vec![sticker(AT, 20.0, 8.0)]),
        None,
        &mut counters,
    );
    let tall = lay_out(
        &mut engine,
        &source_with(TEXT, vec![sticker(AT, 48.0, 48.0)]),
        None,
        &mut counters,
    );
    let line = &tall.lines[0];
    assert!(line.metrics.height_px >= 48.0, "{:?}", line.metrics);
    assert!(line.metrics.height_px > short.lines[0].metrics.height_px);
    assert!(
        tall.objects[0].rect.y >= line.metrics.top_y_px - 0.01,
        "the object does not stick out of the top of its line"
    );
}

#[test]
fn an_object_wraps_with_the_text_around_it() {
    let mut engine = engine();
    let mut counters = TextWorkCounters::default();
    let text = "one two \u{FFFC} three";
    let source = source_with(text, vec![sticker(8, 40.0, 14.0)]);
    let wide = lay_out(&mut engine, &source, Some(400.0), &mut counters);
    assert_eq!(wide.lines.len(), 1);
    let one_two = lay_out(
        &mut engine,
        &TextSource::new("one two "),
        None,
        &mut counters,
    )
    .lines[0]
        .metrics
        .width_px;
    let narrow = lay_out(&mut engine, &source, Some(one_two + 10.0), &mut counters);
    assert!(narrow.lines.len() >= 2);
    assert_eq!(
        narrow.objects[0].line, 1,
        "the object moved down with the words after it"
    );
}

#[test]
fn objects_follow_edits_and_only_sit_on_object_characters() {
    let mut source = source_with(TEXT, vec![sticker(AT, 10.0, 10.0), sticker(0, 1.0, 1.0)]);
    assert_eq!(
        source.objects().len(),
        1,
        "an entry over plain text is dropped"
    );
    source.replace_range(0..2, "hello");
    assert_eq!(source.objects()[0].offset, AT + 3);
    source.replace_range(AT + 3..AT + 6, "");
    assert!(
        source.objects().is_empty(),
        "deleting the character deletes the object"
    );
}
