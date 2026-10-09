use std::time::Duration;

use nana_ui_core::SemanticPalette;

use super::*;
use crate::hit::{ANY_SERIES, PointerGeometry};
use crate::marks::{MarkPass, ShapeKind};
use crate::option::*;

fn theme(option: &ChartOption) -> ChartTheme {
    let palette = SemanticPalette::light();
    ChartTheme::new(&palette, option, |_| [0.5, 0.5, 0.5, 1.0])
}

fn lay(option: &ChartOption, state: &ChartViewState) -> ChartLayout {
    let theme = theme(option);
    layout(&LayoutInput {
        option,
        size: [480.0, 300.0],
        theme: &theme,
        measure: &ApproximateMeasure,
        state,
    })
}

fn weekdays() -> Axis {
    Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"])
}

fn line_option() -> ChartOption {
    ChartOption::new()
        .x_axis(weekdays())
        .y_axis(Axis::value())
        .series(LineSeries::new(
            "visits",
            [820.0, 932.0, 901.0, 934.0, 1290.0, 1330.0, 1320.0],
        ))
}

#[test]
fn a_line_chart_lays_out_a_plot_axes_and_one_line() {
    let layout = lay(&line_option(), &ChartViewState::default());
    let plot = layout.plot.expect("cartesian plot");
    assert!(plot[0] > EDGE && plot[2] < 480.0 - EDGE + 0.5, "{plot:?}");
    assert!(plot[1] >= EDGE && plot[3] < 300.0 - EDGE, "{plot:?}");
    let lines: Vec<_> = layout
        .marks
        .draws
        .iter()
        .zip(&layout.draw_keys)
        .filter(|(draw, key)| draw.pass == MarkPass::Line && key.part == DrawPart::Line)
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0.count, 7);
    // Category labels for every day fit at this width.
    for day in ["Mon", "Sun"] {
        assert!(layout.texts.iter().any(|t| &*t.text == day), "{day}");
    }
    // The value axis rounds 820..1330 from zero to a nice 1400 or 1500.
    assert!(layout.texts.iter().any(|t| &*t.text == "0"));
    assert!(
        layout
            .texts
            .iter()
            .any(|t| &*t.text == "1,500" || &*t.text == "1,400"),
        "{:?}",
        layout
            .texts
            .iter()
            .map(|t| t.text.clone())
            .collect::<Vec<_>>()
    );
    // Higher values sit higher.
    let points = &layout.marks.points[lines[0].0.first as usize..][..7];
    assert!(points[5].to[1] < points[0].to[1]);
}

#[test]
fn points_without_room_for_symbols_hide_them_until_hovered() {
    let values: Vec<f64> = (0..400).map(|i| (i as f64 * 0.1).sin()).collect();
    let option = ChartOption::new()
        .x_axis(Axis::category((0..400).map(|i| i.to_string())))
        .y_axis(Axis::value())
        .series(LineSeries::new("wave", values));
    let layout = lay(&option, &ChartViewState::default());
    let symbols: Vec<_> = layout
        .marks
        .shapes
        .iter()
        .filter(|s| s.kind() == ShapeKind::Symbol as u32)
        .collect();
    assert_eq!(symbols.len(), 400);
    assert!(symbols.iter().all(|s| s.to[2] == 0.0 && s.extra[2] > 0.0));
}

#[test]
fn bars_split_the_band_and_stack_on_their_neighbours() {
    let option = ChartOption::new()
        .x_axis(Axis::category(["a", "b", "c"]))
        .y_axis(Axis::value())
        .series(BarSeries::new("x", [3.0, 4.0, 5.0]).stack("s"))
        .series(BarSeries::new("y", [1.0, -2.0, 2.0]).stack("s"))
        .series(BarSeries::new("z", [2.0, 2.0, 2.0]));
    let layout = lay(&option, &ChartViewState::default());
    let rects: Vec<_> = layout
        .marks
        .shapes
        .iter()
        .filter(|s| s.kind() == ShapeKind::Rect as u32 && s.meta[2] != u32::MAX)
        .collect();
    assert_eq!(rects.len(), 9);
    let of = |series: u32, index: u32| {
        rects
            .iter()
            .find(|s| s.meta[2] == series && s.meta[3] == index)
            .unwrap()
            .to
    };
    // x and y share a slot; z sits beside them.
    assert_eq!(of(0, 0)[0], of(1, 0)[0]);
    assert!(of(2, 0)[0] > of(0, 0)[2]);
    // y at index 0 stacks on x's top, reaching half a pixel into it.
    assert!((of(1, 0)[3] - of(0, 0)[1] - 0.5).abs() < 1e-3);
    // A negative value hangs below the axis, not on x.
    assert!(of(1, 1)[1] > of(0, 1)[3] - 1e-3);
    // Every bar enters from its base.
    for rect in &rects {
        assert!(rect.from[1] == rect.from[3] || rect.from[0] == rect.from[2]);
    }
}

#[test]
fn a_pie_closes_the_circle_and_hides_legend_items() {
    let option = ChartOption::new()
        .legend(Legend::default())
        .series(PieSeries::new(
            "share",
            [
                PieItem::new("a", 1.0),
                PieItem::new("b", 2.0),
                PieItem::new("c", 3.0),
            ],
        ));
    let layout = lay(&option, &ChartViewState::default());
    let sectors: Vec<_> = layout
        .marks
        .shapes
        .iter()
        .filter(|s| s.kind() == ShapeKind::Sector as u32)
        .collect();
    assert_eq!(sectors.len(), 3);
    let total: f32 = sectors.iter().map(|s| s.to[1] - s.to[0]).sum();
    assert!((total - std::f32::consts::TAU).abs() < 1e-4);
    assert_eq!(layout.legend.len(), 3);

    let mut state = ChartViewState::default();
    state.hidden.insert("c".into());
    let hidden = lay(&option, &state);
    let sectors: Vec<_> = hidden
        .marks
        .shapes
        .iter()
        .filter(|s| s.kind() == ShapeKind::Sector as u32)
        .collect();
    assert_eq!(sectors.len(), 2);
    assert!(!hidden.legend[2].selected);
}

#[test]
fn axis_hover_reports_every_series_at_the_category() {
    let option = line_option().series(LineSeries::new("other", [1.0; 7]));
    let layout = lay(&option, &ChartViewState::default());
    let plot = layout.plot.unwrap();
    let x = plot[0] + (plot[2] - plot[0]) * 0.5;
    let hover = layout
        .hover_at(&option, [x, (plot[1] + plot[3]) * 0.5])
        .expect("hover inside the plot");
    assert_eq!(hover.series, ANY_SERIES);
    assert_eq!(hover.index, 3);
    let tooltip = hover.tooltip.unwrap();
    assert_eq!(&*tooltip.title, "Thu");
    assert_eq!(tooltip.rows.len(), 2);
    assert_eq!(&*tooltip.rows[0].value, "934");
    assert!(matches!(hover.pointer, Some(PointerGeometry::Line { .. })));
    assert!(layout.hover_at(&option, [1.0, 1.0]).is_none());
}

#[test]
fn item_hover_finds_the_slice_under_the_pointer() {
    let option = ChartOption::new().series(
        PieSeries::new("share", [PieItem::new("a", 1.0), PieItem::new("b", 1.0)])
            .label(PieLabelPosition::None),
    );
    let layout = lay(&option, &ChartViewState::default());
    // Start at 90° counter-clockwise, clockwise slices: `a` is the right half.
    let hover = layout.hover_at(&option, [240.0 + 40.0, 150.0]).unwrap();
    assert_eq!(hover.index, 0);
    let tooltip = hover.tooltip.unwrap();
    assert_eq!(&*tooltip.title, "share");
    assert_eq!(&*tooltip.rows[0].name, "a");
    let hover = layout.hover_at(&option, [240.0 - 40.0, 150.0]).unwrap();
    assert_eq!(hover.index, 1);
}

#[test]
fn entry_reveals_lines_in_place_and_grows_bars() {
    let option = line_option().series(BarSeries::new("bars", [1.0; 7]));
    let mut layout = lay(&option, &ChartViewState::default());
    crate::transition::begin(&mut layout, None, &option, Duration::from_secs(5));
    let transition = layout.marks.transition.unwrap();
    assert!(transition.reveal);
    assert_eq!(transition.start, Duration::from_secs(5));
    for (draw, key) in layout.marks.draws.iter().zip(&layout.draw_keys) {
        if draw.pass == MarkPass::Line && key.part == DrawPart::Line {
            for p in &layout.marks.points[draw.first as usize..][..draw.count as usize] {
                assert_eq!(p.from, p.to);
            }
        }
    }
    let bar = layout
        .marks
        .shapes
        .iter()
        .find(|s| s.kind() == ShapeKind::Rect as u32 && s.meta[2] == 1)
        .unwrap();
    assert_ne!(bar.from, bar.to);
}

#[test]
fn an_update_starts_from_what_was_shown() {
    let option = line_option();
    let mut first = lay(&option, &ChartViewState::default());
    crate::transition::settle(&mut first);
    let next_option = ChartOption::new()
        .x_axis(weekdays())
        .y_axis(Axis::value())
        .series(LineSeries::new(
            "visits",
            [100.0, 200.0, 300.0, 400.0, 500.0, 600.0, 700.0],
        ));
    let mut next = lay(&next_option, &ChartViewState::default());
    crate::transition::begin(
        &mut next,
        Some(&first),
        &next_option,
        Duration::from_secs(1),
    );
    let transition = next.marks.transition.unwrap();
    assert!(!transition.reveal);
    let (draw, _) = next
        .marks
        .draws
        .iter()
        .zip(&next.draw_keys)
        .find(|(d, k)| d.pass == MarkPass::Line && k.part == DrawPart::Line)
        .unwrap();
    let (old_draw, _) = first
        .marks
        .draws
        .iter()
        .zip(&first.draw_keys)
        .find(|(d, k)| d.pass == MarkPass::Line && k.part == DrawPart::Line)
        .unwrap();
    let new_points = &next.marks.points[draw.first as usize..][..7];
    let old_points = &first.marks.points[old_draw.first as usize..][..7];
    for (new, old) in new_points.iter().zip(old_points) {
        assert!((new.from[0] - old.to[0]).abs() < 1e-3);
        assert!((new.from[1] - old.to[1]).abs() < 1e-3);
    }
    // Halfway through, a third update starts from the midpoint.
    let mid = Duration::from_secs(1) + Duration::from_secs_f32(transition.duration * 0.5);
    let progress = transition.progress(mid);
    let mut third = lay(&option, &ChartViewState::default());
    crate::transition::begin(&mut third, Some(&next), &option, mid);
    let (draw, _) = third
        .marks
        .draws
        .iter()
        .zip(&third.draw_keys)
        .find(|(d, k)| d.pass == MarkPass::Line && k.part == DrawPart::Line)
        .unwrap();
    let p = third.marks.points[draw.first as usize];
    let shown = new_points[0].from[1] + (new_points[0].to[1] - new_points[0].from[1]) * progress;
    assert!((p.from[1] - shown).abs() < 1e-2, "{} vs {shown}", p.from[1]);
}

#[test]
fn radar_and_gauge_lay_out_around_their_centres() {
    let option = ChartOption::new()
        .radar(RadarCoord::new([
            RadarIndicator::new("a").max(10.0),
            RadarIndicator::new("b").max(10.0),
            RadarIndicator::new("c").max(10.0),
            RadarIndicator::new("d").max(10.0),
        ]))
        .series(RadarSeries::new(
            "r",
            [RadarItem::new("one", vec![10.0, 5.0, 10.0, 5.0])],
        ));
    let layout = lay(&option, &ChartViewState::default());
    let closed: Vec<_> = layout
        .marks
        .draws
        .iter()
        .filter(|d| d.series == 0 && d.flags & crate::marks::draw_flags::CLOSED != 0)
        .collect();
    assert!(!closed.is_empty());
    assert!(layout.texts.iter().any(|t| &*t.text == "a"));

    let gauge = ChartOption::new().series(GaugeSeries::new("speed", 50.0));
    let layout = lay(&gauge, &ChartViewState::default());
    let needle = layout
        .marks
        .shapes
        .iter()
        .find(|s| s.kind() == ShapeKind::Needle as u32 && s.meta[2] == 0)
        .unwrap();
    // 50% of 225° → -45°: straight up.
    assert!((needle.to[2] - (-std::f32::consts::FRAC_PI_2)).abs() < 1e-3);
    assert!(layout.texts.iter().any(|t| &*t.text == "50"));
}

#[test]
fn zooming_a_category_axis_keeps_the_window() {
    let option = ChartOption::new()
        .x_axis(Axis::category((0..100).map(|i| format!("c{i}"))))
        .y_axis(Axis::value())
        .data_zoom(DataZoom::slider().range(50.0, 60.0))
        .series(LineSeries::new(
            "v",
            (0..100).map(f64::from).collect::<Vec<_>>(),
        ));
    let layout = lay(&option, &ChartViewState::default());
    let slider = layout.slider.expect("slider");
    assert!(slider.window[0] > slider.track[0]);
    assert!(layout.texts.iter().any(|t| &*t.text == "c50"));
    assert!(!layout.texts.iter().any(|t| &*t.text == "c10"));
}
