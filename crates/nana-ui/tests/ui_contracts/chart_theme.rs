//! A mounted chart measures and paints its axes with the installed theme.
use nana_ui::{NanaTextShaper, runtime::*};
use nana_ui_charts::{Animation, Axis, ChartLayout, ChartOption, LineSeries, Tooltip};
use nana_ui_core::ThemeDefinition;
use std::sync::Arc;

fn settle(cx: &mut AppContext, doc: DocumentId, shaper: &mut NanaTextShaper) {
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
            cx.layout_document_for_viewport(doc, LayoutViewport::new(600.0, 300.0), &seeds)
                .unwrap();
            let scope = cx.take_last_layout_scope();
            cx.shape_text_for_layout_scoped(&scope, shaper).unwrap();
        }
    }
    panic!("chart theme frame did not settle");
}

fn layout(cx: &AppContext, chart: Entity<Chart>) -> Arc<ChartLayout> {
    let nodes = cx.world().extract_nodes(&[chart.stable_id()]);
    let Some(ComponentGeometry::Chart { layout, .. }) = nodes[0].component_geometry.as_deref()
    else {
        panic!("chart geometry");
    };
    Arc::clone(layout)
}

fn check_labels(
    cx: &AppContext,
    chart: Entity<Chart>,
    layout: &ChartLayout,
    shaper: &mut NanaTextShaper,
) {
    let theme = cx.world().theme();
    let typography = theme.typography();
    assert!(!layout.texts.is_empty());
    for text in &layout.texts {
        assert_eq!(text.size, typography.meta, "{}", text.text);
        assert_eq!(text.weight, Some(typography.regular), "{}", text.text);
        assert_eq!(text.color, theme.palette().muted.as_rgba_array());
        let mut style = cx
            .world()
            .computed_style(chart.stable_id())
            .unwrap()
            .clone();
        style.font_size = text.size;
        style.font_weight = text.weight;
        let metrics = shaper.shape(
            StableNodeId::new(u64::MAX).unwrap(),
            &TextContent {
                value: text.text.clone().into(),
            },
            &style,
            TextShapeConstraints::default(),
        );
        assert!(
            text.rect[2] - text.rect[0] >= metrics.width,
            "{}: its measured region must fit the real themed run",
            text.text
        );
        assert!(text.rect[0] >= -0.5 && text.rect[1] >= -0.5);
        assert!(text.rect[2] <= layout.size[0] + 0.5 && text.rect[3] <= layout.size[1] + 0.5);
    }
}

#[test]
fn mounted_axes_remeasure_for_typography_and_recolor_without_replacing_the_chart() {
    let mut cx = AppContext::new();
    let doc = DocumentId::new(910).unwrap();
    let option = ChartOption::new()
        .animation(Animation::disabled())
        .tooltip(Tooltip::hidden())
        .x_axis(Axis::category(["2026-10-01", "2026-10-02", "2026-10-03"]))
        .y_axis(Axis::value().name("播放"))
        .series(LineSeries::new("播放", [1200.0, 2000.0, 1600.0]));
    let chart = cx
        .create_component(
            doc,
            Chart::new(option).style(
                Stack::column(0.0)
                    .width(LengthSpec::Px(600.0))
                    .height(LengthSpec::Px(240.0))
                    .node_style(),
            ),
        )
        .unwrap();
    let mut shaper = NanaTextShaper::default();
    settle(&mut cx, doc, &mut shaper);
    let original = layout(&cx, chart);
    assert!(
        Arc::ptr_eq(&original, &layout(&cx, chart)),
        "unchanged chart layout is cached"
    );
    let original_plot = original.plot.unwrap();

    // Keep the palette and box identical so typography alone must invalidate the memo.
    let mut large = ThemeDefinition::NANA_LIGHT;
    large.typography.meta = 24.0;
    large.typography.regular = 600;
    cx.set_theme_definition(&large).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let grown = layout(&cx, chart);
    check_labels(&cx, chart, &grown, &mut shaper);
    assert!(!Arc::ptr_eq(&original, &grown));
    let grown_plot = grown.plot.unwrap();
    assert!(
        grown_plot[0] > original_plot[0],
        "larger y labels reserve more left margin"
    );
    assert!(
        grown_plot[3] < original_plot[3],
        "larger x labels reserve more bottom margin"
    );
    assert_eq!(
        cx.world().layout_box(chart.stable_id()).unwrap().width,
        600.0
    );

    // A weight-only edit also enters measurement and paint, even at the same font size.
    large.typography.regular = 450;
    cx.set_theme_definition(&large).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let weighted = layout(&cx, chart);
    check_labels(&cx, chart, &weighted, &mut shaper);
    assert!(!Arc::ptr_eq(&grown, &weighted));

    let mut dark = ThemeDefinition::NANA_DARK;
    dark.typography = large.typography;
    cx.set_theme_definition(&dark).unwrap();
    settle(&mut cx, doc, &mut shaper);
    let recolored = layout(&cx, chart);
    check_labels(&cx, chart, &recolored, &mut shaper);
    assert_ne!(weighted.texts[0].color, recolored.texts[0].color);
    assert_eq!(
        weighted.plot, recolored.plot,
        "color alone keeps the plot geometry"
    );

    cx.set_theme_definition(&ThemeDefinition::NANA_LIGHT)
        .unwrap();
    settle(&mut cx, doc, &mut shaper);
    let restored = layout(&cx, chart);
    check_labels(&cx, chart, &restored, &mut shaper);
    assert_eq!(restored.plot, original.plot);
    assert_eq!(
        cx.read(chart, |chart| chart.option.series.len()).unwrap(),
        1
    );
    assert!(cx.world().contains(chart.stable_id()));
}
