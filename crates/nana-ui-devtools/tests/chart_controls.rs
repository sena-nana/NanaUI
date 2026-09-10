#![cfg(feature = "agent")]

use nana_ui::runtime::{
    Button, DocumentId, DonutChart, DonutSlice, LengthSpec, NodeStyle, RuntimeDocument, Stack,
    Text, TimeSeriesChart, TimeSeriesLayer,
};
use nana_ui_core::{Icon, SemanticColorMix, SemanticColorRole as R, ThemeMode};
use nana_ui_devtools::{agent::RuntimeAgentSession, offscreen};
use std::sync::Arc;

#[test]
fn charts_and_icon_button_render_real_geometry_and_hover_in_both_themes() {
    if !offscreen::pixels_available() {
        return;
    }
    let output = std::env::var_os("NANA_CHART_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
    }
    for (name, theme, width, scale) in [
        ("light-wide-1x", ThemeMode::Light, 560, 1.0),
        ("dark-wide-1x", ThemeMode::Dark, 560, 1.0),
        ("light-narrow-2x", ThemeMode::Light, 320, 2.0),
        ("dark-narrow-2x", ThemeMode::Dark, 320, 2.0),
    ] {
        let id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        cx.set_theme(theme).unwrap();
        let root = cx
            .create_component(id, Stack::column(12.0).padding(16.0))
            .unwrap();
        let button_view = Button::new("Save memory").icon(Icon::File);
        let button_style = button_view
            .style
            .clone()
            .surface_mix(SemanticColorMix::new(R::Accent, R::Surface, 0.22))
            .outline_mix(SemanticColorMix::new(R::Accent, R::Border, 0.58), 1.0);
        let button = cx
            .create_detached_component(id, button_view.style(button_style))
            .unwrap();
        let donut = cx
            .create_detached_component(
                id,
                DonutChart::new([
                    DonutSlice {
                        value: 20.0,
                        color: R::Accent,
                    },
                    DonutSlice {
                        value: 30.0,
                        color: R::Success,
                    },
                    DonutSlice {
                        value: 50.0,
                        color: R::Warning,
                    },
                ])
                .labels(["Input", "Output", "Cache"]),
            )
            .unwrap();
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(220.0));
        let chart = cx
            .create_detached_component(
                id,
                TimeSeriesChart::new([4.0, 8.0, 6.0, 10.0])
                    .label("Total")
                    .axis_labels(["09-01", "09-02", "09-03", "09-04"])
                    .tooltip_details(["Cost $0.10", "Cost $0.20", "Cost $0.15", "Cost $0.30"])
                    .stacked([
                        TimeSeriesLayer::new("Input", [1.0, 2.0, 2.0, 3.0], R::Accent),
                        TimeSeriesLayer::new("Output", [3.0, 6.0, 4.0, 7.0], R::Success),
                    ])
                    .style(style),
            )
            .unwrap();
        let detail = cx
            .create_detached_component(id, Text::new("Semantic chart colors and localized values"))
            .unwrap();
        cx.reconcile_children(
            root.stable_id(),
            &[
                button.stable_id(),
                donut.stable_id(),
                chart.stable_id(),
                detail.stable_id(),
            ],
        )
        .unwrap();
        let mut session = RuntimeAgentSession::new_scaled(document, width, 420, scale).unwrap();
        session.flush().unwrap();
        let bounds = session
            .document()
            .context()
            .world()
            .layout_box(donut.stable_id())
            .unwrap();
        session
            .hover_xy(
                bounds.x + bounds.width * 0.8,
                bounds.y + bounds.height * 0.2,
            )
            .unwrap();
        assert_eq!(
            session
                .document()
                .context()
                .read(donut, |c| c.active)
                .unwrap(),
            Some(0)
        );
        if let Some(path) = &output {
            assert!(
                session
                    .screenshot_png(path.join(format!("{name}-donut.png")))
                    .unwrap()
                    .unique_colors
                    > 8
            );
        }
        let bounds = session
            .document()
            .context()
            .world()
            .layout_box(chart.stable_id())
            .unwrap();
        assert_eq!(bounds.height, 220.0);
        session
            .hover_xy(
                bounds.x + 48.0 + (bounds.width - 60.0) * 0.875,
                bounds.y + 50.0,
            )
            .unwrap();
        assert_eq!(
            session
                .document()
                .context()
                .read(donut, |c| c.active)
                .unwrap(),
            None
        );
        assert_eq!(
            session
                .document()
                .context()
                .read(chart, |c| c.active)
                .unwrap(),
            Some(3)
        );
        let (size, pixels) = session.screenshot_rgba().unwrap();
        assert_eq!(size.width, (width as f32 * scale) as u32);
        assert_eq!(size.height, (420.0 * scale) as u32);
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
                    .screenshot_png(path.join(format!("{name}-trend.png")))
                    .unwrap()
                    .unique_colors
                    > 8
            );
        }
    }
}

#[test]
fn donut_ring_has_no_join_cracks_or_translucent_overdraw() {
    if !offscreen::pixels_available() {
        return;
    }
    for count in [1, 48] {
        let id = DocumentId::new(2).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        let model = cx.world().style_model();
        let mut palette = model.palette;
        palette.accent.a = 0.5;
        cx.set_style_tokens(model.theme_mode, model.metrics, palette, model.titlebar)
            .unwrap();
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(144.0));
        layout.height = Some(LengthSpec::Px(144.0));
        let chart = cx
            .create_component(
                id,
                DonutChart::new((0..count).map(|_| DonutSlice {
                    value: 1.0,
                    color: R::Accent,
                }))
                .style(style),
            )
            .unwrap();
        let mut session = RuntimeAgentSession::new_scaled(document, 160, 160, 2.0).unwrap();
        let (size, pixels) = session.screenshot_rgba().unwrap();
        let cx = session.document().context();
        let bounds = cx.world().layout_box(chart.stable_id()).unwrap();
        let view = cx.read(chart, Clone::clone).unwrap();
        let mut reference: Option<[u8; 3]> = None;
        let mut samples = 0;
        for y in 0..size.height {
            for x in 0..size.width {
                let lx = (x as f32 + 0.5) / 2.0;
                let ly = (y as f32 + 0.5) / 2.0;
                let Some(index) = view.slice_at(bounds, lx, ly) else {
                    continue;
                };
                if ![
                    (lx - 1.5, ly),
                    (lx + 1.5, ly),
                    (lx, ly - 1.5),
                    (lx, ly + 1.5),
                ]
                .into_iter()
                .all(|(x, y)| view.slice_at(bounds, x, y) == Some(index))
                {
                    continue;
                }
                let offset = ((y * size.width + x) * 4) as usize;
                let color: [u8; 3] = pixels[offset..offset + 3].try_into().unwrap();
                assert!(
                    color.iter().max().unwrap() - color.iter().min().unwrap() > 15,
                    "ring interior is unpainted at {count}:{lx},{ly}"
                );
                if let Some(reference) = reference {
                    assert!(
                        color
                            .into_iter()
                            .zip(reference)
                            .all(|(a, b)| a.abs_diff(b) <= 2),
                        "alpha overdraw or seam: {count}:{lx},{ly} {color:?} != {reference:?}"
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

#[test]
fn chart_tooltips_keep_viewport_geometry_below_scrolling_ancestors() {
    use nana_ui::runtime::{ScrollAxes, ScrollOffset, ScrollView};
    if !offscreen::pixels_available() {
        return;
    }
    let output = std::env::var_os("NANA_CHART_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
    }
    for trend in [false, true] {
        let id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(id);
        let cx = document.context_mut();
        let root = cx
            .create_component(id, Stack::column(0.0).padding(20.0))
            .unwrap();
        let mut style = NodeStyle::default();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(360.0));
        layout.height = Some(LengthSpec::Px(280.0));
        let scroll = cx
            .create_detached_component(id, ScrollView::new(ScrollAxes::Vertical).style(style))
            .unwrap();
        let content = cx
            .create_detached_component(id, Stack::column(0.0))
            .unwrap();
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(0.0));
        let spacer = cx
            .create_detached_component(id, Stack::column(0.0).style(style))
            .unwrap();
        let chart = if trend {
            let mut chart = TimeSeriesChart::new([4.0, 8.0])
                .axis_labels(["09-01", "09-02"])
                .stacked([TimeSeriesLayer::new("Input", [4.0, 8.0], R::Accent)]);
            Arc::make_mut(&mut chart.style.layout).height = Some(LengthSpec::Px(160.0));
            cx.create_detached_component(id, chart).unwrap().stable_id()
        } else {
            cx.create_detached_component(
                id,
                DonutChart::new([DonutSlice {
                    value: 8.0,
                    color: R::Accent,
                }])
                .labels(["Project"]),
            )
            .unwrap()
            .stable_id()
        };
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(400.0));
        let tail = cx
            .create_detached_component(id, Stack::column(0.0).style(style))
            .unwrap();
        cx.append_child(root, scroll).unwrap();
        cx.append_child(scroll, content).unwrap();
        cx.reconcile_children(
            content.stable_id(),
            &[spacer.stable_id(), chart, tail.stable_id()],
        )
        .unwrap();
        let mut session = RuntimeAgentSession::new_scaled(document, 400, 340, 1.0).unwrap();
        for offset in [0.0, 400.0] {
            session.hover_xy(390.0, 330.0).unwrap();
            session
                .document_mut()
                .context_mut()
                .update_component(spacer, |spacer, _| {
                    let mut style = NodeStyle::default();
                    Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(offset));
                    *spacer = Stack::column(0.0).style(style);
                })
                .unwrap();
            session.flush().unwrap();
            session
                .document_mut()
                .context_mut()
                .scroll_to(scroll, ScrollOffset { x: 0.0, y: offset })
                .unwrap();
            session.flush().unwrap();
            let bounds = session.document().scene().draw_node_bounds(chart).unwrap();
            let point = if trend {
                (bounds.x + bounds.width * 0.8, bounds.y + 80.0)
            } else {
                (bounds.x + 55.0, bounds.y + 33.0)
            };
            session.hover_xy(point.0, point.1).unwrap();
            let cx = session.document().context();
            let tip = cx
                .world()
                .overlay_host(chart)
                .unwrap()
                .active
                .expect("normal chart tooltip");
            let layout = cx.world().layout_box(tip).unwrap();
            let draw = session.document().scene().draw_node_bounds(tip).unwrap();
            assert!(
                (draw.x - layout.x).abs() < 0.01 && (draw.y - layout.y).abs() < 0.01,
                "fixed tooltip received ancestor scroll twice: {draw:?} vs {layout:?}"
            );
            assert!(draw.y >= 0.0 && draw.y + draw.height <= 340.0);
            let tip_primitive = session
                .document()
                .scene()
                .primitives()
                .find(|p| p.node == tip)
                .unwrap();
            assert!(
                session
                    .document()
                    .scene()
                    .draw_primitive(tip_primitive.id)
                    .unwrap()
                    .clips
                    .is_empty(),
                "viewport overlay must escape the scroll clip"
            );
            if let Some(output) = &output {
                let name = if trend { "trend" } else { "donut" };
                assert!(
                    session
                        .screenshot_png(output.join(format!("fixed-{name}-scroll-{offset:.0}.png")))
                        .unwrap()
                        .unique_colors
                        > 8
                );
            }
        }
    }
}
