#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{MessageBridge, WidgetProps};
    use crate::css_map::{
        FlexDirection, FlexWrap, JustifySpec, LayoutStyle, LayoutStyleCss, LengthSpec,
        OverflowSpec, ParentBox,
    };
    use nana_ui::ThemeModeExt;
    use nana_ui_core::{ButtonKind, ThemeMode, UI_BASE_TEXT_SIZE};

    #[derive(Default)]
    struct BoundsOperation {
        containers: Vec<Rectangle>,
    }

    impl widget::Operation for BoundsOperation {
        fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn widget::Operation)) {}

        fn container(&mut self, _id: Option<&widget::Id>, bounds: Rectangle) {
            self.containers.push(bounds);
        }
    }

    #[test]
    fn affine_operation_reports_transformed_accessibility_bounds() {
        let mut recorded = BoundsOperation::default();
        let mut operation = AffineOperation {
            inner: &mut recorded,
            // Rotate 90 degrees then translate x by 100.
            matrix: [0.0, 1.0, -1.0, 0.0, 100.0, 0.0],
        };
        widget::Operation::container(
            &mut operation,
            None,
            Rectangle::new(Point::new(10.0, 20.0), Size::new(30.0, 40.0)),
        );

        assert_eq!(
            recorded.containers,
            [Rectangle::new(Point::new(40.0, 10.0), Size::new(40.0, 30.0))]
        );
    }

    #[test]
    fn writeback_iced_layout_boxes_feeds_document_for_menu_anchors() {
        let store = LayoutBoxStore::new();
        let mut doc = crate::tree::NanaTreeDocument::new(400, 300, 1.0);
        let btn = doc.create_element("button");
        doc.insert(btn, doc.mount_root(), None);
        // Simulate LayoutProbe draw after scrollable.padding(16) chrome.
        store.record(btn, 16.0, 48.0, 96.0, 28.0);
        // Stale measure-style box at origin — must be replaced.
        doc.apply_layout_boxes(&[(
            btn,
            crate::LayoutBox {
                handle: btn,
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
            },
        )]);
        doc.apply_layout_boxes(&store.snapshot());
        let box_ = doc.layout_box(btn).expect("iced writeback");
        assert_eq!((box_.x, box_.y), (16.0, 48.0));
        assert_eq!((box_.width, box_.height), (96.0, 28.0));
    }

    #[test]
    fn view_semantic_tree_installs_layout_probes_without_panic() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.label = "Sort".into();
        bridge.register(1, WidgetKind::Button, props);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((320.0, 200.0)), |e| e);
    }

    #[test]
    fn qualified_runtime_scene_route_does_not_panic_without_bounds() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::Button,
            WidgetProps {
                label: "Run".into(),
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        let scene = UiScene::new();
        let tokens = ThemeMode::Light.tokens();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            tokens,
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    #[test]
    fn status_badge_and_validation_with_empty_scene_do_not_panic() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::StatusBadge,
            WidgetProps {
                label: "Live".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::ValidationMessage,
            WidgetProps {
                hint: "A project is required".into(),
                invalid: true,
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        let scene = UiScene::new();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    #[test]
    fn empty_state_with_button_child_is_not_swallowed_by_scene() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::EmptyState,
            WidgetProps {
                label: "No projects".into(),
                hint: "Create the first project".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::Button,
            WidgetProps {
                label: "Create".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        let scene = UiScene::new();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    #[test]
    fn empty_state_without_button_and_placeholder_leaves_do_not_panic() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::EmptyState,
            WidgetProps {
                label: "No projects".into(),
                hint: "Create the first project".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(2, WidgetKind::Skeleton, WidgetProps::default());
        bridge.register(
            3,
            WidgetKind::LevelMeter,
            WidgetProps {
                progress: 0.4,
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        let scene = UiScene::new();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    fn select_option(value: &str, label: &str) -> crate::bridge::SelectOptionProp {
        crate::bridge::SelectOptionProp {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    fn assert_scene_route(snap: &SemanticSnapshot, id: WidgetId, scene: &UiScene) {
        let widget = snap.get(id).expect("widget");
        with_active_scene(Some(scene), || {
            assert!(
                matches!(
                    qualified_runtime_scene_view::<BridgeEvent>(snap, widget),
                    QualifiedSceneRoute::Scene(_)
                ),
                "widget {id} must paint through Scene when bounds exist"
            );
        });
    }

    fn paint_overlay_scene(
        snap: &SemanticSnapshot,
        scene: &UiScene,
        viewport: (f32, f32),
    ) -> Element<'static, BridgeEvent> {
        view_semantic_tree_static_with_scene(
            snap,
            ThemeMode::Light.tokens(),
            Some(viewport),
            None,
            None,
            None,
            None,
            None,
            Some(scene),
            None,
            |event| event,
        )
    }

    #[test]
    fn dialog_with_button_child_routes_to_scene_without_panic() {
        let mut document = crate::tree::NanaTreeDocument::new(400, 300, 1.0);
        let dialog = document.create_element("nana-dialog");
        let button = document.create_element("nana-button");
        document.insert(dialog, document.mount_root(), None);
        document.insert(button, dialog, None);

        let mut bridge = MessageBridge::new();
        bridge.register(
            dialog.0,
            WidgetKind::Dialog,
            WidgetProps {
                label: "Rename".into(),
                hint: "Choose a name".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            button.0,
            WidgetKind::Button,
            WidgetProps {
                label: "Save".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(button.0, dialog.0, None);
        let snap = bridge.snapshot();
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(dialog.0).unwrap()),
            Some(nana_ui::component_ids::DIALOG)
        );

        document.sync_semantic_styles(&snap);
        document.apply_layout_boxes(&[(
            dialog,
            crate::LayoutBox {
                handle: dialog,
                x: 40.0,
                y: 32.0,
                width: 280.0,
                height: 180.0,
            },
        )]);
        assert!(
            document
                .scene()
                .node_bounds(nana_ui_runtime::StableNodeId::new(dialog.0).unwrap())
                .is_some()
        );
        assert_scene_route(&snap, dialog.0, document.scene());
        let _: Element<'static, BridgeEvent> =
            paint_overlay_scene(&snap, document.scene(), (400.0, 300.0));
    }

    #[test]
    fn drawer_and_popover_route_to_scene_when_qualified() {
        let mut document = crate::tree::NanaTreeDocument::new(480, 320, 1.0);
        let drawer = document.create_element("nana-drawer");
        let popover = document.create_element("nana-popover");
        document.insert(drawer, document.mount_root(), None);
        document.insert(popover, document.mount_root(), None);

        let mut bridge = MessageBridge::new();
        bridge.register(
            drawer.0,
            WidgetKind::Drawer,
            WidgetProps {
                label: "Inspector".into(),
                side: "left".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            popover.0,
            WidgetKind::Popover,
            WidgetProps {
                label: "More".into(),
                hint: "Details".into(),
                toggled: true,
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(drawer.0).unwrap()),
            Some(nana_ui::component_ids::DRAWER)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(popover.0).unwrap()),
            Some(nana_ui::component_ids::POPOVER)
        );

        document.sync_semantic_styles(&snap);
        document.apply_layout_boxes(&[
            (
                drawer,
                crate::LayoutBox {
                    handle: drawer,
                    x: 0.0,
                    y: 0.0,
                    width: 280.0,
                    height: 320.0,
                },
            ),
            (
                popover,
                crate::LayoutBox {
                    handle: popover,
                    x: 120.0,
                    y: 48.0,
                    width: 200.0,
                    height: 120.0,
                },
            ),
        ]);
        assert_scene_route(&snap, drawer.0, document.scene());
        assert_scene_route(&snap, popover.0, document.scene());
        let _: Element<'static, BridgeEvent> =
            paint_overlay_scene(&snap, document.scene(), (480.0, 320.0));
    }

    #[test]
    fn searchable_context_menu_scene_routes_through_runtime() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options: (0..6)
                    .map(|i| select_option(&format!("item-{i}"), &format!("Item {i}")))
                    .collect(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                class_names: vec!["search".into()],
                options: vec![select_option("cut", "Cut")],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options: vec![
                    select_option("file", "File"),
                    select_option("file/rename", "Rename"),
                ],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options: vec![select_option("cut", "Cut"), select_option("copy", "Copy")],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                class_names: vec!["nana-action-menu".into()],
                options: vec![select_option("rename", "Rename")],
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(1).unwrap()),
            Some(nana_ui::component_ids::CONTEXT_MENU),
            "6+ options keep the Runtime search field"
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(2).unwrap()),
            Some(nana_ui::component_ids::CONTEXT_MENU),
            "search class keeps the Runtime search field"
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(3).unwrap()),
            Some(nana_ui::component_ids::CONTEXT_MENU),
            "nested parent/child options Scene-route through Runtime"
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(4).unwrap()),
            Some(nana_ui::component_ids::CONTEXT_MENU)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(5).unwrap()),
            Some(nana_ui::component_ids::ACTION_MENU)
        );

        let scene = UiScene::new();
        with_active_scene(Some(&scene), || {
            assert!(
                !matches!(
                    qualified_runtime_scene_view::<BridgeEvent>(&snap, snap.get(1).unwrap()),
                    QualifiedSceneRoute::Compatibility
                ),
                "searchable menus Scene-route through Runtime"
            );
            assert!(
                !matches!(
                    qualified_runtime_scene_view::<BridgeEvent>(&snap, snap.get(2).unwrap()),
                    QualifiedSceneRoute::Compatibility
                ),
                "search class menus Scene-route through Runtime"
            );
            assert!(
                !matches!(
                    qualified_runtime_scene_view::<BridgeEvent>(&snap, snap.get(3).unwrap()),
                    QualifiedSceneRoute::Compatibility
                ),
                "nested parent/child options must not stay on ContextMenuHost"
            );
        });
        let _: Element<'static, BridgeEvent> = paint_overlay_scene(&snap, &scene, (320.0, 200.0));
    }

    #[test]
    fn dropdown_search_tabs_and_hosts_scene_route() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::Select,
            WidgetProps {
                element_tag: "nana-dropdown".into(),
                class_names: vec!["nana-dropdown".into()],
                options: vec![select_option("code", "Code")],
                value: "code".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::Select,
            WidgetProps {
                element_tag: "nana-search".into(),
                class_names: vec!["nana-search".into()],
                options: vec![select_option("alpha", "Alpha")],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Tabs,
            WidgetProps {
                options: vec![select_option("one", "One"), select_option("two", "Two")],
                value: "one".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::FormField,
            WidgetProps {
                label: "Email".into(),
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(1).unwrap()),
            Some(nana_ui::component_ids::DROPDOWN)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(2).unwrap()),
            Some(nana_ui::component_ids::SEARCH_DROPDOWN)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(3).unwrap()),
            Some(nana_ui::component_ids::TABS)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(4).unwrap()),
            Some(nana_ui::component_ids::FORM_FIELD)
        );
    }

    #[test]
    fn sidebar_and_settings_scene_route() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::SidebarRow,
            WidgetProps {
                label: "工作区".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::SettingsRow,
            WidgetProps {
                label: "主题".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::SettingsCard,
            WidgetProps {
                label: "外观".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(4, WidgetKind::SidebarFrame, WidgetProps::default());
        let snap = bridge.snapshot();
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(1).unwrap()),
            Some(nana_ui::component_ids::SIDEBAR_ROW)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(2).unwrap()),
            Some(nana_ui::component_ids::SETTINGS)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(3).unwrap()),
            Some(nana_ui::component_ids::SETTINGS)
        );
        assert_eq!(
            runtime_component_for_widget(&snap, snap.get(4).unwrap()),
            None
        );
    }

    #[test]
    fn form_field_and_interactive_card_hosts_stay_on_composer() {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            WidgetKind::FormField,
            WidgetProps {
                label: "Email".into(),
                hint: "Required".into(),
                invalid: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            2,
            WidgetKind::Input,
            WidgetProps {
                value: "a@b.c".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.register(
            3,
            WidgetKind::InteractiveCard,
            WidgetProps {
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Text,
            WidgetProps {
                label: "Surface".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(4, 3, None);
        let snap = bridge.snapshot();
        let scene = UiScene::new();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    #[test]
    fn scene_does_not_swallow_card_or_html_text_hosts() {
        let mut bridge = MessageBridge::new();
        let mut card = WidgetProps::default();
        card.label = "Panel".into();
        bridge.register(1, WidgetKind::Card, card);
        let mut heading = WidgetProps::default();
        heading.label = String::new();
        bridge.register(2, WidgetKind::Text, heading);
        let mut title = WidgetProps::default();
        title.label = "Heading".into();
        bridge.register(3, WidgetKind::Text, title);
        let mut area = WidgetProps::default();
        area.value = "notes".into();
        bridge.register(4, WidgetKind::Textarea, area);
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 1, None);
        let snap = bridge.snapshot();
        let scene = UiScene::new();

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(&scene),
            None,
            |event| event,
        );
    }

    #[test]
    fn owned_hosted_tree_builds_qualified_component_from_runtime_scene() {
        let mut document = crate::tree::NanaTreeDocument::new(320, 200, 1.0);
        let button = document.create_element("nana-button");
        document.insert(button, document.mount_root(), None);

        let mut bridge = MessageBridge::new();
        bridge.register(
            button.0,
            WidgetKind::Button,
            WidgetProps {
                label: "Run".into(),
                button_kind: ButtonKind::Primary,
                ..WidgetProps::default()
            },
        );
        let snap = bridge.snapshot();
        document.sync_semantic_styles(&snap);
        document.apply_layout_boxes(&[(
            button,
            crate::LayoutBox {
                handle: button,
                x: 12.0,
                y: 12.0,
                width: 96.0,
                height: 32.0,
            },
        )]);
        assert!(
            document
                .scene()
                .node_bounds(nana_ui_runtime::StableNodeId::new(button.0).unwrap())
                .is_some()
        );

        let _: Element<'static, BridgeEvent> = view_semantic_tree_static_with_scene(
            &snap,
            ThemeMode::Light.tokens(),
            Some((320.0, 200.0)),
            None,
            None,
            None,
            None,
            None,
            Some(document.scene()),
            None,
            |event| event,
        );
    }

    #[test]
    fn missing_nodes_are_measured_without_replacing_iced_boxes() {
        let mut document = crate::tree::NanaTreeDocument::new(320, 200, 1.0);
        let painted = document.create_element("nana-button");
        let fresh = document.create_element("nana-button");
        document.insert(painted, document.mount_root(), None);
        document.insert(fresh, document.mount_root(), None);
        document.apply_layout_boxes(&[(
            painted,
            crate::LayoutBox {
                handle: painted,
                x: 8.0,
                y: 8.0,
                width: 40.0,
                height: 28.0,
            },
        )]);

        let mut bridge = MessageBridge::new();
        bridge.register(
            painted.0,
            WidgetKind::Button,
            WidgetProps {
                label: "Painted".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            fresh.0,
            WidgetKind::Button,
            WidgetProps {
                label: "Fresh".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(fresh.0, painted.0, None);
        bridge.resolve_missing_document_layout(&mut document);

        let painted_box = document.layout_box(painted).expect("iced box stays");
        assert_eq!(
            (painted_box.x, painted_box.y, painted_box.width, painted_box.height),
            (8.0, 8.0, 40.0, 28.0)
        );
        assert!(document.layout_box(fresh).is_some());
    }

    #[test]
    fn definite_scroll_extent_uses_parent_or_viewport() {
        assert_eq!(
            definite_scroll_extent(Some(640.0), 800.0),
            Length::Fixed(640.0)
        );
        // Former `> 1.0` gate treated 1px as invalid → Fill collapse.
        assert_eq!(
            definite_scroll_extent(Some(1.0), 800.0),
            Length::Fixed(1.0)
        );
        assert_eq!(
            definite_scroll_extent(Some(0.0), 800.0),
            Length::Fixed(800.0),
            "non-positive parent falls back to viewport"
        );
        assert_eq!(definite_scroll_extent(None, 720.0), Length::Fixed(720.0));
        assert_eq!(definite_scroll_extent(None, 0.0), Length::Fill);
    }

    #[test]
    fn square_icon_button_resolves_lucide_child_not_empty_label() {
        let mut bridge = MessageBridge::new();
        let mut btn = WidgetProps::default();
        btn.class_names = vec!["anon-toolbar-btn".into()];
        btn.hint = "搜索".into();
        btn.layout.width = Some(LengthSpec::Px(32.0));
        btn.layout.height = Some(LengthSpec::Px(32.0));
        bridge.register(1, WidgetKind::Button, btn);

        let mut glyph = WidgetProps::default();
        glyph.class_names = vec!["lucide".into(), "lucide-search".into()];
        glyph.value = "search".into();
        bridge.register(2, WidgetKind::Icon, glyph);
        bridge.insert_child(2, 1, None);

        let snap = bridge.snapshot();
        let props = &snap.get(1).unwrap().props;
        let children = &snap.get(1).unwrap().children;
        let (icon, label) = resolve_button_icon_and_label(&snap, props, children);
        assert!(
            matches!(icon, Some(ResolvedButtonIcon::Glyph(Icon::Search))),
            "search without path geometry falls back to shell glyph, got {icon:?}"
        );
        assert!(label.is_empty() || label == "搜索");
        assert!(is_square_icon_button(props));
    }

    #[test]
    fn square_svg_button_kind_follows_active_like_icon_button() {
        let mut bridge = MessageBridge::new();
        let mut btn = WidgetProps::default();
        btn.class_names = vec!["anon-toolbar-btn".into()];
        btn.hint = "筛选".into();
        btn.layout.width = Some(LengthSpec::Px(32.0));
        btn.layout.height = Some(LengthSpec::Px(32.0));
        btn.active = true;
        bridge.register(1, WidgetKind::Button, btn);

        let mut glyph = WidgetProps::default();
        glyph.class_names = vec!["lucide".into(), "lucide-filter".into()];
        glyph.value = "filter".into();
        bridge.register(2, WidgetKind::Icon, glyph);
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.value = "M22 3H2l8 9.46V19l4 2v-8.54L22 3z".into();
        bridge.register(3, WidgetKind::Box, path);
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);

        let snap = bridge.snapshot();
        let props = &snap.get(1).unwrap().props;
        let children = &snap.get(1).unwrap().children;
        let (icon, label) = resolve_button_icon_and_label(&snap, props, children);
        assert!(
            matches!(icon, Some(ResolvedButtonIcon::Svg(_))),
            "lucide with path child should resolve SVG, got {icon:?}"
        );
        assert!(label.is_empty() || label == "筛选");
        assert!(is_square_icon_button(props));
        assert_eq!(
            resolved_button_kind(props),
            ButtonKind::Selected,
            "active square SVG toolbar buttons must use Selected (Button has no .selected())"
        );

        let mut inactive = props.clone();
        inactive.active = false;
        assert_eq!(resolved_button_kind(&inactive), ButtonKind::Ghost);
    }

    #[test]
    fn primary_button_keeps_sync_caption_from_text_child() {
        let mut bridge = MessageBridge::new();
        let mut btn = WidgetProps::default();
        btn.class_names = vec!["anon-primary-btn".into()];
        btn.button_kind = ButtonKind::Primary;
        btn.label = "一键同步".into();
        btn.layout.width = Some(LengthSpec::Auto);
        btn.layout.height = Some(LengthSpec::Px(32.0));
        btn.layout.min_width = Some(LengthSpec::Px(72.0));
        bridge.register(1, WidgetKind::Button, btn);
        let mut glyph = WidgetProps::default();
        glyph.class_names = vec!["lucide".into(), "lucide-git-pull-request-arrow".into()];
        glyph.value = "git-pull-request-arrow".into();
        bridge.register(2, WidgetKind::Icon, glyph);
        // Path child must not leak into the caption.
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.value = "M5 9v12 m15 9-3-3 3-3".into();
        bridge.register(4, WidgetKind::Box, path);
        let mut text = WidgetProps::default();
        text.label = "同步".into();
        bridge.register(3, WidgetKind::Text, text);
        bridge.insert_child(2, 1, None);
        bridge.insert_child(4, 2, None);
        bridge.insert_child(3, 1, None);

        let snap = bridge.snapshot();
        let props = &snap.get(1).unwrap().props;
        let children = &snap.get(1).unwrap().children;
        let (icon, label) = resolve_button_icon_and_label(&snap, props, children);
        assert!(
            matches!(icon, Some(ResolvedButtonIcon::Svg(_))),
            "lucide with path child should resolve SVG, got {icon:?}"
        );
        assert_eq!(label, "同步");
        assert!(!is_square_icon_button(props));
    }

    #[test]
    fn button_layout_chrome_consumes_padding_radius_and_color() {
        let mut layout = crate::css_map::LayoutStyle::default();
        layout.apply_css_text(
            "width:32px;height:32px;padding:0;border:0;border-radius:12px;\
             background:transparent;color:#6b7280",
            None,
            None,
        );
        assert!(layout_has_explicit_padding(&layout));
        assert_eq!(layout.border_radius, Some(12.0));
        // `transparent` / `none` clear background (same as SVG fill:none) — not α=0 paint.
        assert!(layout.background.is_none());
        assert!(layout.color.is_some());

        let paint = button_paint_from_layout(&layout);
        assert!(!paint.is_empty());
        assert_eq!(paint.border_radius, Some(12.0));
        assert!(paint.background.is_none());
        assert!(paint.text_color.is_some());

        let pad = button_padding_from_layout(&layout, None).expect("explicit padding");
        assert_eq!(pad.top, 0.0);
        assert_eq!(pad.left, 0.0);

        let consume = button_box_consume(&WidgetKind::Button, &layout);
        assert!(consume.padding);
        assert!(consume.paint);
    }

    #[test]
    fn primary_button_layout_chrome_keeps_horizontal_padding_and_min_width() {
        let mut layout = crate::css_map::LayoutStyle::default();
        layout.apply_css_text(
            "width:auto;min-width:72px;height:32px;padding:0 12px;border-radius:12px;\
             background:#dbeafe;color:#1d4ed8;gap:6px;font-weight:700",
            None,
            None,
        );
        assert_eq!(layout.min_width, Some(LengthSpec::Px(72.0)));
        assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert_eq!(layout.border_radius, Some(12.0));
        assert_eq!(layout.font_weight, Some(700));
        assert_eq!(layout.gap_or(0.0), 6.0);

        let pad = button_padding_from_layout(&layout, None).unwrap();
        assert_eq!(pad.left, 12.0);
        assert_eq!(pad.right, 12.0);
        assert_eq!(pad.top, 0.0);

        let consume = button_box_consume(&WidgetKind::Button, &layout);
        assert!(consume.padding, "outer must not re-apply CSS padding");
        assert!(consume.paint, "outer must not re-paint CSS surface");

        // Non-button widgets still get outer pad/paint.
        let other = button_box_consume(&WidgetKind::Text, &layout);
        assert!(!other.padding);
        assert!(!other.paint);
    }

    #[test]
    fn button_without_css_padding_keeps_control_size_defaults() {
        let layout = crate::css_map::LayoutStyle::default();
        assert!(button_padding_from_layout(&layout, None).is_none());
        let consume = button_box_consume(&WidgetKind::Button, &layout);
        assert!(!consume.padding);
        assert!(!consume.paint);
        assert!(button_paint_from_layout(&layout).is_empty());
    }

    #[test]
    fn content_box_outer_fixed_includes_pad_and_border_even_when_consumed() {
        // Bugbot: consume.padding/paint must not shrink content-box Fixed.
        let mut layout = crate::css_map::LayoutStyle::default();
        layout.apply_css_text(
            "width:100px;height:40px;padding:10px;border-width:5px;\
             background:#fff;box-sizing:content-box",
            None,
            None,
        );
        let consume = button_box_consume(&WidgetKind::Button, &layout);
        assert!(consume.padding, "button consumes CSS padding draw");
        assert!(consume.paint, "button consumes surface paint draw");

        let (w, h) = content_box_outer_axes(&layout, None);
        // content 100×40 + pad 10×2 + border 5×2 → border-box 130×70
        assert_eq!(w, Some(130.0), "outer Fixed width must include pad+border");
        assert_eq!(h, Some(70.0), "outer Fixed height must include pad+border");
        // Same chrome as measure's content-box expansion.
        assert_eq!(
            content_box_outer_fixed(100.0, 10.0, 10.0, 5.0),
            130.0
        );
        assert_eq!(content_box_outer_fixed(40.0, 10.0, 10.0, 5.0), 70.0);
    }

    #[test]
    fn content_box_outer_expands_em_rem_fixed_like_measure() {
        // Bugbot: length_from_spec already folds Em/Rem → Fixed; content-box
        // must still add pad+border (same as measure's is_definite_declared).
        let mut layout = crate::css_map::LayoutStyle::default();
        layout.apply_css_text(
            "width:10em;height:2.5rem;padding:10px;box-sizing:content-box",
            None,
            None,
        );
        assert!(
            layout.width.is_some_and(LengthSpec::is_definite_declared),
            "10em is definite declared"
        );
        assert!(
            layout.height.is_some_and(LengthSpec::is_definite_declared),
            "2.5rem is definite declared"
        );
        // Default CSS medium = 16px → content 160×40; Fixed before chrome.
        match length_from_spec(layout.width, None, &layout, false) {
            Length::Fixed(px) => assert!(
                (px - 160.0).abs() < 0.01,
                "10em → Fixed(160), got {px}"
            ),
            other => panic!("expected Fixed from 10em, got {other:?}"),
        }
        match length_from_spec(layout.height, None, &layout, true) {
            Length::Fixed(px) => assert!(
                (px - 40.0).abs() < 0.01,
                "2.5rem → Fixed(40), got {px}"
            ),
            other => panic!("expected Fixed from 2.5rem, got {other:?}"),
        }
        let (w, h) = content_box_outer_axes(&layout, None);
        // 160×40 + pad 10×2 → border-box 180×60
        assert_eq!(
            w,
            Some(180.0),
            "content-box em Fixed must expand by pad"
        );
        assert_eq!(
            h,
            Some(60.0),
            "content-box rem Fixed must expand by pad"
        );
    }

    #[test]
    fn root_viewport_axis_accepts_one_px() {
        assert_eq!(root_viewport_axis(Some(800.0)), Length::Fixed(800.0));
        // Former `> 1.0` gate treated 1px as invalid → Fill collapse.
        assert_eq!(root_viewport_axis(Some(1.0)), Length::Fixed(1.0));
        assert_eq!(root_viewport_axis(Some(0.0)), Length::Fill);
        assert_eq!(root_viewport_axis(None), Length::Fill);
    }

    #[test]
    fn column_skips_hidden_children_including_gpu() {
        let mut bridge = MessageBridge::new();
        let mut page = WidgetProps::default();
        page.class_names = vec!["anon-page".into()];
        page.layout.apply_css_text(
            "display:grid;grid-template-rows:auto auto auto minmax(0,1fr);gap:12px",
            None,
            None,
        );
        bridge.register(1, WidgetKind::Column, page);

        let mut visible = WidgetProps::default();
        visible.label = "visible".into();
        bridge.register(2, WidgetKind::Text, visible);

        let mut hidden_gpu = WidgetProps::default();
        hidden_gpu.class_names = vec!["nana-gpu-preview".into()];
        hidden_gpu.layout.hidden = true;
        bridge.register(3, WidgetKind::Column, hidden_gpu);

        let mut hidden_text = WidgetProps::default();
        hidden_text.label = "gone".into();
        hidden_text.layout.hidden = true;
        bridge.register(4, WidgetKind::Text, hidden_text);

        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 1, None);
        let snap = bridge.snapshot();
        let visible_ids: Vec<_> = snap
            .get(1)
            .unwrap()
            .children
            .iter()
            .copied()
            .filter(|&id| is_layout_visible(&snap, id))
            .collect();
        assert_eq!(visible_ids, vec![2]);

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((800.0, 600.0)), |e| e);
    }

    #[test]
    fn gpu_preview_placeholder_builds_light_surface_not_navy() {
        let mut bridge = MessageBridge::new();
        let mut slot = WidgetProps::default();
        slot.class_names = vec!["nana-gpu-preview".into()];
        slot.agent_id = "slot.gpu-preview".into();
        // Host WebView marker — must not drive iced paint.
        slot.layout.background = Some([30.0 / 255.0, 41.0 / 255.0, 59.0 / 255.0, 1.0]);
        slot.layout.height = Some(LengthSpec::Px(120.0));
        slot.layout.border_radius = Some(10.0);
        bridge.register(1, WidgetKind::Column, slot);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let surface = tokens.colors.surface;
        assert!(
            (surface.r - 30.0 / 255.0).abs() > 0.2,
            "light surface must not be navy"
        );
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((740.0, 120.0)), |e| e);
    }

    #[test]
    fn text_ellipsis_layout_builds_view() {
        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout.apply_css_text(
            "display:flex;flex-direction:row;width:200px;height:32px",
            None,
            None,
        );
        bridge.register(1, WidgetKind::Row, row_props);
        let mut label = WidgetProps::default();
        label.label = "a very long sidebar label that should ellipsis".into();
        label.layout.apply_css_text(
            "flex:1;min-width:0;text-overflow:ellipsis;white-space:nowrap",
            None,
            None,
        );
        bridge.register(2, WidgetKind::Text, label);
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        assert!(snap.get(2).unwrap().props.layout.uses_text_ellipsis());
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((200.0, 32.0)), |e| e);
    }

    #[test]
    fn typography_layout_drives_text_view_without_panic() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.element_tag = "body".into();
        root.inline_style = "font-size:13px;color:#1a1a1f".into();
        bridge.register(1, WidgetKind::Column, root);

        let mut title = WidgetProps::default();
        title.label = "项目总览".into();
        title.inline_style =
            "font-size:18px;font-weight:600;line-height:1.55;letter-spacing:0.2px;color:#1a1a1f"
                .into();
        bridge.register(2, WidgetKind::Text, title);

        let mut muted = WidgetProps::default();
        muted.label = "卡片标题".into();
        muted.class_names = vec!["card-h2".into()];
        muted.inline_style =
            "font-size:13px;font-weight:600;letter-spacing:0.5px;color:#5a616e".into();
        bridge.register(3, WidgetKind::Text, muted);

        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.inject_stylesheet(
            "body{font-size:13px;color:#1a1a1f}.card-h2{font-size:13px;font-weight:600;letter-spacing:0.5px;color:#5a616e}",
        );

        let snap = bridge.snapshot();
        let title_layout = &snap.get(2).unwrap().props.layout;
        assert_eq!(title_layout.font_size, Some(18.0));
        assert_eq!(title_layout.font_weight, Some(600));
        assert!(title_layout.letter_spacing.unwrap_or(0.0) > 0.0);
        let muted_layout = &snap.get(3).unwrap().props.layout;
        assert_eq!(muted_layout.font_size, Some(13.0));
        assert_eq!(muted_layout.font_weight, Some(600));
        assert!(muted_layout.color.is_some());
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((480.0, 120.0)), |e| e);
    }

    #[test]
    fn label_text_fallback_matches_control_size_text_size() {
        // No CSS font-size → ControlSize::text_size (UI_BASE_TEXT_SIZE ± 1).
        // Guards against the old iced-view hardcodes Medium=14 / Large=18.
        let bare = LayoutStyle::default();
        assert!(bare.font_size.is_none());
        assert_eq!(
            label_text_size_px(ControlSize::Small, &bare),
            UI_BASE_TEXT_SIZE - 1.0
        );
        assert_eq!(
            label_text_size_px(ControlSize::Medium, &bare),
            UI_BASE_TEXT_SIZE
        );
        assert_eq!(
            label_text_size_px(ControlSize::Large, &bare),
            UI_BASE_TEXT_SIZE + 1.0
        );
        assert_eq!(ControlSize::Medium.text_size(), 13.0);
        assert_eq!(ControlSize::Large.text_size(), 14.0);

        let mut with_css = LayoutStyle::default();
        with_css.font_size = Some(18.0);
        assert_eq!(
            label_text_size_px(ControlSize::Medium, &with_css),
            18.0,
            "explicit CSS font-size still wins over ControlSize"
        );

        let mut bridge = MessageBridge::new();
        let mut text = WidgetProps::default();
        text.label = "fallback".into();
        text.size = ControlSize::Large;
        bridge.register(1, WidgetKind::Text, text);
        let snap = bridge.snapshot();
        assert!(snap.get(1).unwrap().props.layout.font_size.is_none());
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((200.0, 40.0)), |e| e);
    }

    #[test]
    fn layout_column_row_label_only_uses_label_text_size_contract() {
        // Guards against layout_column / layout_row / wrap_layout_owned hardcoding
        // text(...).size(14) for label-only containers (no children).
        let mut bridge = MessageBridge::new();

        let mut col = WidgetProps::default();
        col.label = "col-label".into();
        col.size = ControlSize::Medium;
        bridge.register(1, WidgetKind::Column, col);

        let mut row = WidgetProps::default();
        row.label = "row-label".into();
        row.size = ControlSize::Small;
        bridge.register(2, WidgetKind::Row, row);

        let mut box_css = WidgetProps::default();
        box_css.label = "box-css".into();
        box_css.size = ControlSize::Medium;
        box_css
            .layout
            .apply_css_text("font-size:18px", None, None);
        bridge.register(3, WidgetKind::Box, box_css);

        let snap = bridge.snapshot();
        for id in [1u64, 2, 3] {
            let w = snap.get(id).unwrap();
            assert!(w.children.is_empty(), "label-only fixture id={id}");
            assert!(!w.props.label.is_empty());
        }

        let col_layout = &snap.get(1).unwrap().props.layout;
        assert!(col_layout.font_size.is_none());
        assert_eq!(
            label_text_size_px(ControlSize::Medium, col_layout),
            UI_BASE_TEXT_SIZE,
            "Column label-only must not hardcode 14; Medium → UI_BASE_TEXT_SIZE"
        );
        assert_ne!(
            label_text_size_px(ControlSize::Medium, col_layout),
            14.0,
            "regression: old hardcoded size(14) for Medium"
        );

        let row_layout = &snap.get(2).unwrap().props.layout;
        assert_eq!(
            label_text_size_px(ControlSize::Small, row_layout),
            UI_BASE_TEXT_SIZE - 1.0
        );

        let box_layout = &snap.get(3).unwrap().props.layout;
        assert_eq!(box_layout.font_size, Some(18.0));
        assert_eq!(
            label_text_size_px(ControlSize::Medium, box_layout),
            18.0,
            "CSS font-size wins over ControlSize on label-only Box"
        );

        let tokens = ThemeMode::Light.tokens();
        // Borrowed + owned (static) paths both paint label-only column/row/box.
        let _borrowed: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((240.0, 80.0)), |e| e);
        let _static: Element<'static, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((240.0, 80.0)), |e| e);
    }

    #[test]
    fn iced_flow_skips_position_absolute_children() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.layout.apply_css_text(
            "position:relative;display:flex;flex-direction:column;width:200px;height:120px",
            None,
            None,
        );
        bridge.register(1, WidgetKind::Column, root);

        let mut flow = WidgetProps::default();
        flow.label = "flow".into();
        flow.layout
            .apply_css_text("width:60px;height:40px", None, None);
        bridge.register(2, WidgetKind::Box, flow);

        let mut badge = WidgetProps::default();
        badge.label = "badge".into();
        badge.layout.apply_css_text(
            "position:absolute;top:8px;left:100px;width:40px;height:24px",
            None,
            None,
        );
        bridge.register(3, WidgetKind::Box, badge);

        let mut after = WidgetProps::default();
        after.label = "after".into();
        after
            .layout
            .apply_css_text("width:60px;height:40px", None, None);
        bridge.register(4, WidgetKind::Box, after);

        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 1, None);
        let snap = bridge.snapshot();
        let in_flow: Vec<_> = snap
            .get(1)
            .unwrap()
            .children
            .iter()
            .copied()
            .filter(|&id| is_in_flow_layout(&snap, id))
            .collect();
        assert_eq!(in_flow, vec![2, 4], "absolute must leave iced flow");
        assert!(
            is_layout_visible(&snap, 3),
            "absolute stays visible for measure/overlay"
        );
        assert!(!is_in_flow_layout(&snap, 3));

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((200.0, 120.0)), |e| e);
    }

    #[test]
    fn iced_flow_skips_position_fixed_and_root_collects() {
        let mut bridge = MessageBridge::new();
        let mut root = WidgetProps::default();
        root.layout.apply_css_text(
            "display:flex;flex-direction:column;width:200px;height:120px",
            None,
            None,
        );
        bridge.register(1, WidgetKind::Column, root);

        let mut flow = WidgetProps::default();
        flow.label = "flow".into();
        flow.layout
            .apply_css_text("width:60px;height:40px", None, None);
        bridge.register(2, WidgetKind::Box, flow);

        let mut pin = WidgetProps::default();
        pin.label = "pin".into();
        pin.layout.apply_css_text(
            "position:fixed;top:8px;right:12px;width:40px;height:24px;z-index:2",
            None,
            None,
        );
        bridge.register(3, WidgetKind::Box, pin);

        let mut after = WidgetProps::default();
        after.label = "after".into();
        after
            .layout
            .apply_css_text("width:60px;height:40px", None, None);
        bridge.register(4, WidgetKind::Box, after);

        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 1, None);
        let snap = bridge.snapshot();
        let in_flow: Vec<_> = snap
            .get(1)
            .unwrap()
            .children
            .iter()
            .copied()
            .filter(|&id| is_in_flow_layout(&snap, id))
            .collect();
        assert_eq!(in_flow, vec![2, 4], "fixed must leave iced flow");
        assert!(snap.get(3).unwrap().props.layout.is_fixed());
        assert_eq!(collect_css_fixed_ids(&snap), vec![3]);
        let (x, y, w, h) =
            resolve_fixed_box(&snap.get(3).unwrap().props.layout, 200.0, 120.0);
        assert!((x - 148.0).abs() < 0.01);
        assert!((y - 8.0).abs() < 0.01);
        assert!((w - 40.0).abs() < 0.01);
        assert!((h - 24.0).abs() < 0.01);

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((200.0, 120.0)), |e| e);
    }

    #[test]
    fn resolve_container_height_min_zero_is_not_fill() {
        let mut layout = LayoutStyle::default();
        layout.min_height = Some(LengthSpec::Px(0.0));
        let parent = ParentBox::from_viewport(800.0, 600.0);
        assert_eq!(
            resolve_container_height(&layout, parent),
            None,
            "min-height:0 must not force Length::Fill"
        );

        layout.flex_grow = Some(1.0);
        assert_eq!(
            resolve_container_height(&layout, parent),
            Some(Length::Fill),
            "flex-grow still participates in the height chain"
        );

        let mut percent = LayoutStyle::default();
        percent.apply_css_text("min-height: 100%", None, None);
        assert_eq!(percent.height, Some(LengthSpec::Fill));
        assert_eq!(
            resolve_container_height(&percent, parent),
            Some(Length::Fill),
            "min-height:100% maps to height Fill before resolve"
        );
    }

    #[test]
    fn iced_percent_gap_matches_measure_against_content_box() {
        // Same CB math as T-F13 / T-F14: LengthSpec retained until layout.
        let mut row = LayoutStyle::default();
        row.apply_css_text(
            "display:flex;flex-direction:row;gap:10%;width:200px;height:80px",
            None,
            None,
        );
        let row_box = row.resolve_content_box(ParentBox::from_viewport(220.0, 100.0));
        assert_eq!(
            row.main_gap_against(FlexDirection::Row, row_box),
            20.0,
            "row gap% → content width"
        );

        let mut col = LayoutStyle::default();
        col.apply_css_text(
            "display:flex;flex-direction:column;gap:10%;width:200px;height:300px",
            None,
            None,
        );
        let col_box = col.resolve_content_box(ParentBox::from_viewport(220.0, 320.0));
        assert_eq!(
            col.main_gap_against(FlexDirection::Column, col_box),
            30.0,
            "column gap% → content height"
        );
    }

    #[test]
    fn iced_wrap_cross_gap_percent_falls_back_when_height_auto() {
        // T-W05: wrap auto-height → content_h 0; cross_gap% must use width (iced spacing).
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:10% 12px;width:200px",
            None,
            None,
        );
        let child_box = layout.resolve_content_box(ParentBox::from_viewport(200.0, 160.0));
        assert!(
            child_box.height.is_none() || child_box.height == Some(0.0),
            "auto height must stay indefinite before shrink-to-fit"
        );
        assert_eq!(
            layout.main_gap_against(FlexDirection::Row, child_box),
            12.0
        );
        assert_eq!(
            layout.cross_gap_against(FlexDirection::Row, child_box),
            20.0,
            "row-gap% falls back to width when height indefinite"
        );
    }

    #[test]
    fn iced_wrap_reverse_uses_same_cross_gap_percent() {
        // T-W06: wrap-reverse shares cross_gap% CB math; line order is iced `.rev()`.
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap-reverse;gap:10% 12px;width:200px",
            None,
            None,
        );
        assert_eq!(layout.flex_wrap, FlexWrap::WrapReverse);
        let child_box = layout.resolve_content_box(ParentBox::from_viewport(200.0, 160.0));
        assert_eq!(layout.cross_gap_against(FlexDirection::Row, child_box), 20.0);
        assert_eq!(layout.main_gap_against(FlexDirection::Row, child_box), 12.0);
    }

    #[test]
    fn layout_style_gap_padding_row_builds_view() {
        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = LayoutStyle {
            direction: Some(FlexDirection::Row),
            gap: Some(LengthSpec::Px(12.0)),
            padding: Some(LengthSpec::Px(8.0)),
            width: Some(LengthSpec::Fill),
            justify_content: JustifySpec::SpaceBetween,
            ..LayoutStyle::default()
        };
        bridge.register(1, WidgetKind::Row, row_props);
        bridge.register(
            2,
            WidgetKind::Button,
            WidgetProps {
                label: "A".into(),
                button_kind: ButtonKind::Primary,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "B".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        let snap = bridge.snapshot();
        assert_eq!(snap.get(1).unwrap().kind, WidgetKind::Row);
        assert_eq!(
            snap.get(1).unwrap().props.layout.justify_content,
            JustifySpec::SpaceBetween
        );
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((800.0, 600.0)), |e| e);
    }

    #[test]
    fn flex_grow_maps_to_fill_portion_weight() {
        let mut layout = LayoutStyle::default();
        layout.flex_grow = Some(2.0);
        assert_eq!(
            length_from_spec(Some(LengthSpec::Fill), None, &layout, true),
            Length::FillPortion(200)
        );
        layout.flex_grow = Some(1.0);
        assert_eq!(
            length_from_spec(Some(LengthSpec::Fill), None, &layout, true),
            Length::FillPortion(100)
        );
    }

    #[test]
    fn flex_main_overrides_shrink_fixed_like_measure() {
        // T-F18: 150+150 @200 shrink:1 → 100+100 (iced applies Fixed override).
        let mut parent = LayoutStyle::default();
        parent.apply_css_text(
            "display:flex;flex-direction:row;width:200px;height:80px;gap:0",
            None,
            None,
        );
        let mut bridge = MessageBridge::new();
        let mut row = WidgetProps::default();
        row.layout = parent.clone();
        bridge.register(1, WidgetKind::Row, row);
        for (id, label) in [(2u64, "a"), (3, "b")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.apply_css_text(
                "width:150px;flex-shrink:1;height:40px",
                None,
                None,
            );
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let visible = snap.get(1).unwrap().children.clone();
        let child_box = ParentBox::new(Some(200.0), Some(80.0));
        let sizes = flex_main_overrides(&snap, &visible, &parent, child_box, FlexDirection::Row)
            .expect("definite main");
        assert!((sizes[0].unwrap() - 100.0).abs() < 0.01);
        assert!((sizes[1].unwrap() - 100.0).abs() < 0.01);

        // Intrinsic width:auto must not become Fixed(0) (heading text wrap).
        let mut auto_parent = LayoutStyle::default();
        auto_parent.apply_css_text(
            "display:flex;flex-direction:row;width:300px;height:40px;justify-content:space-between;gap:10px",
            None,
            None,
        );
        let mut auto_bridge = MessageBridge::new();
        let mut auto_row = WidgetProps::default();
        auto_row.layout = auto_parent.clone();
        auto_bridge.register(10, WidgetKind::Row, auto_row);
        let mut title = WidgetProps::default();
        title.label = "heading".into();
        title.layout.apply_css_text("height:auto", None, None);
        auto_bridge.register(11, WidgetKind::Column, title);
        auto_bridge.insert_child(11, 10, None);
        let mut actions = WidgetProps::default();
        actions.layout.apply_css_text("flex:0 0 auto;height:auto", None, None);
        auto_bridge.register(12, WidgetKind::Row, actions);
        auto_bridge.insert_child(12, 10, None);
        let auto_snap = auto_bridge.snapshot();
        let auto_visible = auto_snap.get(10).unwrap().children.clone();
        let auto_sizes = flex_main_overrides(
            &auto_snap,
            &auto_visible,
            &auto_parent,
            ParentBox::new(Some(300.0), Some(40.0)),
            FlexDirection::Row,
        )
        .expect("definite main");
        assert_eq!(
            auto_sizes[0], None,
            "width:auto must stay Fit (no Fixed(0) override)"
        );

        // T-F19: min-width freeze → 120+80
        let mut a = snap.get(2).unwrap().props.layout.clone();
        a.apply_css_text("width:150px;flex-shrink:1;min-width:120px;height:40px", None, None);
        let b = snap.get(3).unwrap().props.layout.clone();
        let styles = [&a, &b];
        let frozen = crate::measure::resolve_flex_children_main_sizes(
            &styles,
            FlexDirection::Row,
            200.0,
            Some(200.0),
            0.0,
        );
        assert!((frozen[0] - 120.0).abs() < 0.01);
        assert!((frozen[1] - 80.0).abs() < 0.01);
    }

    #[test]
    fn flex_grow_column_child_consumes_height_fill() {
        let mut child = LayoutStyle::default();
        child.flex_grow = Some(1.0);
        assert_eq!(
            child.child_main_length(FlexDirection::Column),
            Some(LengthSpec::Fill)
        );
        assert_eq!(
            child.child_main_length(FlexDirection::Row),
            Some(LengthSpec::Fill)
        );
    }

    #[test]
    fn writeback_containing_blocks_fill_chain_matches_resolve_content_box() {
        let mut bridge = MessageBridge::new();
        let mut shell = WidgetProps::default();
        shell.layout.width = Some(LengthSpec::Fill);
        shell.layout.height = Some(LengthSpec::Fill);
        shell.layout.padding = Some(LengthSpec::Px(10.0));
        bridge.register(1, WidgetKind::Column, shell);
        bridge.register(2, WidgetKind::Box, WidgetProps::default());
        bridge.insert_child(2, 1, None);

        let viewport = ParentBox::from_viewport(500.0, 400.0);
        writeback_containing_blocks(&mut bridge, viewport);
        let expected = bridge
            .get(1)
            .unwrap()
            .props
            .layout
            .resolve_content_box(viewport);
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_width,
            expected.width
        );
        assert_eq!(
            bridge.get(2).unwrap().props.containing_block_height,
            expected.height
        );
        assert_eq!(expected.width, Some(480.0));
        assert_eq!(expected.height, Some(380.0));
    }

    #[test]
    fn percent_width_uses_parent_box() {
        let layout = LayoutStyle {
            width: Some(LengthSpec::Percent(50.0)),
            ..LayoutStyle::default()
        };
        let len = length_from_spec(layout.width, Some(400.0), &layout, false);
        assert_eq!(len, Length::Fixed(200.0));
    }

    #[test]
    fn flex_wrap_uses_parent_width_when_row_width_auto() {
        let mut layout = LayoutStyle::default();
        layout.direction = Some(FlexDirection::Row);
        layout.flex_wrap = FlexWrap::Wrap;
        layout.width = Some(LengthSpec::Auto);
        layout.gap = Some(LengthSpec::Px(8.0));
        let parent = ParentBox::from_viewport(200.0, 160.0);
        let child_box = layout.resolve_content_box(parent);
        assert!(
            child_box.width.is_none(),
            "auto width must not invent a definite content box"
        );
        let wrap_w = wrap_content_width(&layout, child_box, parent);
        assert_eq!(wrap_w, Some(200.0));

        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = layout;
        bridge.register(1, WidgetKind::Row, row_props);
        for (id, label) in [(2u64, "a"), (3, "b"), (4, "c"), (5, "d")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        let lines =
            chunk_row_wrap_lines(&snap.get(1).unwrap().props.layout, &children, &snap, wrap_w);
        assert_eq!(lines.len(), 2, "200px parent must wrap four 80px children");
        assert_eq!(lines[0], vec![2, 3]);
        assert_eq!(lines[1], vec![4, 5]);

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((200.0, 160.0)), |e| e);
    }

    #[test]
    fn missing_gap_defaults_to_zero_like_measure() {
        let layout = LayoutStyle::default();
        assert!(layout.gap.is_none());
        assert_eq!(layout.main_gap(FlexDirection::Row), 0.0);
        assert_eq!(layout.cross_gap(FlexDirection::Row), 0.0);
        assert_eq!(layout_gap(&layout), 0.0);
        assert_eq!(
            flex_item_spacing(layout_gap(&layout), JustifySpec::SpaceAround),
            0.0
        );
        assert_eq!(
            flex_item_spacing(14.0, JustifySpec::SpaceBetween),
            0.0,
            "distributed justify must not also use row.spacing(gap)"
        );
        assert_eq!(flex_item_spacing(14.0, JustifySpec::Start), 14.0);
    }

    #[test]
    fn two_value_gap_uses_column_gap_on_row_main_axis() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:row;gap:4px 12px;flex-wrap:wrap;width:200px",
            None,
            None,
        );
        assert_eq!(layout.main_gap(FlexDirection::Row), 12.0);
        assert_eq!(layout.cross_gap(FlexDirection::Row), 4.0);

        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Row, row_props);
        for (id, label) in [(2u64, "a"), (3, "b"), (4, "c"), (5, "d")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        // 80 + 12 + 80 = 172 ≤ 200 → two per line (column-gap, not row-gap).
        let lines = chunk_row_wrap_lines(&layout, &children, &snap, Some(200.0));
        assert_eq!(lines, vec![vec![2, 3], vec![4, 5]]);
    }

    #[test]
    fn chunk_column_wrap_lines_breaks_by_height() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap;gap:8px;height:100px;width:200px",
            None,
            None,
        );

        let mut bridge = MessageBridge::new();
        let mut col_props = WidgetProps::default();
        col_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Column, col_props);
        for (id, label) in [(2u64, "a"), (3, "b"), (4, "c"), (5, "d")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        // 40+8+40=88 ≤ 100; +8+40=136 > 100 → two per column.
        let lines = chunk_column_wrap_lines(&layout, &children, &snap, Some(100.0), Some(200.0));
        assert_eq!(lines, vec![vec![2, 3], vec![4, 5]]);
    }

    #[test]
    fn column_wrap_reverse_paint_order_matches_measure() {
        // Chunk order is DOM flex-line order; view reverses for WrapReverse (T-W08).
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap-reverse;gap:8px;height:100px;width:200px",
            None,
            None,
        );
        assert_eq!(layout.flex_wrap, FlexWrap::WrapReverse);

        let mut bridge = MessageBridge::new();
        let mut col_props = WidgetProps::default();
        col_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Column, col_props);
        for (id, label) in [(2u64, "a"), (3, "b"), (4, "c"), (5, "d")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        let lines = chunk_column_wrap_lines(&layout, &children, &snap, Some(100.0), Some(200.0));
        assert_eq!(lines, vec![vec![2, 3], vec![4, 5]], "chunk stays DOM order");
        let paint: Vec<Vec<WidgetId>> = lines.into_iter().rev().collect();
        assert_eq!(
            paint,
            vec![vec![4, 5], vec![2, 3]],
            "WrapReverse paint: cd then ab (cd@x0 / ab@x88)"
        );
    }

    #[test]
    fn chunk_column_wrap_lines_counts_vertical_margin_percent() {
        // margin-% resolves against container content width (CSS / measure).
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:column;flex-wrap:wrap;gap:0;height:100px;width:200px",
            None,
            None,
        );

        let mut bridge = MessageBridge::new();
        let mut col_props = WidgetProps::default();
        col_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Column, col_props);
        for (id, label) in [(2u64, "a"), (3, "b")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            // 10% of 200 = 20 → outer 80; two → 160 > 100 → one per column.
            child.layout.margin_top = Some(LengthSpec::Percent(10.0));
            child.layout.margin_bottom = Some(LengthSpec::Percent(10.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        let lines = chunk_column_wrap_lines(&layout, &children, &snap, Some(100.0), Some(200.0));
        assert_eq!(
            lines,
            vec![vec![2], vec![3]],
            "vertical margin % against content width must participate in column-wrap packing"
        );
        // Missing width base would treat % as 0 → outer 40+40=80 ≤ 100 (false same-column).
        let wrong = chunk_column_wrap_lines(&layout, &children, &snap, Some(100.0), None);
        assert_eq!(
            wrong,
            vec![vec![2, 3]],
            "sanity: without content_w, % margins collapse and falsely pack together"
        );
    }

    #[test]
    fn chunk_row_wrap_lines_skips_hidden_children() {
        let mut layout = LayoutStyle::default();
        layout.direction = Some(FlexDirection::Row);
        layout.flex_wrap = FlexWrap::Wrap;
        layout.gap = Some(LengthSpec::Px(8.0));
        layout.width = Some(LengthSpec::Px(200.0));

        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Row, row_props);
        for (id, label, hidden) in [
            (2u64, "a", false),
            (3, "hidden", true),
            (4, "b", false),
            (5, "c", false),
            (6, "d", false),
        ] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            child.layout.hidden = hidden;
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        let lines = chunk_row_wrap_lines(&layout, &children, &snap, Some(200.0));
        assert_eq!(
            lines,
            vec![vec![2, 4], vec![5, 6]],
            "hidden sibling must not consume wrap main-axis width"
        );
    }

    #[test]
    fn chunk_row_wrap_lines_counts_horizontal_margin() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text(
            "display:flex;flex-direction:row;flex-wrap:wrap;gap:8px;width:200px",
            None,
            None,
        );

        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = layout.clone();
        bridge.register(1, WidgetKind::Row, row_props);
        for (id, label) in [(2u64, "a"), (3, "b")] {
            let mut child = WidgetProps::default();
            child.label = label.into();
            child.layout.width = Some(LengthSpec::Px(80.0));
            child.layout.height = Some(LengthSpec::Px(40.0));
            child.layout.margin_left = Some(LengthSpec::Px(16.0));
            child.layout.margin_right = Some(LengthSpec::Px(16.0));
            bridge.register(id, WidgetKind::Box, child);
            bridge.insert_child(id, 1, None);
        }
        let snap = bridge.snapshot();
        let children = snap.get(1).unwrap().children.clone();
        // Content 80 alone would fit two (80+8+80=168); outer 112 does not.
        let lines = chunk_row_wrap_lines(&layout, &children, &snap, Some(200.0));
        assert_eq!(
            lines,
            vec![vec![2], vec![3]],
            "horizontal margin must participate in flex-wrap main-axis packing"
        );
    }

    #[test]
    fn space_between_with_gap_builds_without_double_spacing() {
        let mut bridge = MessageBridge::new();
        let mut row_props = WidgetProps::default();
        row_props.layout = LayoutStyle {
            direction: Some(FlexDirection::Row),
            gap: Some(LengthSpec::Px(14.0)),
            width: Some(LengthSpec::Px(480.0)),
            height: Some(LengthSpec::Px(44.0)),
            justify_content: JustifySpec::SpaceBetween,
            ..LayoutStyle::default()
        };
        assert_eq!(
            flex_item_spacing(
                layout_gap(&row_props.layout),
                row_props.layout.justify_content
            ),
            0.0
        );
        bridge.register(1, WidgetKind::Row, row_props);
        bridge.register(
            2,
            WidgetKind::Text,
            WidgetProps {
                label: "Label".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Switch,
            WidgetProps {
                label: "On".into(),
                toggled: true,
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((480.0, 44.0)), |e| e);
    }

    #[test]
    fn scrollport_child_containing_box_drops_height() {
        let mut body = LayoutStyle::default();
        body.overflow_y = OverflowSpec::Auto;
        body.height = Some(LengthSpec::Fill);
        body.width = Some(LengthSpec::Fill);
        let child_box = ParentBox::new(Some(200.0), Some(606.0));
        let flow = flow_child_containing_box(&body, child_box);
        assert_eq!(flow.width, Some(200.0));
        assert!(
            flow.height.is_none(),
            "scroll content must not keep a definite height CB (Fill→0 under iced)"
        );
        let plain = LayoutStyle::default();
        let kept = flow_child_containing_box(&plain, child_box);
        assert_eq!(kept.height, Some(606.0));
    }

    #[test]
    fn scrollport_fill_height_child_restores_viewport_cb() {
        let mut port = LayoutStyle::default();
        port.overflow_y = OverflowSpec::Auto;
        let flow = ParentBox::new(Some(720.0), None);
        let viewport = ParentBox::new(Some(720.0), Some(560.0));
        let mut fill_child = LayoutStyle::default();
        fill_child.height = Some(LengthSpec::Fill);
        let restored = parent_box_for_flow_child(&port, flow, viewport, &fill_child);
        assert_eq!(restored.height, Some(560.0));
        assert_eq!(restored.width, Some(720.0));

        let auto_child = LayoutStyle::default();
        let kept = parent_box_for_flow_child(&port, flow, viewport, &auto_child);
        assert!(kept.height.is_none(), "intrinsic children stay indefinite");
    }

    #[test]
    fn grid_track_fallback_auto_is_shrink_not_fr() {
        assert_eq!(
            grid_track_fallback_length(GridTrack::Auto),
            Length::Fit,
            "auto must measure content (Fit), not share free space like 1fr"
        );
        assert!(matches!(
            grid_track_fallback_length(GridTrack::Fr(1.0)),
            Length::FillPortion(100)
        ));
    }

    #[test]
    fn scrollport_fill_grow_column_skips_definite_seed_path() {
        let mut body = WidgetProps::default();
        body.layout.flex_grow = Some(1.0);
        body.layout.height = Some(LengthSpec::Fill);
        body.layout.overflow_y = OverflowSpec::Auto;
        assert!(
            !needs_definite_fill_column(&body),
            "sidebar body scrollport must use wrap_layout_owned, not Fixed-height seed"
        );
        body.layout.overflow_y = OverflowSpec::Visible;
        assert!(
            needs_definite_fill_column(&body),
            "non-scroll Fill+grow columns still seed a definite CB"
        );
    }

    #[test]
    fn overflow_y_auto_builds_scrollable_node() {
        let mut bridge = MessageBridge::new();
        let mut props = WidgetProps::default();
        props.layout.overflow_y = OverflowSpec::Auto;
        props.layout.height = Some(LengthSpec::Fill);
        props.layout.flex_grow = Some(1.0);
        bridge.register(1, WidgetKind::Column, props);
        bridge.register(
            2,
            WidgetKind::Text,
            WidgetProps {
                label: "long".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        assert!(snap.get(1).unwrap().props.layout.scrolls_y());
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_viewport(&snap, tokens, Some((400.0, 300.0)), |e| e);
    }

    #[test]
    fn sidebar_main_grid_tracks_apply() {
        let mut bridge = MessageBridge::new();
        let mut body = WidgetProps::default();
        body.class_names = vec!["nana-workspace-shell__body".into()];
        body.layout.apply_class_layout_hints(&body.class_names);
        bridge.register(1, WidgetKind::Row, body);
        let mut side = WidgetProps::default();
        side.class_names = vec!["nana-workspace-shell__sidebar".into()];
        side.layout.apply_class_layout_hints(&side.class_names);
        bridge.register(2, WidgetKind::Column, side);
        let mut main = WidgetProps::default();
        main.class_names = vec!["nana-workspace-shell__primary".into()];
        main.layout.apply_class_layout_hints(&main.class_names);
        bridge.register(3, WidgetKind::Column, main);
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        let snap = bridge.snapshot();
        let cols = snap
            .get(1)
            .unwrap()
            .props
            .layout
            .grid_columns
            .as_ref()
            .unwrap();
        assert_eq!(cols[0], GridTrack::Px(220.0));
        assert_eq!(
            snap.get(2).unwrap().props.layout.width,
            Some(LengthSpec::Px(220.0))
        );
        assert_eq!(
            snap.get(3).unwrap().props.layout.width,
            Some(LengthSpec::Fill)
        );
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((800.0, 600.0)), |e| e);
    }

    #[test]
    fn fr_portion_keeps_fractional_weights_commensurate() {
        // 1fr and 1.3fr must share a scale — whole-number shortcuts (1 vs 130)
        // previously made overview `1.3fr 1fr` lay out as ~130:1.
        assert_eq!(fr_portion(1.0), 100);
        assert_eq!(fr_portion(2.0), 200);
        assert_eq!(fr_portion(0.5), 50);
        assert_eq!(fr_portion(1.3), 130);
        assert_eq!(fr_portion(1.5), 150);
    }

    #[test]
    fn fr_only_auto_width_grid_still_resolves_track_cbs() {
        // overview-grid style: width:auto + 1.3fr 1fr must inject per-track
        // parent_box widths even though iced wrappers stay FillPortion.
        let mut grid = LayoutStyle::default();
        grid.apply_css_text(
            "display:grid;grid-template-columns:1.3fr 1fr;gap:12px",
            None,
            None,
        );
        assert!(prefer_fill_portion_grid_tracks(&grid, FlexDirection::Row));
        let tracks = grid.active_grid_columns().expect("columns");
        let outers = resolve_grid_track_outers(tracks, 460.0, 12.0, 0.0, &[0.0, 0.0]);
        // budget 460, gap 12 → free split 1.3:1
        assert_eq!(outers.len(), 2);
        assert!(outers[0] > outers[1], "1.3fr > 1fr: {} vs {}", outers[0], outers[1]);
        assert!(outers[1] > 100.0, "second track must stay definite, got {}", outers[1]);
        let ratio = outers[0] / outers[1];
        assert!((ratio - 1.3).abs() < 0.05, "ratio={ratio}");
    }

    #[test]
    fn width_auto_flow_box_inherits_parent_cb() {
        let mut layout = LayoutStyle::default();
        // Column / unset direction + width:auto under a definite parent CB.
        layout.direction = Some(FlexDirection::Column);
        let pb = resolve_flow_content_box(&layout, ParentBox::new(Some(400.0), Some(300.0)));
        assert_eq!(pb.width, Some(400.0));
        layout.width = Some(LengthSpec::Shrink);
        let shrink = resolve_flow_content_box(&layout, ParentBox::new(Some(400.0), Some(300.0)));
        assert!(shrink.width.is_none());
        // Row flex items without grid tracks stay intrinsic on width:auto.
        let mut row = LayoutStyle::default();
        row.direction = Some(FlexDirection::Row);
        let row_pb = resolve_flow_content_box(&row, ParentBox::new(Some(400.0), Some(300.0)));
        assert!(row_pb.width.is_none());
        // Grid row still injects CB width for nested Fill/%.
        let mut grid = LayoutStyle::default();
        grid.direction = Some(FlexDirection::Row);
        grid.apply_css_text("display:grid;grid-template-columns:1fr 1fr", None, None);
        let grid_pb = resolve_flow_content_box(&grid, ParentBox::new(Some(400.0), Some(300.0)));
        assert_eq!(grid_pb.width, Some(400.0));
    }

    #[test]
    fn overflow_hidden_marks_clips_overflow() {
        let mut layout = LayoutStyle::default();
        layout.apply_css_text("overflow:hidden;width:100%", None, None);
        assert!(layout.clips_overflow());
        assert!(!layout.scrolls_y());
    }

    #[test]
    fn text_host_flex_h2_uses_row_axis_not_column() {
        // `.card h2 { display:flex; align-items:center }` must stay a row so
        // center is vertical. A forced column centers the title horizontally.
        let mut flex = LayoutStyle::default();
        flex.apply_css_text(
            "display:flex;align-items:center;gap:10px;letter-spacing:0.5px",
            None,
            None,
        );
        assert!(
            !text_host_column_axis(&flex),
            "display:flex text host must use row axis"
        );

        let mut column = LayoutStyle::default();
        column.apply_css_text("display:flex;flex-direction:column;align-items:center", None, None);
        assert!(text_host_column_axis(&column));

        let block = LayoutStyle::default();
        assert!(
            text_host_column_axis(&block),
            "plain text hosts without flex stay column"
        );
    }

    #[test]
    fn auto_height_column_stays_shrink_after_space_between_push() {
        // iced Column::push encloses Fill spacers into column height; CSS
        // height:auto must be re-pinned to Shrink or headings eat siblings.
        let layout = LayoutStyle::default();
        let col = iced::widget::column![].width(Length::Fill);
        let children: Vec<Element<'_, ()>> = vec![
            text("heading").into(),
            text("chart").into(),
        ];
        let col = push_justified(col, children, JustifySpec::SpaceBetween, 8.0);
        let pinned = pin_flex_container_main_length(
            col,
            None,
            &layout,
            ParentBox::new(Some(400.0), None),
            true,
        );
        assert_eq!(
            pinned.size().height,
            Length::Fit,
            "height:auto column must stay Fit after SpaceBetween push"
        );
        let pinned_fill: iced::widget::Column<'_, ()> = pin_flex_container_main_length(
            iced::widget::column![].width(Length::Fill),
            None,
            &layout,
            ParentBox::new(Some(400.0), Some(200.0)),
            true,
        );
        assert_eq!(
            pinned_fill.size().height,
            Length::Fit,
            "height:auto column uses Fit (intrinsic, no Shrink compression)"
        );
    }

    #[test]
    fn auto_height_row_stays_shrink_after_space_between_push() {
        let layout = LayoutStyle::default();
        let row = iced::widget::row![].width(Length::Fill);
        let children: Vec<Element<'_, ()>> = vec![
            text("left").into(),
            text("right").into(),
        ];
        let row = push_justified_row(row, children, JustifySpec::SpaceBetween, 10.0);
        let pinned = pin_flex_row_cross_or_main_height(row, None, &layout);
        assert_eq!(
            pinned.size().height,
            Length::Fit,
            "height:auto row must stay Fit after SpaceBetween push"
        );
    }

    #[test]
    fn grid_track_fallback_percent_and_fr_share_portion_scale() {
        // Percent must use the same ×100 FillPortion scale as fr; a bare
        // FillPortion(1) for any % made `25% 1fr` lay out as ~1:100.
        assert_eq!(
            grid_track_fallback_length(GridTrack::Percent(25.0)),
            Length::FillPortion(25)
        );
        assert_eq!(
            grid_track_fallback_length(GridTrack::Fr(1.0)),
            Length::FillPortion(100)
        );
        assert_eq!(
            grid_track_fallback_length(GridTrack::Percent(50.0)),
            Length::FillPortion(50)
        );
        // 1.3fr 1fr bypass: fractional fr stays commensurate with whole fr.
        assert_eq!(
            grid_track_fallback_length(GridTrack::Fr(1.3)),
            Length::FillPortion(130)
        );
        let pct = match grid_track_fallback_length(GridTrack::Percent(25.0)) {
            Length::FillPortion(p) => p,
            other => panic!("expected FillPortion, got {other:?}"),
        };
        let fr = match grid_track_fallback_length(GridTrack::Fr(1.0)) {
            Length::FillPortion(p) => p,
            other => panic!("expected FillPortion, got {other:?}"),
        };
        assert_eq!(
            pct * 4,
            fr,
            "25% 1fr fallback ratio must be 1:4, not 1:100"
        );
    }

    #[test]
    fn grid_track_outers_deduct_child_margins_from_budget() {
        // content 300, margins 10+10 on each of 2 children → budget 260, gap 0
        // tracks 100px 1fr → 100 + 160; outers add per-child margins → 120 + 180
        let tracks = vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)];
        let margins = [20.0f32, 20.0];
        let outers = resolve_grid_track_outers(&tracks, 300.0, 0.0, 40.0, &margins);
        assert_eq!(outers.len(), 2);
        assert!((outers[0] - 120.0).abs() < 0.01, "fixed track + margin");
        assert!((outers[1] - 180.0).abs() < 0.01, "fr track + margin");
        // Without margin add-back, Fixed(track) wrapping margin-padding would
        // shrink content below the track (double deduct vs measure).
        assert!(outers[0] > 100.0);
        assert!(outers.iter().sum::<f32>() <= 300.0 + 0.01);
    }

    #[test]
    fn grid_track_outers_fixed_only_no_double_expand() {
        // Two 100px tracks, each child margin 10+10, content 300, gap 20
        // budget = 300 - 40 = 260; tracks stay 100,100; outers 120,120; +gap20 = 260
        let tracks = vec![GridTrack::Px(100.0), GridTrack::Px(100.0)];
        let margins = [20.0f32, 20.0];
        let outers = resolve_grid_track_outers(&tracks, 300.0, 20.0, 40.0, &margins);
        assert!((outers[0] - 120.0).abs() < 0.01);
        assert!((outers[1] - 120.0).abs() < 0.01);
        assert!((outers[0] + outers[1] + 20.0 - 260.0).abs() < 0.01);
    }

    #[test]
    fn grid_track_fallback_honors_minmax_min_px() {
        let floor = grid_track_fallback_length(GridTrack::MinMax {
            min_px: 400.0,
            fr: 1.0,
            max_px: None,
        });
        assert_eq!(
            floor,
            Length::FillPortion(100).min(400.0),
            "unknown content_w must still floor minmax min_px"
        );
        let plain = grid_track_fallback_length(GridTrack::MinMax {
            min_px: 0.0,
            fr: 1.5,
            max_px: None,
        });
        assert_eq!(plain, Length::FillPortion(150));
        let capped = grid_track_fallback_length(GridTrack::MinMax {
            min_px: 50.0,
            fr: 1.0,
            max_px: Some(120.0),
        });
        assert_eq!(capped, Length::FillPortion(100).min(50.0).max(120.0));
    }

    #[test]
    fn semantic_button_switch_input_tabs_sidebar_view() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Button,
            WidgetProps {
                label: "Go".into(),
                button_kind: ButtonKind::Primary,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Switch,
            WidgetProps {
                label: "On".into(),
                toggled: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Input,
            WidgetProps {
                value: "hello".into(),
                placeholder: "Type".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Tabs,
            WidgetProps {
                value: "a".into(),
                options: vec![
                    crate::bridge::SelectOptionProp {
                        value: "a".into(),
                        label: "A".into(),
                        disabled: false,
                    },
                    crate::bridge::SelectOptionProp {
                        value: "b".into(),
                        label: "B".into(),
                        disabled: false,
                    },
                ],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::SidebarRow,
            WidgetProps {
                label: "Home".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 1, None);
        bridge.insert_child(5, 1, None);
        bridge.insert_child(6, 1, None);

        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> = view_semantic_tree(&snap, tokens, |e| e);
        assert_eq!(snap.widgets.len(), 6);
        assert!(snap.get(2).unwrap().kind == WidgetKind::Button);
        assert!(snap.get(6).unwrap().kind == WidgetKind::SidebarRow);
    }

    #[test]
    fn sidebar_row_and_list_item_consume_layout_gap_with_leading_icon() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        let mut row = WidgetProps {
            label: "仓库名".into(),
            ..WidgetProps::default()
        };
        row.class_names = vec!["nana-sidebar-row".into()];
        row.layout.apply_css_text(
            "display:flex;align-items:center;gap:6px;padding:0 10px;height:28px",
            None,
            None,
        );
        bridge.register(2, WidgetKind::SidebarRow, row);
        let mut icon = WidgetProps::default();
        icon.class_names = vec!["lucide".into(), "lucide-folder".into()];
        icon.value = "folder".into();
        icon.layout.width = Some(LengthSpec::Px(14.0));
        icon.layout.height = Some(LengthSpec::Px(14.0));
        bridge.register(3, WidgetKind::Icon, icon);
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.value = "M3 7h18v10H3z".into();
        bridge.register(4, WidgetKind::Box, path);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 3, None);
        bridge.insert_child(2, 1, None);

        let snap = bridge.snapshot();
        let props = &snap.get(2).unwrap().props;
        assert!((props.layout.gap_or(0.0) - 6.0).abs() < 0.01);
        let (leading, label) = resolve_row_leading_and_label::<BridgeEvent>(
            &snap,
            props,
            &snap.get(2).unwrap().children,
            ThemeMode::Light.tokens(),
        );
        assert!(leading.is_some(), "sidebar row must resolve leading SVG/glyph");
        assert_eq!(label, "仓库名");

        let mut list = WidgetProps {
            label: "议题".into(),
            ..WidgetProps::default()
        };
        list.layout.apply_css_text("gap:8px", None, None);
        bridge.register(5, WidgetKind::ListItem, list);
        assert!((bridge.get(5).unwrap().props.layout.gap_or(0.0) - 8.0).abs() < 0.01);

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> = view_semantic_tree(&snap, tokens, |e| e);
    }

    #[test]
    fn drawer_side_and_width_from_props() {
        let mut props = WidgetProps {
            side: "left".into(),
            ..WidgetProps::default()
        };
        assert_eq!(drawer_side_from_props(&props), DrawerSide::Left);
        props.side = "right".into();
        assert_eq!(drawer_side_from_props(&props), DrawerSide::Right);
        props.side.clear();
        props.class_names = vec!["nana-drawer-left".into()];
        assert_eq!(drawer_side_from_props(&props), DrawerSide::Left);
        props.layout.width = Some(LengthSpec::Px(280.0));
        assert!((drawer_width_from_props(&props) - 280.0).abs() < 0.01);
    }

    #[test]
    fn drawer_semantic_view_builds() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Drawer,
            WidgetProps {
                label: "检查器".into(),
                hint: "详情".into(),
                side: "left".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        assert_eq!(WidgetKind::parse("nana-drawer"), Some(WidgetKind::Drawer));
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, None, None, |e| e);
        let _static: Element<'static, BridgeEvent> = view_semantic_tree_static_with_editors(
            &snap,
            tokens,
            Some((480.0, 320.0)),
            None,
            None,
            |e| e,
        );
    }

    #[test]
    fn drawer_footer_slot_partitions_children() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Drawer,
            WidgetProps {
                label: "设置".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "正文".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Button,
            WidgetProps {
                label: "应用".into(),
                class_names: vec!["drawer-footer".into()],
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);
        let snap = bridge.snapshot();
        assert!(is_drawer_footer_props(&snap.get(4).unwrap().props));
        assert!(!is_drawer_footer_props(&snap.get(3).unwrap().props));
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, None, None, |e| e);
    }

    #[test]
    fn drawer_footer_action_confirm_cancel_and_neutral() {
        let mut confirm = WidgetProps {
            label: "应用".into(),
            class_names: vec!["drawer-footer-confirm".into()],
            ..WidgetProps::default()
        };
        assert_eq!(drawer_footer_action(&confirm), DrawerFooterAction::Confirm);
        assert_eq!(drawer_footer_confirm_value(&confirm), "confirm");
        confirm.value = "apply".into();
        assert_eq!(drawer_footer_confirm_value(&confirm), "apply");

        let cancel = WidgetProps {
            label: "取消".into(),
            class_names: vec!["drawer-footer-cancel".into()],
            button_kind: ButtonKind::Primary,
            ..WidgetProps::default()
        };
        assert_eq!(drawer_footer_action(&cancel), DrawerFooterAction::Cancel);

        let primary = WidgetProps {
            label: "下一步".into(),
            button_kind: ButtonKind::Primary,
            ..WidgetProps::default()
        };
        assert_eq!(drawer_footer_action(&primary), DrawerFooterAction::Confirm);

        let neutral = WidgetProps {
            label: "更多".into(),
            button_kind: ButtonKind::Ghost,
            ..WidgetProps::default()
        };
        assert_eq!(drawer_footer_action(&neutral), DrawerFooterAction::Neutral);

        let drawer_id = 9;
        let btn_id = 4;
        assert_eq!(
            drawer_footer_press_event(drawer_id, &confirm, btn_id),
            BridgeEvent::SelectValue {
                id: drawer_id,
                value: "apply".into(),
            }
        );
        assert_eq!(
            drawer_footer_press_event(drawer_id, &cancel, btn_id),
            BridgeEvent::Toggle {
                id: drawer_id,
                value: false,
            }
        );
        assert_eq!(
            drawer_footer_press_event(drawer_id, &neutral, btn_id),
            BridgeEvent::Press { id: btn_id }
        );
    }

    #[test]
    fn drawer_footer_actions_semantic_view_builds() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Drawer,
            WidgetProps {
                label: "检查器".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Text,
            WidgetProps {
                label: "正文".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Row,
            WidgetProps {
                class_names: vec!["drawer-footer".into()],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::Button,
            WidgetProps {
                label: "取消".into(),
                class_names: vec!["drawer-footer-cancel".into()],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            6,
            WidgetKind::Button,
            WidgetProps {
                label: "应用".into(),
                class_names: vec!["drawer-footer-confirm".into()],
                button_kind: ButtonKind::Primary,
                value: "apply".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);
        bridge.insert_child(5, 4, None);
        bridge.insert_child(6, 4, None);
        let snap = bridge.snapshot();
        assert!(is_drawer_footer_props(&snap.get(4).unwrap().props));
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, None, None, |e| e);
        let _static: Element<'static, BridgeEvent> = view_semantic_tree_static_with_editors(
            &snap,
            tokens,
            Some((480.0, 320.0)),
            None,
            None,
            |e| e,
        );
    }

    #[test]
    fn confirm_dialog_props_detect_alertdialog_and_danger() {
        let mut props = WidgetProps {
            role: "alertdialog".into(),
            label: "删除仓库".into(),
            hint: "此操作不可撤销".into(),
            active: true,
            ..WidgetProps::default()
        };
        assert!(is_confirm_dialog_props(&props));
        assert!(!confirm_dialog_danger(&props));
        props.button_kind = ButtonKind::Danger;
        assert!(confirm_dialog_danger(&props));
        props.role.clear();
        props.class_names = vec!["nana-confirm-dialog".into()];
        props.attrs.insert("data-variant".into(), "danger".into());
        assert!(is_confirm_dialog_props(&props));
        assert!(confirm_dialog_danger(&props));
    }

    #[test]
    fn confirm_dialog_semantic_view_builds() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Dialog,
            WidgetProps {
                role: "alertdialog".into(),
                label: "确认删除".into(),
                hint: "删除后无法恢复".into(),
                button_kind: ButtonKind::Danger,
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, None, None, |e| e);
        let _static: Element<'static, BridgeEvent> = view_semantic_tree_static_with_editors(
            &snap,
            tokens,
            Some((480.0, 320.0)),
            None,
            None,
            |e| e,
        );
    }

    #[test]
    fn dialog_select_and_popover_semantic_view() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Select,
            WidgetProps {
                value: "a".into(),
                options: vec![
                    crate::bridge::SelectOptionProp {
                        value: "a".into(),
                        label: "Alpha".into(),
                        disabled: false,
                    },
                    crate::bridge::SelectOptionProp {
                        value: "b".into(),
                        label: "Beta".into(),
                        disabled: false,
                    },
                ],
                ..WidgetProps::default()
            },
        );
        bridge.register(
            3,
            WidgetKind::Dialog,
            WidgetProps {
                label: "Confirm".into(),
                hint: "Body hint".into(),
                active: true,
                ..WidgetProps::default()
            },
        );
        bridge.register(
            4,
            WidgetKind::Popover,
            WidgetProps {
                label: "More".into(),
                toggled: true,
                hint: "Popover body".into(),
                ..WidgetProps::default()
            },
        );
        bridge.register(
            5,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options: vec![crate::bridge::SelectOptionProp {
                    value: "cut".into(),
                    label: "Cut".into(),
                    disabled: false,
                }],
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.insert_child(3, 1, None);
        bridge.insert_child(4, 1, None);
        bridge.insert_child(5, 1, None);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> = view_semantic_tree(&snap, tokens, |e| e);
        assert_eq!(snap.get(2).unwrap().kind, WidgetKind::Select);
        assert_eq!(snap.get(3).unwrap().kind, WidgetKind::Dialog);
        assert_eq!(snap.get(4).unwrap().kind, WidgetKind::Popover);
        assert_eq!(snap.get(5).unwrap().kind, WidgetKind::ContextMenu);
        assert!(WidgetKind::parse("nana-dialog") == Some(WidgetKind::Dialog));
        assert!(
            crate::widget_map::resolve_kind_from_hints("div", None, Some("dialog"), None)
                == Some(WidgetKind::Dialog)
        );
    }

    #[test]
    fn teleport_mount_root_overlay_coexists_with_css_fixed() {
        // X7: Dialog under scaffolded body (Teleport target) + anonymous fixed.
        let mut bridge = MessageBridge::new();
        bridge.ensure_document_roots(1, 2);
        bridge.register(
            3,
            WidgetKind::Dialog,
            WidgetProps {
                label: "Teleport Dialog".into(),
                active: true,
                class_names: vec!["nana-dialog".into()],
                ..WidgetProps::default()
            },
        );
        let mut pin = WidgetProps::default();
        pin.label = "pin".into();
        pin.layout.apply_css_text(
            "position:fixed;top:8px;right:12px;width:40px;height:24px;z-index:2",
            None,
            None,
        );
        bridge.register(4, WidgetKind::Box, pin);
        bridge.insert_child(3, 2, None);
        bridge.insert_child(4, 2, None);

        let snap = bridge.snapshot();
        assert!(snap.get(3).unwrap().kind.is_overlay());
        assert!(
            !is_in_flow_layout(&snap, 3),
            "open Overlay leaves flow and paints on root Nana Overlay stack"
        );
        assert_eq!(
            collect_open_overlay_ids(&snap),
            vec![3],
            "open Dialog collected for viewport overlay stack"
        );
        assert!(!is_in_flow_layout(&snap, 4), "fixed leaves iced flow");
        assert_eq!(collect_css_fixed_ids(&snap), vec![4]);
        assert!(
            !snap.get(3).unwrap().props.layout.is_fixed(),
            "Dialog must not rely on CSS fixed"
        );

        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&snap, tokens, Some((400.0, 300.0)), |e| e);

        bridge.unregister(3);
        let after = bridge.snapshot();
        assert!(after.get(3).is_none(), "Dialog remove must not leak");
        assert!(after.get(4).is_some(), "fixed sibling survives");
        assert_eq!(collect_css_fixed_ids(&after), vec![4]);
        let _after_view: Element<'_, BridgeEvent> =
            view_semantic_tree_static_with_viewport(&after, tokens, Some((400.0, 300.0)), |e| e);
    }

    #[test]
    fn textarea_uses_host_owned_editor_content() {
        use crate::editor_store::EditorStore;
        use iced::widget::text_editor;

        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::Textarea,
            WidgetProps {
                value: "hello\nworld".into(),
                placeholder: "Notes".into(),
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        let mut editors = EditorStore::new();
        editors.ensure(2, "hello\nworld");
        editors.perform(2, text_editor::Action::Edit(text_editor::Edit::Enter));
        assert!(editors.text(2).contains('\n'));
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, Some(&editors), None, |e| e);
        let _static: Element<'static, BridgeEvent> = view_semantic_tree_static_with_editors(
            &snap,
            tokens,
            Some((480.0, 320.0)),
            Some(&editors),
            None,
            |e| e,
        );
    }

    #[test]
    fn context_menu_uses_host_owned_menu_store() {
        use crate::menu_store::MenuStore;

        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options: vec![
                    crate::bridge::SelectOptionProp {
                        value: "file".into(),
                        label: "文件".into(),
                        disabled: false,
                    },
                    crate::bridge::SelectOptionProp {
                        value: "file/rename".into(),
                        label: "重命名".into(),
                        disabled: false,
                    },
                    crate::bridge::SelectOptionProp {
                        value: "file/remove".into(),
                        label: "删除".into(),
                        disabled: false,
                    },
                ],
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        let mut menus = MenuStore::new();
        menus.sync_from_snapshot(&snap);
        assert_eq!(menus.get(2).unwrap().items.len(), 1);
        assert_eq!(menus.get(2).unwrap().items[0].children.len(), 2);
        let remove = &menus.get(2).unwrap().items[0].children[1];
        assert!(remove.danger);
        assert!(remove.confirm_label.is_some());
        assert!(menus.arm_danger_confirm(2, "file/remove"));
        assert_eq!(menus.pending(2), Some("file/remove"));
        assert!(!menus.arm_danger_confirm(2, "file/remove"));
        assert!(menus.pending(2).is_none());
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> =
            view_semantic_tree_with_editors(&snap, tokens, None, None, Some(&menus), |e| e);
    }

    #[test]
    fn surface_paint_from_style_builds_view() {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        let mut props = WidgetProps::default();
        props.class_names = vec!["anon-panel".into()];
        props
            .layout
            .apply_css_property("background-color", "#f0f0f5", None, None);
        props.layout.apply_css_text(
            "border-radius:12px;padding:8px",
            None,
            None,
        );
        assert!(props.layout.background.is_some());
        bridge.register(2, WidgetKind::Column, props);
        bridge.insert_child(2, 1, None);
        let snap = bridge.snapshot();
        let tokens = ThemeMode::Light.tokens();
        let _view: Element<'_, BridgeEvent> = view_semantic_tree(&snap, tokens, |e| e);
    }
}
