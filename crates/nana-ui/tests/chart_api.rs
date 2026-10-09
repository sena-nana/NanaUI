//! The chart surface an application builds from: the examples in
//! `docs/components/chart.md`, compiled.
#![cfg(feature = "charts")]

use nana_ui::runtime::chart::{
    Animation, AreaStyle, Axis, BarSeries, ChartOption, DataZoom, GaugeSeries, Legend, LineSeries,
    PieItem, PieSeries, RadarCoord, RadarIndicator, RadarItem, RadarSeries, ScatterSeries,
};
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AppContext, Chart, ChartEvent, DocumentId};

fn weekly() -> ChartOption {
    ChartOption::new()
        .legend(Legend::default())
        .x_axis(Axis::category(["周一", "周二", "周三", "周四", "周五"]))
        .y_axis(Axis::value())
        .series(BarSeries::new("访问", [120.0, 200.0, 150.0, 80.0, 70.0]))
        .series(LineSeries::new("转化", [80.0, 132.0, 101.0, 134.0, 90.0]).smooth(true))
}

#[test]
fn the_documented_charts_build_and_mount() {
    let _ = widget(Chart::new(weekly()).label("本周访问")).on(|event: &ChartEvent| {
        let _ = event;
    });
    let options = [
        weekly()
            .data_zoom(DataZoom::inside())
            .data_zoom(DataZoom::slider()),
        ChartOption::new().series(
            PieSeries::new("占比", [PieItem::new("甲", 3.0), PieItem::new("乙", 1.0)])
                .ring(0.5, 0.8)
                .corner_radius(4.0),
        ),
        ChartOption::new()
            .x_axis(Axis::value())
            .y_axis(Axis::value())
            .series(ScatterSeries::new("点", vec![[1.0, 2.0], [3.0, 4.0]])),
        ChartOption::new()
            .radar(RadarCoord::new([
                RadarIndicator::new("a").max(1.0),
                RadarIndicator::new("b").max(1.0),
                RadarIndicator::new("c").max(1.0),
            ]))
            .series(
                RadarSeries::new("雷达", [RadarItem::new("x", vec![0.5, 0.8, 0.3])])
                    .area(AreaStyle::default()),
            ),
        ChartOption::new()
            .animation(Animation::disabled())
            .series(GaugeSeries::new("负载", 64.0)),
    ];
    let mut cx = AppContext::new();
    let document = DocumentId::new(1).unwrap();
    for option in options {
        let chart = cx.create_component(document, Chart::new(option)).unwrap();
        assert!(cx.world().contains(chart.stable_id()));
    }
}
