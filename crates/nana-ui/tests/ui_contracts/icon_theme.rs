//! Icon geometry, its painted primitive and the neighboring text share live tokens.
use nana_ui::{HeadlessInput, NanaTextShaper, runtime::*};
use nana_ui_core::{ControlSize, Icon, ThemeDefinition};
use nana_ui_platform::{InputModifiers, InputPayload, KeyInput, KeyState};
use nana_ui_scene::{ScenePrimitiveKind, UiScene};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
    for _ in 0..6 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        cx.layout_document(doc, LayoutViewport::new(800.0, 800.0))
            .unwrap();
        if cx.shape_text_for_layout(doc, shaper).unwrap() {
            cx.layout_document(doc, LayoutViewport::new(800.0, 800.0))
                .unwrap();
        }
    }
    cx.rebuild_hit_test(doc);
}

fn painted_sizes(scene: &UiScene, id: StableNodeId) -> Vec<f32> {
    scene
        .primitives()
        .filter(|p| p.node == id && matches!(p.kind, ScenePrimitiveKind::Icon { .. }))
        .map(|p| {
            assert_eq!(p.bounds.width, p.bounds.height);
            p.bounds.width
        })
        .collect()
}

#[test]
fn mounted_glyph_and_button_icons_follow_size_roles_without_losing_focus_or_numeric_overrides() {
    for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(930).unwrap();
        let root = cx
            .create_component(doc, Stack::column(8.0).align(AlignSpec::Start))
            .unwrap();
        let plain = cx
            .create_detached_component(doc, Button::new("保存").size(size))
            .unwrap();
        let button = cx
            .create_detached_component(
                doc,
                Button::new("保存")
                    .size(size)
                    .icon(Icon::File)
                    .trailing_icon(Icon::ChevronDown),
            )
            .unwrap();
        let overridden = cx
            .create_detached_component(
                doc,
                Button::new("保存")
                    .size(size)
                    .icon(Icon::File)
                    .icon_size(9.0)
                    .icon_gap(6.0),
            )
            .unwrap();
        let action = cx
            .create_detached_component(doc, IconButton::new(Icon::File, "保存").size(size))
            .unwrap();
        let compact = cx
            .create_detached_component(
                doc,
                IconButton::new(Icon::File, "工具").size(size).compact(true),
            )
            .unwrap();
        let glyph = cx
            .create_detached_component(doc, IconGlyph::new(Icon::File))
            .unwrap();
        let literal = cx
            .create_detached_component(doc, IconGlyph::new(Icon::File).size(20.0))
            .unwrap();
        cx.append_child(root, plain).unwrap();
        cx.append_child(root, button).unwrap();
        cx.append_child(root, overridden).unwrap();
        cx.append_child(root, action).unwrap();
        cx.append_child(root, compact).unwrap();
        cx.append_child(root, glyph).unwrap();
        cx.append_child(root, literal).unwrap();
        let children = cx.world().node(root.stable_id()).unwrap().children;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        cx.on(action, move |_, _: &Activate, _| {
            observed.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let mut shaper = NanaTextShaper::default();
        settle(&mut cx, doc, &mut shaper);
        cx.focus_node(doc, action.stable_id()).unwrap();
        let mut input = HeadlessInput::bind(&mut cx, doc);
        let mut scene = UiScene::new();
        for large in [false, true, false] {
            let mut theme = ThemeDefinition::NANA_LIGHT;
            if large {
                theme.typography.meta = 18.0;
                theme.typography.body = 20.0;
                theme.typography.section = 22.0;
                theme.typography.line = 30.0;
                theme.typography.line_tall = 32.0;
                theme.tokens.spacing.sm = 10.0;
                theme.tokens.spacing.page = 40.0;
                theme.tokens.metrics.compact_control_height = 40.0;
                theme.tokens.metrics.control_height = 48.0;
            }
            cx.set_theme_definition(&theme).unwrap();
            settle(&mut cx, doc, &mut shaper);
            scene.apply_delta(cx.world().extract_document(doc), []);
            let typography = cx.world().theme().typography();
            let expected = size.text_size_in(typography);
            let plain_box = cx.world().layout_box(plain.stable_id()).unwrap();
            let button_box = cx.world().layout_box(button.stable_id()).unwrap();
            assert!(
                (button_box.width - plain_box.width - 2.0 * (expected + theme.tokens.spacing.sm))
                    .abs()
                    < 0.5
            );
            assert_eq!(painted_sizes(&scene, button.stable_id()), vec![expected; 2]);
            let ComponentGeometry::Button {
                icon: Some((_, lead)),
                label,
                ..
            } = cx.world().component_geometry(button.stable_id()).unwrap()
            else {
                panic!("button geometry")
            };
            assert!((label.bounds.x - lead.x - lead.width - theme.tokens.spacing.sm).abs() < 0.5);
            assert_eq!(label.font_size, expected);
            assert!(
                (cx.world().layout_box(overridden.stable_id()).unwrap().width
                    - plain_box.width
                    - 15.0)
                    .abs()
                    < 0.5
            );
            assert_eq!(painted_sizes(&scene, overridden.stable_id()), vec![9.0]);
            assert_eq!(painted_sizes(&scene, action.stable_id()), vec![expected]);
            assert_eq!(
                cx.world().layout_box(action.stable_id()).unwrap().width,
                size.height_in(theme.tokens.metrics)
            );
            assert_eq!(
                cx.world().layout_box(compact.stable_id()).unwrap().width,
                theme.tokens.spacing.page
            );
            assert_eq!(
                painted_sizes(&scene, compact.stable_id()),
                vec![typography.body]
            );
            assert_eq!(
                cx.world().layout_box(glyph.stable_id()).unwrap().width,
                typography.meta
            );
            assert_eq!(
                painted_sizes(&scene, glyph.stable_id()),
                vec![typography.meta]
            );
            assert_eq!(
                cx.world().layout_box(literal.stable_id()).unwrap().width,
                20.0
            );
            assert_eq!(painted_sizes(&scene, literal.stable_id()), vec![20.0]);
            assert_eq!(cx.world().focused(doc), Some(action.stable_id()));
            assert_eq!(
                cx.world().node(root.stable_id()).unwrap().children,
                children
            );
            input
                .route(
                    &mut cx,
                    InputPayload::Key(KeyInput::named(
                        "Enter",
                        "Enter",
                        KeyState::Pressed,
                        InputModifiers::default(),
                    )),
                )
                .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}

#[test]
fn glyph_role_and_expression_keep_painted_and_layout_extents_identical() {
    use nana_ui_core::{SpacingStep, ThemeExpr, ThemeToken, TypeRole};
    let mut cx = AppContext::new();
    let doc = DocumentId::new(931).unwrap();
    let root = cx
        .create_component(doc, Stack::column(8.0).align(AlignSpec::Start))
        .unwrap();
    let role = cx
        .create_detached_component(doc, IconGlyph::new(Icon::File).size_role(TypeRole::Display))
        .unwrap();
    let expression = ThemeExpr::token(ThemeToken::Type(TypeRole::Title))
        .scaled(1.25)
        .plus(ThemeToken::Space(SpacingStep::Xs).into());
    let derived = cx
        .create_detached_component(
            doc,
            IconGlyph::new(Icon::File).size_theme(expression.clone()),
        )
        .unwrap();
    let literal = cx
        .create_detached_component(
            doc,
            IconGlyph::new(Icon::File)
                .size_theme(expression.clone())
                .size(17.0),
        )
        .unwrap();
    cx.append_child(root, role).unwrap();
    cx.append_child(root, derived).unwrap();
    cx.append_child(root, literal).unwrap();
    let mut shaper = NanaTextShaper::default();
    let mut scene = UiScene::new();
    for large in [false, true, false] {
        let mut theme = ThemeDefinition::NANA_LIGHT;
        if large {
            theme.typography.display = 32.0;
            theme.typography.title = 28.0;
            theme.tokens.spacing.xs = 9.0;
        }
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        scene.apply_delta(cx.world().extract_document(doc), []);
        for (glyph, expected) in [
            (role, theme.typography.display),
            (derived, expression.resolve(cx.world().theme())),
            (literal, 17.0),
        ] {
            let bounds = cx.world().layout_box(glyph.stable_id()).unwrap();
            assert_eq!([bounds.width, bounds.height], [expected; 2]);
            assert_eq!(painted_sizes(&scene, glyph.stable_id()), vec![expected]);
        }
    }
}
