//! Object labels: an inline object that shows a small label (an editor's
//! marker tag) takes the label's room on its line, and the layout places the
//! label's runs inside it.

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    InlineObject, InlineObjectMetrics, NativeTextEngine, OBJECT_REPLACEMENT, ObjectLabel,
    TextConstraints, TextEngine, TextKind, TextLayout, TextSource, TextStyle, TextWorkCounters,
};
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
        font_size_px: 20.0,
        ..TextStyle::default()
    }
}

/// "前￼后" with a zero-sized object at the replacement character, labelled
/// `label` when given.
fn source(label: Option<&str>) -> TextSource {
    let text = format!("前{OBJECT_REPLACEMENT}后");
    let offset = "前".len();
    let mut source = TextSource::new(text);
    source.set_objects(vec![InlineObject {
        offset,
        id: 7,
        metrics: InlineObjectMetrics::default(),
    }]);
    if let Some(label) = label {
        source.set_labels(vec![ObjectLabel {
            offset,
            text: Arc::from(label),
        }]);
    }
    source
}

fn lay_out(engine: &mut NativeTextEngine, source: &TextSource) -> Arc<TextLayout> {
    let mut counters = TextWorkCounters::default();
    engine.layout(
        TextKind::Label,
        source,
        &style(),
        &TextConstraints::default(),
        &mut counters,
    )
}

#[test]
fn a_labelled_object_takes_its_labels_room() {
    let mut engine = engine();
    let bare = lay_out(&mut engine, &source(None));
    assert_eq!(bare.objects.len(), 1);
    assert_eq!(
        bare.objects[0].rect.width, 0.0,
        "an unlabelled marker takes none"
    );
    assert!(bare.labels.is_empty());

    let tagged = lay_out(&mut engine, &source(Some("暂停")));
    let object = tagged.objects[0];
    let label = &tagged.labels[0];
    let label_width: f32 = label.runs.iter().map(|run| run.advance_px).sum();
    assert!(label_width > 0.0, "the label is shaped");
    assert!(
        object.rect.width > label_width,
        "the object holds the label and its padding: {} vs {label_width}",
        object.rect.width
    );
    assert!(
        (tagged.bounds.width - bare.bounds.width - object.rect.width).abs() < 0.5,
        "the text after it moves on by exactly the tag"
    );
    // The label sits inside the tag, which sits inside the object.
    assert!(label.rect.x >= object.rect.x && label.rect.right() <= object.rect.right() + 0.01);
    let first = label.runs.first().expect("runs").origin_x_px;
    assert!(first > label.rect.x, "padding before the label");
    assert!(first + label_width < label.rect.right(), "padding after it");
    assert_eq!(label.offset, "前".len());
}

#[test]
fn labels_ride_on_their_objects_through_edits() {
    let mut source = source(Some("暂停"));
    source.replace_range(0..0, "啊");
    assert_eq!(source.labels()[0].offset, "啊前".len());
    assert_eq!(source.objects()[0].offset, "啊前".len());
    let at = source.labels()[0].offset;
    source.replace_range(at..at + OBJECT_REPLACEMENT.len_utf8(), "");
    assert!(
        source.labels().is_empty(),
        "removing the object drops its label"
    );
}

#[test]
fn a_label_without_its_object_is_dropped() {
    let mut source = TextSource::new("plain");
    source.set_labels(vec![ObjectLabel {
        offset: 0,
        text: Arc::from("x"),
    }]);
    assert!(source.labels().is_empty());
}
