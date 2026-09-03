use super::*;
use crate::css_map::JustifySpec;
use nana_js_engine::HostValue;
use std::collections::BTreeSet;

/// Every canonical `as_str` must parse back to its kind. This is the
/// compiler-unreachable seam the macro cannot enforce: a kind row whose
/// `as_str` is not a `parse` key would silently return `None` at runtime.
#[test]
fn kind_table_roundtrips_every_as_str() {
    let mut seen = BTreeSet::new();
    for kind in WidgetKind::ALL {
        assert_eq!(
            WidgetKind::parse(kind.as_str()),
            Some(*kind),
            "as_str {:?} does not parse back",
            kind.as_str()
        );
        assert!(
            seen.insert(kind.as_str()),
            "duplicate as_str {:?}",
            kind.as_str()
        );
    }
}

/// Every `element_tag` must map back to *some* widget kind through the
/// facade's tag resolution (HTML downlevel table or `nana-` prefix parse).
#[test]
fn kind_element_tags_resolve_back_to_a_kind() {
    for kind in WidgetKind::ALL {
        assert!(
            crate::widget_map::resolve_kind_from_hints(kind.element_tag(), None, None, None)
                .is_some(),
            "element_tag {:?} (kind {}) no longer resolves to any WidgetKind",
            kind.element_tag(),
            kind.as_str()
        );
    }
}

/// Every kind's tag must land on a Runtime component unless it is pure
/// facade layout with no Runtime-qualified projection. Keep this list
/// explicit: adding a kind without a Runtime landing spot is a decision,
/// not an accident.
#[test]
fn kind_tags_resolve_in_builtin_runtime_registry() {
    // Vue-only kinds with no Runtime component of their own: the assembly
    // rules in tree.rs downgrade them (Chip → `nana.button` variant, Radio
    // → SegmentedControl radio chrome), so their element_tag intentionally
    // does not resolve on its own.
    const FACADE_ONLY: &[WidgetKind] = &[WidgetKind::Chip, WidgetKind::Radio];
    let context = nana_ui_runtime::AppContext::new();
    let mut unresolved: Vec<String> = Vec::new();
    for kind in WidgetKind::ALL {
        if FACADE_ONLY.contains(kind) {
            continue;
        }
        let resolved = context
            .resolve_component_tag(kind.element_tag())
            .or_else(|| context.resolve_component_tag(kind.as_str()));
        let available =
            nana_ui_runtime::component_descriptors::builtin_component(kind.element_tag())
                .is_none_or(|entry| entry.compiled);
        if available && resolved.is_none() {
            unresolved.push(format!("{} (tag {:?})", kind.as_str(), kind.element_tag()));
        }
    }
    assert!(
        unresolved.is_empty(),
        "kinds without a builtin Runtime landing spot — add them to FACADE_ONLY or register a component: {unresolved:?}"
    );
}

#[test]
fn img_src_cascades_onto_content_image() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "img".into();
    props.attrs.insert("src".into(), "hero.png".into());
    props.inline_style = "object-fit: contain".into();
    bridge.register(3, WidgetKind::Box, props);
    let layout = &bridge.get(3).expect("img").props.layout;
    match &layout.paint.content_image {
        Some(nana_ui_core::BackgroundImage::Url { url, fit, .. }) => {
            assert_eq!(url, "hero.png");
            assert_eq!(*fit, nana_ui_core::BackgroundImageFit::Contain);
        }
        other => panic!("expected img content_image, got {other:?}"),
    }
}

#[test]
fn video_poster_binds_content_image_and_iframe_skips_src() {
    let mut bridge = MessageBridge::new();
    let mut video = WidgetProps::default();
    video.element_tag = "video".into();
    video.attrs.insert("poster".into(), "frame.png".into());
    video.attrs.insert("src".into(), "clip.mp4".into());
    bridge.register(3, WidgetKind::Box, video);
    match &bridge
        .get(3)
        .expect("video")
        .props
        .layout
        .paint
        .content_image
    {
        Some(nana_ui_core::BackgroundImage::Url { url, .. }) => {
            assert_eq!(url, "frame.png");
        }
        other => panic!("expected video poster, got {other:?}"),
    }
    assert!(
        bridge
            .get(3)
            .expect("video")
            .props
            .layout
            .paint
            .skipped_replaced
            .is_none()
    );

    let mut slotted = WidgetProps::default();
    slotted.element_tag = "video".into();
    slotted.attrs.insert("poster".into(), "frame.png".into());
    slotted.attrs.insert("data-nana-video".into(), "7".into());
    bridge.register(6, WidgetKind::Video, slotted);
    let paint = &bridge.get(6).expect("slotted video").props.layout.paint;
    assert!(
        paint.content_image.is_none(),
        "HostTexture video must not also paint poster"
    );
    assert!(paint.skipped_replaced.is_none());

    let mut bare = WidgetProps::default();
    bare.element_tag = "video".into();
    bare.attrs.insert("src".into(), "clip.mp4".into());
    bridge.register(4, WidgetKind::Box, bare);
    let paint = &bridge.get(4).expect("bare video").props.layout.paint;
    assert!(paint.content_image.is_none());
    assert_eq!(paint.skipped_replaced.as_deref(), Some("video"));

    let mut iframe = WidgetProps::default();
    iframe.element_tag = "iframe".into();
    iframe
        .attrs
        .insert("src".into(), "https://example.com".into());
    bridge.register(5, WidgetKind::Box, iframe);
    let paint = &bridge.get(5).expect("iframe").props.layout.paint;
    assert!(
        paint.content_image.is_none(),
        "iframe must not load src as an image"
    );
    assert_eq!(paint.skipped_replaced.as_deref(), Some("iframe"));
}

#[test]
fn canvas_without_slot_is_not_a_2d_bitmap() {
    let mut bridge = MessageBridge::new();
    let mut bare = WidgetProps::default();
    bare.element_tag = "canvas".into();
    bare.attrs.insert("width".into(), "300".into());
    bare.attrs.insert("height".into(), "150".into());
    bare.attrs.insert("src".into(), "frame.png".into());
    bridge.register(3, WidgetKind::Box, bare);
    let paint = &bridge.get(3).expect("bare canvas").props.layout.paint;
    assert!(
        paint.content_image.is_none(),
        "bare <canvas> must not bind src as a 2d bitmap"
    );
    assert_eq!(paint.skipped_replaced.as_deref(), Some("canvas"));

    let mut slotted = WidgetProps::default();
    slotted.element_tag = "canvas".into();
    slotted.attrs.insert("data-nana-canvas".into(), "42".into());
    slotted.attrs.insert("src".into(), "frame.png".into());
    bridge.register(4, WidgetKind::Box, slotted);
    let paint = &bridge.get(4).expect("slotted canvas").props.layout.paint;
    assert!(
        paint.content_image.is_none(),
        "HostTexture canvas must not pretend to be content_image"
    );
    assert!(paint.skipped_replaced.is_none());
}

#[test]
fn measure_layout_boxes_place_row_children() {
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mut row = WidgetProps::default();
    row.layout.apply_css_text(
        "display:flex;flex-direction:row;gap:8px;width:200px;height:40px",
        None,
        None,
    );
    bridge.register(3, WidgetKind::Row, row);
    bridge.insert_child(3, 2, None);
    let mut a = WidgetProps::default();
    a.layout.width = Some(LengthSpec::Px(40.0));
    a.layout.height = Some(LengthSpec::Px(20.0));
    bridge.register(4, WidgetKind::Box, a);
    bridge.insert_child(4, 3, None);
    let mut b = WidgetProps::default();
    b.layout.width = Some(LengthSpec::Px(40.0));
    b.layout.height = Some(LengthSpec::Px(20.0));
    bridge.register(5, WidgetKind::Box, b);
    bridge.insert_child(5, 3, None);

    let root = {
        fn to_node(bridge: &MessageBridge, id: WidgetId) -> Option<crate::LayoutNode> {
            let w = bridge.get(id)?;
            let children = w
                .children
                .iter()
                .filter_map(|&c| to_node(bridge, c))
                .collect();
            Some(crate::LayoutNode::with_children(
                id.to_string(),
                w.props.layout.clone(),
                children,
            ))
        }
        to_node(&bridge, 2).expect("body")
    };
    let boxes: std::collections::BTreeMap<_, _> = crate::measure_layout(&root, 400.0, 300.0)
        .into_iter()
        .collect();
    let a = boxes.get("4").expect("a");
    let b = boxes.get("5").expect("b");
    assert!(
        (b.x - a.x - a.width - 8.0).abs() < 0.5,
        "row gap should separate children"
    );
}

#[test]
fn display_block_does_not_rewrite_row_widget_kind() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());

    bridge.patch_prop(1, "flex-direction", &HostValue::string("row"));
    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("display:block;flex-direction:row"),
    );

    let widget = bridge.get(1).expect("row widget");
    assert_eq!(widget.kind, WidgetKind::Row);
    assert_eq!(
        widget.props.layout.display,
        Some(nana_ui_core::DisplaySpec::Block)
    );
}

#[test]
fn create_button_and_press_queues_event() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        10,
        WidgetKind::Button,
        WidgetProps {
            label: "Increment".into(),
            button_kind: ButtonKind::Primary,
            ..WidgetProps::default()
        },
    );
    let events = bridge.note_press(10);
    assert_eq!(events, vec!["press", "click"]);
    let pending = bridge.drain_events();
    assert_eq!(pending, vec![BridgeEvent::Press { id: 10 }]);
}

#[test]
fn column_insert_builds_tree_snapshot() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            label: "0".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Button,
        WidgetProps {
            label: "inc".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 1, None);
    let snap = bridge.snapshot();
    assert_eq!(snap.roots, vec![1]);
    assert_eq!(snap.widgets[0].id, 1);
    assert_eq!(snap.widgets[1].props.label, "0");
    assert_eq!(snap.widgets[2].kind, WidgetKind::Button);
}

#[test]
fn props_from_map_parses_button() {
    let mut map = BTreeMap::new();
    map.insert("label".into(), HostValue::string("Go"));
    map.insert("kind".into(), HostValue::string("primary"));
    map.insert("disabled".into(), HostValue::Bool(true));
    let props = WidgetProps::from_map(&map);
    assert_eq!(props.label, "Go");
    assert_eq!(props.button_kind, ButtonKind::Primary);
    assert!(props.disabled);
}

#[test]
fn theme_inject_bumps_revision() {
    let mut bridge = MessageBridge::new();
    let r0 = bridge.revision();
    bridge.set_theme(ThemeMode::Dark);
    assert_eq!(bridge.theme(), ThemeMode::Dark);
    assert!(bridge.revision() > r0);
    assert_eq!(bridge.theme_label(), "dark");
}

#[test]
fn inject_stylesheet_recascades_matching_subtree_only() {
    let mut bridge = MessageBridge::new();
    let mut parent = WidgetProps::default();
    parent.class_names = vec!["scope".into()];
    parent.element_tag = "div".into();
    bridge.register(1, WidgetKind::Column, parent);
    let mut child = WidgetProps::default();
    child.class_names = vec!["leaf".into()];
    child.element_tag = "span".into();
    bridge.register(2, WidgetKind::Row, child);
    bridge.insert_child(2, 1, None);
    let mut other = WidgetProps::default();
    other.class_names = vec!["unrelated".into()];
    other.element_tag = "div".into();
    bridge.register(3, WidgetKind::Column, other);

    bridge.inject_stylesheet(
        r#"
            .scope { --gap-size: 10px; }
            .leaf { gap: var(--gap-size); }
            .unrelated { gap: 40px; }
            "#,
    );
    assert_eq!(
        bridge.get(2).unwrap().props.layout.gap,
        Some(LengthSpec::Px(10.0))
    );
    assert_eq!(
        bridge.get(3).unwrap().props.layout.gap,
        Some(LengthSpec::Px(40.0))
    );

    let r0 = bridge.revision();
    bridge.inject_stylesheet(".nope { gap: 99px; color: red; }");
    assert_eq!(bridge.revision(), r0);
    assert_eq!(
        bridge.get(3).unwrap().props.layout.gap,
        Some(LengthSpec::Px(40.0))
    );

    bridge.inject_stylesheet(".scope { --gap-size: 24px; }");
    assert_eq!(
        bridge.get(2).unwrap().props.layout.gap,
        Some(LengthSpec::Px(24.0))
    );
}

#[test]
fn direction_rtl_parent_style_recascades_child_logical() {
    let mut bridge = MessageBridge::new();
    let mut parent = WidgetProps::default();
    parent.element_tag = "div".into();
    bridge.register(1, WidgetKind::Column, parent);
    let mut child = WidgetProps::default();
    child.element_tag = "div".into();
    child.class_names = vec!["box".into()];
    bridge.register(2, WidgetKind::Box, child);
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".box { padding-inline-start: 12px; }");
    assert_eq!(
        bridge.get(2).unwrap().props.layout.padding_left,
        Some(LengthSpec::Px(12.0))
    );
    assert!(bridge.get(2).unwrap().props.layout.padding_right.is_none());

    bridge.patch_prop(1, "style", &HostValue::string("direction: rtl"));
    let child_layout = &bridge.get(2).unwrap().props.layout;
    assert_eq!(child_layout.dir, Some(crate::css_map::DirSpec::Rtl));
    assert_eq!(child_layout.padding_right, Some(LengthSpec::Px(12.0)));
    assert!(child_layout.padding_left.is_none());
}

#[test]
fn html_dir_rtl_host_attr_remaps_stylesheet_padding_inline_start() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "div".into();
    props.class_names = vec!["box".into()];
    props.attrs.insert("dir".into(), "rtl".into());
    bridge.register(1, WidgetKind::Box, props);
    bridge.inject_stylesheet(".box { padding-inline-start: 12px; }");
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.dir, Some(crate::css_map::DirSpec::Rtl));
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
    assert!(layout.padding_left.is_none());
    assert!(!layout.flex_reverse);
}

#[test]
fn html_dir_rtl_patch_prop_remaps_stylesheet_logical() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "div".into();
    props.class_names = vec!["box".into()];
    bridge.register(1, WidgetKind::Box, props);
    bridge.inject_stylesheet(".box { padding-inline-start: 12px; }");
    assert_eq!(
        bridge.get(1).unwrap().props.layout.padding_left,
        Some(LengthSpec::Px(12.0))
    );

    bridge.patch_prop(1, "dir", &HostValue::string("rtl"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.dir, Some(crate::css_map::DirSpec::Rtl));
    assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
    assert!(layout.padding_left.is_none());
}

#[test]
fn html_dir_auto_fail_closed_does_not_fake_ltr_or_rtl() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "div".into();
    props.class_names = vec!["box".into()];
    props.attrs.insert("dir".into(), "auto".into());
    bridge.register(1, WidgetKind::Box, props);
    bridge.inject_stylesheet(".box { padding-inline-start: 12px; }");
    let layout = &bridge.get(1).unwrap().props.layout;
    assert!(layout.dir.is_none());
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
    assert!(layout.padding_right.is_none());
}

#[test]
fn html_dir_loses_to_author_css_direction() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "div".into();
    props.class_names = vec!["box".into()];
    props.attrs.insert("dir".into(), "rtl".into());
    bridge.register(1, WidgetKind::Box, props);
    bridge.inject_stylesheet(".box { direction: ltr; padding-inline-start: 12px; }");
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.dir, Some(crate::css_map::DirSpec::Ltr));
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(12.0)));
    assert!(layout.padding_right.is_none());
}

#[test]
fn document_appearance_syncs_backdrop_fields() {
    let mut bridge = MessageBridge::new();
    let mut dataset = BTreeMap::new();
    dataset.insert("backdrop".into(), "translucent".into());
    dataset.insert("backdropTarget".into(), "main".into());
    dataset.insert("titlebarFollowsSidebar".into(), "false".into());
    let mut style = BTreeMap::new();
    style.insert("--nana-backdrop-opacity".into(), "0.5".into());
    bridge.apply_document_appearance(&dataset, &style);
    let appearance = bridge.appearance();
    assert_eq!(
        appearance.window_material(),
        WindowMaterialMode::Translucent
    );
    assert_eq!(appearance.backdrop_target(), BackdropTarget::Main);
    assert!(!appearance.titlebar_follows_sidebar());
    assert!((appearance.backdrop_opacity() - 0.5).abs() < f32::EPSILON);
    let snap = bridge.snapshot();
    assert_eq!(snap.appearance.backdrop_target(), BackdropTarget::Main);
    dataset.insert("backdrop".into(), "mica".into());
    bridge.apply_document_appearance(&dataset, &style);
    assert_eq!(
        bridge.appearance().window_material(),
        WindowMaterialMode::Mica
    );

    style.clear();
    style.insert("--lilia-backdrop-opacity".into(), "0.4".into());
    bridge.apply_document_appearance(&dataset, &style);
    assert!(
        (bridge.appearance().backdrop_opacity() - 0.4).abs() < f32::EPSILON,
        "Lilia CSS variable must write Appearance opacity"
    );
}

#[test]
fn document_appearance_syncs_theme_from_dataset() {
    let mut bridge = MessageBridge::new();
    assert_eq!(bridge.theme(), ThemeMode::Light);
    let mut dataset = BTreeMap::new();
    dataset.insert("theme".into(), "dark".into());
    bridge.apply_document_appearance(&dataset, &BTreeMap::new());
    assert_eq!(bridge.theme(), ThemeMode::Dark);
    assert_eq!(bridge.snapshot().theme, ThemeMode::Dark);
    dataset.insert("theme".into(), "light".into());
    bridge.apply_document_appearance(&dataset, &BTreeMap::new());
    assert_eq!(bridge.theme(), ThemeMode::Light);
}

#[test]
fn theme_change_reapplies_var_bg_from_data_theme_rules() {
    // Primary content uses `background: var(--bg)`. Document vars must track
    // theme — not stick on the light overlay from blind last-wins merge.
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["surface".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        r#"
            :root { --bg: #181818; }
            :root[data-theme="light"] { --bg: #ffffff; }
            .surface { background: var(--bg); width: 100px; height: 40px; }
            "#,
    );
    let light_bg = bridge.get(1).unwrap().props.layout.background;
    assert_eq!(
        light_bg,
        Some([1.0, 1.0, 1.0, 1.0]),
        "default ThemeMode::Light must resolve light --bg"
    );

    let mut dataset = BTreeMap::new();
    dataset.insert("theme".into(), "dark".into());
    bridge.apply_document_appearance(&dataset, &BTreeMap::new());
    let dark_bg = bridge.get(1).unwrap().props.layout.background;
    assert_eq!(
        dark_bg,
        Some([24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0]),
        "dark theme must drop light overlay and keep :root --bg"
    );

    dataset.insert("theme".into(), "light".into());
    bridge.apply_document_appearance(&dataset, &BTreeMap::new());
    assert_eq!(
        bridge.get(1).unwrap().props.layout.background,
        Some([1.0, 1.0, 1.0, 1.0])
    );
}

#[test]
fn lightningcss_companion_tokens_paint_light_under_html_scaffold() {
    // Real LiliaGithub companion shape under html/body scaffold (orphans
    // previously rematched bare :root and stuck on #181818/#202020).
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    assert_eq!(
        bridge
            .get(1)
            .unwrap()
            .props
            .attrs
            .get("data-theme")
            .map(String::as_str),
        Some("light")
    );
    let mut props = WidgetProps::default();
    props.class_names = vec!["surface".into()];
    props.element_tag = "div".into();
    bridge.register(3, WidgetKind::Column, props);
    bridge.insert_child(3, 2, None);
    bridge.inject_stylesheet(
        r#"
            :root{--bg:#181818;--bg-elev:#202020}
            @supports (color:lab(0% 0 0)){:root{--bg:lab(8.244% 0 0);--bg-elev:lab(12% 0 0)}}
            :root[data-theme=light]{--bg:#fff;--bg-elev:#f3f4f6}
            @supports (color:lab(0% 0 0)){:root[data-theme=light]{--bg:lab(100% 0 0)}}
            .surface{background:var(--bg);width:100px;height:40px}
            .raised{background:var(--bg-elev);width:100px;height:40px}
            "#,
    );
    assert_eq!(
        bridge.get(3).unwrap().props.layout.background,
        Some([1.0, 1.0, 1.0, 1.0]),
        "scaffold + lightningcss light tokens must paint white --bg"
    );
    let mut raised = WidgetProps::default();
    raised.class_names = vec!["raised".into()];
    raised.element_tag = "div".into();
    bridge.register(4, WidgetKind::Column, raised);
    bridge.insert_child(4, 2, None);
    // Class may arrive after register in Vue; re-inject is the host path that
    // rebuilds cascade for late nodes — here a no-op stylesheet bump.
    bridge.inject_stylesheet(".raised{background:var(--bg-elev)}");
    assert_eq!(
        bridge.get(4).unwrap().props.layout.background,
        Some([243.0 / 255.0, 244.0 / 255.0, 246.0 / 255.0, 1.0]),
        "light --bg-elev must resolve (not dark #202020)"
    );
}

#[cfg(feature = "scene-view")]
#[test]
fn snapshot_theme_tokens_honor_backdrop_and_titlebar_follow() {
    use crate::theme_tokens_from_snapshot;

    let mut bridge = MessageBridge::new();
    let mut dataset = BTreeMap::new();
    dataset.insert("backdrop".into(), "translucent".into());
    dataset.insert("backdropTarget".into(), "sidebar".into());
    dataset.insert("titlebarFollowsSidebar".into(), "false".into());
    let mut style = BTreeMap::new();
    style.insert("--nana-backdrop-opacity".into(), "0.5".into());
    bridge.apply_document_appearance(&dataset, &style);
    let snap = bridge.snapshot();
    let tokens = theme_tokens_from_snapshot(&snap, true);
    assert!((tokens.colors.surface.a - 0.5).abs() < f32::EPSILON);
    assert!((tokens.titlebar.a - 1.0).abs() < f32::EPSILON);
    assert!((tokens.colors.background.a - 1.0).abs() < f32::EPSILON);
}

#[test]
fn stylesheet_cascade_drives_anonymous_class_layout() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
            r#"
            .anon-shell { display:grid; grid-template-rows:minmax(0,1fr); height:100%; width:100%; overflow:hidden; }
            .anon-grid { display:flex; flex-direction:row; flex-wrap:wrap; gap:12px; height:100%; }
            .anon-card { padding:12px; border-radius:16px; }
            "#,
        );
    let mut shell = WidgetProps::default();
    shell.element_tag = "div".into();
    bridge.register(1, WidgetKind::Column, shell);
    bridge.patch_prop(1, "class", &HostValue::string("anon-shell"));
    let shell_layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(shell_layout.height, Some(LengthSpec::Fill));
    assert_eq!(shell_layout.width, Some(LengthSpec::Fill));
    assert!(
        shell_layout
            .grid_rows
            .as_ref()
            .is_some_and(|r| !r.is_empty())
    );

    let mut grid = WidgetProps::default();
    grid.element_tag = "div".into();
    bridge.register(2, WidgetKind::Column, grid);
    bridge.patch_prop(2, "class", &HostValue::string("anon-grid"));
    let grid_layout = &bridge.get(2).unwrap().props.layout;
    assert_eq!(grid_layout.direction, Some(FlexDirection::Row));
    assert_eq!(grid_layout.gap, Some(LengthSpec::Px(12.0)));
    assert_eq!(grid_layout.height, Some(LengthSpec::Fill));
    assert_eq!(
        bridge.get(2).unwrap().kind,
        WidgetKind::Column,
        "CSS flex-direction must not rewrite WidgetKind"
    );

    // Late stylesheet injection re-applies onto existing nodes.
    bridge.inject_stylesheet(".anon-shell { padding: 20px; }");
    assert_eq!(
        bridge.get(1).unwrap().props.layout.padding,
        Some(LengthSpec::Px(20.0))
    );
}

#[test]
fn html_and_class_downlevel_to_foundations() {
    assert_eq!(
        resolve_kind_from_hints("div", None, None, None),
        Some(WidgetKind::Column)
    );
    assert_eq!(
        resolve_kind_from_hints("button", None, None, None),
        Some(WidgetKind::Button)
    );
    assert_eq!(
        resolve_kind_from_hints("input", None, None, Some("checkbox")),
        Some(WidgetKind::Checkbox)
    );
    assert_eq!(
        resolve_kind_from_hints("div", Some("nana-tabs"), Some("tablist"), None),
        Some(WidgetKind::Tabs)
    );
    assert_eq!(
        resolve_kind_from_hints("button", Some("nana-chip is-selected"), None, None),
        Some(WidgetKind::Chip)
    );
    assert_eq!(
        resolve_kind_from_hints("li", None, None, None),
        Some(WidgetKind::ListItem)
    );
}

#[test]
fn class_patch_upgrades_layout_to_chip() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "class", &HostValue::string("nana-chip"));
    assert_eq!(bridge.get(1).unwrap().kind, WidgetKind::Chip);
}

#[test]
fn class_patch_does_not_demote_button_to_column() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.label = "搜索".into();
    bridge.register(1, WidgetKind::Button, props);
    // Re-resolve uses tag "div"; anonymous toolbar class must not demote Button.
    bridge.patch_prop(1, "class", &HostValue::string("anon-toolbar-btn"));
    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("width:32px;height:32px;border-radius:6px"),
    );
    assert_eq!(bridge.get(1).unwrap().kind, WidgetKind::Button);
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(32.0))
    );
}

#[test]
fn style_patch_sets_gap_padding_and_row_direction() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("display:flex; flex-direction:row; gap:12px; padding:8px; width:100%"),
    );
    let w = bridge.get(1).unwrap();
    assert_eq!(w.kind, WidgetKind::Column);
    assert_eq!(w.props.layout.direction, Some(FlexDirection::Row));
    assert_eq!(w.props.layout.gap, Some(LengthSpec::Px(12.0)));
    assert_eq!(w.props.layout.padding, Some(LengthSpec::Px(8.0)));
    assert_eq!(w.props.layout.width, Some(LengthSpec::Fill));
}

#[test]
fn padding_prop_and_style_preserve_percent_without_containing_block() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "padding", &HostValue::string("10%"));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.padding,
        Some(LengthSpec::Percent(10.0)),
        "padding prop must not drop % when percent base is unknown"
    );
    bridge.patch_prop(1, "style", &HostValue::string("margin:5%;padding-left:8%"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.margin, Some(LengthSpec::Percent(5.0)));
    assert_eq!(layout.padding_left, Some(LengthSpec::Percent(8.0)));
    let pad = layout.resolved_padding_against(Some(200.0));
    let margin = layout.resolved_margin_against(Some(200.0));
    assert_eq!(pad.left, 16.0);
    assert_eq!(margin.top, 10.0);
}

#[test]
fn gap_prop_clears_row_and_column_gap_longhands() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "style", &HostValue::string("gap: 8px 20px"));
    {
        let layout = &bridge.get(1).unwrap().props.layout;
        assert!(layout.gap.is_none());
        assert_eq!(layout.row_gap, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.column_gap, Some(LengthSpec::Px(20.0)));
    }
    // Uniform `gap` prop must reset axis longhands (CSS shorthand cascade).
    bridge.patch_prop(1, "gap", &HostValue::string("12px"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.gap, Some(LengthSpec::Px(12.0)));
    assert!(layout.row_gap.is_none(), "row-gap longhand cleared");
    assert!(layout.column_gap.is_none(), "column-gap longhand cleared");
    assert_eq!(layout.resolved_row_gap(), 12.0);
    assert_eq!(layout.resolved_column_gap(), 12.0);

    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("row-gap: 4px; column-gap: 6px"),
    );
    bridge.patch_prop(1, "gap", &HostValue::Number(10.0));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.gap, Some(LengthSpec::Px(10.0)));
    assert!(layout.row_gap.is_none());
    assert!(layout.column_gap.is_none());
}

#[test]
fn gap_percent_survives_until_layout_resolve_after_cb_sync() {
    // Early style/% gap before any CB write — must not drop or freeze wrong px.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "gap", &HostValue::string("10%"));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.gap,
        Some(LengthSpec::Percent(10.0)),
        "gap prop must keep % when CB is unknown"
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.resolved_column_gap(),
        0.0,
        "without CB, % gap does not invent px"
    );

    bridge.patch_prop(1, "style", &HostValue::string("gap:8%"));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.gap,
        Some(LengthSpec::Percent(8.0))
    );

    // Later CB sync: same LengthSpec re-resolves (no style re-patch required).
    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(250.0, 100.0));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.gap, Some(LengthSpec::Percent(8.0)));
    assert_eq!(
        layout.resolved_column_gap_against(bridge.get(1).unwrap().props.containing_block_width),
        20.0
    );
}

#[test]
fn style_patch_uses_parent_content_width_as_percent_base() {
    let mut bridge = MessageBridge::new();
    let mut parent = WidgetProps::default();
    parent.layout.width = Some(LengthSpec::Px(200.0));
    parent.layout.padding = Some(LengthSpec::Px(0.0));
    bridge.register(1, WidgetKind::Column, parent);
    bridge.register(2, WidgetKind::Box, WidgetProps::default());
    bridge.insert_child(2, 1, None);
    assert_eq!(
        bridge.get(2).unwrap().props.containing_block_width,
        Some(200.0),
        "insert_child syncs child CB from parent content width"
    );
    bridge.patch_prop(
        2,
        "style",
        &HostValue::string("margin:10%;padding:5%;gap:10%"),
    );
    let child = &bridge.get(2).unwrap().props;
    assert_eq!(child.layout.margin, Some(LengthSpec::Percent(10.0)));
    assert_eq!(child.layout.padding, Some(LengthSpec::Percent(5.0)));
    assert_eq!(
        child.layout.gap,
        Some(LengthSpec::Percent(10.0)),
        "gap % must stay LengthSpec like margin/padding (not eager px)"
    );
    let base = child.containing_block_width;
    assert_eq!(child.layout.resolved_margin_against(base).top, 20.0);
    assert_eq!(child.layout.resolved_padding_against(base).left, 10.0);
    assert_eq!(
        child.layout.resolved_column_gap_against(base),
        20.0,
        "gap % resolves against CB at layout time"
    );
}

#[test]
fn set_containing_block_feeds_style_percent_base() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.set_containing_block(1, Some(400.0), None);
    bridge.patch_prop(1, "style", &HostValue::string("margin-top:10%;gap:5%"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.margin_top, Some(LengthSpec::Percent(10.0)));
    assert_eq!(layout.gap, Some(LengthSpec::Percent(5.0)));
    assert_eq!(layout.resolved_margin_against(Some(400.0)).top, 40.0);
    assert_eq!(layout.resolved_column_gap_against(Some(400.0)), 20.0);
}

#[test]
fn sync_layout_containing_blocks_fill_parent_chain() {
    let mut bridge = MessageBridge::new();
    let mut shell = WidgetProps::default();
    shell.layout.width = Some(LengthSpec::Fill);
    shell.layout.height = Some(LengthSpec::Fill);
    shell.layout.padding = Some(LengthSpec::Px(20.0));
    bridge.register(1, WidgetKind::Column, shell);
    bridge.register(2, WidgetKind::Box, WidgetProps::default());
    bridge.insert_child(2, 1, None);

    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(400.0, 300.0));
    assert_eq!(
        bridge.get(1).unwrap().props.containing_block_width,
        Some(400.0),
        "root CB = viewport"
    );
    assert_eq!(
        bridge.get(2).unwrap().props.containing_block_width,
        Some(360.0),
        "Fill parent content = viewport − padding"
    );
    assert_eq!(
        bridge.get(2).unwrap().props.containing_block_height,
        Some(260.0)
    );

    // Next style patch on child uses Fill-chain CB for gap %.
    bridge.patch_prop(2, "style", &HostValue::string("margin:10%;gap:10%"));
    let child = &bridge.get(2).unwrap().props;
    assert_eq!(child.layout.margin, Some(LengthSpec::Percent(10.0)));
    assert_eq!(child.layout.gap, Some(LengthSpec::Percent(10.0)));
    assert_eq!(
        child
            .layout
            .resolved_margin_against(child.containing_block_width)
            .top,
        36.0
    );
    assert_eq!(
        child
            .layout
            .resolved_column_gap_against(child.containing_block_width),
        36.0
    );
}

#[test]
fn sync_layout_containing_blocks_nested_fill_percent_padding() {
    // viewport 500×400
    // A Fill pad 20px → content 460×360
    // B Fill pad 10% (of 460) → pad 46 → content 368×268
    // C CB = 368×268; gap 10% → 36.8
    let mut bridge = MessageBridge::new();
    let mut a = WidgetProps::default();
    a.layout.width = Some(LengthSpec::Fill);
    a.layout.height = Some(LengthSpec::Fill);
    a.layout.padding = Some(LengthSpec::Px(20.0));
    bridge.register(1, WidgetKind::Column, a);

    let mut b = WidgetProps::default();
    b.layout.width = Some(LengthSpec::Fill);
    b.layout.height = Some(LengthSpec::Fill);
    b.layout.padding = Some(LengthSpec::Percent(10.0));
    bridge.register(2, WidgetKind::Column, b);
    bridge.insert_child(2, 1, None);

    bridge.register(3, WidgetKind::Box, WidgetProps::default());
    bridge.insert_child(3, 2, None);

    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(500.0, 400.0));
    assert_eq!(
        bridge.get(1).unwrap().props.containing_block_width,
        Some(500.0)
    );
    assert_eq!(
        bridge.get(2).unwrap().props.containing_block_width,
        Some(460.0)
    );
    assert_eq!(
        bridge.get(2).unwrap().props.containing_block_height,
        Some(360.0)
    );
    assert_eq!(
        bridge.get(3).unwrap().props.containing_block_width,
        Some(368.0),
        "second Fill level subtracts % padding of mid CB"
    );
    assert_eq!(
        bridge.get(3).unwrap().props.containing_block_height,
        Some(268.0)
    );

    bridge.patch_prop(3, "style", &HostValue::string("margin:10%;gap:10%"));
    let leaf = &bridge.get(3).unwrap().props;
    assert_eq!(leaf.layout.gap, Some(LengthSpec::Percent(10.0)));
    assert!(
        (leaf
            .layout
            .resolved_column_gap_against(leaf.containing_block_width)
            - 36.8)
            .abs()
            < 0.01
    );
    assert!(
        (leaf
            .layout
            .resolved_margin_against(leaf.containing_block_width)
            .top
            - 36.8)
            .abs()
            < 0.01
    );
}

#[test]
fn style_patch_justify_space_between_and_flex_grow() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("justify-content:space-between; align-items:center"),
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.justify_content,
        JustifySpec::SpaceBetween
    );
    bridge.register(2, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(
        2,
        "style",
        &HostValue::string("flex:1; min-width:0; overflow-y:auto"),
    );
    let child = &bridge.get(2).unwrap().props.layout;
    assert_eq!(child.flex_grow, Some(1.0));
    assert!(child.allow_shrink);
    assert!(child.scrolls_y());
}

#[test]
fn nana_settings_row_class_maps_layout() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "class", &HostValue::string("nana-settings-row"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.direction, Some(FlexDirection::Row));
    assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
    assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
}

#[test]
fn prop_style_does_not_break_nana_settings_row_contract() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "class", &HostValue::string("nana-settings-row"));
    // Vue layout props land in prop_style and would otherwise wipe the row
    // contract if class hints were not re-applied after prop_style.
    bridge.patch_prop(1, "flex-direction", &HostValue::string("column"));
    bridge.patch_prop(1, "gap", &HostValue::Number(4.0));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.direction, Some(FlexDirection::Row));
    assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
    assert_eq!(layout.justify_content, JustifySpec::SpaceBetween);
    assert!(
        !bridge.get(1).unwrap().props.prop_style.is_empty(),
        "layout props must still record prop_style"
    );
    // Full cascade rebuild (e.g. late stylesheet) must keep the same contract.
    bridge.inject_stylesheet(".unused-rule { color: red; }");
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.direction, Some(FlexDirection::Row));
    assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
}

#[test]
fn incremental_layout_prop_preserves_stylesheet_important() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.class_names = vec!["sized".into(), "min-w-0".into()];
    props.element_tag = "div".into();
    bridge.register(1, WidgetKind::Column, props);
    bridge.inject_stylesheet(
        ".sized { width: 80px !important; height: 40px !important; min-width: 72px !important; }",
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(80.0))
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.height,
        Some(LengthSpec::Px(40.0))
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.min_width,
        Some(LengthSpec::Px(72.0)),
        "stylesheet min-width !important must beat min-w-0 hint"
    );

    // Ordinary layout props must not drop the important tail.
    bridge.patch_prop(1, "width", &HostValue::string("200px"));
    bridge.patch_prop(1, "height", &HostValue::string("90px"));
    bridge.patch_prop(1, "min-width", &HostValue::string("10px"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(
        layout.width,
        Some(LengthSpec::Px(80.0)),
        "stylesheet width !important must survive patchProp(width)"
    );
    assert_eq!(
        layout.height,
        Some(LengthSpec::Px(40.0)),
        "stylesheet height !important must survive patchProp(height)"
    );
    assert_eq!(
        layout.min_width,
        Some(LengthSpec::Px(72.0)),
        "stylesheet min-width !important must survive patchProp(min-width) and min-w-0"
    );

    // Prop important still beats stylesheet important after the same rebuild.
    bridge.patch_prop(1, "width", &HostValue::string("200px !important"));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(200.0)),
        "prop !important must beat stylesheet !important"
    );
}

#[test]
fn incremental_layout_prop_keeps_inline_important() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.class_names = vec!["titlebar".into()];
    props.element_tag = "div".into();
    bridge.register(1, WidgetKind::Row, props);
    bridge.patch_prop(1, "style", &HostValue::string("height: 80px !important"));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.height,
        Some(LengthSpec::Px(80.0)),
        "inline height !important must beat titlebar hint 36"
    );

    // A later width prop must re-run the important tail, not let the hint win.
    bridge.patch_prop(1, "width", &HostValue::string("200px"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(
        layout.height,
        Some(LengthSpec::Px(80.0)),
        "inline height !important must survive patchProp(width) after titlebar hints"
    );
    assert_eq!(layout.width, Some(LengthSpec::Px(200.0)));
}

#[test]
fn inline_custom_prop_important_resolves_as_layout_length() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(
        1,
        "style",
        &HostValue::string("--w: 80px !important; width: calc(var(--w) + 10px); height: 40px"),
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(90.0)),
        "inline --w with !important must strip so calc(var(--w) + 10px) is 90"
    );
}

#[test]
fn custom_property_inheritance_from_parent_scope() {
    // Parent sets --row-h; child uses var(--row-h) without defining it.
    // Class-scoped props must not pollute the document flat map.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "class", &HostValue::string("menu"));
    let mut child = WidgetProps::default();
    child.class_names = vec!["item".into()];
    bridge.register(2, WidgetKind::Row, child);
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(
        r#"
            :root { --pad: 4px; }
            .other { --row-h: 99px; }
            .menu { --row-h: 28px; gap: var(--pad); }
            .item { height: var(--row-h); width: var(--missing, 40px); }
            "#,
    );
    let menu = &bridge.get(1).unwrap().props.layout;
    assert_eq!(menu.gap, Some(LengthSpec::Px(4.0)));
    let item = &bridge.get(2).unwrap().props.layout;
    assert_eq!(
        item.height,
        Some(LengthSpec::Px(28.0)),
        "child must inherit parent --row-h, not .other's 99px"
    );
    assert_eq!(item.width, Some(LengthSpec::Px(40.0)));
}

#[test]
fn unrelated_stylesheet_keeps_mount_root_fill_contract() {
    // inject_stylesheet must not break the viewport → mount → % height chain
    // by clearing scaffold Fill without a public class contract to restore it.
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mount = &bridge.get(2).unwrap().props;
    assert!(
        mount.class_names.iter().any(|c| c == "nana-mount-root"),
        "scaffold must expose nana-mount-root"
    );
    assert_eq!(mount.layout.width, Some(LengthSpec::Fill));
    assert_eq!(mount.layout.height, Some(LengthSpec::Fill));

    bridge.inject_stylesheet(".unrelated { color: red; padding: 4px; }");
    let mount = &bridge.get(2).unwrap().props.layout;
    assert_eq!(
        mount.width,
        Some(LengthSpec::Fill),
        "mount-root width Fill must survive unrelated stylesheet"
    );
    assert_eq!(
        mount.height,
        Some(LengthSpec::Fill),
        "mount-root height Fill must survive unrelated stylesheet"
    );
    let html = &bridge.get(1).unwrap().props.layout;
    assert_eq!(html.width, Some(LengthSpec::Fill));
    assert_eq!(html.height, Some(LengthSpec::Fill));
}

#[test]
fn sidebar_frame_nana_tag_applies_contract_width() {
    // Custom-element tag `nana-sidebar-frame` mirrors the public class contract.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::SidebarFrame, WidgetProps::default());
    assert_eq!(
        bridge.get(1).unwrap().props.element_tag,
        "nana-sidebar-frame"
    );
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(220.0)),
        "nana-sidebar-frame tag must apply class-contract width"
    );
}

#[test]
fn sidebar_frame_non_contract_tag_does_not_invent_width() {
    // WidgetKind::SidebarFrame alone (foreign/div tag) must not invent 220px.
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "div".into();
    bridge.register(1, WidgetKind::SidebarFrame, props);
    assert!(
        bridge.get(1).unwrap().props.layout.width.is_none(),
        "non-contract tag must not invent SidebarFrame width"
    );

    // Orphan reparent with a foreign tag stays honest.
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mut row = WidgetProps::default();
    row.class_names = vec!["nana-workspace-shell__body".into()];
    bridge.register(3, WidgetKind::Row, row);
    bridge.insert_child(3, 2, None);
    let mut orphan = WidgetProps::default();
    orphan.element_tag = "div".into();
    bridge.register(4, WidgetKind::SidebarFrame, orphan);
    assert!(!bridge.get(3).unwrap().children.contains(&4));
    bridge.reparent_orphans();
    assert!(
        bridge.get(3).unwrap().children.contains(&4),
        "orphan SidebarFrame should reparent under workspace row"
    );
    assert!(
        bridge.get(4).unwrap().props.layout.width.is_none(),
        "reparent_orphans must not invent width for non-contract tags"
    );
}

#[test]
fn reparent_orphans_ignores_bare_flex_row() {
    // A random flex-row without workspace/resources identity must not host
    // orphan sidebars.
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mut row = WidgetProps::default();
    row.class_names = vec!["flex-row".into()];
    bridge.register(3, WidgetKind::Row, row);
    bridge.insert_child(3, 2, None);
    let mut orphan = WidgetProps::default();
    orphan.element_tag = "nana-sidebar-frame".into();
    bridge.register(4, WidgetKind::SidebarFrame, orphan);
    bridge.reparent_orphans();
    assert!(
        !bridge.get(3).unwrap().children.contains(&4),
        "bare flex-row must not receive orphan SidebarFrame"
    );
}

#[test]
fn reparent_sidebar_footer_slot_under_live_frame() {
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mut body = WidgetProps::default();
    body.class_names = vec!["nana-workspace-shell__body".into(), "flex-row".into()];
    bridge.register(3, WidgetKind::Row, body);
    bridge.insert_child(3, 2, None);
    let mut frame = WidgetProps::default();
    frame.element_tag = "nana-sidebar-frame".into();
    frame.class_names = vec!["nana-sidebar-frame".into()];
    frame.agent_id = "sidebar".into();
    bridge.register(4, WidgetKind::SidebarFrame, frame);
    bridge.insert_child(4, 3, None);
    let mut top = WidgetProps::default();
    top.class_names = vec!["nana-sidebar-frame__top".into()];
    top.attrs.insert("data-slot".into(), "sidebar-top".into());
    bridge.register(5, WidgetKind::Column, top);
    bridge.insert_child(5, 4, None);
    let mut body_slot = WidgetProps::default();
    body_slot.class_names = vec!["nana-sidebar-frame__body".into()];
    body_slot
        .attrs
        .insert("data-slot".into(), "sidebar-body".into());
    bridge.register(6, WidgetKind::Column, body_slot);
    bridge.insert_child(6, 4, None);
    // Orphan footer slot + content (simulates remount detach-then-fail).
    let mut footer = WidgetProps::default();
    footer.class_names = vec!["nana-sidebar-frame__footer".into()];
    footer
        .attrs
        .insert("data-slot".into(), "sidebar-footer".into());
    bridge.register(7, WidgetKind::Column, footer);
    let mut content = WidgetProps::default();
    content.class_names = vec!["sb-footer".into()];
    bridge.register(8, WidgetKind::Column, content);
    let mut settings = WidgetProps::default();
    settings.agent_id = "sidebar.footer.settings".into();
    settings.class_names = vec!["sb-footer__btn".into()];
    bridge.register(9, WidgetKind::Row, settings);
    bridge.insert_child(9, 8, None);
    assert_eq!(bridge.get(4).unwrap().children.len(), 2);
    bridge.reparent_orphans();
    let frame = bridge.get(4).unwrap();
    assert!(
        frame.children.contains(&7),
        "footer slot must reattach under live SidebarFrame: {:?}",
        frame.children
    );
    assert!(
        bridge.get(7).unwrap().children.contains(&8),
        "footer content must reattach under footer slot"
    );
    assert!(
        bridge.get(8).unwrap().children.contains(&9),
        "settings action must stay under footer content"
    );
}

#[test]
fn reparent_orphans_prefers_resources_content_host() {
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    // Workspace row without shell-body contract — resources content must win.
    let mut workspace = WidgetProps::default();
    workspace.class_names = vec!["flex-row".into()];
    bridge.register(3, WidgetKind::Row, workspace);
    bridge.insert_child(3, 2, None);
    let mut resources = WidgetProps::default();
    resources.region = "resources".into();
    resources.agent_id = "workspace.region.sidebar".into();
    resources
        .attrs
        .insert("data-region-role".into(), "resources".into());
    bridge.register(4, WidgetKind::Column, resources);
    bridge.insert_child(4, 3, None);
    let mut content = WidgetProps::default();
    content.class_names = vec!["nana-workspace-region__content".into()];
    bridge.register(5, WidgetKind::Column, content);
    bridge.insert_child(5, 4, None);
    let mut orphan = WidgetProps::default();
    orphan.element_tag = "nana-sidebar-frame".into();
    bridge.register(6, WidgetKind::SidebarFrame, orphan);
    bridge.reparent_orphans();
    assert!(
        bridge.get(5).unwrap().children.contains(&6),
        "orphan must reparent under resources content, not workspace flex-row"
    );
    assert!(!bridge.get(3).unwrap().children.contains(&6));
}

#[test]
fn reparent_orphans_workspace_fallback_seeds_finite_height_cb() {
    // When resources remount leaves no reachable content host, orphan
    // SidebarFrame attaches under nana-workspace-shell__body. The auto-height
    // shell content wrapper must not leave Fill CB height as None — otherwise
    // overflow-y scrollports paint at 0.
    let mut bridge = MessageBridge::new();
    bridge.ensure_document_roots(1, 2);
    let mut shell = WidgetProps::default();
    shell.class_names = vec!["nana-app-shell".into()];
    shell.layout.height = Some(LengthSpec::Fill);
    shell.layout.width = Some(LengthSpec::Fill);
    bridge.register(3, WidgetKind::Column, shell);
    bridge.insert_child(3, 2, None);
    let mut content = WidgetProps::default();
    content.class_names = vec!["nana-app-shell__content".into()];
    // Grid-track content is often height-auto in CSS; give Fill so the
    // workspace Fill child still receives a definite CB in this unit test.
    content.layout.height = Some(LengthSpec::Fill);
    bridge.register(4, WidgetKind::Column, content);
    bridge.insert_child(4, 3, None);
    let mut workspace = WidgetProps::default();
    workspace.class_names = vec!["nana-workspace-shell__body".into(), "flex-row".into()];
    workspace.layout.width = Some(LengthSpec::Fill);
    workspace.layout.height = Some(LengthSpec::Fill);
    bridge.register(5, WidgetKind::Row, workspace);
    bridge.insert_child(5, 4, None);
    let mut primary = WidgetProps::default();
    primary.region = "primary".into();
    primary.layout.width = Some(LengthSpec::Fill);
    primary.layout.height = Some(LengthSpec::Fill);
    bridge.register(6, WidgetKind::Column, primary);
    bridge.insert_child(6, 5, None);
    let mut orphan = WidgetProps::default();
    orphan.element_tag = "nana-sidebar-frame".into();
    orphan.class_names = vec!["nana-sidebar-frame".into()];
    orphan.layout.apply_class_layout_hints(&orphan.class_names);
    bridge.register(7, WidgetKind::SidebarFrame, orphan);
    let mut body = WidgetProps::default();
    body.class_names = vec!["nana-sidebar-frame__body".into()];
    body.layout.apply_class_layout_hints(&body.class_names);
    bridge.register(8, WidgetKind::Column, body);
    bridge.insert_child(8, 7, None);

    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(1280.0, 800.0));
    bridge.reparent_orphans();
    assert!(
        bridge.get(5).unwrap().children.contains(&7),
        "orphan must reparent under workspace when resources host is gone"
    );
    let body_cb = bridge.get(8).unwrap().props.containing_block_height;
    assert!(
        body_cb.is_some_and(|h| h > 100.0),
        "sidebar body CB height must stay finite after workspace fallback, got {body_cb:?}"
    );
}

#[test]
fn data_region_role_maps_into_widget_region() {
    use nana_js_engine::HostValue;

    let mut patched = WidgetProps::default();
    patched.apply_prop("data-region-role", &HostValue::string("resources"));
    assert_eq!(patched.region, "resources");
    assert_eq!(
        patched.attrs.get("data-region-role").map(String::as_str),
        Some("resources")
    );
}

#[test]
fn data_slot_sidebar_body_applies_frame_body_hints() {
    use nana_js_engine::HostValue;

    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "data-slot", &HostValue::string("sidebar-body"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert_eq!(layout.flex_grow, Some(1.0));
    assert_eq!(layout.height, Some(LengthSpec::Fill));
    assert!(
        bridge
            .get(1)
            .unwrap()
            .props
            .class_names
            .iter()
            .any(|c| c == "nana-sidebar-frame__body")
    );
}

#[test]
fn region_views_keep_lilia_resources_and_sidebar_agent_in_primary() {
    // Lilia ResourcePanel (`data-region-role=resources` / workspace.region.sidebar)
    // and SecondaryPanel (`agent-id=sidebar`) are in-tree workspace chrome —
    // not DesktopShell Navigation opt-in. Lifting them emptied Primary and
    // stacked remount leftovers into an empty-looking shell sidebar.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::SidebarFrame,
        WidgetProps {
            agent_id: "sidebar".into(),
            element_tag: "nana-sidebar-frame".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Column,
        WidgetProps {
            region: "resources".into(),
            agent_id: "workspace.region.sidebar".into(),
            class_names: vec!["lilia-workspace-region--resources".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            region: "primary".into(),
            agent_id: "workspace.region.main".into(),
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(3, 1, None);
    bridge.insert_child(2, 3, None);
    bridge.insert_child(4, 1, None);
    let views = bridge.snapshot().region_views();
    assert!(
        views.navigation.widgets.is_empty(),
        "Lilia resources/sidebar must not invent DesktopShell Navigation"
    );
    assert!(
        views.primary.widgets.iter().any(|w| w.id == 2),
        "SidebarFrame stays in Primary"
    );
    assert!(
        views.primary.widgets.iter().any(|w| w.id == 3),
        "resources shell stays in Primary"
    );
    assert!(
        views.primary.widgets.iter().any(|w| w.id == 4),
        "primary region stays in Primary"
    );
    assert!(views.overlapping_ids().is_empty());
}

#[test]
fn deep_child_combinator_stylesheet_matches_full_ancestry() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(".a > .b > .c > .leaf { gap: 18px; width: 100%; }");
    for (id, class) in [(1, "a"), (2, "b"), (3, "c"), (4, "leaf")] {
        let mut props = WidgetProps::default();
        props.element_tag = "div".into();
        bridge.register(id, WidgetKind::Column, props);
        bridge.patch_prop(id, "class", &HostValue::string(class));
    }
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 3, None);
    // Re-apply after tree links so MatchContext sees the full parent chain.
    bridge.patch_prop(4, "class", &HostValue::string("leaf"));
    let leaf = &bridge.get(4).unwrap().props.layout;
    assert_eq!(leaf.gap, Some(LengthSpec::Px(18.0)));
    assert_eq!(leaf.width, Some(LengthSpec::Fill));
}

#[test]
fn element_id_alone_does_not_invent_region_layout() {
    // Layout must come from stylesheet / class hints / inline — not an
    // id|data-region-id whitelist (sidebar/main/primary/left).
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(1, "id", &HostValue::string("sidebar"));
    let layout = &bridge.get(1).unwrap().props.layout;
    assert!(layout.width.is_none());
    assert_ne!(layout.width, Some(LengthSpec::Px(220.0)));
    assert!(layout.flex_grow.is_none());

    bridge.register(2, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(2, "data-region-id", &HostValue::string("main"));
    let main = &bridge.get(2).unwrap().props.layout;
    assert!(main.width.is_none());
    assert!(main.flex_grow.is_none());

    // Public shell class contract still sizes the sidebar.
    bridge.register(3, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(
        3,
        "class",
        &HostValue::string("nana-workspace-shell__sidebar"),
    );
    assert_eq!(
        bridge.get(3).unwrap().props.layout.width,
        Some(LengthSpec::Px(220.0))
    );
}

#[test]
fn tabs_select_value_updates_props() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        5,
        WidgetKind::Tabs,
        WidgetProps {
            options: vec![
                SelectOptionProp {
                    value: "a".into(),
                    label: "A".into(),
                    disabled: false,
                },
                SelectOptionProp {
                    value: "b".into(),
                    label: "B".into(),
                    disabled: false,
                },
            ],
            value: "a".into(),
            ..WidgetProps::default()
        },
    );
    let names = bridge.note_select_value(5, "b");
    assert!(names.contains(&"update:modelValue"));
    assert_eq!(bridge.get(5).unwrap().props.value, "b");
    assert!(bridge.get(5).unwrap().props.active);
}

#[test]
#[cfg(feature = "calendar")]
#[cfg(feature = "charts")]
#[cfg(feature = "controls")]
#[cfg(feature = "rich-text")]
#[cfg(feature = "graph-canvas")]
#[cfg(feature = "image-viewer")]
fn widget_kind_parses_catalog_professional_aliases() {
    assert_eq!(
        WidgetKind::parse("nana-command-palette"),
        Some(WidgetKind::CommandPalette)
    );
    assert_eq!(
        WidgetKind::parse("command-palette"),
        Some(WidgetKind::CommandPalette)
    );
    assert_eq!(WidgetKind::parse("commandpalette"), None);
    assert_eq!(
        WidgetKind::parse("nana-tree-view"),
        Some(WidgetKind::TreeView)
    );
    assert_eq!(WidgetKind::parse("tree-view"), Some(WidgetKind::TreeView));
    assert_eq!(WidgetKind::parse("treeview"), None);
    assert_eq!(WidgetKind::parse("nana-calendar"), None);
    assert_eq!(
        WidgetKind::parse("calendar-heatmap"),
        Some(WidgetKind::CalendarHeatmap)
    );
    assert_eq!(WidgetKind::parse("calendar"), None);
    assert_eq!(
        WidgetKind::parse("nana-image-viewer"),
        Some(WidgetKind::ImageViewer)
    );
    assert_eq!(
        WidgetKind::parse("image-viewer"),
        Some(WidgetKind::ImageViewer)
    );
    assert_eq!(WidgetKind::parse("nana-markdown"), None);
    assert_eq!(
        WidgetKind::parse("native-markdown"),
        Some(WidgetKind::NativeMarkdown)
    );
    assert_eq!(WidgetKind::parse("markdown"), None);
    assert_eq!(
        WidgetKind::parse("nana-graph-canvas"),
        Some(WidgetKind::GraphCanvas)
    );
    assert_eq!(
        WidgetKind::parse("graph-canvas"),
        Some(WidgetKind::GraphCanvas)
    );
    assert_eq!(WidgetKind::parse("graphcanvas"), None);
    assert_eq!(
        WidgetKind::parse("nana-workspace"),
        Some(WidgetKind::Workspace)
    );
    assert_eq!(WidgetKind::parse("nana-dock"), Some(WidgetKind::Dock));
    assert_eq!(
        WidgetKind::parse("nana-split-pane"),
        Some(WidgetKind::SplitPane)
    );
    assert_eq!(WidgetKind::parse("split-pane"), Some(WidgetKind::SplitPane));
    assert_eq!(
        WidgetKind::parse("nana-app-shell"),
        Some(WidgetKind::AppShell)
    );
    assert_eq!(WidgetKind::parse("app-shell"), Some(WidgetKind::AppShell));
    assert_eq!(
        WidgetKind::parse("nana-settings-page"),
        Some(WidgetKind::SettingsPage)
    );
    assert_eq!(
        WidgetKind::parse("settings-page"),
        Some(WidgetKind::SettingsPage)
    );
    assert_eq!(WidgetKind::parse("settingspage"), None);
    assert_eq!(WidgetKind::parse("form"), None);
    assert_eq!(WidgetKind::parse("nana-form"), None);
    assert_eq!(WidgetKind::parse("form-field"), Some(WidgetKind::FormField));
    assert_eq!(WidgetKind::parse("formfield"), None);
    assert_eq!(
        WidgetKind::parse("nana-form-field"),
        Some(WidgetKind::FormField)
    );
    assert_eq!(WidgetKind::FormField.as_str(), "form-field");
    assert_eq!(WidgetKind::FormField.element_tag(), "nana-form-field");
    assert_eq!(WidgetKind::SettingsPage.as_str(), "settings-page");
    assert_eq!(WidgetKind::SettingsPage.element_tag(), "nana-settings-page");
    assert_eq!(WidgetKind::CommandPalette.as_str(), "command-palette");
    assert_eq!(
        WidgetKind::CommandPalette.element_tag(),
        "nana-command-palette"
    );
    assert_eq!(
        WidgetKind::CalendarHeatmap.element_tag(),
        "nana-calendar-heatmap"
    );
    assert_eq!(WidgetKind::NativeMarkdown.as_str(), "native-markdown");
    assert_eq!(
        WidgetKind::NativeMarkdown.element_tag(),
        "nana-native-markdown"
    );
    assert_eq!(WidgetKind::GraphCanvas.element_tag(), "nana-graph-canvas");
    assert_eq!(WidgetKind::GraphCanvas.as_str(), "graph-canvas");
    assert_eq!(
        WidgetKind::parse("nana-icon-button"),
        Some(WidgetKind::IconButton)
    );
    assert_eq!(
        WidgetKind::parse("nana-number-input"),
        Some(WidgetKind::NumberInput)
    );
    assert_eq!(WidgetKind::parse("nana-number"), None);
    assert_eq!(WidgetKind::parse("nana-divider"), Some(WidgetKind::Divider));
    assert_eq!(
        WidgetKind::parse("nana-thumbnail"),
        Some(WidgetKind::Thumbnail)
    );
    assert_eq!(WidgetKind::parse("nana-list"), Some(WidgetKind::List));
    assert_eq!(
        WidgetKind::parse("nana-scroll-view"),
        Some(WidgetKind::ScrollView)
    );
    assert_eq!(WidgetKind::parse("nana-scroll"), None);
    assert_eq!(WidgetKind::parse("nana-table"), Some(WidgetKind::Table));
    assert_eq!(
        WidgetKind::parse("nana-table-row"),
        Some(WidgetKind::TableRow)
    );
    assert_eq!(
        WidgetKind::parse("nana-table-cell"),
        Some(WidgetKind::TableCell)
    );
    assert_eq!(
        WidgetKind::parse("nana-reorder-list"),
        Some(WidgetKind::ReorderList)
    );
    assert_eq!(
        WidgetKind::parse("nana-time-series-chart"),
        Some(WidgetKind::TimeSeriesChart)
    );
    assert_eq!(
        WidgetKind::parse("nana-desktop-shell"),
        Some(WidgetKind::DesktopShell)
    );
    assert_eq!(
        WidgetKind::parse("nana-app-title-bar"),
        Some(WidgetKind::AppTitleBar)
    );
    assert_eq!(WidgetKind::parse("title-bar"), None);
    assert_eq!(
        WidgetKind::parse("nana-pane-chrome"),
        Some(WidgetKind::PaneChrome)
    );
    assert_eq!(
        WidgetKind::parse("nana-sidebar-section"),
        Some(WidgetKind::SidebarSection)
    );
    assert_eq!(
        WidgetKind::parse("nana-sidebar-footer"),
        Some(WidgetKind::SidebarFooter)
    );
    assert_eq!(
        WidgetKind::parse("nana-settings-collapsible-card"),
        Some(WidgetKind::SettingsCollapsibleCard)
    );
    assert_eq!(WidgetKind::ScrollView.element_tag(), "nana-scroll-view");
    assert_eq!(
        WidgetKind::parse("nana-gpu"),
        Some(WidgetKind::GpuTextureView)
    );
    assert_eq!(WidgetKind::parse("gpu-view"), Some(WidgetKind::GpuView));
    assert_eq!(WidgetKind::parse("nana-virtual-list"), None);
    assert_eq!(WidgetKind::GpuTextureView.element_tag(), "nana-gpu");
    assert_eq!(WidgetKind::parse("nana-video"), Some(WidgetKind::Video));
    assert_eq!(WidgetKind::Video.as_str(), "video");
    assert_eq!(WidgetKind::Video.element_tag(), "nana-video");
    assert_eq!(WidgetKind::IconButton.as_str(), "icon-button");
    assert!(WidgetKind::CommandPalette.is_overlay());
    assert!(WidgetKind::ImageViewer.is_overlay());
    assert!(!WidgetKind::GraphCanvas.is_overlay());
    assert!(!WidgetKind::Workspace.is_overlay());
    assert!(!WidgetKind::TreeView.is_overlay());
    assert!(!WidgetKind::ScrollView.is_overlay());
}

#[test]
fn overlay_toggle_false_clears_active_and_toggled() {
    // Opened via `active`/`open` (common Vue path); dismiss must clear both
    // because overlay_is_open = active || toggled.
    for kind in [
        WidgetKind::Dialog,
        WidgetKind::Drawer,
        WidgetKind::Popover,
        WidgetKind::ContextMenu,
    ] {
        let mut bridge = MessageBridge::new();
        bridge.register(
            1,
            kind,
            WidgetProps {
                active: true,
                toggled: true,
                ..WidgetProps::default()
            },
        );
        assert!(kind.is_overlay());
        let names = bridge.note_toggle(1, false);
        assert!(names.contains(&"update:modelValue"));
        let props = &bridge.get(1).unwrap().props;
        assert!(!props.active, "{kind:?} active should clear on dismiss");
        assert!(!props.toggled, "{kind:?} toggled should clear on dismiss");
        assert!(
            !(props.active || props.toggled),
            "{kind:?} must not remain open after Toggle false"
        );
    }
}

#[test]
fn overlay_toggle_false_clears_active_only_open() {
    // Opened with active=true, toggled=false (apply_prop "active" path).
    let mut bridge = MessageBridge::new();
    bridge.register(
        2,
        WidgetKind::Dialog,
        WidgetProps {
            active: true,
            toggled: false,
            ..WidgetProps::default()
        },
    );
    bridge.note_toggle(2, false);
    let props = &bridge.get(2).unwrap().props;
    assert!(!props.active);
    assert!(!props.toggled);
}

#[test]
fn modal_presence_opens_dialog_without_fixed() {
    // Teleport dialog: aria-modal presence, no open= — Nana Overlay only.
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.role = "dialog".into();
    props.class_names = vec!["nana-dialog".into()];
    props.attrs.insert("aria-modal".into(), "true".into());
    props.layout.position = crate::css_map::PositionSpec::Fixed;
    bridge.register(10, WidgetKind::Dialog, props);
    let w = bridge.get(10).unwrap();
    assert!(
        w.props.active && w.props.toggled,
        "presence must open Dialog"
    );
    assert_eq!(
        w.props.layout.position,
        crate::css_map::PositionSpec::Static,
        "must strip deferred fixed — Nana Overlay only"
    );
}

#[test]
fn non_overlay_keeps_css_fixed_for_viewport_subset() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.label = "pin".into();
    props.layout.apply_css_text(
        "position:fixed;top:0;left:0;width:40px;height:24px",
        None,
        None,
    );
    bridge.register(20, WidgetKind::Box, props);
    let w = bridge.get(20).unwrap();
    assert_eq!(w.props.layout.position, crate::css_map::PositionSpec::Fixed);
    assert!(w.props.layout.is_fixed());
    assert!(!w.props.layout.position.is_unsupported_positioning());
}

#[test]
fn modal_role_patch_promotes_and_opens() {
    let mut bridge = MessageBridge::new();
    bridge.register(11, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(11, "class", &HostValue::string("nana-dialog"));
    bridge.patch_prop(11, "role", &HostValue::string("dialog"));
    bridge.patch_prop(11, "aria-modal", &HostValue::string("true"));
    let w = bridge.get(11).unwrap();
    assert_eq!(w.kind, WidgetKind::Dialog);
    assert!(w.props.active && w.props.toggled);
}

#[test]
fn closed_nana_dialog_does_not_auto_open_from_class() {
    // NanaDialog stays mounted with class nana-dialog while open=false.
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.class_names = vec!["nana-dialog".into()];
    props.role = "dialog".into();
    props.active = false;
    props.toggled = false;
    bridge.register(12, WidgetKind::Dialog, props);
    let w = bridge.get(12).unwrap();
    assert!(!w.props.active && !w.props.toggled);
}

#[test]
fn dropdown_class_maps_to_dropdown_not_fixed_menu() {
    let mut bridge = MessageBridge::new();
    bridge.register(13, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(13, "class", &HostValue::string("nana-dropdown"));
    assert_eq!(bridge.get(13).unwrap().kind, WidgetKind::Dropdown);
    // Unregistered id is a no-op; register panel explicitly.
    bridge.register(14, WidgetKind::Column, WidgetProps::default());
    bridge.patch_prop(14, "class", &HostValue::string("nana-select"));
    bridge.patch_prop(14, "role", &HostValue::string("listbox"));
    assert_eq!(bridge.get(14).unwrap().kind, WidgetKind::Select);
}

/// Runtime keeps `nana.select`, `nana.dropdown` and `nana.search-dropdown` apart, so
/// the bridge must not fold three option fields into one kind.
#[test]
fn select_dropdown_and_search_stay_distinct_kinds() {
    assert_eq!(WidgetKind::parse("nana-select"), Some(WidgetKind::Select));
    assert_eq!(WidgetKind::parse("pick-list"), None);
    assert_eq!(
        WidgetKind::parse("nana-dropdown"),
        Some(WidgetKind::Dropdown)
    );
    assert_eq!(WidgetKind::parse("nana-search"), None);
    assert_eq!(
        WidgetKind::parse("nana-search-dropdown"),
        Some(WidgetKind::SearchDropdown)
    );
    assert_eq!(WidgetKind::parse("search"), None);
    for kind in [
        WidgetKind::Select,
        WidgetKind::Dropdown,
        WidgetKind::SearchDropdown,
    ] {
        assert!(kind.is_choice_field(), "{kind:?} is an option field");
        assert_eq!(WidgetKind::parse(kind.element_tag()), Some(kind));
        assert_eq!(WidgetKind::parse(kind.as_str()), Some(kind));
    }
}

#[test]
fn switch_toggle_does_not_force_active() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        3,
        WidgetKind::Switch,
        WidgetProps {
            toggled: true,
            active: false,
            ..WidgetProps::default()
        },
    );
    bridge.note_toggle(3, false);
    let props = &bridge.get(3).unwrap().props;
    assert!(!props.toggled);
    assert!(!props.active);
    bridge.note_toggle(3, true);
    let props = &bridge.get(3).unwrap().props;
    assert!(props.toggled);
    assert!(
        !props.active,
        "Switch toggle must not set active (overlay-only sync)"
    );
}

#[test]
fn patch_prop_switch_input_segmented_semantics() {
    let mut bridge = MessageBridge::new();

    // Switch: boolean disabled + toggled via `.` modifier and plain keys.
    bridge.register(1, WidgetKind::Switch, WidgetProps::default());
    bridge.patch_prop(1, ".disabled", &HostValue::Bool(true));
    bridge.patch_prop(1, "toggled", &HostValue::Bool(true));
    let sw = bridge.get(1).unwrap();
    assert!(sw.props.disabled);
    assert!(sw.props.toggled);

    // Input: value + placeholder + disabled false clears.
    bridge.register(2, WidgetKind::Input, WidgetProps::default());
    bridge.patch_prop(2, ".value", &HostValue::string("typed"));
    bridge.patch_prop(2, "placeholder", &HostValue::string("hint"));
    bridge.patch_prop(2, "disabled", &HostValue::Bool(false));
    let input = bridge.get(2).unwrap();
    assert_eq!(input.props.value, "typed");
    assert_eq!(input.props.placeholder, "hint");
    assert!(!input.props.disabled);

    // Segmented: options array + value selection.
    bridge.register(3, WidgetKind::Segmented, WidgetProps::default());
    let options = HostValue::Array(vec![
        HostValue::Object(
            [
                ("value".into(), HostValue::string("light")),
                ("label".into(), HostValue::string("浅色")),
            ]
            .into_iter()
            .collect(),
        ),
        HostValue::Object(
            [
                ("value".into(), HostValue::string("dark")),
                ("label".into(), HostValue::string("暗色")),
            ]
            .into_iter()
            .collect(),
        ),
    ]);
    bridge.patch_prop(3, "options", &options);
    bridge.patch_prop(3, "value", &HostValue::string("dark"));
    let seg = bridge.get(3).unwrap();
    assert_eq!(seg.props.options.len(), 2);
    assert_eq!(seg.props.options[1].label, "暗色");
    assert_eq!(seg.props.value, "dark");
}

#[test]
fn patch_prop_svg_attrs_and_force_attr() {
    let mut bridge = MessageBridge::new();
    bridge.register(4, WidgetKind::Icon, WidgetProps::default());
    bridge.patch_prop(4, "viewBox", &HostValue::string("0 0 24 24"));
    bridge.patch_prop(4, "^xlink:href", &HostValue::string("#star"));
    let icon = bridge.get(4).unwrap();
    assert!(
        icon.props.attrs.contains_key("view-box") || icon.props.attrs.contains_key("viewbox"),
        "viewBox must land in attrs, got {:?}",
        icon.props.attrs
    );
    assert_eq!(
        icon.props.attrs.get("xlink:href").map(String::as_str),
        Some("#star")
    );
}

#[test]
fn chart_svg_pins_min_height_to_author_px_height() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "svg".into();
    bridge.register(9, WidgetKind::Box, props);
    bridge.patch_prop(9, "viewBox", &HostValue::string("0 0 905 125"));
    bridge.patch_prop(9, "width", &HostValue::Number(905.0));
    bridge.patch_prop(9, "height", &HostValue::Number(125.0));
    let w = bridge.get(9).unwrap();
    assert_eq!(
        w.props.layout.height,
        Some(LengthSpec::Px(125.0)),
        "height attr must map to layout"
    );
    assert_eq!(
        w.props.layout.min_height,
        Some(LengthSpec::Px(125.0)),
        "chart svg must pin min-height so flex cannot crush weekday rows, got {:?}",
        w.props.layout.min_height
    );

    // overflow:hidden keeps CSS min-size:auto → 0 (may shrink).
    let mut clipped = WidgetProps::default();
    clipped.element_tag = "svg".into();
    bridge.register(10, WidgetKind::Box, clipped);
    bridge.patch_prop(10, "viewBox", &HostValue::string("0 0 40 20"));
    bridge.patch_prop(10, "height", &HostValue::Number(20.0));
    bridge.patch_prop(10, "style", &HostValue::string("overflow: hidden"));
    let c = bridge.get(10).unwrap();
    assert!(
        c.props.layout.min_height.is_none()
            || matches!(c.props.layout.min_height, Some(LengthSpec::Px(mh)) if mh < 1.0),
        "overflow:hidden chart svg must not raise min-height, got {:?}",
        c.props.layout.min_height
    );
}

#[test]
fn patch_prop_stroke_dash_attrs_stay_out_of_value() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.element_tag = "circle".into();
    props.value = String::new();
    props.hint = String::new();
    bridge.register(7, WidgetKind::Box, props);
    bridge.patch_prop(7, "stroke-dasharray", &HostValue::string("68 32"));
    bridge.patch_prop(7, "stroke-dashoffset", &HostValue::string("-12.5"));
    bridge.patch_prop(7, "pathLength", &HostValue::string("100"));
    let w = bridge.get(7).unwrap();
    assert_eq!(
        w.props.attrs.get("stroke-dasharray").map(String::as_str),
        Some("68 32")
    );
    assert_eq!(
        w.props.attrs.get("stroke-dashoffset").map(String::as_str),
        Some("-12.5")
    );
    assert!(
        w.props.attrs.contains_key("pathlength")
            || w.props.attrs.contains_key("path-length")
            || w.props.attrs.contains_key("pathLength"),
        "pathLength in attrs, got {:?}",
        w.props.attrs
    );
    assert!(
        w.props.value.is_empty(),
        "dasharray must not clobber value, got {:?}",
        w.props.value
    );
    assert!(
        w.props.hint.is_empty(),
        "dashoffset must not clobber hint, got {:?}",
        w.props.hint
    );
}

#[test]
fn overlay_select_value_closes_after_confirm() {
    // ConfirmDialog / Drawer footer emit SelectValue on the overlay id;
    // confirm must close unless product keeps it open.
    for (kind, value) in [
        (WidgetKind::Dialog, "confirm"),
        (WidgetKind::Drawer, "confirm"),
        (WidgetKind::ContextMenu, "item-a"),
    ] {
        let mut bridge = MessageBridge::new();
        bridge.register(
            4,
            kind,
            WidgetProps {
                active: true,
                toggled: true,
                ..WidgetProps::default()
            },
        );
        let names = bridge.note_select_value(4, value);
        assert!(names.contains(&"update:modelValue"));
        let props = &bridge.get(4).unwrap().props;
        assert_eq!(props.value, value);
        assert!(
            !props.active && !props.toggled,
            "{kind:?} should close after SelectValue confirm"
        );
    }
}

#[test]
fn overlay_patch_prop_false_clears_both_open_flags() {
    // Vue often patches only `active` / `selected` or only `model-value`/`toggled`.
    // overlay_is_open = active || toggled, so the other side must clear too.
    for kind in [
        WidgetKind::Dialog,
        WidgetKind::Drawer,
        WidgetKind::Popover,
        WidgetKind::ContextMenu,
    ] {
        for key in [
            "active",
            "open",
            "selected",
            "aria-selected",
            "aria-pressed",
            "toggled",
            "model-value",
        ] {
            let mut bridge = MessageBridge::new();
            bridge.register(
                10,
                kind,
                WidgetProps {
                    active: true,
                    toggled: true,
                    ..WidgetProps::default()
                },
            );
            bridge.patch_prop(10, key, &HostValue::Bool(false));
            let props = &bridge.get(10).unwrap().props;
            assert!(
                !props.active && !props.toggled,
                "{kind:?} patch {key}=false must clear both open flags"
            );
        }
    }
}

#[test]
fn overlay_patch_selected_false_closes_when_toggled_stuck() {
    // Regression: apply_prop writes selected → active only; without bilateral
    // sync, toggled stays true and overlay_is_open remains open.
    let mut bridge = MessageBridge::new();
    bridge.register(
        13,
        WidgetKind::Popover,
        WidgetProps {
            active: true,
            toggled: true,
            ..WidgetProps::default()
        },
    );
    bridge.patch_prop(13, "selected", &HostValue::Bool(false));
    let props = &bridge.get(13).unwrap().props;
    assert!(!props.active, "selected=false must clear active");
    assert!(
        !props.toggled,
        "selected=false must clear toggled (bilateral sync)"
    );
}

#[test]
fn overlay_patch_prop_true_syncs_both_open_flags() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        11,
        WidgetKind::Dialog,
        WidgetProps {
            active: false,
            toggled: false,
            ..WidgetProps::default()
        },
    );
    bridge.patch_prop(11, "active", &HostValue::Bool(true));
    let props = &bridge.get(11).unwrap().props;
    assert!(
        props.active && props.toggled,
        "active=true should open both"
    );

    bridge.patch_prop(11, "active", &HostValue::Bool(false));
    bridge.patch_prop(11, "model-value", &HostValue::Bool(true));
    let props = &bridge.get(11).unwrap().props;
    assert!(
        props.active && props.toggled,
        "model-value=true should open both"
    );
}

#[test]
fn overlay_patch_model_value_string_does_not_reopen() {
    // After SelectValue close, Vue may patch model-value to the confirm string.
    let mut bridge = MessageBridge::new();
    bridge.register(
        12,
        WidgetKind::Dialog,
        WidgetProps {
            active: false,
            toggled: false,
            value: String::new(),
            ..WidgetProps::default()
        },
    );
    bridge.patch_prop(12, "model-value", &HostValue::string("confirm"));
    let props = &bridge.get(12).unwrap().props;
    assert_eq!(props.value, "confirm");
    assert!(
        !props.active && !props.toggled,
        "string model-value must not reopen overlay"
    );
}

#[test]
fn switch_patch_model_value_does_not_force_active() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        13,
        WidgetKind::Switch,
        WidgetProps {
            toggled: true,
            active: false,
            ..WidgetProps::default()
        },
    );
    bridge.patch_prop(13, "model-value", &HostValue::Bool(false));
    let props = &bridge.get(13).unwrap().props;
    assert!(!props.toggled);
    assert!(!props.active);
    bridge.patch_prop(13, "model-value", &HostValue::Bool(true));
    let props = &bridge.get(13).unwrap().props;
    assert!(props.toggled);
    assert!(
        !props.active,
        "Switch model-value must not set active (overlay-only sync)"
    );
}

#[test]
fn region_views_are_mutually_exclusive_by_region_tags() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            // DesktopShell Navigation requires an explicit region token —
            // agent suffixes like `.navigation` alone must not invent lift.
            region: "global-navigation".into(),
            agent_id: "nana.workspace.navigation".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::SidebarRow,
        WidgetProps {
            label: "Home".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Card,
        WidgetProps {
            label: "card".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Column,
        WidgetProps {
            role: "inspector".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        7,
        WidgetKind::Text,
        WidgetProps {
            label: "facts".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);
    bridge.insert_child(5, 4, None);
    bridge.insert_child(6, 1, None);
    bridge.insert_child(7, 6, None);
    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views.overlapping_ids().is_empty(),
        "region views must not share widget ids: {:?}",
        views.overlapping_ids()
    );
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
    assert!(views.inspector.widgets.iter().any(|w| w.id == 6));
    assert!(views.primary.widgets.iter().any(|w| w.id == 4));
    assert!(views.primary.widgets.iter().any(|w| w.id == 5));
    assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
    assert!(!views.primary.widgets.iter().any(|w| w.id == 6 || w.id == 7));
}

#[test]
fn region_views_do_not_harvest_untagged_sidebar_or_cards() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(2, WidgetKind::SidebarFrame, WidgetProps::default());
    bridge.register(
        3,
        WidgetKind::SidebarRow,
        WidgetProps {
            label: "Home".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Card,
        WidgetProps {
            label: "card".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::SettingsCard,
        WidgetProps {
            label: "外观".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);
    bridge.insert_child(5, 1, None);
    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views.navigation.widgets.is_empty(),
        "untagged SidebarFrame must not be claimed as Navigation"
    );
    assert!(
        views.inspector.widgets.is_empty(),
        "untagged Card/Settings must not be claimed as Inspector"
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 2));
    assert!(views.primary.widgets.iter().any(|w| w.id == 4));
    assert!(views.primary.widgets.iter().any(|w| w.id == 5));
    assert_eq!(views.primary.widgets.len(), snap.widgets.len());
    assert!(views.overlapping_ids().is_empty());
}

#[test]
fn region_views_collapse_shell_grid_when_nav_column_extracted() {
    // NanaWorkspaceShell body: 220px + 1fr. Tagged nav leaves Primary with
    // only the primary column — stale 2-col grid must not squeeze it to 220.
    let mut bridge = MessageBridge::new();
    let mut body = WidgetProps::default();
    body.class_names = vec!["nana-workspace-shell__body".into()];
    body.layout.apply_class_layout_hints(&body.class_names);
    bridge.register(1, WidgetKind::Row, body);
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "nana.workspace.sidebar".into(),
            class_names: vec!["nana-workspace-shell__sidebar".into()],
            layout: {
                let mut l = LayoutStyle::default();
                l.apply_class_layout_hints(&["nana-workspace-shell__sidebar".into()]);
                l
            },
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Column,
        WidgetProps {
            region: "primary".into(),
            agent_id: "nana.workspace.primary".into(),
            class_names: vec!["nana-workspace-shell__primary".into()],
            layout: {
                let mut l = LayoutStyle::default();
                l.apply_class_layout_hints(&["nana-workspace-shell__primary".into()]);
                l
            },
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Text,
        WidgetProps {
            label: "main content".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 1, None);
    bridge.insert_child(4, 3, None);

    let snap = bridge.snapshot();
    assert_eq!(
        snap.get(1)
            .unwrap()
            .props
            .layout
            .grid_columns
            .as_ref()
            .map(|c| c.len()),
        Some(2),
        "full forest keeps 2-col shell body"
    );

    let views = snap.region_views();
    assert!(views.overlapping_ids().is_empty());
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(views.primary.widgets.iter().any(|w| w.id == 3));
    assert!(!views.primary.widgets.iter().any(|w| w.id == 2));

    let body = views.primary.get(1).expect("shell body remains in primary");
    assert_eq!(body.children, vec![3], "only primary column remains");
    let cols = body
        .props
        .layout
        .grid_columns
        .as_ref()
        .expect("collapsed track list");
    assert_eq!(
        cols.len(),
        1,
        "stale 2-col grid must collapse to remaining child count"
    );
    assert_eq!(
        cols[0],
        GridTrack::MinMax {
            min_px: 0.0,
            fr: 1.0,
            max_px: None,
        }
    );

    // Measure: primary column must get full body width, not ~220.
    let mut body_style = body.props.layout.clone();
    body_style.width = Some(LengthSpec::Px(800.0));
    body_style.height = Some(LengthSpec::Px(400.0));
    let mut primary_style = views.primary.get(3).unwrap().props.layout.clone();
    primary_style.height = Some(LengthSpec::Fill);
    let tree = crate::measure::LayoutNode::with_children(
        "body",
        body_style,
        vec![crate::measure::LayoutNode::leaf("primary", primary_style)],
    );
    let boxes = crate::measure::measure_layout(&tree, 800.0, 400.0);
    let primary_box = boxes
        .iter()
        .find(|(id, _)| id == "primary")
        .map(|(_, b)| b)
        .expect("primary box");
    assert!(
        primary_box.width > 500.0,
        "primary must not be squeezed to sidebar track (~220); got {}",
        primary_box.width
    );
    assert!((primary_box.width - 800.0).abs() < 1.0);
}

#[test]
fn region_views_claim_hollow_outer_shell_after_nested_nav_lift() {
    // Lilia-shaped: workspace row → outer start panel (+ resize chrome) →
    // nested tagged SidebarFrame. Nested tag must not leave a fixed-width
    // empty shell in Primary beside DesktopShell Navigation.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            agent_id: "workspace.region.sidebar".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["nana-workspace-region__content".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::SidebarFrame,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "sidebar".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Text,
        WidgetProps {
            label: "项目总览".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Column,
        WidgetProps {
            role: "separator".into(),
            agent_id: "workspace.region.sidebar.resize".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        7,
        WidgetKind::Column,
        WidgetProps {
            agent_id: "workspace.region.main".into(),
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(6, 2, None);
    bridge.insert_child(4, 3, None);
    bridge.insert_child(5, 4, None);
    bridge.insert_child(7, 1, None);

    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(views.overlapping_ids().is_empty());
    assert!(
        views.navigation.widgets.iter().any(|w| w.id == 4),
        "nested tagged frame still projects to Navigation"
    );
    assert!(
        !views
            .primary
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3 || w.id == 6),
        "hollow outer shell + resize chrome must leave Primary: {:?}",
        views
            .primary
            .widgets
            .iter()
            .map(|w| w.id)
            .collect::<Vec<_>>()
    );
    assert!(
        views.primary.widgets.iter().any(|w| w.id == 7),
        "main column stays in Primary"
    );
    assert!(
        !views.primary.widgets.iter().any(|w| w.id == 4 || w.id == 5),
        "nav content exclusive of Primary"
    );
}

#[test]
fn inspector_slice_keeps_region_tagged_settings() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            agent_id: "nana.workspace.inspector".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::SettingsCard,
        WidgetProps {
            label: "Inspector card".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::SettingsCard,
        WidgetProps {
            label: "Primary appearance".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);
    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views
            .inspector
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3)
    );
    assert!(
        !views.inspector.widgets.iter().any(|w| w.id == 4),
        "untagged Primary SettingsCard must stay out of Inspector"
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 4));
    assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
    assert!(views.overlapping_ids().is_empty());
}

#[test]
fn data_region_prop_maps_into_widget_props() {
    use nana_js_engine::HostValue;
    use std::collections::BTreeMap;

    let mut map = BTreeMap::new();
    map.insert("data-region".into(), HostValue::string("global-navigation"));
    map.insert(
        "data-agent-id".into(),
        HostValue::string("nana.workspace.sidebar"),
    );
    let props = WidgetProps::from_map(&map);
    assert_eq!(props.region, "global-navigation");
    assert_eq!(props.agent_id, "nana.workspace.sidebar");

    let mut patched = WidgetProps::default();
    patched.apply_prop("data-region", &HostValue::string("section-navigation"));
    assert_eq!(patched.region, "section-navigation");
}

#[test]
fn region_views_honor_data_region_and_sidebar_agent_contract() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    // nanavue NanaWorkspaceShell aside / NanaSidebarFrame contract tags
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "nana.workspace.sidebar".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::SidebarFrame,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "sidebar.main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            region: "section-navigation".into(),
            agent_id: "nana.sidebar-nav".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Column,
        WidgetProps {
            region: "primary".into(),
            agent_id: "nana.workspace.primary".into(),
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Text,
        WidgetProps {
            label: "body".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 2, None);
    bridge.insert_child(5, 1, None);
    bridge.insert_child(6, 5, None);

    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views.overlapping_ids().is_empty(),
        "overlapping: {:?}",
        views.overlapping_ids()
    );
    assert!(
        !views.navigation.widgets.is_empty(),
        "contract-tagged navigation must be non-empty"
    );
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
    assert!(views.navigation.widgets.iter().any(|w| w.id == 4));
    assert!(
        !views
            .navigation
            .widgets
            .iter()
            .any(|w| w.id == 5 || w.id == 6),
        "primary-tagged content must not enter navigation"
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 5));
    assert!(views.primary.widgets.iter().any(|w| w.id == 6));
    assert!(
        !views
            .primary
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3 || w.id == 4),
        "navigation-tagged nodes must be exclusive of primary"
    );
}

#[test]
fn region_views_limited_excludes_truncated_tagged_seeds_from_primary() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "nav.a".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            label: "A".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            region: "section-navigation".into(),
            agent_id: "nav.b".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Text,
        WidgetProps {
            label: "B".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Column,
        WidgetProps {
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);
    bridge.insert_child(5, 4, None);
    bridge.insert_child(6, 1, None);

    let snap = bridge.snapshot();
    let views = snap.region_views_limited(1, 0);
    assert!(
        views.overlapping_ids().is_empty(),
        "overlapping: {:?}",
        views.overlapping_ids()
    );
    assert_eq!(
        views.navigation.roots.len(),
        1,
        "nav_limit=1 projects a single seed"
    );
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(
        !views
            .navigation
            .widgets
            .iter()
            .any(|w| w.id == 4 || w.id == 5),
        "truncated tagged seed must not appear in the limited projection"
    );
    assert!(
        !views
            .primary
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3 || w.id == 4 || w.id == 5),
        "all region-tagged nodes must leave primary even when truncated"
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 6));
    assert!(views.inspector.widgets.is_empty());
}

#[test]
fn region_views_inspector_nested_under_nav_is_exclusive() {
    // Nearest-tag rule: inspector under a navigation ancestor belongs to
    // Inspector only — nav must not re-harvest that subtree.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "nana.workspace.sidebar".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::SidebarRow,
        WidgetProps {
            label: "Home".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            role: "inspector".into(),
            agent_id: "nana.workspace.inspector".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Text,
        WidgetProps {
            label: "facts".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Column,
        WidgetProps {
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 2, None);
    bridge.insert_child(5, 4, None);
    bridge.insert_child(6, 1, None);

    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views.overlapping_ids().is_empty(),
        "overlapping: {:?}",
        views.overlapping_ids()
    );
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
    assert!(
        !views
            .navigation
            .widgets
            .iter()
            .any(|w| w.id == 4 || w.id == 5),
        "inspector subtree must not remain in navigation"
    );
    assert!(views.inspector.widgets.iter().any(|w| w.id == 4));
    assert!(views.inspector.widgets.iter().any(|w| w.id == 5));
    assert!(
        !views
            .inspector
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3)
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 6));
    assert!(
        !views
            .primary
            .widgets
            .iter()
            .any(|w| w.id == 2 || w.id == 3 || w.id == 4 || w.id == 5)
    );
}

#[test]
fn region_views_dual_tagged_node_prefers_inspector() {
    // Dual Navigation+Inspector markers on one node → Inspector wins.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            role: "inspector".into(),
            agent_id: "nana.workspace.sidebar".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            label: "panel".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);

    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(
        views.overlapping_ids().is_empty(),
        "overlapping: {:?}",
        views.overlapping_ids()
    );
    assert!(
        views.navigation.widgets.is_empty(),
        "dual-tagged seed must not also project as navigation"
    );
    assert!(views.inspector.widgets.iter().any(|w| w.id == 2));
    assert!(views.inspector.widgets.iter().any(|w| w.id == 3));
    assert!(views.primary.widgets.iter().any(|w| w.id == 4));
    assert!(!views.primary.widgets.iter().any(|w| w.id == 2 || w.id == 3));
}

#[test]
fn region_views_limited_keeps_nav_insp_exclusive_after_truncation() {
    // Truncated nav seed still claims its forest from primary; nested
    // inspector under a kept nav seed stays exclusive of navigation.
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Row, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            region: "global-navigation".into(),
            agent_id: "nav.kept".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            label: "nav-body".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        4,
        WidgetKind::Column,
        WidgetProps {
            region: "inspector".into(),
            agent_id: "nana.workspace.inspector".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        5,
        WidgetKind::Text,
        WidgetProps {
            label: "insp-body".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        6,
        WidgetKind::Column,
        WidgetProps {
            region: "section-navigation".into(),
            agent_id: "nav.truncated".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        7,
        WidgetKind::Text,
        WidgetProps {
            label: "trunc".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        8,
        WidgetKind::Column,
        WidgetProps {
            label: "main".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 2, None);
    bridge.insert_child(5, 4, None);
    bridge.insert_child(6, 1, None);
    bridge.insert_child(7, 6, None);
    bridge.insert_child(8, 1, None);

    let snap = bridge.snapshot();
    let views = snap.region_views_limited(1, 1);
    assert!(
        views.overlapping_ids().is_empty(),
        "overlapping after limit: {:?}",
        views.overlapping_ids()
    );
    assert_eq!(views.navigation.roots.len(), 1);
    assert!(views.navigation.widgets.iter().any(|w| w.id == 2));
    assert!(views.navigation.widgets.iter().any(|w| w.id == 3));
    assert!(
        !views
            .navigation
            .widgets
            .iter()
            .any(|w| w.id == 4 || w.id == 5),
        "nested inspector must stay out of limited navigation"
    );
    assert!(
        !views
            .navigation
            .widgets
            .iter()
            .any(|w| w.id == 6 || w.id == 7),
        "truncated nav seed omitted from projection"
    );
    assert!(views.inspector.widgets.iter().any(|w| w.id == 4));
    assert!(views.inspector.widgets.iter().any(|w| w.id == 5));
    assert!(
        !views
            .primary
            .widgets
            .iter()
            .any(|w| { matches!(w.id, 2..=7) }),
        "all region-owned ids claimed from primary despite truncation"
    );
    assert!(views.primary.widgets.iter().any(|w| w.id == 8));
}

#[test]
fn untagged_forest_stays_in_primary_without_region_tags() {
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            agent_id: "nana.workspace.primary".into(),
            region: "primary".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            label: "only primary".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    let snap = bridge.snapshot();
    let views = snap.region_views();
    assert!(views.navigation.widgets.is_empty());
    assert!(views.inspector.widgets.is_empty());
    assert_eq!(views.primary.widgets.len(), snap.widgets.len());
    assert!(views.overlapping_ids().is_empty());
}

#[test]
fn migration_component_props_keep_typed_semantics() {
    let props = WidgetProps::from_map(&BTreeMap::from([
        (
            "cardKind".into(),
            nana_js_engine::HostValue::string("raised"),
        ),
        (
            "controlPosition".into(),
            nana_js_engine::HostValue::string("start"),
        ),
        ("autoHeight".into(), nana_js_engine::HostValue::Bool(true)),
        ("loading".into(), nana_js_engine::HostValue::Bool(true)),
        ("invalid".into(), nana_js_engine::HostValue::Bool(true)),
        ("readonly".into(), nana_js_engine::HostValue::Bool(true)),
        ("type".into(), nana_js_engine::HostValue::string("password")),
        ("step".into(), nana_js_engine::HostValue::Number(0.25)),
    ]));

    assert_eq!(props.card_kind, CardKind::Raised);
    assert_eq!(props.control_position, SwitchControlPosition::Start);
    assert!(props.auto_height);
    assert!(props.loading);
    assert!(props.invalid);
    assert!(props.read_only);
    assert!(props.secure);
    assert_eq!(props.step, 0.25);
}

#[test]
fn hover_stylesheet_restyle_applies_only_while_hovered() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["ok".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".ok { background: rgb(0, 0, 255); } .ok:hover { background: red; }");
    let idle = bridge.get(1).expect("widget").props.layout.background;
    {
        bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
            hovered: BTreeMap::from([(1, ())]),
            ..Default::default()
        });
        bridge.reapply_layout_for(1);
    }
    let hovered = bridge.get(1).expect("widget").props.layout.background;
    assert_ne!(idle, hovered);
    assert_eq!(hovered, Some([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn interactive_dirty_ids_cover_subject_descendants_and_focus_chains() {
    // tree: 1 root → 2 card → 3 icon; 1 → 4 button
    let mut bridge = MessageBridge::new();
    bridge.register(1, WidgetKind::Column, WidgetProps::default());
    bridge.register(2, WidgetKind::Card, WidgetProps::default());
    bridge.register(3, WidgetKind::Icon, WidgetProps::default());
    bridge.register(4, WidgetKind::Button, WidgetProps::default());
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.insert_child(4, 1, None);

    // Steady state: identical snapshot → nothing recascades.
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        hovered: BTreeMap::from([(2, ())]),
        ..Default::default()
    });
    let steady = bridge
        .interactive_dirty_ids(&InteractiveRuntimeSnapshot {
            hovered: BTreeMap::from([(2, ())]),
            ..Default::default()
        })
        .expect("previous snapshot exists");
    assert!(steady.is_empty(), "unchanged snapshot must dirty nobody");

    // New hover subject appears: only it is dirty — the already-hovered
    // card subtree keeps its state and stays out.
    let moved = bridge
        .interactive_dirty_ids(&InteractiveRuntimeSnapshot {
            hovered: BTreeMap::from([(2, ()), (4, ())]),
            ..Default::default()
        })
        .expect("previous snapshot exists");
    assert!(
        !moved.contains(&2) && !moved.contains(&3),
        "card subtree out"
    );
    assert!(moved.contains(&4), "new hover subject in");

    // Card gains hover: the card and its `.icon` descendant recascade.
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot::default());
    let card_hover = bridge
        .interactive_dirty_ids(&InteractiveRuntimeSnapshot {
            hovered: BTreeMap::from([(2, ())]),
            ..Default::default()
        })
        .expect("previous snapshot exists");
    assert!(card_hover.contains(&2) && card_hover.contains(&3));
    assert!(!card_hover.contains(&4), "sibling button stays out");

    // Focus move: old/new subjects, their descendants, and both
    // `:focus-within` ancestor chains.
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        focused: Some(3),
        ..Default::default()
    });
    let focus_move = bridge
        .interactive_dirty_ids(&InteractiveRuntimeSnapshot {
            focused: Some(4),
            ..Default::default()
        })
        .expect("previous snapshot exists");
    for id in [3, 4, 1, 2] {
        assert!(focus_move.contains(&id), "widget {id} must be dirty");
    }
}

#[test]
fn interactive_pass_re_cascades_only_dirty_widgets_after_hover_ends() {
    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["ok".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["ok".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".ok { background: rgb(0, 0, 255); } .ok:hover { background: red; }");
    // Widget 1 was hovered in the previous pass; the Runtime snapshot the
    // pass collects from an idle document is empty, so hover ends.
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        hovered: BTreeMap::from([(1, ())]),
        ..Default::default()
    });
    bridge.reapply_layout_for(1);
    assert_eq!(
        bridge.get(1).expect("hovered").props.layout.background,
        Some([1.0, 0.0, 0.0, 1.0])
    );
    bridge.reapply_interactive_cascade(&mut doc);
    assert_eq!(
        bridge.get(1).expect("unhovered").props.layout.background,
        Some([0.0, 0.0, 1.0, 1.0]),
        "hover end must restore the base paint"
    );
    assert_eq!(
        bridge.get(2).expect("idle sibling").props.layout.background,
        Some([0.0, 0.0, 1.0, 1.0])
    );
    // Steady state: a second pass with no runtime change keeps the paint.
    bridge.reapply_interactive_cascade(&mut doc);
    assert_eq!(
        bridge.get(1).expect("steady").props.layout.background,
        Some([0.0, 0.0, 1.0, 1.0])
    );
}

#[test]
fn card_hover_restyles_descendant_icon() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        10,
        WidgetKind::Card,
        WidgetProps {
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        11,
        WidgetKind::Icon,
        WidgetProps {
            class_names: vec!["icon".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(11, 10, None);
    bridge.inject_stylesheet(".icon { color: blue; } .card:hover .icon { color: red; }");
    let idle = {
        bridge.cascade.interactive_runtime = None;
        bridge.reapply_layout_for(11);
        bridge.get(11).expect("icon").props.layout.color
    };
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        hovered: BTreeMap::from([(10, ())]),
        ..Default::default()
    });
    bridge.reapply_layout_for(11);
    assert_eq!(
        bridge.get(11).expect("icon").props.layout.color,
        Some([1.0, 0.0, 0.0, 1.0])
    );
    bridge.cascade.interactive_runtime = None;
    bridge.reapply_layout_for(11);
    assert_eq!(bridge.get(11).expect("icon").props.layout.color, idle);
}

#[test]
fn inject_stylesheet_keeps_interactive_and_generated_buckets() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
            ".chip::before { content: \"\"; width: 4px; } .chip:hover { color: red; } .panel::-webkit-scrollbar { width: 8px; }",
        );
    assert_eq!(bridge.generated_pseudo_rule_count(), 1);
    assert_eq!(bridge.interactive_rule_count(), 1);
    assert_eq!(bridge.scrollbar_pseudo_rule_count(), 1);
}

#[test]
fn stylesheet_object_fit_cascades_onto_img_layout() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "img".into(),
            attrs: {
                let mut attrs = BTreeMap::new();
                attrs.insert("object-fit".into(), "contain".into());
                attrs
            },
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet("img { object-fit: cover; }");
    assert_eq!(
        bridge.get(1).expect("img").props.layout.paint.object_fit,
        Some(nana_ui_core::BackgroundImageFit::Cover),
        "stylesheet object-fit must beat the HTML presentational hint"
    );
}

#[test]
fn webkit_scrollbar_pseudo_skins_originating_layout() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["panel".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        r#"
            .panel::-webkit-scrollbar { width: 8px; background: #111111; }
            .panel::-webkit-scrollbar-thumb { background: #ff0000; height: 4px; }
            "#,
    );
    let skin = bridge
        .get(1)
        .expect("panel")
        .props
        .layout
        .paint
        .scrollbar
        .expect("scrollbar skin");
    assert!((skin.thickness.unwrap() - 8.0).abs() < 0.01);
    assert!((skin.thumb_thickness.unwrap() - 4.0).abs() < 0.01);
    let track = skin.track_color.expect("track");
    assert!((track[0] - 0x11 as f32 / 255.0).abs() < 0.02);
    let thumb = skin.thumb_color.expect("thumb");
    assert!((thumb[0] - 1.0).abs() < 0.01);
    assert!(thumb[1].abs() < 0.01);
}

#[test]
fn placeholder_pseudo_paints_input_and_skips_non_inputs() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Input,
        WidgetProps {
            element_tag: "input".into(),
            placeholder: "hint".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["field".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
            "input::placeholder { color: gray; opacity: 0.5 } .field::placeholder { color: red; width: 40px; }",
        );
    let input = bridge.get(1).expect("input");
    assert_eq!(
        input.props.layout.placeholder_color,
        Some([0.5, 0.5, 0.5, 1.0])
    );
    assert_eq!(input.props.layout.placeholder_opacity, Some(0.5));
    let div = bridge.get(2).expect("div");
    assert!(div.props.layout.placeholder_color.is_none());
    assert!(div.props.layout.width.is_none());
    assert!(
        !bridge.snapshot().widgets.iter().any(|w| {
            w.props.attrs.get(GENERATED_PSEUDO_ATTR).map(String::as_str) == Some("placeholder")
        }),
        "::placeholder must not materialize a generated box"
    );
}

#[test]
fn has_descendant_present_restyles_parent() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            class_names: vec!["badge".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".card { width: 10px; } .card:has(.badge) { width: 80px; }");
    let parent = bridge.get(1).expect("card");
    assert_eq!(parent.props.layout.width, Some(LengthSpec::Px(80.0)));
}

#[test]
fn has_descendant_present_restyles_parent_on_insert_and_remove() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".card { width: 10px; } .card:has(.badge) { width: 80px; }");
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(10.0))
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            class_names: vec!["badge".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(80.0))
    );
    bridge.unregister(2);
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(10.0))
    );
}

#[test]
fn has_descendant_present_restyles_parent_on_class_toggle() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(2, WidgetKind::Text, WidgetProps::default());
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".card { color: black; } .card:has(.badge) { color: red; }");
    assert_eq!(
        bridge.get(1).expect("card").props.layout.color,
        Some([0.0, 0.0, 0.0, 1.0])
    );

    bridge.patch_prop(2, "class", &HostValue::string("badge"));
    assert_eq!(
        bridge.get(1).expect("card").props.layout.color,
        Some([1.0, 0.0, 0.0, 1.0])
    );

    bridge.patch_prop(2, "classname", &HostValue::string("plain"));
    assert_eq!(
        bridge.get(1).expect("card").props.layout.color,
        Some([0.0, 0.0, 0.0, 1.0])
    );
}

#[test]
fn inject_font_face_skips_failed_load() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
        r#"
            @font-face {
                font-family: "Display";
                src: url("./missing.woff2");
                font-weight: 400;
            }
            "#,
    );
    assert!(
        bridge.registered_font_faces().is_empty(),
        "failed/missing font load must not register"
    );
}

#[test]
fn inject_font_face_unknown_local_is_fail_closed() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
        r#"
            @font-face {
                font-family: "Display";
                src: local("DefinitelyNotANanaFont_xyz");
                font-weight: 400;
            }
            "#,
    );
    assert!(
        bridge.registered_font_faces().is_empty(),
        "unmatched local() must fail closed"
    );
}

#[test]
fn inject_font_face_local_then_url_falls_back_to_url() {
    let jail = std::env::temp_dir().join(format!(
        "nanaui-bridge-font-{}-local-fallback",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&jail);
    std::fs::create_dir_all(&jail).expect("jail");
    std::fs::write(
        jail.join("ok.ttf"),
        include_bytes!("../../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
    )
    .expect("ttf");
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(jail.clone());
    bridge.inject_stylesheet(
        r#"
            @font-face {
                font-family: "Display";
                src: local("DefinitelyNotANanaFont_xyz"), url("./ok.ttf");
                font-weight: 400;
            }
            "#,
    );
    assert_eq!(bridge.registered_font_faces().len(), 1);
    assert_eq!(bridge.registered_font_faces()[0].family, "Display");
    assert_eq!(
        bridge.registered_font_faces()[0].src[0],
        crate::css_at_rule::FontFaceSrc::Local("DefinitelyNotANanaFont_xyz".into())
    );
    let _ = std::fs::remove_dir_all(&jail);
}

#[test]
fn inject_font_face_url_then_unknown_local_does_not_register_on_miss() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
        r#"
            @font-face {
                font-family: "Display";
                src: url("./missing.woff2"), local("DefinitelyNotANanaFont_xyz");
                font-weight: 400;
            }
            "#,
    );
    assert!(
        bridge.registered_font_faces().is_empty(),
        "missing url then unmatched local() must not register"
    );
}

#[test]
fn inject_font_face_falls_back_to_next_src_url() {
    let jail = std::env::temp_dir().join(format!(
        "nanaui-bridge-font-{}-src-fallback",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&jail);
    std::fs::create_dir_all(&jail).expect("jail");
    std::fs::write(
        jail.join("ok.ttf"),
        include_bytes!("../../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
    )
    .expect("ttf");
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(jail.clone());
    bridge.inject_stylesheet(
        r#"
            @font-face {
                font-family: "Display";
                src: url("./missing.woff2") format("woff2") tech("color-COLRv0"),
                     url("./ok.ttf") format("truetype");
                font-weight: 400;
            }
            "#,
    );
    assert_eq!(bridge.registered_font_faces().len(), 1);
    assert_eq!(bridge.registered_font_faces()[0].family, "Display");
    assert_eq!(
        crate::css_at_rule::font_face_url_srcs(&bridge.registered_font_faces()[0])
            .collect::<Vec<_>>(),
        vec!["./missing.woff2", "./ok.ttf"]
    );
    let _ = std::fs::remove_dir_all(&jail);
}

#[test]
fn inject_font_face_registers_once_and_dedupes() {
    let jail =
        std::env::temp_dir().join(format!("nanaui-bridge-font-{}-dedupe", std::process::id()));
    let _ = std::fs::remove_dir_all(&jail);
    std::fs::create_dir_all(&jail).expect("jail");
    std::fs::write(
        jail.join("Display.woff2"),
        include_bytes!("../../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
    )
    .expect("font");
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(jail.clone());
    let css = r#"
            @font-face {
                font-family: "Display";
                src: url("./Display.woff2");
                font-weight: 400;
            }
        "#;
    bridge.inject_stylesheet(css);
    bridge.inject_stylesheet(css);
    assert_eq!(bridge.registered_font_faces().len(), 1);
    assert_eq!(bridge.registered_font_faces()[0].family, "Display");
    assert_eq!(bridge.registered_font_faces()[0].weight, Some(400));
    let _ = std::fs::remove_dir_all(&jail);
}

#[test]
fn inject_print_font_face_is_not_registered_on_screen() {
    let jail =
        std::env::temp_dir().join(format!("nanaui-bridge-font-{}-print", std::process::id()));
    let _ = std::fs::remove_dir_all(&jail);
    std::fs::create_dir_all(&jail).expect("jail");
    std::fs::write(
        jail.join("print.ttf"),
        include_bytes!("../../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
    )
    .expect("font");
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(jail.clone());
    bridge.inject_stylesheet(
        r#"
            @media print {
                @font-face {
                    font-family: "PrintOnly";
                    src: url("./print.ttf");
                }
                .print { width: 10px; }
            }
            "#,
    );
    assert!(
        bridge.registered_font_faces().is_empty(),
        "unmatched @media print @font-face must not register"
    );
    let _ = std::fs::remove_dir_all(&jail);
}

#[test]
fn inject_font_url_resolves_relative_to_importing_sheet() {
    let jail = std::env::temp_dir().join(format!("nanaui-bridge-font-{}-rel", std::process::id()));
    let sheets = jail.join("sheets");
    let fonts = sheets.join("fonts");
    let _ = std::fs::remove_dir_all(&jail);
    std::fs::create_dir_all(&fonts).expect("fonts");
    std::fs::write(
        fonts.join("n.ttf"),
        include_bytes!("../../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"),
    )
    .expect("ttf");
    std::fs::write(
        sheets.join("theme.css"),
        r#"
            @font-face {
                font-family: "Rel";
                src: url("./fonts/n.ttf");
                font-weight: 400;
            }
            "#,
    )
    .expect("theme");
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(jail.clone());
    bridge.inject_stylesheet("@import url(\"sheets/theme.css\");");
    assert_eq!(bridge.registered_font_faces().len(), 1);
    assert_eq!(bridge.registered_font_faces()[0].family, "Rel");
    let _ = std::fs::remove_dir_all(&jail);
}

#[test]
fn media_min_width_recascades_when_viewport_changes() {
    let mut bridge = MessageBridge::new();
    let mut props = WidgetProps::default();
    props.class_names = vec!["wide".into()];
    props.element_tag = "div".into();
    bridge.register(1, WidgetKind::Column, props);
    bridge.inject_stylesheet("@media (min-width: 800px) { .wide { width: 100px; } }");
    // Default media env is 960×640, so the rule applies on inject.
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(100.0))
    );
    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(400.0, 300.0));
    assert!(
        bridge.get(1).unwrap().props.layout.width.is_none(),
        "narrow viewport must drop the min-width:800px rule"
    );
    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(900.0, 300.0));
    assert_eq!(
        bridge.get(1).unwrap().props.layout.width,
        Some(LengthSpec::Px(100.0))
    );
}

#[test]
fn transition_duration_from_stylesheet_is_nonzero() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["btn".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".btn { transition: opacity 0.2s; }");
    let motion = bridge.computed_motion_for(1).expect("motion");
    assert_eq!(motion.transition_duration, "0.2s");
    assert!(motion.has_transition());
}

#[test]
fn media_layer_and_has_drive_bridge_cascade() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["badge".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(
        r#"
            @layer base, override;
            @layer base { .card { gap: 4px; } }
            @layer override { .card { gap: 8px; } }
            .card:has(.badge) { width: 40px; }
            @media (prefers-color-scheme: dark) { .card { height: 20px; } }
            "#,
    );
    assert_eq!(
        bridge.get(1).expect("card").props.layout.gap,
        Some(LengthSpec::Px(8.0))
    );
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(40.0))
    );
    assert!(bridge.get(1).expect("card").props.layout.height.is_none());
    bridge.set_theme(ThemeMode::Dark);
    assert_eq!(
        bridge.get(1).expect("card").props.layout.height,
        Some(LengthSpec::Px(20.0))
    );
}

#[test]
fn focus_recascade_updates_runtime_node_style() {
    use nana_ui_runtime::StableNodeId;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let field = doc.create_element("input");
    doc.insert(field, root, None);
    bridge.register(
        field.0,
        WidgetKind::Input,
        WidgetProps {
            class_names: vec!["field".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".field { background: rgb(0, 0, 255); } .field:focus { background: rgb(0, 255, 0); }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.set_focus(field);
    bridge.reapply_interactive_cascade(&mut doc);
    bridge.sync_cascaded_layout_into_runtime(&mut doc);
    doc.flush_host_frame();
    let style = doc
        .world()
        .node_style(StableNodeId::new(field.0).unwrap())
        .expect("field runtime style");
    assert_eq!(style.layout.background, Some([0.0, 1.0, 0.0, 1.0]));
}

#[test]
fn not_disabled_matches_enabled_button_and_skips_disabled() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["btn".into()],
            element_tag: "button".into(),
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet("button:not(:disabled) { width: 24px; }");
    assert_eq!(
        bridge.get(1).expect("btn").props.layout.width,
        Some(LengthSpec::Px(24.0))
    );
    bridge.patch_prop(1, "disabled", &HostValue::Bool(true));
    assert!(bridge.get(1).expect("btn").props.layout.width.is_none());
}

#[test]
fn subject_disabled_restyles_on_patch() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Button,
        WidgetProps {
            element_tag: "button".into(),
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet("button:disabled { width: 18px; }");
    assert!(bridge.get(1).expect("btn").props.layout.width.is_none());
    bridge.patch_prop(1, "disabled", &HostValue::Bool(true));
    assert_eq!(
        bridge.get(1).expect("btn").props.layout.width,
        Some(LengthSpec::Px(18.0))
    );
}

#[test]
fn checked_plus_label_restyles_following_sibling() {
    let mut bridge = MessageBridge::new();
    let mut input_attrs = BTreeMap::new();
    input_attrs.insert("type".into(), "checkbox".into());
    bridge.register(
        1,
        WidgetKind::Checkbox,
        WidgetProps {
            element_tag: "input".into(),
            attrs: input_attrs,
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "label".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(3, WidgetKind::Column, WidgetProps::default());
    bridge.insert_child(1, 3, None);
    bridge.insert_child(2, 3, None);
    bridge.inject_stylesheet("input:checked + label { width: 20px; }");
    assert!(bridge.get(2).expect("label").props.layout.width.is_none());
    bridge.patch_prop(1, "checked", &HostValue::Bool(true));
    assert_eq!(
        bridge.get(2).expect("label").props.layout.width,
        Some(LengthSpec::Px(20.0))
    );
    bridge.patch_prop(1, "checked", &HostValue::Bool(false));
    assert!(bridge.get(2).expect("label").props.layout.width.is_none());
}

#[test]
fn empty_matches_whitespace_text_but_not_element_or_text() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".box:empty { width: 8px; } .box:not(:empty) { height: 12px; }");
    assert_eq!(
        bridge.get(1).expect("empty").props.layout.width,
        Some(LengthSpec::Px(8.0))
    );
    assert!(bridge.get(1).expect("empty").props.layout.height.is_none());

    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "#text".into(),
            label: " \u{00a0}\n\t".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    assert_eq!(
        bridge.get(1).expect("ws").props.layout.width,
        Some(LengthSpec::Px(8.0)),
        "whitespace-only UTF-8 text still :empty"
    );

    bridge.set_label(2, "hi");
    assert!(bridge.get(1).expect("text").props.layout.width.is_none());
    assert_eq!(
        bridge.get(1).expect("text").props.layout.height,
        Some(LengthSpec::Px(12.0))
    );

    let mut bridge2 = MessageBridge::new();
    bridge2.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge2.register(
        2,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "span".into(),
            ..WidgetProps::default()
        },
    );
    bridge2.inject_stylesheet(".box:empty { width: 8px; } .box:not(:empty) { height: 12px; }");
    bridge2.insert_child(2, 1, None);
    assert!(bridge2.get(1).expect("el").props.layout.width.is_none());
    assert_eq!(
        bridge2.get(1).expect("el").props.layout.height,
        Some(LengthSpec::Px(12.0))
    );
}

#[test]
fn only_child_and_nth_last_restyle_on_insert() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["row".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "p".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(
        ".row > :only-child { width: 10px; } .row > :nth-last-child(1) { height: 6px; }",
    );
    assert_eq!(
        bridge.get(2).expect("only").props.layout.width,
        Some(LengthSpec::Px(10.0))
    );
    assert_eq!(
        bridge.get(2).expect("only").props.layout.height,
        Some(LengthSpec::Px(6.0))
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "p".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(3, 1, None);
    assert!(bridge.get(2).expect("first").props.layout.width.is_none());
    assert!(bridge.get(2).expect("first").props.layout.height.is_none());
    assert_eq!(
        bridge.get(3).expect("last").props.layout.height,
        Some(LengthSpec::Px(6.0))
    );
}

#[test]
fn host_set_element_text_is_not_empty() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(".box:empty { width: 8px; } .box:not(:empty) { height: 12px; }");
    assert_eq!(
        bridge.get(1).expect("empty").props.layout.width,
        Some(LengthSpec::Px(8.0))
    );
    assert!(bridge.get(1).expect("empty").props.layout.height.is_none());
    bridge.set_label(1, "hello");
    assert!(
        bridge.get(1).expect("text").props.layout.width.is_none(),
        "setElementText / host label is content; :empty must not match"
    );
    assert_eq!(
        bridge.get(1).expect("text").props.layout.height,
        Some(LengthSpec::Px(12.0))
    );
}

#[test]
fn dialog_checked_attr_does_not_match_and_toggle_off_clears() {
    let mut dialog = MessageBridge::new();
    dialog.register(
        1,
        WidgetKind::Dialog,
        WidgetProps {
            element_tag: "dialog".into(),
            ..WidgetProps::default()
        },
    );
    dialog.inject_stylesheet("dialog:checked { width: 20px; }");
    dialog.patch_prop(1, "checked", &HostValue::Bool(true));
    assert!(
        dialog.get(1).expect("dialog").props.layout.width.is_none(),
        "non-checkable host checked attr must not match :checked"
    );

    let mut box_bridge = MessageBridge::new();
    let mut attrs = BTreeMap::new();
    attrs.insert("type".into(), "checkbox".into());
    box_bridge.register(
        1,
        WidgetKind::Checkbox,
        WidgetProps {
            element_tag: "input".into(),
            attrs,
            ..WidgetProps::default()
        },
    );
    box_bridge.note_toggle(1, true);
    assert!(
        box_bridge
            .get(1)
            .expect("on")
            .props
            .attrs
            .contains_key("checked")
    );
    box_bridge.note_toggle(1, false);
    assert!(
        !box_bridge
            .get(1)
            .expect("off")
            .props
            .attrs
            .contains_key("checked"),
        "note_toggle(false) must remove attrs[\"checked\"]"
    );
    box_bridge.patch_prop(1, "checked", &HostValue::Bool(true));
    assert!(
        box_bridge
            .get(1)
            .expect("patched on")
            .props
            .attrs
            .contains_key("checked")
    );
    box_bridge.patch_prop(1, "toggled", &HostValue::Bool(false));
    assert!(
        !box_bridge
            .get(1)
            .expect("toggled off")
            .props
            .attrs
            .contains_key("checked"),
        "toggled=false must remove attrs[\"checked\"]"
    );
}

#[test]
fn generated_pseudo_excluded_from_sibling_counts() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["row".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "p".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    let mut after_attrs = BTreeMap::new();
    after_attrs.insert(GENERATED_PSEUDO_ATTR.into(), "after".into());
    bridge.register(
        3,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            attrs: after_attrs,
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(3, 1, None);
    bridge.inject_stylesheet(
        ".row > :only-child { width: 10px; } .row > :nth-last-child(1) { height: 6px; }",
    );
    assert_eq!(
        bridge.get(2).expect("real").props.layout.width,
        Some(LengthSpec::Px(10.0)),
        ":only-child stays true when the extra sibling is a generated box"
    );
    assert_eq!(
        bridge.get(2).expect("real").props.layout.height,
        Some(LengthSpec::Px(6.0)),
        "last real element is :nth-last-child(1) despite parent ::after"
    );
    assert!(
        bridge.get(3).expect("after").props.layout.width.is_none()
            && bridge.get(3).expect("after").props.layout.height.is_none()
    );
}

#[test]
fn empty_span_child_makes_parent_not_empty() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "span".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".box:empty { width: 8px; } .box:not(:empty) { height: 12px; }");
    assert!(
        bridge.get(1).expect("wrap").props.layout.width.is_none(),
        "empty span is an element child; parent must not match :empty"
    );
    assert_eq!(
        bridge.get(1).expect("wrap").props.layout.height,
        Some(LengthSpec::Px(12.0))
    );
}

#[test]
fn create_element_p_with_create_text_is_not_empty() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "p".into(),
            ..WidgetProps::default()
        },
    );
    // createText: WidgetKind::Text, empty tag → register fills nana-text.
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            label: "hello".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 2, None);
    bridge.inject_stylesheet(
            ".box:empty { width: 8px; } .box:not(:empty) { height: 12px; } p:not(:empty) { padding: 2px; }",
        );
    assert!(
        bridge.get(1).expect("parent").props.layout.width.is_none(),
        "createElement p + createText hello must not match parent :empty"
    );
    assert_eq!(
        bridge.get(1).expect("parent").props.layout.height,
        Some(LengthSpec::Px(12.0))
    );
    assert_eq!(
        bridge.get(2).expect("p").props.layout.padding,
        Some(LengthSpec::Px(2.0)),
        "p is not :empty: createText grandchild is non-whitespace text"
    );
}

#[test]
fn text_node_excluded_from_element_sibling_counts() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["row".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Text,
        WidgetProps {
            label: "Hello".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Text,
        WidgetProps {
            element_tag: "span".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 1, None);
    bridge.inject_stylesheet(
            ".row > :first-child { width: 10px; } .row > :only-child { height: 8px; } .row > :nth-last-child(1) { padding: 4px; }",
        );
    assert!(
        bridge.get(2).expect("text").props.layout.width.is_none()
            && bridge.get(2).expect("text").props.layout.height.is_none()
            && bridge.get(2).expect("text").props.layout.padding.is_none(),
        "createText / nana-text is not an element sibling"
    );
    assert_eq!(
        bridge.get(3).expect("span").props.layout.width,
        Some(LengthSpec::Px(10.0)),
        "span is :first-child among elements"
    );
    assert_eq!(
        bridge.get(3).expect("span").props.layout.height,
        Some(LengthSpec::Px(8.0)),
        "span is :only-child among elements"
    );
    assert_eq!(
        bridge.get(3).expect("span").props.layout.padding,
        Some(LengthSpec::Px(4.0)),
        "nth-last-child(1) is the span, not the text widget"
    );
}

#[test]
fn hover_not_disabled_does_not_apply_when_disabled() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["btn".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".btn { background: rgb(0, 0, 255); } .btn:hover:not(:disabled) { background: red; }",
    );
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        hovered: BTreeMap::from([(1, ())]),
        ..Default::default()
    });
    bridge.reapply_layout_for(1);
    assert_eq!(
        bridge.get(1).expect("btn").props.layout.background,
        Some([1.0, 0.0, 0.0, 1.0])
    );
    bridge.patch_prop(1, "disabled", &HostValue::Bool(true));
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        hovered: BTreeMap::from([(1, ())]),
        ..Default::default()
    });
    bridge.reapply_layout_for(1);
    assert_eq!(
        bridge.get(1).expect("disabled").props.layout.background,
        Some([0.0, 0.0, 1.0, 1.0])
    );
}

#[test]
fn focus_within_parent_restyles_when_child_focused() {
    use nana_ui_runtime::StableNodeId;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let parent = doc.create_element("div");
    let child = doc.create_element("input");
    doc.insert(parent, root, None);
    doc.insert(child, parent, None);
    bridge.register(
        parent.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["field".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        child.0,
        WidgetKind::Input,
        WidgetProps {
            class_names: vec!["inner".into()],
            element_tag: "input".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(child.0, parent.0, None);
    bridge.inject_stylesheet(
            ".field { background: rgb(0, 0, 255); } .field:focus-within { background: rgb(0, 255, 0); }",
        );
    assert_eq!(
        bridge
            .get(parent.0)
            .expect("parent")
            .props
            .layout
            .background,
        Some([0.0, 0.0, 1.0, 1.0])
    );
    doc.set_focus(child);
    bridge.on_runtime_focus_change(&mut doc, None, Some(child.0));
    bridge.sync_cascaded_layout_into_runtime(&mut doc);
    doc.flush_host_frame();
    let style = doc
        .world()
        .node_style(StableNodeId::new(parent.0).unwrap())
        .expect("parent runtime style");
    assert_eq!(style.layout.background, Some([0.0, 1.0, 0.0, 1.0]));
    assert!(
        !bridge.has_interactive_css(),
        "focus-within must stay on the static cascade, not the hover recascade bucket"
    );
    doc.clear_focus();
    bridge.on_runtime_focus_change(&mut doc, Some(child.0), None);
    bridge.sync_cascaded_layout_into_runtime(&mut doc);
    doc.flush_host_frame();
    let style = doc
        .world()
        .node_style(StableNodeId::new(parent.0).unwrap())
        .expect("parent runtime style after blur");
    assert_eq!(
        style.layout.background,
        Some([0.0, 0.0, 1.0, 1.0]),
        "blur must drop :focus-within on the parent"
    );
}

#[test]
fn focus_within_ignores_stale_snapshot_after_interactive_bucket_drops() {
    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let parent = doc.create_element("div");
    let child = doc.create_element("input");
    doc.insert(parent, root, None);
    doc.insert(child, parent, None);
    bridge.register(
        parent.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["field".into()],
            element_tag: "div".into(),
            ..WidgetProps::default()
        },
    );
    bridge.register(
        child.0,
        WidgetKind::Input,
        WidgetProps {
            class_names: vec!["inner".into()],
            element_tag: "input".into(),
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(child.0, parent.0, None);
    bridge.inject_stylesheet(
        ".field { background: rgb(0, 0, 255); } \
             .field:focus-within { background: rgb(0, 255, 0); } \
             @media (min-width: 800px) { .btn:hover { background: red; } }",
    );
    assert!(
        bridge.has_interactive_css(),
        "wide default viewport must keep the hover bucket"
    );
    doc.set_focus(child);
    bridge.on_runtime_focus_change(&mut doc, None, Some(child.0));
    assert_eq!(
        bridge
            .get(parent.0)
            .expect("parent")
            .props
            .layout
            .background,
        Some([0.0, 1.0, 0.0, 1.0])
    );
    // Narrow viewport drops the hover rule; static :focus-within remains.
    bridge.sync_layout_containing_blocks(ParentBox::from_viewport(400.0, 300.0));
    assert!(
        !bridge.has_interactive_css(),
        "media flatten must drop the interactive bucket"
    );
    assert!(
        bridge.cascade.interactive_runtime.is_none(),
        "unused hover snapshot must not outlive the interactive bucket"
    );
    assert_eq!(
        bridge
            .get(parent.0)
            .expect("parent")
            .props
            .layout
            .background,
        Some([0.0, 1.0, 0.0, 1.0]),
        "live focus must still match :focus-within after the hover bucket drops"
    );
    // Plant a stale snapshot the way a missed clear used to leave focused=child.
    bridge.cascade.interactive_runtime = Some(InteractiveRuntimeSnapshot {
        focused: Some(child.0),
        ..Default::default()
    });
    doc.clear_focus();
    bridge.on_runtime_focus_change(&mut doc, Some(child.0), None);
    assert_eq!(
        bridge
            .get(parent.0)
            .expect("parent")
            .props
            .layout
            .background,
        Some([0.0, 0.0, 1.0, 1.0]),
        "blur must follow cascade_focused, not a leftover snapshot.focused"
    );
    assert!(bridge.cascade.interactive_runtime.is_none());
}

#[test]
fn transition_samples_mid_timeline_into_runtime_paint() {
    use nana_ui_runtime::StableNodeId;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let btn = doc.create_element("button");
    doc.insert(btn, root, None);
    bridge.register(
        btn.0,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["btn".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
             .btn:hover { background: red; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.set_runtime_clock_for_test(std::time::Duration::ZERO);
    doc.set_pointer_hover(0, Some(btn));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(std::time::Duration::from_millis(100));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let style = doc
        .world()
        .node_style(StableNodeId::new(btn.0).unwrap())
        .expect("btn runtime style");
    let bg = style.layout.background.expect("interpolated background");
    assert!(
        (bg[0] - 0.5).abs() < 0.05 && (bg[2] - 0.5).abs() < 0.05,
        "expected mid-transition purple-ish background, got {bg:?}"
    );
    doc.set_runtime_clock_for_test(std::time::Duration::from_millis(220));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let finished = doc
        .world()
        .node_style(StableNodeId::new(btn.0).unwrap())
        .expect("btn runtime style");
    assert_eq!(finished.layout.background, Some([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn transition_property_limits_lerp_to_listed_longhands() {
    use crate::css_interactive_apply::{
        CssPaintSnapshot, lerp_paint_for_properties, parse_transition_properties,
    };

    let from = CssPaintSnapshot {
        opacity: Some(0.2),
        background: Some([0.0, 0.0, 1.0, 1.0]),
        ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
    };
    let to = CssPaintSnapshot {
        opacity: Some(1.0),
        background: Some([1.0, 0.0, 0.0, 1.0]),
        ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
    };
    let mid = lerp_paint_for_properties(&from, &to, 0.5, &parse_transition_properties("opacity"));
    assert_eq!(mid.opacity, Some(0.6));
    assert_eq!(mid.background, Some([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn keyframes_animation_updates_runtime_node_opacity() {
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["spin".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        "@keyframes spin { from { opacity: 0; } to { opacity: 1; } } \
             .spin { animation: spin 1s linear; width: 40px; height: 40px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let frame = doc.advance_css_animations(Duration::from_millis(500));
    assert!(bridge.apply_css_animation_samples(&mut doc, frame));
    doc.flush_host_frame();
    let style = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("host runtime style");
    let opacity = style.layout.opacity.expect("animated opacity");
    assert!(
        (opacity - 0.5).abs() < 0.08,
        "mid keyframe opacity expected ~0.5, got {opacity}"
    );
}

#[test]
fn host_clock_dual_advance_applies_samples_once() {
    use nana_ui_runtime::StableNodeId;
    use std::time::{Duration, Instant};

    let host_epoch = Instant::now();
    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    doc.set_host_animation_epoch(host_epoch);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let btn = doc.create_element("button");
    doc.insert(btn, root, None);
    bridge.register(
        btn.0,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["btn".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
             .btn:hover { background: red; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.set_pointer_hover(0, Some(btn));
    bridge.reapply_interactive_cascade(&mut doc);
    let transition_start = doc.runtime_now();

    let first_now = transition_start + Duration::from_millis(100);
    let vue_frame = doc.advance_css_animations(first_now);
    assert!(bridge.apply_css_animation_samples(&mut doc, vue_frame));
    doc.flush_host_frame();
    let mid_style = doc
        .world()
        .node_style(StableNodeId::new(btn.0).unwrap())
        .expect("btn runtime style");
    let mid_bg = mid_style
        .layout
        .background
        .expect("interpolated background");
    assert!(
        (mid_bg[0] - 0.5).abs() < 0.05 && (mid_bg[2] - 0.5).abs() < 0.05,
        "expected mid-transition paint after first advance, got {mid_bg:?}"
    );

    bridge.reapply_interactive_cascade(&mut doc);
    doc.flush_host_frame();
    let after_reapply = doc
        .world()
        .node_style(StableNodeId::new(btn.0).unwrap())
        .expect("btn runtime style");
    let after_reapply_bg = after_reapply
        .layout
        .background
        .expect("background after reapply");
    assert_eq!(
        after_reapply_bg, mid_bg,
        "reapply_interactive_cascade must not tick CSS animations when host epoch is set"
    );

    let second_now = transition_start + Duration::from_millis(150);
    let host_frame = doc.advance_css_animations(second_now);
    assert!(bridge.apply_css_animation_samples(&mut doc, host_frame));
    doc.flush_host_frame();
    let host_style = doc
        .world()
        .node_style(StableNodeId::new(btn.0).unwrap())
        .expect("btn runtime style");
    let host_bg = host_style
        .layout
        .background
        .expect("host-advanced background");
    assert!(
        host_bg[0] > mid_bg[0] && host_bg[0] < 1.0,
        "host clock must continue the timeline, got {host_bg:?} after mid {mid_bg:?}"
    );
}

#[test]
fn hover_retarget_mid_transition_uses_unhovered_target() {
    use crate::css_interactive_apply::CssPaintSnapshot;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let btn = doc.create_element("button");
    doc.insert(btn, root, None);
    bridge.register(
        btn.0,
        WidgetKind::Button,
        WidgetProps {
            class_names: vec!["btn".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".btn { background: rgb(0, 0, 255); transition: background 200ms linear; } \
             .btn:hover { background: red; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.set_runtime_clock_for_test(std::time::Duration::ZERO);
    doc.set_pointer_hover(0, Some(btn));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(std::time::Duration::from_millis(100));
    assert!(bridge.tick_css_animations(&mut doc));

    doc.set_pointer_hover(0, None);
    bridge.reapply_interactive_cascade(&mut doc);
    let transition_to = bridge
        .css_transition_target(btn.0)
        .expect("hover-out should retarget the running transition");
    assert_eq!(
        transition_to.background,
        Some([0.0, 0.0, 1.0, 1.0]),
        "retarget destination must be the unhovered paint"
    );
    let current = CssPaintSnapshot::from_layout(&bridge.get(btn.0).expect("widget").props.layout);
    assert!(
        current
            .background
            .is_some_and(|bg| bg[0] > 0.0 && bg[0] < 1.0),
        "mid-transition paint should still reflect hover-in progress before retarget catches up, got {current:?}"
    );
}

#[test]
fn px_width_transition_dirties_layout() {
    use nana_ui_core::LengthSpec;
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let box_el = doc.create_element("div");
    doc.insert(box_el, root, None);
    bridge.register(
        box_el.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["box".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".box { width: 40px; height: 20px; transition: width 200ms linear; } \
             .box:hover { width: 80px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.set_runtime_clock_for_test(Duration::ZERO);
    doc.set_pointer_hover(0, Some(box_el));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(Duration::from_millis(100));
    assert!(bridge.tick_css_animations(&mut doc));
    let node = StableNodeId::new(box_el.0).expect("box id");
    let dirty = doc.world().pending_layout_dirty();
    assert!(
        dirty.contains(&node),
        "px width transition must dirty LAYOUT, got {dirty:?}"
    );
    doc.flush_host_frame();
    let style = doc.world().node_style(node).expect("box runtime style");
    let width = match style.layout.width {
        Some(LengthSpec::Px(px)) => px,
        other => panic!("expected interpolated px width, got {other:?}"),
    };
    assert!(
        (width - 60.0).abs() < 1.0,
        "mid-transition width expected ~60px, got {width}"
    );
}

#[test]
fn has_plus_transition_interactive_pass_builds_forest_once() {
    let mut bridge = MessageBridge::new();
    for i in 1..=20u64 {
        bridge.register(
            i,
            WidgetKind::Column,
            WidgetProps {
                element_tag: "div".into(),
                class_names: vec!["card".into()],
                ..WidgetProps::default()
            },
        );
        let badge = 100 + i;
        bridge.register(
            badge,
            WidgetKind::Box,
            WidgetProps {
                element_tag: "span".into(),
                class_names: vec!["badge".into()],
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(badge, i, None);
    }
    bridge.inject_stylesheet(".card:has(.badge) { color: red; } .card { transition: color 0.2s; }");
    let builds_after_inject = bridge.cascade.relative_forest_builds.get();
    let nodes_after_inject = bridge.cascade.relative_forest_nodes.get();
    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    bridge.reapply_interactive_cascade(&mut doc);
    let builds = bridge.cascade.relative_forest_builds.get() - builds_after_inject;
    let nodes = bridge.cascade.relative_forest_nodes.get() - nodes_after_inject;
    let n = bridge.widgets.len();
    assert_eq!(
        builds, 1,
        "interactive recascade must share one relative forest, got {builds}"
    );
    assert_eq!(
        nodes, n,
        "forest must clone each widget once (O(N)), not per-node root trees"
    );
    assert!(n >= 40);
}

#[test]
fn inject_only_has_recascades_matching_parent() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["badge".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".card:has(.badge) { width: 40px; }");
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(40.0)),
        "inject of only :has must recascade the parent"
    );
}

#[test]
fn unregister_badge_drops_has_style_on_parent() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["card".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["badge".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.inject_stylesheet(".card:has(.badge) { width: 40px; }");
    assert_eq!(
        bridge.get(1).expect("card").props.layout.width,
        Some(LengthSpec::Px(40.0))
    );
    bridge.unregister(2);
    assert!(
        bridge.get(1).expect("card").props.layout.width.is_none(),
        "parent must lose :has style after the badge is unregistered"
    );
}

#[test]
fn insert_at_index_zero_updates_nth_child_siblings() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["row".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        2,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["item".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        3,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["item".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(2, 1, None);
    bridge.insert_child(3, 1, None);
    bridge.inject_stylesheet(".row > :nth-child(2) { width: 10px; }");
    assert!(bridge.get(2).expect("first").props.layout.width.is_none());
    assert_eq!(
        bridge.get(3).expect("second").props.layout.width,
        Some(LengthSpec::Px(10.0))
    );
    bridge.register(
        4,
        WidgetKind::Box,
        WidgetProps {
            element_tag: "span".into(),
            class_names: vec!["item".into()],
            ..WidgetProps::default()
        },
    );
    bridge.insert_child(4, 1, Some(2));
    assert!(
        bridge
            .get(3)
            .expect("old second")
            .props
            .layout
            .width
            .is_none(),
        "old :nth-child(2) must drop after a prepend"
    );
    assert_eq!(
        bridge.get(2).expect("old first").props.layout.width,
        Some(LengthSpec::Px(10.0)),
        "old first becomes :nth-child(2) after insert at index 0"
    );
}

#[test]
fn unsupported_nth_child_of_stays_closed_after_sibling_class_changes() {
    let mut bridge = MessageBridge::new();
    bridge.register(
        1,
        WidgetKind::Column,
        WidgetProps {
            element_tag: "div".into(),
            class_names: vec!["row".into()],
            ..WidgetProps::default()
        },
    );
    for (id, noted) in [(2, true), (3, false), (4, true)] {
        bridge.register(
            id,
            WidgetKind::Box,
            WidgetProps {
                element_tag: "span".into(),
                class_names: if noted {
                    vec!["noted".into()]
                } else {
                    Vec::new()
                },
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(id, 1, None);
    }
    bridge.inject_stylesheet(".row > :nth-child(even of .noted) { width: 10px; }");
    assert!(
        bridge
            .get(2)
            .expect("first noted")
            .props
            .layout
            .width
            .is_none()
    );
    assert_eq!(
        bridge.get(4).expect("second noted").props.layout.width,
        None
    );
    bridge.patch_prop(3, "class", &nana_js_engine::HostValue::string("noted"));
    assert!(
        bridge
            .get(4)
            .expect("was second noted")
            .props
            .layout
            .width
            .is_none(),
        "unsupported `of` selector must remain unapplied"
    );
    assert_eq!(
        bridge.get(3).expect("new second noted").props.layout.width,
        None
    );
}

#[test]
fn layout_width_transition_lerps_px_and_syncs_only_that_widget() {
    use nana_ui_runtime::StableNodeId;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let panel = doc.create_element("div");
    let sibling = doc.create_element("div");
    doc.insert(panel, root, None);
    doc.insert(sibling, root, None);
    bridge.register(
        panel.0,
        WidgetKind::Box,
        WidgetProps {
            class_names: vec!["panel".into()],
            ..WidgetProps::default()
        },
    );
    bridge.register(
        sibling.0,
        WidgetKind::Box,
        WidgetProps {
            class_names: vec!["static".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".panel { width: 40px; height: 20px; transition: width 200ms linear; } \
             .panel:hover { width: 80px; } \
             .static { width: 16px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    let sibling_width = bridge.get(sibling.0).expect("sibling").props.layout.width;
    doc.set_runtime_clock_for_test(std::time::Duration::ZERO);
    doc.set_pointer_hover(0, Some(panel));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(std::time::Duration::from_millis(100));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let mid = bridge
        .get(panel.0)
        .expect("panel")
        .props
        .layout
        .width
        .expect("interpolated width");
    match mid {
        LengthSpec::Px(v) => assert!(
            (v - 60.0).abs() < 0.6,
            "expected mid-transition width ~60px, got {v}"
        ),
        other => panic!("layout transition must write px, got {other:?}"),
    }
    assert_eq!(
        bridge.get(sibling.0).expect("sibling").props.layout.width,
        sibling_width,
        "incremental layout must not rewrite the static sibling"
    );
    let style = doc
        .world()
        .node_style(StableNodeId::new(panel.0).unwrap())
        .expect("panel runtime style");
    match style.layout.width {
        Some(LengthSpec::Px(v)) => assert!((v - 60.0).abs() < 0.6),
        other => panic!("runtime layout width should be interpolated px, got {other:?}"),
    }
    doc.set_runtime_clock_for_test(std::time::Duration::from_millis(220));
    assert!(bridge.tick_css_animations(&mut doc));
    let completes = bridge.take_motion_completes();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].event_type, "transitionend");
    assert_eq!(completes[0].widget_id, panel.0);
    assert_eq!(completes[0].property_name, "width");
    assert!((completes[0].elapsed_time - 0.2).abs() < 0.001);
    assert_eq!(
        bridge.get(panel.0).expect("panel").props.layout.width,
        Some(LengthSpec::Px(80.0))
    );
}

#[test]
fn keyframes_finish_queues_animationend() {
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["spin".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        "@keyframes spin { from { opacity: 0; } to { opacity: 1; } } \
             .spin { animation: spin 200ms linear; width: 40px; height: 40px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let frame = doc.advance_css_animations(Duration::from_millis(220));
    assert!(bridge.apply_css_animation_samples(&mut doc, frame));
    let completes = bridge.take_motion_completes();
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].event_type, "animationend");
    assert_eq!(completes[0].animation_name, "spin");
    assert_eq!(completes[0].widget_id, host.0);
}

#[test]
fn recascade_does_not_restart_same_name_keyframes() {
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["spin".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        "@keyframes spin { from { opacity: 0; } to { opacity: 1; } } \
             .spin { animation: spin 1s linear infinite; width: 40px; height: 40px; } \
             .spin:hover { color: red; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    doc.set_runtime_clock_for_test(Duration::from_millis(250));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let mid = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("style")
        .layout
        .opacity
        .expect("mid opacity");
    assert!(
        (mid - 0.25).abs() < 0.08,
        "expected ~0.25 at 250ms, got {mid}"
    );

    doc.set_pointer_hover(0, Some(host));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(Duration::from_millis(500));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let later = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("style")
        .layout
        .opacity
        .expect("later opacity");
    assert!(
        (later - 0.5).abs() < 0.08,
        "same-name recascade must keep the clock (expect ~0.5 at 500ms), got {later}"
    );
}

#[test]
fn recascade_restarts_same_name_keyframes_after_finish() {
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["spin".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        "@keyframes spin { from { opacity: 0; } to { opacity: 1; } } \
             .spin { animation: spin 200ms linear; width: 40px; height: 40px; } \
             .spin:hover { color: red; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    doc.set_runtime_clock_for_test(Duration::from_millis(220));
    assert!(bridge.tick_css_animations(&mut doc));
    let first = bridge.take_motion_completes();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].event_type, "animationend");

    doc.set_pointer_hover(0, Some(host));
    bridge.reapply_interactive_cascade(&mut doc);
    doc.set_runtime_clock_for_test(Duration::from_millis(320));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let mid = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("style")
        .layout
        .opacity
        .expect("restarted opacity");
    assert!(
        (mid - 0.5).abs() < 0.08,
        "finished same-name recascade must start again (~0.5 at 100ms), got {mid}"
    );

    doc.set_runtime_clock_for_test(Duration::from_millis(440));
    assert!(bridge.tick_css_animations(&mut doc));
    let second = bridge.take_motion_completes();
    assert_eq!(
        second.len(),
        1,
        "restarted timeline must complete once more"
    );
    assert_eq!(second[0].event_type, "animationend");
    assert_eq!(second[0].animation_name, "spin");
    assert_eq!(second[0].widget_id, host.0);
}

#[test]
fn flip_paint_transform_enters_runtime_and_clears_after_move_transition() {
    use nana_ui_core::PaintTransform;
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let item = doc.create_element("li");
    doc.insert(item, root, None);
    bridge.register(
        item.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["item".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        ".item { width: 40px; height: 16px; } \
             .item-move { transition: transform 200ms linear; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    let width_before = doc
        .world()
        .node_style(StableNodeId::new(item.0).unwrap())
        .expect("style")
        .layout
        .width;

    doc.set_runtime_clock_for_test(Duration::ZERO);
    bridge.set_paint_transform(item.0, "translate(30px, 0px)", &mut doc);
    doc.flush_host_frame();
    let overlay = doc
        .world()
        .node_style(StableNodeId::new(item.0).unwrap())
        .expect("style");
    assert_eq!(
        overlay.layout.transform,
        Some(PaintTransform {
            e: 30.0,
            ..PaintTransform::default()
        })
    );
    assert_eq!(
        overlay.layout.width, width_before,
        "paint transform must not recascade layout"
    );

    bridge.set_paint_transform(item.0, "", &mut doc);
    bridge.patch_prop(item.0, "class", &HostValue::string("item item-move"));
    bridge.maybe_release_flip_paint_transform(item.0, &mut doc);
    doc.set_runtime_clock_for_test(Duration::from_millis(100));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let mid = doc
        .world()
        .node_style(StableNodeId::new(item.0).unwrap())
        .expect("style")
        .layout
        .transform
        .expect("mid flip transform");
    assert!(
        (mid.e - 15.0).abs() < 1.0,
        "expected ~15px mid FLIP, got {}",
        mid.e
    );

    doc.set_runtime_clock_for_test(Duration::from_millis(220));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let done = doc
        .world()
        .node_style(StableNodeId::new(item.0).unwrap())
        .expect("style");
    assert_eq!(
        done.layout.transform, None,
        "FLIP must not leave a leftover transform"
    );
    assert_eq!(done.layout.width, width_before);
}

#[test]
fn paint_transform_does_not_restart_same_name_keyframes() {
    use nana_ui_runtime::StableNodeId;
    use std::time::Duration;

    let mut doc = crate::tree::NanaTreeDocument::new(800, 600, 1.0);
    let mut bridge = MessageBridge::new();
    let root = doc.mount_root();
    let host = doc.create_element("div");
    doc.insert(host, root, None);
    bridge.register(
        host.0,
        WidgetKind::Column,
        WidgetProps {
            class_names: vec!["spin".into()],
            ..WidgetProps::default()
        },
    );
    bridge.inject_stylesheet(
        "@keyframes spin { from { opacity: 0; } to { opacity: 1; } } \
             .spin { animation: spin 1s linear infinite; width: 40px; height: 40px; }",
    );
    bridge.resolve_document_layout(&mut doc);
    doc.flush_host_frame();
    doc.set_runtime_clock_for_test(Duration::from_millis(250));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let mid = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("style")
        .layout
        .opacity
        .expect("mid opacity");
    assert!(
        (mid - 0.25).abs() < 0.08,
        "expected ~0.25 at 250ms, got {mid}"
    );

    bridge.set_paint_transform(host.0, "translate(8px, 0px)", &mut doc);
    doc.flush_host_frame();
    doc.set_runtime_clock_for_test(Duration::from_millis(500));
    assert!(bridge.tick_css_animations(&mut doc));
    doc.flush_host_frame();
    let later = doc
        .world()
        .node_style(StableNodeId::new(host.0).unwrap())
        .expect("style")
        .layout
        .opacity
        .expect("later opacity");
    assert!(
        (later - 0.5).abs() < 0.08,
        "paint transform must keep the keyframe clock (expect ~0.5 at 500ms), got {later}"
    );
    assert_eq!(
        later_transform_e(&doc, host.0),
        8.0,
        "FLIP overlay must still be on LayoutStyle.transform"
    );
}

fn later_transform_e(doc: &crate::tree::NanaTreeDocument, id: u64) -> f32 {
    use nana_ui_runtime::StableNodeId;
    doc.world()
        .node_style(StableNodeId::new(id).unwrap())
        .expect("style")
        .layout
        .transform
        .expect("paint transform")
        .e
}
