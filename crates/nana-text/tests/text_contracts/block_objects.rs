//! Object placements: an object on a line of its own, and one scaled down to
//! the room its line has. The source keeps its bytes and the object its
//! authored size; only the layout's lines and boxes change.

use crate::support;

use std::sync::Arc;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    Affinity, InlineEnvelope, InlineObject, InlineObjectMetrics, LineBreakCause, NativeTextEngine,
    ObjectFlow, ObjectPlacement, TextConstraints, TextEngine, TextKind, TextLayout, TextSource,
    TextStyle, TextWorkCounters,
};
use nana_ui_core::TextWrapBreak;
use support::corpus::fixture_bytes;

const OBJECT: char = '\u{FFFC}';
const OBJECT_LEN: usize = OBJECT.len_utf8();

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

/// A 240 × 120 picture.
fn picture(offset: usize) -> InlineObject {
    InlineObject {
        offset,
        id: offset as u64,
        metrics: InlineObjectMetrics {
            width_px: 240.0,
            ascent_px: 120.0,
            descent_px: 0.0,
            envelope: None,
        },
    }
}

/// `text` with a picture at each U+FFFC, every one placed as `placement`
/// says.
fn source(text: &str, placement: impl Fn(usize) -> ObjectPlacement) -> TextSource {
    let offsets: Vec<usize> = text
        .char_indices()
        .filter(|(_, ch)| *ch == OBJECT)
        .map(|(at, _)| at)
        .collect();
    let mut source = TextSource::new(text);
    source.set_objects(offsets.iter().copied().map(picture).collect());
    source.set_placements(offsets.iter().copied().map(placement).collect());
    source
}

fn block(offset: usize) -> ObjectPlacement {
    ObjectPlacement::new(offset, ObjectFlow::Block)
}

fn fitted_block(offset: usize) -> ObjectPlacement {
    block(offset).fit_width(true)
}

/// A label, unwrapped, with an authored newline as a line break.
fn label(
    engine: &mut NativeTextEngine,
    source: &TextSource,
    width: Option<f32>,
    counters: &mut TextWorkCounters,
) -> Arc<TextLayout> {
    let constraints = TextConstraints {
        preserve_lines: true,
        max_width_px: width,
        ..TextConstraints::default()
    };
    engine.layout(TextKind::Label, source, &style(), &constraints, counters)
}

#[test]
fn a_block_object_takes_a_line_of_its_own_and_the_source_keeps_its_bytes() {
    let mut engine = engine();
    for (text, lines) in [
        ("\u{FFFC}", 1),
        ("a\u{FFFC}b", 3),
        ("a\u{FFFC}", 2),
        ("\u{FFFC}b", 2),
        ("a\n\u{FFFC}\nb", 3),
        ("\u{FFFC}\u{FFFC}", 2),
        ("a\u{FFFC}\u{FFFC}b", 4),
        ("a\n\n\u{FFFC}", 3),
        ("\u{FFFC}\n", 2),
    ] {
        let source = source(text, block);
        let layout = label(&mut engine, &source, None, &mut TextWorkCounters::default());
        assert_eq!(source.text(), text);
        assert_eq!(layout.lines.len(), lines, "{text:?}: {:?}", layout.lines);
        assert_eq!(
            layout.objects.len(),
            text.matches(OBJECT).count(),
            "{text:?}"
        );
        for placed in &layout.objects {
            assert_eq!(
                layout.lines[placed.line as usize].source,
                placed.offset..placed.offset + OBJECT_LEN,
                "{text:?}: the object's line holds it alone"
            );
        }
    }
    // The same text inline is one line.
    let inline = source("a\u{FFFC}b", |offset| {
        ObjectPlacement::new(offset, ObjectFlow::Inline)
    });
    assert!(
        inline.placements().is_empty(),
        "a default placement is dropped"
    );
    let layout = label(&mut engine, &inline, None, &mut TextWorkCounters::default());
    assert_eq!(layout.lines.len(), 1);
}

#[test]
fn a_block_edge_is_a_wrap_so_a_caret_has_a_place_on_both_lines() {
    let mut engine = engine();
    let text = "a\u{FFFC}b";
    let source = source(text, block);
    let layout = label(&mut engine, &source, None, &mut TextWorkCounters::default());
    let causes: Vec<_> = layout.lines.iter().map(|line| line.break_cause).collect();
    assert_eq!(
        causes,
        [
            LineBreakCause::Wrap,
            LineBreakCause::Wrap,
            LineBreakCause::EndOfText
        ]
    );
    // The byte before the object ends the first line upstream and starts the
    // object's line downstream; the byte after it likewise.
    for (line, byte) in [(0u32, 1usize), (1, 1 + OBJECT_LEN)] {
        let stops = layout.caret_stops(line, text);
        let last = stops.last().expect("the line has carets");
        assert_eq!(last.caret.byte, byte, "line {line}");
        assert_eq!(last.caret.affinity, Affinity::Upstream, "line {line}");
        let next = layout.caret_stops(line + 1, text);
        assert_eq!(next.first().unwrap().caret.byte, byte, "line {}", line + 1);
    }
}

#[test]
fn a_fitted_object_scales_to_its_line_and_every_edit_geometry_reads_the_scaled_box() {
    let mut engine = engine();
    let text = "a\u{FFFC}b";
    let source = source(text, fitted_block);
    let layout = label(
        &mut engine,
        &source,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let placed = layout.objects[0];
    assert!(
        (placed.rect.width - 100.0).abs() < 0.01,
        "{:?}",
        placed.rect
    );
    assert!(
        (placed.rect.height - 50.0).abs() < 0.01,
        "the aspect is kept: {:?}",
        placed.rect
    );
    let line = &layout.lines[placed.line as usize];
    assert!(line.bounds.height >= 50.0 - 0.01, "the line holds it");
    assert_eq!(
        source.objects()[0].metrics.width_px,
        240.0,
        "the source keeps the authored size"
    );
    for (fraction, byte) in [(0.1, 1), (0.9, 1 + OBJECT_LEN)] {
        let hit = layout.hit_test_text(
            text,
            placed.rect.x + placed.rect.width * fraction,
            placed.rect.y + placed.rect.height * 0.5,
        );
        assert_eq!(hit.caret.byte, byte);
        assert_eq!(hit.caret.line, placed.line);
        let caret = layout.caret_geometry(hit.caret).unwrap();
        let edge = if fraction < 0.5 {
            placed.rect.x
        } else {
            placed.rect.right()
        };
        assert!((caret.x_px - edge).abs() < 0.01);
    }
    let selected = layout.selection_rects(1..1 + OBJECT_LEN);
    assert_eq!(selected.len(), 1);
    assert!((selected[0].width - placed.rect.width).abs() < 0.01);

    let wide = label(
        &mut engine,
        &source,
        Some(500.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(wide.objects[0].rect.width, 240.0, "never scaled up");
    let unbounded = label(&mut engine, &source, None, &mut TextWorkCounters::default());
    assert_eq!(
        unbounded.objects[0].rect.width, 240.0,
        "no budget, no scaling"
    );
}

#[test]
fn an_inline_fitted_object_wraps_with_the_text_at_its_scaled_width() {
    let mut engine = engine();
    let text = "hello \u{FFFC} world";
    let source = source(text, |offset| {
        ObjectPlacement::new(offset, ObjectFlow::Inline).fit_width(true)
    });
    let constraints = TextConstraints {
        max_width_px: Some(120.0),
        wrap: Some(TextWrapBreak::Word),
        ..TextConstraints::default()
    };
    let layout = engine.layout(
        TextKind::Paragraph,
        &source,
        &style(),
        &constraints,
        &mut TextWorkCounters::default(),
    );
    let placed = layout.objects[0];
    assert!(
        (placed.rect.width - 120.0).abs() < 0.01,
        "{:?}",
        placed.rect
    );
    assert!(
        (placed.rect.height - 60.0).abs() < 0.01,
        "{:?}",
        placed.rect
    );
    assert!(
        layout.lines.len() >= 3,
        "it is still inline, broken around: {:?}",
        layout.lines
    );
    assert!(
        layout
            .lines
            .iter()
            .all(|line| line.bounds.width <= 120.0 + 0.01),
        "no line sticks out: {:?}",
        layout.lines
    );
}

#[test]
fn the_room_a_fitted_object_gets_leaves_its_envelope_gap() {
    let mut engine = engine();
    let text = "\u{FFFC}";
    let mut source = TextSource::new(text);
    let mut object = picture(0);
    object.metrics.envelope = Some(InlineEnvelope {
        gap_px: 20.0,
        gap_shrink_px: 0.0,
        ..InlineEnvelope::default()
    });
    source.set_objects(vec![object]);
    source.set_placements(vec![fitted_block(0)]);
    let layout = label(
        &mut engine,
        &source,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let placed = layout.objects[0];
    assert!(
        (placed.rect.width - 80.0).abs() < 0.01,
        "budget less the gap: {:?}",
        placed.rect
    );
    assert!(
        (placed.rect.height - 40.0).abs() < 0.01,
        "{:?}",
        placed.rect
    );
    assert!(
        layout.lines[0].bounds.width <= 100.0 + 0.01,
        "object and gap fit the line: {:?}",
        layout.lines[0]
    );
    assert!(
        !layout
            .overflow
            .contains(nana_text::OverflowFlags::CLIPPED_WIDTH)
    );
}

#[test]
fn a_fitted_object_adds_nothing_to_min_content_and_its_width_to_max_content() {
    let mut engine = engine();
    let constraints = TextConstraints::default();
    let fitted = source("ab\u{FFFC}", fitted_block);
    let own = source("ab\u{FFFC}", block);
    let text_only = engine.intrinsic_widths(&TextSource::new("ab"), &style(), &constraints);

    let fitted = engine.intrinsic_widths(&fitted, &style(), &constraints);
    let own = engine.intrinsic_widths(&own, &style(), &constraints);
    assert_eq!(own.min_px, 240.0, "an unfitted picture cannot shrink");
    assert_eq!(own.max_px, 240.0);
    assert_eq!(fitted.max_px, 240.0, "max-content is its own width");
    assert!(
        (fitted.min_px - text_only.min_px).abs() < 0.01,
        "min-content is the text's alone: {fitted:?} vs {text_only:?}"
    );
    assert!(fitted.min_px < 240.0);
}

#[test]
fn changing_a_placement_relays_out_from_the_same_runs_and_never_aliases_a_layout() {
    let mut engine = engine();
    let text = "a\u{FFFC}b";
    let mut source = source(text, fitted_block);
    let first = label(
        &mut engine,
        &source,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(first.lines.len(), 3);

    source.set_placements(vec![ObjectPlacement::new(1, ObjectFlow::Inline)]);
    assert!(source.placements().is_empty());
    let mut work = TextWorkCounters::default();
    let inline = label(&mut engine, &source, Some(100.0), &mut work);
    assert_eq!(inline.lines.len(), 1);
    assert_eq!(inline.objects[0].rect.width, 240.0);
    assert_ne!(first.id, inline.id);
    assert_eq!(work.shape_cache_misses, Some(0), "{work:?}");

    // Two sources at the same revision, placed differently, are two layouts.
    let fitted = source_at_revision(text, Some(fitted_block(1)));
    let unfitted = source_at_revision(text, Some(block(1)));
    let plain = source_at_revision(text, Some(ObjectPlacement::new(1, ObjectFlow::Inline)));
    assert_eq!(fitted.revision(), unfitted.revision());
    assert_eq!(fitted.revision(), plain.revision());
    let fitted = label(
        &mut engine,
        &fitted,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let unfitted = label(
        &mut engine,
        &unfitted,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let plain = label(
        &mut engine,
        &plain,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(fitted.objects[0].rect.width, 100.0);
    assert_eq!(unfitted.objects[0].rect.width, 240.0);
    assert_eq!(unfitted.lines.len(), 3);
    assert_eq!(plain.lines.len(), 1);
    assert_ne!(fitted.id, unfitted.id, "fitting is a layout input");
    assert_ne!(unfitted.id, plain.id, "flow is a layout input");
}

/// A source built in the same steps whatever the placement, so every one is
/// at the same revision.
fn source_at_revision(text: &str, placement: Option<ObjectPlacement>) -> TextSource {
    let mut source = TextSource::new(text);
    source.set_objects(vec![picture(1)]);
    source.set_placements(placement.into_iter().collect());
    source
}

#[test]
fn placements_ride_on_their_objects_through_edits() {
    let mut source = source("a\u{FFFC}b\u{FFFC}", block);
    assert_eq!(source.placements().len(), 2);
    source.replace_range(0..1, "xyz");
    let offsets: Vec<usize> = source.placements().iter().map(|p| p.offset).collect();
    assert_eq!(offsets, [3, 3 + OBJECT_LEN + 1]);
    assert_eq!(
        source.placement_at(3).map(|p| p.flow),
        Some(ObjectFlow::Block)
    );
    source.replace_range(3..3 + OBJECT_LEN, "");
    let offsets: Vec<usize> = source.placements().iter().map(|p| p.offset).collect();
    assert_eq!(offsets, [4], "deleting an object drops its placement");

    // One on no object is dropped.
    source.set_placements(vec![block(0), block(4)]);
    assert_eq!(source.placements().len(), 1);
    source.set_text("plain");
    assert!(source.placements().is_empty());
}

#[test]
fn a_folded_source_keeps_its_placements() {
    let mut engine = engine();
    let text = "a\n\u{FFFC}b";
    let source = source(text, block);
    let constraints = TextConstraints {
        preserve_lines: false,
        ..TextConstraints::default()
    };
    let layout = engine.layout(
        TextKind::Label,
        &source,
        &style(),
        &constraints,
        &mut TextWorkCounters::default(),
    );
    assert_eq!(
        layout.lines.len(),
        3,
        "the newline folds to a space, the block still breaks: {:?}",
        layout.lines
    );
}

#[test]
fn a_source_serialized_before_placements_reads_with_none() {
    let mut source = TextSource::new("a\u{FFFC}");
    source.set_objects(vec![picture(1)]);
    let mut value = serde_json::to_value(&source).unwrap();
    value.as_object_mut().unwrap().remove("placements");
    let read: TextSource = serde_json::from_value(value).unwrap();
    assert!(read.placements().is_empty());
    assert_eq!(read.objects(), source.objects());

    let placement: ObjectPlacement = serde_json::from_str(r#"{"offset":1}"#).unwrap();
    assert_eq!(placement, ObjectPlacement::new(1, ObjectFlow::Inline));
}
