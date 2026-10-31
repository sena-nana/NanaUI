//! 文字背景在深浅主题中的默认对比色回归，验证作者色、透明合成和 retained 排版。

use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_ui_core::{LayoutStyle, PaintColor, RichSpanStyle, RichText, ThemeAppearance};
use nana_ui_runtime::{
    DocumentId, Entity, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeKind, NodeStyle,
    RichTextView, StableNodeId,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const AUTHOR: PaintColor = PaintColor::srgb([1.0, 0.0, 0.0, 0.7]);

/// 创建带真实祖先表面的文字节点，避免用主题名称猜测半透明背景的底色。
fn document(
    theme: ThemeAppearance,
    base: [f32; 4],
    layer: [f32; 4],
    highlight: [f32; 4],
) -> (
    RuntimeDocument,
    NanaTextEngineShaper,
    Entity<RichTextView>,
    RichText,
) {
    let doc = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(doc);
    runtime.context_mut().set_preset_theme(theme).unwrap();
    let style = RichSpanStyle::new().background_color(PaintColor::srgb(highlight));
    let rich = RichText::new("abc")
        .with_span(0..1, style.clone())
        .with_span(1..2, style.color(AUTHOR));
    let entity = runtime
        .context_mut()
        .create_component(doc, RichTextView::new(rich.clone()))
        .unwrap();
    let root = StableNodeId::new(1000).unwrap();
    let panel = StableNodeId::new(1001).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(root, doc, NodeKind::Document);
    queue.create(
        panel,
        doc,
        NodeKind::Element {
            tag: "panel".into(),
        },
    );
    queue.insert(root, panel, None);
    queue.insert(panel, entity.stable_id(), None);
    for (id, background) in [(root, base), (panel, layer)] {
        queue.set_style(
            id,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    background: Some(background),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
    }
    runtime.context_mut().commit_mutations(queue).unwrap();
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .unwrap();
    let mut shaper = NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
    settle(&mut runtime, &mut shaper);
    (runtime, shaper, entity, rich)
}

fn settle(runtime: &mut RuntimeDocument, shaper: &mut NanaTextEngineShaper) {
    for _ in 0..4 {
        runtime
            .flush(LayoutViewport::new(400.0, 300.0), shaper)
            .unwrap();
    }
}

fn fill_at(runtime: &RuntimeDocument, entity: Entity<RichTextView>, at: usize) -> [f32; 4] {
    runtime
        .scene()
        .primitives()
        .find_map(|p| {
            if p.node != entity.stable_id() {
                return None;
            }
            let ScenePrimitiveKind::Text { color, spans, .. } = &p.kind else {
                return None;
            };
            Some(
                spans
                    .iter()
                    .find(|span| span.start <= at && at < span.end)
                    .map(|span| span.color)
                    .or(*color)
                    .unwrap()
                    .to_srgb(),
            )
        })
        .unwrap()
}

#[test]
fn default_fill_adapts_to_highlight_and_actual_alpha_backdrop_in_both_themes() {
    let cases = [
        (
            ThemeAppearance::Dark,
            BLACK,
            [0.0; 4],
            [1.0, 1.0, 0.0, 1.0],
            BLACK,
        ),
        (
            ThemeAppearance::Light,
            WHITE,
            [0.0; 4],
            [1.0, 1.0, 0.0, 1.0],
            BLACK,
        ),
        (
            ThemeAppearance::Dark,
            BLACK,
            [0.0; 4],
            [1.0, 1.0, 0.0, 0.03],
            WHITE,
        ),
        (
            ThemeAppearance::Dark,
            BLACK,
            [1.0, 1.0, 1.0, 0.5],
            [1.0, 1.0, 0.0, 0.2],
            BLACK,
        ),
        (
            ThemeAppearance::Light,
            WHITE,
            [0.0; 4],
            [0.0, 0.0, 1.0, 1.0],
            WHITE,
        ),
    ];
    for (theme, base, layer, highlight, expected) in cases {
        let (runtime, _, entity, rich) = document(theme, base, layer, highlight);
        let fill = fill_at(&runtime, entity, 0);
        assert!(
            fill[..3].iter().all(|channel| {
                if expected == BLACK {
                    *channel < 0.25
                } else {
                    *channel > 0.75
                }
            }),
            "theme={theme:?} highlight={highlight:?} fill={fill:?}"
        );
        assert_eq!(
            fill_at(&runtime, entity, 1),
            AUTHOR.to_srgb(),
            "作者文字色保留"
        );
        assert_eq!(
            runtime.context().world().rich_text(entity.stable_id()),
            Some(&rich)
        );
        assert_eq!(rich.style_at(0).unwrap().paint.color, None);
    }
}

#[test]
fn highlight_contrast_is_paint_only_and_leaves_plain_text_color_unchanged() {
    let (mut runtime, mut shaper, entity, mut rich) =
        document(ThemeAppearance::Dark, BLACK, [0.0; 4], [0.0, 0.0, 1.0, 1.0]);
    let plain = fill_at(&runtime, entity, 2);
    let (before, layout) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    let layout = Arc::clone(layout);
    rich.apply_span(
        0..1,
        &RichSpanStyle::new().background_color(PaintColor::srgb([1.0, 1.0, 0.0, 1.0])),
    );
    runtime
        .context_mut()
        .set_component(entity, RichTextView::new(rich))
        .unwrap();
    runtime
        .flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
        .unwrap();
    assert_eq!(fill_at(&runtime, entity, 0), BLACK);
    assert_eq!(fill_at(&runtime, entity, 2), plain);
    let (after, retained) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    assert_eq!(before, after);
    assert!(Arc::ptr_eq(&layout, retained));
    let work = runtime.context().world().last_text_work_counters();
    assert_eq!(work.text_nodes_shaped, 0);
    assert_eq!(work.layouts_created, 0);
}

#[test]
fn ancestor_surface_change_refreshes_translucent_highlight_without_text_extraction() {
    let (mut runtime, mut shaper, entity, rich) = document(
        ThemeAppearance::Dark,
        BLACK,
        [0.0; 4],
        [1.0, 1.0, 0.0, 0.03],
    );
    assert!(fill_at(&runtime, entity, 0)[0] > 0.75);
    let (before, _) = runtime
        .context()
        .world()
        .text_layout(entity.stable_id())
        .unwrap();
    for surface in [WHITE, BLACK, WHITE] {
        let mut queue = MutationQueue::new();
        queue.set_style(
            StableNodeId::new(1000).unwrap(),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    background: Some(surface),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        runtime.context_mut().commit_mutations(queue).unwrap();
        runtime
            .flush(LayoutViewport::new(400.0, 300.0), &mut shaper)
            .unwrap();
        if surface == WHITE {
            assert_eq!(fill_at(&runtime, entity, 0), BLACK);
        } else {
            assert!(fill_at(&runtime, entity, 0)[0] > 0.75);
        }
        assert_eq!(
            runtime.context().world().rich_text(entity.stable_id()),
            Some(&rich)
        );
        assert_eq!(
            runtime
                .context()
                .world()
                .text_layout(entity.stable_id())
                .unwrap()
                .0,
            before
        );
        let work = runtime.context().world().last_text_work_counters();
        assert_eq!(work.text_nodes_shaped, 0);
        assert_eq!(work.layouts_created, 0);
    }
}
