//! Validation feedback must paint the same complete paragraph it measures.
use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_core::{LengthSpec, ThemeDefinition, ValidationIntent};
use nana_ui_scene::{ScenePrimitiveKind, UiScene};

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
    for _ in 0..8 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
            .unwrap();
        if cx.shape_text_for_layout(doc, shaper).unwrap() {
            cx.layout_document(doc, LayoutViewport::new(600.0, 400.0))
                .unwrap();
        }
    }
}

#[test]
fn long_validation_keeps_its_final_instruction_visible_after_theme_changes() {
    let message =
        "当前投币数仍是0枚，与投币前相同。确认没有扣币后可以解除这条记录，解除后不会自动再次投币";
    let tail = message.find("解除后").unwrap();
    for compact in [true, false] {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(921).unwrap();
        let root = cx
            .create_component(doc, Stack::column(0.0).width(LengthSpec::Px(300.0)))
            .unwrap();
        let notice = cx
            .create_detached_component(
                doc,
                ValidationMessage::new(message, ValidationIntent::Warning).compact(compact),
            )
            .unwrap();
        cx.append_child(root, notice).unwrap();
        let mut shaper = NanaTextShaper::default();
        let mut previous = 0.0;
        for font in [12.0, 24.0, 10.0] {
            let mut theme = ThemeDefinition::NANA_DARK;
            theme.typography.hint = font;
            theme.typography.meta = font;
            theme.typography.line = font + 6.0;
            cx.set_theme_definition(&theme).unwrap();
            settle(&mut cx, doc, &mut shaper);
            let mut scene = UiScene::new();
            scene.apply_delta(cx.world().extract_document(doc), []);
            let primitive = scene
                .primitives()
                .find(|primitive| {
                    primitive.node == notice.stable_id()
                        && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
                })
                .unwrap();
            let ScenePrimitiveKind::Text {
                content,
                size,
                weight,
                line_height,
                wrap,
                ellipsis,
                max_lines,
                ..
            } = &primitive.kind
            else {
                unreachable!()
            };
            let mut style = cx
                .world()
                .computed_style(notice.stable_id())
                .unwrap()
                .clone();
            style.font_size = *size;
            style.font_weight = *weight;
            style.line_height = *line_height;
            let constraints = TextShapeConstraints {
                max_width: Some(primitive.bounds.width),
                max_height: Some(primitive.bounds.height),
                wrap: *wrap,
                ellipsis: *ellipsis,
                max_lines: *max_lines,
                ..TextShapeConstraints::default()
            };
            let text = TextContent {
                value: content.clone(),
            };
            let painted = shaper.shape(notice.stable_id(), &text, &style, constraints);
            let highlights = shaper.text_highlights(
                notice.stable_id(),
                &text,
                (tail, message.len()),
                &style,
                constraints,
            );
            let tail_width = shaper
                .shape(
                    StableNodeId::new(u64::MAX).unwrap(),
                    &TextContent {
                        value: message[tail..].into(),
                    },
                    &style,
                    TextShapeConstraints::default(),
                )
                .width;
            assert!(
                (highlights.iter().map(|rect| rect.width).sum::<f32>() - tail_width).abs() < 1.0,
                "all final-instruction glyphs are present, including across line breaks"
            );
            assert!(
                painted.height > font + 6.0,
                "the actual paint paragraph wraps: {painted:?}"
            );
            assert!(painted.height <= primitive.bounds.height + 0.5);
            assert!(
                !highlights.is_empty(),
                "the final instruction has visible glyphs"
            );
            assert!(
                highlights.iter().all(|rect| {
                    rect.y >= 0.0
                        && rect.y + rect.height <= primitive.bounds.height + 0.5
                        && rect.x + rect.width <= primitive.bounds.width + 0.5
                }),
                "the final instruction is inside the measured paint region: {highlights:?}"
            );
            let (_, final_y, final_height) = shaper.text_position(
                notice.stable_id(),
                &text,
                message.len(),
                &style,
                constraints,
            );
            assert!(final_y > 0.0 && final_y + final_height <= primitive.bounds.height + 0.5);
            if font == 24.0 {
                assert!(primitive.bounds.height > previous);
            }
            if font == 10.0 {
                assert!(primitive.bounds.height < previous);
            }
            previous = primitive.bounds.height;
        }
    }
}

#[test]
fn validation_preserves_explicit_single_line_overflow() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(922).unwrap();
    let mut style = NodeStyle::default();
    let layout = std::sync::Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Px(140.0));
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    let notice = cx
        .create_component(
            doc,
            ValidationMessage::new(
                "An explicitly single-line warning may still request overflow elision.",
                ValidationIntent::Warning,
            )
            .style(style),
        )
        .unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let mut scene = UiScene::new();
    scene.apply_delta(cx.world().extract_document(doc), []);
    let primitive = scene
        .primitives()
        .find(|primitive| {
            primitive.node == notice.stable_id()
                && matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
        })
        .unwrap();
    let ScenePrimitiveKind::Text { wrap, ellipsis, .. } = &primitive.kind else {
        unreachable!()
    };
    assert!(
        !wrap && *ellipsis,
        "explicit overflow remains an authored choice"
    );
}
