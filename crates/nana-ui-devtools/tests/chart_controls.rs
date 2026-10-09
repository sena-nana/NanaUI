#![cfg(feature = "agent")]

use nana_ui::runtime::chart::hit::{ANY_SERIES, ItemRegion};
use nana_ui::runtime::chart::{
    Animation, AreaStyle, Axis, BarSeries, ChartColor, ChartOption, GaugeSeries, Legend,
    LineSeries, PieItem, PieLabelPosition, PieSeries, RadarCoord, RadarIndicator, RadarItem,
    RadarSeries,
};
use nana_ui::runtime::{Chart, DocumentId, LengthSpec, NodeStyle, RuntimeDocument, Stack};
use nana_ui_core::ThemeAppearance;
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};
use std::sync::Arc;

fn sized(width: Option<f32>, height: f32) -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = width.map(LengthSpec::Px);
    layout.height = Some(LengthSpec::Px(height));
    style
}

fn weekly() -> ChartOption {
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri"];
    ChartOption::new()
        .animation(Animation::disabled())
        .legend(Legend::default())
        .x_axis(Axis::category(days))
        .y_axis(Axis::value())
        .series(
            BarSeries::new("Input", [12.0, 20.0, 15.0, 8.0, 17.0])
                .border_radius([3.0, 3.0, 0.0, 0.0]),
        )
        .series(
            LineSeries::new("Total", [18.0, 26.0, 22.0, 30.0, 24.0])
                .smooth(true)
                .area(AreaStyle::default()),
        )
}

#[test]
fn charts_render_and_hover_in_both_themes() {
    if !offscreen::pixels_available() {
        return;
    }
    let output = std::env::var_os("NANA_CHART_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
    }
    for (name, theme, width, scale) in [
        ("light-wide-1x", ThemeAppearance::Light, 560, 1.0),
        ("dark-wide-1x", ThemeAppearance::Dark, 560, 1.0),
        ("light-narrow-2x", ThemeAppearance::Light, 320, 2.0),
        ("dark-narrow-2x", ThemeAppearance::Dark, 320, 2.0),
    ] {
        let id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        cx.set_preset_theme(theme).unwrap();
        let root = cx
            .create_component(id, Stack::column(12.0).padding(16.0))
            .unwrap();
        let trend = cx
            .create_detached_component(id, Chart::new(weekly()).style(sized(None, 240.0)))
            .unwrap();
        let pie = cx
            .create_detached_component(
                id,
                Chart::new(
                    ChartOption::new().animation(Animation::disabled()).series(
                        PieSeries::new(
                            "Tokens",
                            [
                                PieItem::new("Input", 20.0),
                                PieItem::new("Output", 30.0),
                                PieItem::new("Cache", 50.0),
                            ],
                        )
                        .ring(0.5, 0.85)
                        .label(PieLabelPosition::None),
                    ),
                )
                .style(sized(Some(160.0), 160.0)),
            )
            .unwrap();
        let radar = cx
            .create_detached_component(
                id,
                Chart::new(
                    ChartOption::new()
                        .animation(Animation::disabled())
                        .radar(RadarCoord::new(
                            ["Speed", "Power", "Range", "Cost", "Comfort"]
                                .map(|name| RadarIndicator::new(name).max(10.0)),
                        ))
                        .series(
                            RadarSeries::new(
                                "Car",
                                [RadarItem::new("A", vec![8.0, 6.0, 7.0, 4.0, 9.0])],
                            )
                            .area(AreaStyle::default()),
                        ),
                )
                .style(sized(None, 200.0)),
            )
            .unwrap();
        let gauge = cx
            .create_detached_component(
                id,
                Chart::new(
                    ChartOption::new()
                        .animation(Animation::disabled())
                        .series(GaugeSeries::new("Load", 72.0)),
                )
                .style(sized(None, 200.0)),
            )
            .unwrap();
        cx.reconcile_children(
            root.stable_id(),
            &[
                trend.stable_id(),
                pie.stable_id(),
                radar.stable_id(),
                gauge.stable_id(),
            ],
        )
        .unwrap();
        let mut session = RuntimeAgentSession::new_scaled(document, width, 900, scale).unwrap();
        session.flush().unwrap();

        // Over the fourth category: every series there is emphasised.
        let bounds = session
            .document()
            .context()
            .world()
            .layout_box(trend.stable_id())
            .unwrap();
        let plot = {
            let nodes = session
                .document()
                .context()
                .world()
                .extract_nodes(&[trend.stable_id()]);
            match nodes[0].component_geometry.as_deref() {
                Some(nana_ui::runtime::ComponentGeometry::Chart { layout, .. }) => {
                    layout.plot.unwrap()
                }
                other => panic!("chart geometry, got {other:?}"),
            }
        };
        session
            .hover_xy(
                bounds.x + plot[0] + (plot[2] - plot[0]) * 0.7,
                bounds.y + (plot[1] + plot[3]) * 0.5,
            )
            .unwrap();
        let hover = session
            .document()
            .context()
            .read(trend, |chart| chart.hover_state())
            .unwrap();
        assert_eq!(hover.current, [ANY_SERIES, 3], "{name}");

        // Over the pie's last slice.
        let bounds = session
            .document()
            .context()
            .world()
            .layout_box(pie.stable_id())
            .unwrap();
        session
            .hover_xy(
                bounds.x + bounds.width * 0.5 - bounds.width * 0.3,
                bounds.y + bounds.height * 0.5,
            )
            .unwrap();
        let hover = session
            .document()
            .context()
            .read(pie, |chart| chart.hover_state())
            .unwrap();
        assert_eq!(hover.current, [0, 2], "{name}");
        assert_eq!(
            session
                .document()
                .context()
                .read(trend, |chart| chart.hover_state().current)
                .unwrap()[0],
            u32::MAX,
            "{name}: leaving the trend clears it"
        );

        let (size, pixels) = session.screenshot_rgba().unwrap();
        assert_eq!(size.width, (width as f32 * scale) as u32);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[0] != pixel[1])
        );
        if let Some(path) = &output {
            assert!(
                session
                    .screenshot_png(path.join(format!("{name}.png")))
                    .unwrap()
                    .unique_colors
                    > 8
            );
        }
    }
}

#[test]
fn a_translucent_ring_has_no_join_cracks_or_overdraw() {
    if !offscreen::pixels_available() {
        return;
    }
    for count in [1, 48] {
        let id = DocumentId::new(2).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        let chart = cx
            .create_component(
                id,
                Chart::new(
                    ChartOption::new().animation(Animation::disabled()).series(
                        PieSeries::new(
                            "ring",
                            (0..count).map(|index| {
                                PieItem::new(format!("{index}"), 1.0)
                                    .color(ChartColor::Rgba([0.2, 0.4, 0.9, 0.5]))
                            }),
                        )
                        .ring(0.5, 0.9)
                        .pad(0.0)
                        .label(PieLabelPosition::None),
                    ),
                )
                .style(sized(Some(144.0), 144.0)),
            )
            .unwrap();
        let mut session = RuntimeAgentSession::new_scaled(document, 160, 160, 2.0).unwrap();
        let (size, pixels) = session.screenshot_rgba().unwrap();
        let cx = session.document().context();
        let bounds = cx.world().layout_box(chart.stable_id()).unwrap();
        let nodes = cx.world().extract_nodes(&[chart.stable_id()]);
        let Some(nana_ui::runtime::ComponentGeometry::Chart { layout, .. }) =
            nodes[0].component_geometry.as_deref()
        else {
            panic!("chart geometry");
        };
        let slice_at = |x: f32, y: f32| {
            layout.hit.items.iter().position(|item| match item.region {
                ItemRegion::Sector {
                    center,
                    start,
                    end,
                    inner,
                    outer,
                } => ItemRegion::Sector {
                    center,
                    start,
                    end,
                    inner,
                    outer: outer - nana_ui::runtime::chart::hit::EMPHASIS_GROWTH,
                }
                .contains([x - bounds.x, y - bounds.y]),
                region => region.contains([x - bounds.x, y - bounds.y]),
            })
        };
        let mut reference: Option<[u8; 3]> = None;
        let mut samples = 0;
        for y in 0..size.height {
            for x in 0..size.width {
                let lx = (x as f32 + 0.5) / 2.0;
                let ly = (y as f32 + 0.5) / 2.0;
                let Some(index) = slice_at(lx, ly) else {
                    continue;
                };
                // Clear of the ring's edges and of the antialiased joins.
                if ![
                    (lx - 1.5, ly),
                    (lx + 1.5, ly),
                    (lx, ly - 1.5),
                    (lx, ly + 1.5),
                ]
                .into_iter()
                .all(|(x, y)| slice_at(x, y) == Some(index))
                {
                    continue;
                }
                let offset = ((y * size.width + x) * 4) as usize;
                let color: [u8; 3] = pixels[offset..offset + 3].try_into().unwrap();
                if let Some(reference) = reference {
                    assert!(
                        color
                            .into_iter()
                            .zip(reference)
                            .all(|(a, b)| a.abs_diff(b) <= 3),
                        "overdraw or seam: {count}:{lx},{ly} {color:?} != {reference:?}"
                    );
                } else {
                    reference = Some(color);
                }
                samples += 1;
            }
        }
        assert!(samples > 100);
        if let Some(path) = std::env::var_os("NANA_CHART_CAPTURE_DIR") {
            session
                .screenshot_png(
                    std::path::Path::new(&path).join(format!("alpha-ring-{count}-2x.png")),
                )
                .unwrap();
        }
    }
}
