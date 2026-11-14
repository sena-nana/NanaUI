//! Retained navigation chrome resolves its layout and painted glyphs from one live theme.
use nana_ui::{HeadlessInput, NanaTextShaper, runtime::*};
use nana_ui_core::{
    ControlSize, Icon, LineHeightSpec, SettingsModel, SettingsState, SettingsTab, SettingsTabId,
    ThemeDefinition,
};
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

fn theme(large: bool) -> ThemeDefinition {
    let mut theme = ThemeDefinition::NANA_LIGHT;
    if large {
        theme.typography.meta = 19.0;
        theme.typography.body = 21.0;
        theme.typography.section = 23.0;
        theme.typography.line = 30.0;
        theme.typography.line_tall = 32.0;
        theme.tokens.metrics.compact_control_height = 44.0;
        theme.tokens.metrics.control_height = 48.0;
        theme.tokens.border.hairline = 2.0;
    }
    theme
}

fn painted_icon(scene: &UiScene, id: StableNodeId, expected: f32) {
    let icons: Vec<_> = scene
        .primitives()
        .filter(|p| p.node == id && matches!(p.kind, ScenePrimitiveKind::Icon { .. }))
        .collect();
    assert_eq!(icons.len(), 1, "one visible glyph for {id:?}");
    assert_eq!(icons[0].bounds.width, expected);
    assert_eq!(icons[0].bounds.height, expected);
}

fn press_enter(input: &mut HeadlessInput, cx: &mut AppContext) {
    input
        .route(
            cx,
            InputPayload::Key(KeyInput::named(
                "Enter",
                "Enter",
                KeyState::Pressed,
                InputModifiers::default(),
            )),
        )
        .unwrap();
}

#[test]
fn mounted_shell_and_menu_glyphs_keep_geometry_focus_and_actions_through_theme_changes() {
    for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(934).unwrap();
        let root = cx
            .create_component(doc, Stack::column(8.0).align(AlignSpec::Start))
            .unwrap();
        let leading = cx
            .create_detached_component(doc, SidebarRowIcon::new(Icon::File))
            .unwrap();
        let footer = cx
            .create_detached_component(
                doc,
                SidebarFooterButton::new("设置", Icon::Settings).size(size),
            )
            .unwrap();
        let section = cx
            .create_detached_component(doc, SidebarSection::new("收藏").collapsible(true))
            .unwrap();
        let menu = cx
            .create_detached_component(
                doc,
                ActionMenuItem::new("打开").leading(Icon::File).size(size),
            )
            .unwrap();
        cx.append_child(root, leading).unwrap();
        cx.append_child(root, footer).unwrap();
        cx.append_child(root, section).unwrap();
        cx.append_child(root, menu).unwrap();
        cx.assemble_sidebar_section(section).unwrap();
        let disclosure = cx.read(section, |s| s.disclosure).unwrap().unwrap();
        let children = cx.world().node(root.stable_id()).unwrap().children;
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        cx.on(footer, move |_, _: &Activate, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        let mut shaper = NanaTextShaper::default();
        settle(&mut cx, doc, &mut shaper);
        cx.focus_node(doc, footer.stable_id()).unwrap();
        let mut input = HeadlessInput::bind(&mut cx, doc);
        let mut scene = UiScene::new();
        for large in [false, true, false] {
            let theme = theme(large);
            cx.set_theme_definition(&theme).unwrap();
            settle(&mut cx, doc, &mut shaper);
            scene.apply_delta(cx.world().extract_document(doc), []);
            for id in [leading.stable_id(), disclosure] {
                let bounds = cx.world().layout_box(id).unwrap();
                assert_eq!(
                    (bounds.width, bounds.height),
                    (theme.typography.meta, theme.typography.meta)
                );
                painted_icon(&scene, id, theme.typography.meta);
            }
            let font = size.text_size_in(cx.world().theme().typography());
            painted_icon(&scene, footer.stable_id(), font);
            painted_icon(&scene, menu.stable_id(), font);
            let ComponentGeometry::ActionMenuItem {
                icon: Some((_, icon, _)),
                label,
                ..
            } = cx.world().component_geometry(menu.stable_id()).unwrap()
            else {
                panic!("menu icon geometry");
            };
            assert_eq!(label.font_size, font);
            assert!((label.bounds.x - icon.x - icon.width - theme.tokens.spacing.md).abs() < 0.5);
            assert_eq!(cx.world().focused(doc), Some(footer.stable_id()));
            assert_eq!(
                cx.world().node(root.stable_id()).unwrap().children,
                children
            );
            assert!(cx.read(section, |s| s.state.expanded()).unwrap());
            press_enter(&mut input, &mut cx);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}

#[test]
fn mounted_settings_slots_follow_meta_line_and_hairline_without_replacing_navigation() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(935).unwrap();
    let root = cx
        .create_component(doc, Stack::column(8.0).align(AlignSpec::Start))
        .unwrap();
    let model = SettingsModel::new(
        "appearance",
        [SettingsTab::new("appearance", "外观").icon(Icon::Settings)],
    )
    .unwrap();
    let sidebar = cx
        .create_detached_component(
            doc,
            SettingsSidebar::new(model.clone(), SettingsState::new(&model)),
        )
        .unwrap();
    // The shell owns the sidebar's extent, independent of the settings content.
    let mut sidebar_host = NodeStyle::default();
    Arc::make_mut(&mut sidebar_host.layout).height = Some(LengthSpec::Px(180.0));
    let host = cx
        .create_detached_component(doc, Stack::column(0.0).style(sidebar_host))
        .unwrap();
    cx.append_child(root, host).unwrap();
    cx.append_child(host, sidebar).unwrap();
    cx.assemble_settings_sidebar(sidebar).unwrap();
    let assembly = cx.read(sidebar, |s| s.assembly.clone()).unwrap().unwrap();
    let tab = SettingsTabId::from("appearance");
    let nav_row = assembly.tab_rows[&tab];
    let leading = assembly.tab_leadings[&tab];
    let row = cx
        .create_detached_component(
            doc,
            SettingsRow::new("播放").hint("主题提示行").divided(true),
        )
        .unwrap();
    cx.append_child(root, row).unwrap();
    cx.assemble_settings_row(row).unwrap();
    let hint = cx.read(row, |r| r.hint_slot).unwrap().unwrap();
    let summary = cx
        .create_detached_component(doc, Text::new("高级选项"))
        .unwrap();
    let details = cx
        .create_detached_component(doc, Text::new("保留用户状态"))
        .unwrap();
    let card = cx
        .create_detached_component(
            doc,
            SettingsCollapsibleCard::new(true)
                .summary(summary.stable_id())
                .details(details.stable_id()),
        )
        .unwrap();
    cx.append_child(root, card).unwrap();
    cx.assemble_settings_collapsible_card(card).unwrap();
    let card_children = cx.world().node(card.stable_id()).unwrap().children;
    let divider = card_children[1];
    let disclosure = *cx
        .world()
        .node(card_children[0])
        .unwrap()
        .children
        .last()
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    cx.on(sidebar, move |_, _: &SettingsTabSelected, _| {
        seen.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    cx.focus_node(doc, nav_row).unwrap();
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let mut scene = UiScene::new();
    for large in [false, true, false] {
        let theme = theme(large);
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        scene.apply_delta(cx.world().extract_document(doc), []);
        painted_icon(&scene, leading, theme.typography.meta);
        let bounds = cx.world().layout_box(leading).unwrap();
        assert_eq!(
            (bounds.width, bounds.height),
            (theme.typography.meta, theme.typography.meta)
        );
        painted_icon(&scene, disclosure, theme.typography.line);
        assert_eq!(
            cx.world().layout_box(disclosure).unwrap().height,
            theme.tokens.metrics.compact_control_height
        );
        assert_eq!(
            cx.world().layout_box(divider).unwrap().height,
            theme.tokens.border.hairline
        );
        assert_eq!(
            cx.world().computed_style(hint).unwrap().line_height,
            Some(LineHeightSpec::Absolute(theme.typography.line))
        );
        let layout = cx.world().layout_result(row.stable_id()).unwrap();
        assert!(
            (layout.border_box.height + layout.border_box.y
                - layout.padding_box.height
                - layout.padding_box.y
                - theme.tokens.border.hairline)
                .abs()
                < 0.01
        );
        assert_eq!(
            cx.world().node(card.stable_id()).unwrap().children,
            card_children
        );
        assert!(cx.read(card, |c| c.expanded).unwrap());
        assert_eq!(cx.world().focused(doc), Some(nav_row));
        press_enter(&mut input, &mut cx);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
