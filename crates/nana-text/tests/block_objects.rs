//! 块对象视觉断行、等比缩放和编辑几何的契约测试。

mod support;

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem};
use nana_text::{
    InlineObject, InlineObjectMetrics, NativeTextEngine, TextConstraints, TextEngine, TextKind,
    TextSource, TextStyle, TextWorkCounters,
};
use nana_ui_core::RichObjectFlow;
use support::corpus::fixture_bytes;

fn engine() -> NativeTextEngine {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    fonts
        .register_bytes(fixture_bytes("noto-sans-sc"), &FaceDescriptor::default())
        .unwrap();
    NativeTextEngine::new(fonts)
}

fn object(offset: usize) -> InlineObject {
    InlineObject {
        offset,
        id: offset as u64,
        metrics: InlineObjectMetrics {
            width_px: 240.0,
            ascent_px: 120.0,
            descent_px: 0.0,
            envelope: None,
        },
        flow: RichObjectFlow::Block,
        fit_width: true,
    }
}

fn source(text: &str) -> TextSource {
    let mut source = TextSource::new(text);
    source.set_objects(
        text.char_indices()
            .filter(|(_, ch)| *ch == '\u{FFFC}')
            .map(|(at, _)| object(at))
            .collect(),
    );
    source
}

fn layout(
    engine: &mut NativeTextEngine,
    source: &TextSource,
    width: Option<f32>,
    counters: &mut TextWorkCounters,
) -> std::sync::Arc<nana_text::TextLayout> {
    engine.layout(
        TextKind::Label,
        source,
        &TextStyle {
            font_family: Some(std::sync::Arc::from("Noto Sans SC")),
            ..TextStyle::default()
        },
        &TextConstraints {
            preserve_lines: true,
            max_width_px: width,
            ..TextConstraints::default()
        },
        counters,
    )
}

#[test]
fn block_boundaries_merge_with_existing_newlines_and_keep_source_bytes() {
    let mut engine = engine();
    for (text, lines) in [
        ("\u{FFFC}", 1),
        ("a\u{FFFC}b", 3),
        ("a\n\u{FFFC}\nb", 3),
        ("\u{FFFC}\u{FFFC}", 2),
        ("a\u{FFFC}\u{FFFC}b", 4),
        ("a\n\n\u{FFFC}", 3),
    ] {
        let source = source(text);
        let result = layout(&mut engine, &source, None, &mut TextWorkCounters::default());
        assert_eq!(source.text(), text);
        assert_eq!(result.lines.len(), lines, "{text:?}: {:?}", result.lines);
        for placed in &result.objects {
            assert_eq!(
                result.lines[placed.line as usize].source,
                placed.offset..placed.offset + '\u{FFFC}'.len_utf8()
            );
        }
    }
}

#[test]
fn fit_width_scales_the_layout_box_and_all_edit_geometry() {
    let mut engine = engine();
    let source = source("a\u{FFFC}b");
    let result = layout(
        &mut engine,
        &source,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let placed = result.objects[0];
    assert_eq!(result.lines[0].break_cause, nana_text::LineBreakCause::Wrap);
    assert_eq!(result.lines[1].break_cause, nana_text::LineBreakCause::Wrap);
    // 同一字节位置上下两行均可落光标；前一行末端使用 upstream affinity。
    let stops = result.caret_stops(placed.line, source.text());
    let last = stops.last().unwrap();
    assert_eq!(last.caret.byte, 4);
    assert_eq!(last.caret.affinity, nana_text::Affinity::Upstream);
    assert!((placed.rect.width - 100.0).abs() < 0.01);
    assert!((placed.rect.height - 50.0).abs() < 0.01);
    assert_eq!(source.objects()[0].metrics.width_px, 240.0);
    assert_eq!(source.objects()[0].metrics.ascent_px, 120.0);
    for (fraction, byte) in [(0.1, 1), (0.9, 4)] {
        let hit = result.hit_test_text(
            source.text(),
            placed.rect.x + placed.rect.width * fraction,
            placed.rect.y + placed.rect.height * 0.5,
        );
        assert_eq!(hit.caret.byte, byte);
        assert_eq!(hit.caret.line, placed.line);
        let caret = result.caret_geometry(hit.caret).unwrap();
        let expected = if fraction < 0.5 {
            placed.rect.x
        } else {
            placed.rect.right()
        };
        assert!((caret.x_px - expected).abs() < 0.01);
    }
    let selected = result.selection_rects(1..4);
    assert_eq!(selected.len(), 1);
    assert!((selected[0].width - placed.rect.width).abs() < 0.01);
    let wide = layout(
        &mut engine,
        &source,
        Some(500.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(
        wide.objects[0].rect.width, 240.0,
        "smaller images keep authored size"
    );
}

#[test]
fn changing_flow_or_fitting_relayouts_without_reshaping_or_cache_aliases() {
    let mut engine = engine();
    let mut source = source("a\u{FFFC}b");
    let initial = layout(
        &mut engine,
        &source,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    let mut changed = object(1);
    changed.flow = RichObjectFlow::Inline;
    changed.fit_width = false;
    source.set_objects(vec![changed]);
    let mut work = TextWorkCounters::default();
    let inline = layout(&mut engine, &source, Some(100.0), &mut work);
    assert_eq!(inline.lines.len(), 1);
    assert_eq!(inline.objects[0].rect.width, 240.0);
    assert_ne!(initial.id, inline.id);
    assert_eq!(work.shape_cache_misses, Some(0));
    // 独立 source 可有相同 revision，布局缓存仍须包含模式与缩放标志。
    let mut equivalent = TextSource::new("a\u{FFFC}b");
    equivalent.set_objects(vec![object(1)]);
    let block = layout(
        &mut engine,
        &equivalent,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(block.lines.len(), 3);
    assert_eq!(block.objects[0].rect.width, 100.0);
    let mut same_revision = TextSource::new("a\u{FFFC}b");
    let mut inline_object = object(1);
    inline_object.flow = RichObjectFlow::Inline;
    same_revision.set_objects(vec![inline_object]);
    assert_eq!(same_revision.revision(), equivalent.revision());
    let inline = layout(
        &mut engine,
        &same_revision,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(inline.lines.len(), 1);
    assert_ne!(inline.id, block.id, "flow is a distinct layout cache input");
    let mut same_revision = TextSource::new("a\u{FFFC}b");
    let mut unscaled = object(1);
    unscaled.fit_width = false;
    same_revision.set_objects(vec![unscaled]);
    let unscaled = layout(
        &mut engine,
        &same_revision,
        Some(100.0),
        &mut TextWorkCounters::default(),
    );
    assert_eq!(unscaled.lines.len(), 3);
    assert_eq!(unscaled.objects[0].rect.width, 240.0);
    assert_ne!(
        unscaled.id, block.id,
        "fit policy is a distinct layout cache input"
    );
}

#[test]
fn old_serialized_objects_default_to_inline_and_original_size() {
    let old = r#"{"offset":0,"id":1,"metrics":{"width_px":20,"ascent_px":10,"descent_px":0}}"#;
    let value: InlineObject = serde_json::from_str(old).unwrap();
    assert_eq!(value.flow, RichObjectFlow::Inline);
    assert!(!value.fit_width);
}
