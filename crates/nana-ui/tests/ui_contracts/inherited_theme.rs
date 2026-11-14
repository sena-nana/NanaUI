//! Installed theme defaults inherit through the retained tree and chip chrome.
use nana_ui::{HeadlessInput, NanaTextShaper, PointerPhase, runtime::*};
use nana_ui_core::{LineHeightSpec, SemanticPalette, ThemeDefinition};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
    for _ in 0..6 {
        let work = cx.take_system_work();
        cx.resolve_styles(&work.style).unwrap();
        cx.shape_text(&work.text, shaper).unwrap();
        cx.layout_document(doc, LayoutViewport::new(800.0, 500.0))
            .unwrap();
        if cx.shape_text_for_layout(doc, shaper).unwrap() {
            cx.layout_document(doc, LayoutViewport::new(800.0, 500.0))
                .unwrap();
        }
    }
    cx.rebuild_hit_test(doc);
}

fn natural_width(cx: &AppContext, text: StableNodeId, shaper: &mut NanaTextShaper) -> f32 {
    shaper
        .shape(
            StableNodeId::new(u64::MAX).unwrap(),
            &TextContent {
                value: cx.world().text(text).unwrap().into(),
            },
            cx.world().computed_style(text).unwrap(),
            TextShapeConstraints::default(),
        )
        .width
}

#[test]
fn default_text_inherits_body_regular_and_line_without_overriding_its_parent() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(911).unwrap();
    let root = cx
        .create_component(doc, Stack::column(0.0).align(AlignSpec::Start))
        .unwrap();
    let plain = cx
        .create_detached_component(doc, Text::new("Theme default typography"))
        .unwrap();
    cx.append_child(root, plain).unwrap();
    let scope = cx
        .create_detached_component(
            doc,
            Stack::column(0.0)
                .align(AlignSpec::Start)
                .with_layout(|layout| {
                    layout.font_size = Some(17.0);
                    layout.font_weight = Some(500);
                    layout.line_height = Some(LineHeightSpec::Relative(1.5));
                }),
        )
        .unwrap();
    let overridden = cx
        .create_detached_component(doc, Text::new("Inherited local typography"))
        .unwrap();
    cx.append_child(root, scope).unwrap();
    cx.append_child(scope, overridden).unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let original = cx.world().layout_box(plain.stable_id()).unwrap();
    let local = cx.world().layout_box(overridden.stable_id()).unwrap();

    let mut theme = ThemeDefinition::NANA_DARK;
    theme.typography.body = 24.0;
    theme.typography.regular = 600;
    theme.typography.line = 36.0;
    cx.set_theme_definition(&theme).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let style = cx.world().computed_style(plain.stable_id()).unwrap();
    assert_eq!(style.font_size, 24.0);
    assert_eq!(style.font_weight, Some(600));
    let changed = cx.world().layout_box(plain.stable_id()).unwrap();
    assert!(
        changed.width > original.width + 30.0,
        "{original:?} -> {changed:?}"
    );
    assert!((changed.width - natural_width(&cx, plain.stable_id(), &mut shaper)).abs() < 1.0);
    assert!((changed.height - 36.0).abs() < 1.0, "{changed:?}");
    let style = cx.world().computed_style(overridden.stable_id()).unwrap();
    assert_eq!(style.font_size, 17.0);
    assert_eq!(style.font_weight, Some(500));
    assert_eq!(style.line_height, Some(LineHeightSpec::Relative(1.5)));
    let local_after = cx.world().layout_box(overridden.stable_id()).unwrap();
    assert_eq!(
        (local_after.width, local_after.height),
        (local.width, local.height)
    );

    cx.set_theme_definition(&theme.with_palette(SemanticPalette::light()))
        .unwrap();
    let work = cx.take_system_work();
    assert!(
        work.layout_frontier_seeds.is_empty(),
        "palette must not lay out text again"
    );
}

#[test]
fn chip_keeps_its_focus_actions_and_natural_label_when_theme_metrics_change() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(912).unwrap();
    let root = cx
        .create_component(doc, Stack::column(0.0).align(AlignSpec::Start))
        .unwrap();
    let chip = cx
        .create_detached_component(doc, Chip::new("可选主题标签").dismissible(true))
        .unwrap();
    cx.append_child(root, chip).unwrap();
    cx.assemble_chip(chip).unwrap();
    let activations = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&activations);
    cx.on(chip, move |_, _: &Activate, _| {
        recorded.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let dismissals = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&dismissals);
    cx.on(chip, move |_, _: &ChipDismissed, _| {
        recorded.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let original = cx.world().layout_box(chip.stable_id()).unwrap();
    let original_children = cx.world().node(chip.stable_id()).unwrap().children;
    assert_eq!(original_children.len(), 1, "dismiss close is assembled");
    let label = match cx.world().component_geometry(chip.stable_id()).unwrap() {
        ComponentGeometry::Button { label, .. } => label,
        _ => panic!("chip must paint its button geometry"),
    };
    let mut input = HeadlessInput::bind(&mut cx, doc);
    let (x, y) = (
        label.bounds.x + 5.0,
        label.bounds.y + label.bounds.height / 2.0,
    );
    assert_eq!(
        cx.pointer_target(doc, x, y),
        Some(chip.stable_id()),
        "chip={original:?} label={label:?} close={:?}",
        cx.world().layout_box(original_children[0])
    );
    input.pointer(&mut cx, PointerPhase::Down, x, y).unwrap();
    input.pointer(&mut cx, PointerPhase::Up, x, y).unwrap();
    assert_eq!(activations.load(Ordering::SeqCst), 1);
    assert_eq!(cx.world().focused(doc), Some(chip.stable_id()));

    let mut theme = ThemeDefinition::NANA_DARK;
    theme.typography.body = 24.0;
    theme.typography.line = 36.0;
    theme.typography.meta = 24.0;
    theme.typography.medium = 700;
    theme.tokens.metrics.compact_control_height = 46.0;
    theme.tokens.metrics.compact_control_padding_x = 14.0;
    theme.tokens.spacing.xs = 9.0;
    cx.set_theme_definition(&theme).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let changed = cx.world().layout_box(chip.stable_id()).unwrap();
    assert!(changed.width > original.width + 30.0);
    assert!((changed.height - 46.0).abs() < 0.5, "{changed:?}");
    assert_eq!(cx.world().focused(doc), Some(chip.stable_id()));
    assert_eq!(
        cx.world().node(chip.stable_id()).unwrap().children,
        original_children
    );
    let label = match cx.world().component_geometry(chip.stable_id()).unwrap() {
        ComponentGeometry::Button { label, .. } => label,
        _ => unreachable!(),
    };
    assert_eq!(label.font_size, 24.0);
    assert_eq!(label.font_weight, Some(700));
    assert!(label.bounds.width + 0.5 >= natural_width(&cx, chip.stable_id(), &mut shaper));
    let close = cx.world().layout_box(original_children[0]).unwrap();
    assert_eq!(close.width, theme.tokens.metrics.compact_control_height);
    assert!(close.y >= changed.y);
    assert!(close.y + close.height <= changed.y + changed.height);
    assert!((close.height - changed.height + 2.0 * theme.tokens.border.hairline).abs() < 0.5);
    for y in [close.y + 1.0, close.y + close.height - 1.0] {
        assert_eq!(
            cx.pointer_target(doc, close.x + close.width / 2.0, y),
            Some(original_children[0]),
            "the full close target must stay inside the chip"
        );
    }
    let inset = theme.tokens.metrics.compact_control_padding_x + theme.tokens.border.hairline;
    assert!((label.bounds.x - changed.x - inset).abs() < 0.5);
    assert!(
        (changed.x + changed.width - close.x - close.width - inset).abs() < 0.5,
        "chip={changed:?} label={label:?} close={close:?} inset={inset}"
    );
    assert!((close.x - label.bounds.x - label.bounds.width - theme.tokens.spacing.xs).abs() < 0.5);
    input
        .pointer(
            &mut cx,
            PointerPhase::Down,
            label.bounds.x + 5.0,
            changed.y + changed.height / 2.0,
        )
        .unwrap();
    input
        .pointer(
            &mut cx,
            PointerPhase::Up,
            label.bounds.x + 5.0,
            changed.y + changed.height / 2.0,
        )
        .unwrap();
    assert_eq!(activations.load(Ordering::SeqCst), 2);
    input
        .pointer(
            &mut cx,
            PointerPhase::Down,
            close.x + close.width / 2.0,
            close.y + close.height / 2.0,
        )
        .unwrap();
    input
        .pointer(
            &mut cx,
            PointerPhase::Up,
            close.x + close.width / 2.0,
            close.y + close.height / 2.0,
        )
        .unwrap();
    assert_eq!(dismissals.load(Ordering::SeqCst), 1);
    assert_eq!(activations.load(Ordering::SeqCst), 2);
}
