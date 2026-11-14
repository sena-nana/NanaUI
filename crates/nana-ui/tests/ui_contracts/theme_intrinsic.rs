//! Real shaping must export natural width while retaining constrained paint.
use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_core::{LengthSpec, TextWeight, ThemeDefinition, TypeRole, WordBreakSpec};
use nana_ui_scene::{ScenePrimitiveKind, UiScene};
use std::sync::Arc;

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
    let viewport = LayoutViewport::new(600.0, 400.0);
    for _ in 0..8 {
        let work = cx.take_system_work();
        let mut seeds = work.layout_frontier_seeds.clone();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        seeds.extend(cx.take_layout_frontier_seeds(doc));
        if work.is_empty() && seeds.is_empty() {
            return;
        }
        if !seeds.is_empty() {
            cx.layout_document_for_viewport(doc, viewport, &seeds)
                .unwrap();
            let scope = cx.take_last_layout_scope();
            cx.shape_text_for_layout_scoped(&scope, shaper).unwrap();
        }
    }
    panic!("the real shaping/layout frame did not settle");
}

fn width(cx: &AppContext, shaper: &mut NanaTextShaper, text: StableNodeId) -> f32 {
    let style = cx.world().computed_style(text).unwrap();
    shaper
        .shape(
            StableNodeId::new(u64::MAX).unwrap(),
            &TextContent {
                value: cx.world().text(text).unwrap().into(),
            },
            style,
            TextShapeConstraints::default(),
        )
        .width
}

fn ellipsized(cx: &AppContext, doc: DocumentId, text: StableNodeId) -> bool {
    cx.world()
        .extract_document(doc)
        .into_iter()
        .find(|node| node.id == text)
        .unwrap()
        .text_layout
        .unwrap()
        .layout
        .overflow
        .contains(nana_text::OverflowFlags::ELLIPSIZED)
}

fn clamped_text(value: &str, lines: u16) -> Text {
    let mut text = Text::new(value)
        .typography(TypeRole::Meta)
        .text_weight(TextWeight::Medium);
    let layout = Arc::make_mut(&mut text.style.layout);
    layout.line_clamp = Some(lines);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.word_break = Some(WordBreakSpec::BreakWord);
    text
}

#[test]
fn clamped_thumbnail_badge_tracks_real_natural_width_when_theme_grows_and_shrinks() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(901).unwrap();
    let mut cover = Thumbnail::empty();
    let layout = Arc::make_mut(&mut cover.style.layout);
    layout.width = Some(LengthSpec::Px(300.0));
    layout.height = Some(LengthSpec::Px(180.0));
    let cover = cx.create_component(doc, cover).unwrap();
    let badge = cx
        .create_detached_component(doc, Thumbnail::badge())
        .unwrap();
    let text = cx
        .create_detached_component(doc, clamped_text("24:00", 1))
        .unwrap();
    cx.append_child(cover, badge).unwrap();
    cx.append_child(badge, text).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let original = cx.world().layout_box(badge.stable_id()).unwrap().width;
    for font_size in [30.0, 8.0] {
        let mut theme = ThemeDefinition::NANA_DARK;
        theme.typography.meta = font_size;
        theme.typography.line = font_size + 8.0;
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        let natural = width(&cx, &mut shaper, text.stable_id());
        let text_box = cx.world().layout_box(text.stable_id()).unwrap();
        let badge_box = cx.world().layout_box(badge.stable_id()).unwrap();
        assert!(
            text_box.width + 0.5 >= natural,
            "font {font_size}: {text_box:?} < natural {natural}"
        );
        assert!(
            !ellipsized(&cx, doc, text.stable_id()),
            "auto badge must show the complete duration at font {font_size}"
        );
        assert!(
            (badge_box.width - natural - 12.0).abs() < 1.0,
            "badge tracks its text plus theme padding: {badge_box:?}, natural {natural}"
        );
        if font_size > 12.0 {
            assert!(badge_box.width > original);
        } else {
            assert!(badge_box.width < original);
        }
    }
}

#[test]
fn a_clamped_fit_content_bubble_grows_to_natural_width_but_respects_a_parent_limit() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(902).unwrap();
    let root = cx
        .create_component(
            doc,
            Stack::column(0.0)
                .align(AlignSpec::Start)
                .width(LengthSpec::Px(500.0)),
        )
        .unwrap();
    let bubble = cx
        .create_detached_component(
            doc,
            Stack::column(0.0)
                .width(LengthSpec::Shrink)
                .with_layout(|layout| {
                    layout.max_width = Some(LengthSpec::Px(160.0));
                    layout.align_items = AlignSpec::Start;
                }),
        )
        .unwrap();
    let message =
        "A message that must stay within its bubble and clamp after two lines. ".repeat(3);
    let text = cx
        .create_detached_component(doc, clamped_text(&message, 2))
        .unwrap();
    cx.append_child(root, bubble).unwrap();
    cx.append_child(bubble, text).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    for font_size in [30.0, 8.0] {
        let mut theme = ThemeDefinition::NANA_DARK;
        theme.typography.meta = font_size;
        theme.typography.line = font_size + 8.0;
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        let bubble_box = cx.world().layout_box(bubble.stable_id()).unwrap();
        let text_box = cx.world().layout_box(text.stable_id()).unwrap();
        assert!(
            bubble_box.width <= 160.5 && text_box.width <= 160.5,
            "parent constraint holds: {bubble_box:?} {text_box:?}"
        );
        let retained = cx
            .world()
            .extract_document(doc)
            .into_iter()
            .find(|node| node.id == text.stable_id())
            .unwrap()
            .text_layout
            .unwrap();
        assert!(retained.layout.lines.len() <= 2);
        assert!(
            retained
                .layout
                .overflow
                .contains(nana_text::OverflowFlags::ELLIPSIZED),
            "a constrained long message still ellipsizes at font {font_size}"
        );
        assert!(
            (bubble_box.width - 160.0).abs() < 0.5,
            "bubble uses available room instead of its old truncated width: {bubble_box:?}"
        );
    }
}

#[test]
fn progress_default_track_follows_theme_but_explicit_thickness_keeps_its_geometry() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(903).unwrap();
    let root = cx
        .create_component(doc, Stack::column(0.0).width(LengthSpec::Px(300.0)))
        .unwrap();
    let default = cx
        .create_detached_component(doc, Progress::new(25.0, 100.0))
        .unwrap();
    let explicit = cx
        .create_detached_component(doc, Progress::new(25.0, 100.0).thickness(2.0))
        .unwrap();
    cx.append_child(root, default).unwrap();
    cx.append_child(root, explicit).unwrap();
    let transport = cx
        .create_detached_component(doc, MediaTransportBar::new().mini().live(true))
        .unwrap();
    cx.append_child(root, transport).unwrap();
    cx.assemble_media_transport_bar(transport).unwrap();
    let slots = cx.read(transport, |bar| bar.slots().clone()).unwrap();
    let mut shaper = NanaTextShaper::default();
    for girth in [6.0, 12.0] {
        let mut theme = ThemeDefinition::NANA_DARK;
        theme.tokens.spacing.sm = girth;
        theme.tokens.metrics.control_height = if girth > 6.0 { 40.0 } else { 32.0 };
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        let strip = cx.world().layout_box(slots.strip.unwrap()).unwrap();
        let meter = cx.world().layout_box(slots.live_progress.unwrap()).unwrap();
        assert_eq!(strip.height, 2.0, "mini keeps its business rail thickness");
        assert!(
            (meter.y + meter.height / 2.0 - strip.y - strip.height / 2.0).abs() < 0.5,
            "the themed live meter stays centered on its 2px clip"
        );
        let volume = cx.world().layout_box(slots.volume.unwrap()).unwrap();
        assert!(
            (volume.width - theme.tokens.metrics.control_height * 2.0).abs() < 0.5,
            "mini volume follows the control size token"
        );
        for (bar, expected) in [(default, girth), (explicit, 2.0)] {
            assert_eq!(
                cx.world().layout_box(bar.stable_id()).unwrap().height,
                expected
            );
            let ComponentGeometry::Progress {
                track,
                fill,
                corner_radius,
                ..
            } = cx.world().component_geometry(bar.stable_id()).unwrap()
            else {
                panic!("progress paint");
            };
            assert_eq!(track.height, expected);
            assert_eq!(fill.height, expected);
            assert_eq!(corner_radius, expected / 2.0);
        }
    }
    cx.update_component(transport, |bar, _| {
        bar.density = MediaTransportDensity::Regular;
    })
    .unwrap();
    cx.sync_media_transport_bar(transport).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let volume = cx.world().node_style(slots.volume.unwrap()).unwrap();
    assert!(volume.layout.theme.width.is_none() && volume.layout.theme.min_width.is_none());
    let meter = cx.world().node_style(slots.live_progress.unwrap()).unwrap();
    assert!(
        meter.layout.theme.offset_top.is_none(),
        "leaving mini clears its themed placement"
    );
}

#[test]
fn range_readouts_share_the_installed_typography_with_real_shaping() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(904).unwrap();
    let range = cx
        .create_component(
            doc,
            RangeField::new(100000.0, 0.0, 100000.0, 1.0)
                .label("Radius")
                .unit("pixels"),
        )
        .unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    for (body, meta, line) in [(18.0, 14.0, 24.0), (30.0, 24.0, 40.0), (10.0, 8.0, 16.0)] {
        let mut theme = ThemeDefinition::NANA_DARK;
        theme.typography.body = body;
        theme.typography.meta = meta;
        theme.typography.line = line;
        theme.typography.medium = 550;
        theme.typography.regular = 450;
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        let ComponentGeometry::Range {
            label,
            value,
            unit,
            track,
            ..
        } = cx.world().component_geometry(range.stable_id()).unwrap()
        else {
            panic!("range geometry");
        };
        assert_eq!(
            track.height, body,
            "ordinary thumb follows the control type role"
        );
        for (region, size, weight) in [
            (label.unwrap(), body, 550),
            (value, body, 550),
            (unit.unwrap(), meta, 450),
        ] {
            assert_eq!(region.font_size, size);
            assert_eq!(region.font_weight, Some(weight));
            let mut style = cx
                .world()
                .computed_style(range.stable_id())
                .unwrap()
                .clone();
            style.font_size = region.font_size;
            style.font_weight = region.font_weight;
            let metrics = shaper.shape(
                StableNodeId::new(u64::MAX).unwrap(),
                &TextContent {
                    value: region.content.clone(),
                },
                &style,
                TextShapeConstraints::default(),
            );
            assert!(
                region.bounds.width + 0.5 >= metrics.width,
                "readout must fit its real shaped text: {region:?} vs {metrics:?}"
            );
            assert!(
                region.bounds.height + 0.5 >= metrics.height,
                "readout must fit its real line box: {region:?} vs {metrics:?}"
            );
        }
    }
}

#[test]
fn list_metadata_uses_meta_regular_when_a_mounted_theme_changes() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(905).unwrap();
    let mut row = ListItem::new("Title").note("Note").detail("Detail");
    // Metadata is optional in a fit-content row; give both regions real room.
    Arc::make_mut(&mut row.style.layout).width = Some(LengthSpec::Px(400.0));
    let item = cx.create_component(doc, row).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    for (meta, regular) in [(18.0, 550), (10.0, 400)] {
        let mut theme = ThemeDefinition::NANA_DARK;
        theme.typography.meta = meta;
        theme.typography.regular = regular;
        cx.set_theme_definition(&theme).unwrap();
        settle(&mut cx, doc, &mut shaper);
        let ComponentGeometry::ListItem { note, detail, .. } =
            cx.world().component_geometry(item.stable_id()).unwrap()
        else {
            panic!("list geometry");
        };
        for region in [
            note.expect("note remains visible"),
            detail.expect("detail remains visible"),
        ] {
            assert_eq!(region.font_size, meta);
            assert_eq!(region.font_weight, Some(regular));
        }
        let mut scene = UiScene::new();
        scene.apply_delta(cx.world().extract_document(doc), []);
        let metadata: Vec<_> = scene
            .primitives()
            .filter(|primitive| {
                primitive.node == item.stable_id() && matches!(primitive.id.slot, 3 | 4)
            })
            .collect();
        assert_eq!(metadata.len(), 2);
        for primitive in metadata {
            let ScenePrimitiveKind::Text {
                line_height,
                size,
                weight,
                ..
            } = &primitive.kind
            else {
                panic!("metadata paints text");
            };
            assert_eq!(*size, meta);
            assert_eq!(*weight, Some(regular));
            assert!((line_height.unwrap().resolve_px(*size) - primitive.bounds.height).abs() < 0.5);
        }
        assert_eq!(cx.world().text(item.stable_id()), Some("Title"));
    }
}
