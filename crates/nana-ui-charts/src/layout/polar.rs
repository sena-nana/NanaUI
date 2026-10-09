//! Series around a centre: pie (and ring, rose), radar and gauge.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::builder::{Polyline, fill_style, line_style, symbol_style, with_alpha};
use super::cartesian::{symbol_at, symbol_shape};
use super::{
    ChartText, DrawKey, DrawPart, LABEL_GAP, LayoutInput, MarkBuilder, TextAlign, text_rect,
};
use crate::hit::{HitModel, ItemHit, ItemRegion};
use crate::marks::{GpuShape, SHAPE_EMPHASIS_GROW, ShapeKind, SymbolShape, draw_flags};
use crate::option::{
    GaugeSeries, Length, PieLabelPosition, PieSeries, RadarCoord, RadarSeries, RadarShape,
    RoseType, Series, Symbol,
};
use crate::scale;
use crate::smooth::Along;

/// Screen angle (radians, clockwise from 3 o'clock) of an ECharts angle
/// (degrees, counter-clockwise from 3 o'clock).
fn screen_angle(degrees: f32) -> f32 {
    -degrees.to_radians()
}

fn polar(center: [f32; 2], radius: f32, angle: f32) -> [f32; 2] {
    [
        center[0] + radius * angle.cos(),
        center[1] + radius * angle.sin(),
    ]
}

fn center_of(frame: [f32; 4], center: [Length; 2]) -> [f32; 2] {
    let (w, h) = (frame[2] - frame[0], frame[3] - frame[1]);
    [
        frame[0] + center[0].resolve(w),
        frame[1] + center[1].resolve(h),
    ]
}

/// Half the smaller side: what percent radii are of.
fn radius_basis(frame: [f32; 4]) -> f32 {
    ((frame[2] - frame[0]).min(frame[3] - frame[1]) * 0.5).max(1.0)
}

pub(super) fn layout(
    input: &LayoutInput<'_>,
    frame: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) {
    for (index, series) in input.option.series.iter().enumerate() {
        match series {
            Series::Pie(pie) => layout_pie(input, index, pie, frame, builder, texts, hit),
            Series::Radar(radar) => {
                if let Some(coord) = &input.option.radar {
                    layout_radar(input, index, radar, coord, frame, builder, texts, hit);
                }
            }
            Series::Gauge(gauge) if !input.state.is_hidden(&gauge.name) => {
                layout_gauge(input, index, gauge, frame, builder, texts, hit);
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_pie(
    input: &LayoutInput<'_>,
    series_index: usize,
    pie: &PieSeries,
    frame: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) {
    let theme = input.theme;
    let measure = input.measure;
    let size = theme.font_size;
    let series = series_index as u32;
    let center = center_of(frame, pie.center);
    let basis = radius_basis(frame);
    let mut outer = pie.radius[1].resolve(basis).max(1.0);
    let inner = pie.radius[0].resolve(basis).clamp(0.0, outer);
    // Visible items keep their original index (colors, legend, hover).
    let items: Vec<(usize, f64, [f32; 4], Arc<str>)> = pie
        .data
        .iter()
        .enumerate()
        .filter(|(_, item)| !input.state.is_hidden(&item.name))
        .map(|(index, item)| {
            let color = item
                .color
                .map_or_else(|| theme.series_color(index), |c| theme.resolve(c));
            let value = if item.value.is_finite() {
                item.value.max(0.0)
            } else {
                0.0
            };
            (index, value, color, item.name.clone())
        })
        .collect();
    let total: f64 = items.iter().map(|item| item.1).sum();
    let max_value = items.iter().map(|item| item.1).fold(0.0, f64::max);
    if items.is_empty() {
        return;
    }
    // Outside labels need room beside the ring.
    let outside = pie.label == PieLabelPosition::Outside;
    let label_sizes: Vec<[f32; 2]> = items
        .iter()
        .map(|(_, value, _, name)| measure.measure(&label_text(pie, name, *value), size))
        .collect();
    if outside {
        let widest = label_sizes.iter().map(|s| s[0]).fold(0.0, f32::max);
        let room_x = (frame[2] - frame[0]) * 0.5 - widest - LINE_REACH - LABEL_GAP;
        let room_y = (frame[3] - frame[1]) * 0.5 - label_sizes[0][1];
        outer = outer.min(room_x).min(room_y).max(outer.min(16.0));
    }
    let inner = inner.min(outer * 0.95);
    let count = items.len() as f32;
    let start = screen_angle(pie.start_angle);
    let direction = if pie.clockwise { 1.0 } else { -1.0 };
    let min_angle = pie.min_angle.to_radians();
    // Each item's angular span.
    let mut spans: Vec<f32> = items
        .iter()
        .map(|(_, value, _, _)| match pie.rose {
            Some(RoseType::Area) => TAU / count,
            _ if total > 0.0 => (*value / total) as f32 * TAU,
            _ => TAU / count,
        })
        .collect();
    if min_angle > 0.0 {
        // Lift small slices to the minimum, taking the excess off the rest.
        let lifted: f32 = spans.iter().map(|s| (min_angle - s).max(0.0)).sum();
        let big: f32 = spans.iter().filter(|s| **s > min_angle).sum();
        for span in &mut spans {
            if *span < min_angle {
                *span = min_angle;
            } else if big > 0.0 {
                *span -= lifted * (*span / big);
            }
        }
    }
    let mut angle = start;
    let mut labels: Vec<PieLabel> = Vec::new();
    for ((item, span), label_size) in items.iter().zip(&spans).zip(&label_sizes) {
        let (index, value, color, name) = item;
        let (a0, a1) = if direction > 0.0 {
            (angle, angle + span)
        } else {
            (angle - span, angle)
        };
        angle += span * direction;
        let radius = match pie.rose {
            Some(_) if max_value > 0.0 => {
                let ratio = (*value / max_value) as f32;
                let ratio = if pie.rose == Some(RoseType::Area) {
                    ratio.sqrt()
                } else {
                    ratio
                };
                inner + (outer - inner) * ratio.max(0.08)
            }
            _ => outer,
        };
        let style = builder.style(fill_style(*color, *color));
        let mut shape = GpuShape::new(
            ShapeKind::Sector,
            SHAPE_EMPHASIS_GROW,
            style,
            series,
            *index as u32,
        );
        shape.to = [a0, a1, inner, radius];
        // Entry sweeps every slice out of the start angle.
        shape.from = [start, start, inner, radius];
        shape.extra = [center[0], center[1], pie.corner_radius, pie.pad];
        builder.shape(
            shape,
            DrawKey {
                series,
                part: DrawPart::Shapes,
                run: 0,
            },
            0,
        );
        hit.items.push(ItemHit {
            region: ItemRegion::Sector {
                center,
                start: a0,
                end: a1,
                inner,
                outer: radius + 6.0,
            },
            series,
            index: *index as u32,
            title: Some(pie.name.clone()),
            row: Some(name.clone()),
            value: *value,
            color: *color,
        });
        let mid = (a0 + a1) * 0.5;
        match pie.label {
            PieLabelPosition::Outside if *span > 0.02 => labels.push(PieLabel {
                mid,
                radius,
                size: *label_size,
                text: label_text(pie, name, *value),
                color: *color,
                y: 0.0,
            }),
            PieLabelPosition::Inside if *span > 0.15 => {
                let text = label_text(pie, name, *value);
                let at = polar(center, (inner + radius) * 0.5, mid);
                texts.push(ChartText {
                    rect: text_rect(
                        measure,
                        &text,
                        size,
                        at,
                        TextAlign::Center,
                        TextAlign::Center,
                    ),
                    text,
                    color: theme.on_series,
                    size,
                    weight: None,
                });
            }
            _ => {}
        }
    }
    builder.close_shapes();
    if pie.label == PieLabelPosition::Center {
        let text = pie.name.clone();
        texts.push(ChartText {
            rect: text_rect(
                measure,
                &text,
                size,
                center,
                TextAlign::Center,
                TextAlign::Center,
            ),
            text,
            color: theme.text,
            size,
            weight: None,
        });
    }
    place_pie_labels(input, series, center, frame, &mut labels, builder, texts);
}

/// How far an outside label's guide reaches past the ring.
const LINE_REACH: f32 = 22.0;

struct PieLabel {
    mid: f32,
    radius: f32,
    size: [f32; 2],
    text: Arc<str>,
    color: [f32; 4],
    y: f32,
}

fn label_text(pie: &PieSeries, name: &Arc<str>, value: f64) -> Arc<str> {
    match &pie.label_formatter {
        Some(formatter) => formatter.format(value),
        None => name.clone(),
    }
}

/// Outside labels: each side sorted top to bottom and pushed apart, with a
/// two-segment guide from the slice to the label.
fn place_pie_labels(
    input: &LayoutInput<'_>,
    series: u32,
    center: [f32; 2],
    frame: [f32; 4],
    labels: &mut [PieLabel],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
) {
    let size = input.theme.font_size;
    for right in [true, false] {
        let mut side: Vec<&mut PieLabel> = labels
            .iter_mut()
            .filter(|label| (label.mid.cos() >= 0.0) == right)
            .collect();
        for label in &mut side {
            label.y = polar(center, label.radius + LINE_REACH * 0.5, label.mid)[1];
        }
        side.sort_by(|a, b| a.y.total_cmp(&b.y));
        // Push down where neighbours overlap, then back up from the bottom.
        let mut floor = frame[1];
        for label in &mut side {
            let half = label.size[1] * 0.5;
            label.y = label.y.max(floor + half);
            floor = label.y + half + 2.0;
        }
        let mut ceiling = frame[3];
        for label in side.iter_mut().rev() {
            let half = label.size[1] * 0.5;
            label.y = label.y.min(ceiling - half);
            ceiling = label.y - half - 2.0;
        }
        for label in side {
            let edge = polar(center, label.radius, label.mid);
            let elbow_r = label.radius + LINE_REACH * 0.5;
            let mut elbow = polar(center, elbow_r, label.mid);
            elbow[1] = label.y;
            let sign = if right { 1.0 } else { -1.0 };
            let end = [elbow[0] + sign * LINE_REACH * 0.5, label.y];
            let style = builder.style(line_style(label.color, 1.0, [0.0, 0.0]));
            builder.polyline(
                Polyline {
                    points: &[edge, elbow, end],
                    bases: None,
                    from: None,
                    from_bases: None,
                },
                Some(style),
                None,
                1.0,
                series,
                1,
                0,
                Along::X,
            );
            let anchor = [end[0] + sign * nana_ui_core::space::XS, label.y];
            texts.push(ChartText {
                rect: text_rect(
                    input.measure,
                    &label.text,
                    size,
                    anchor,
                    if right {
                        TextAlign::Start
                    } else {
                        TextAlign::End
                    },
                    TextAlign::Center,
                ),
                text: label.text.clone(),
                color: input.theme.text,
                size,
                weight: None,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_radar(
    input: &LayoutInput<'_>,
    series_index: usize,
    radar: &RadarSeries,
    coord: &RadarCoord,
    frame: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) {
    let theme = input.theme;
    let measure = input.measure;
    let size = theme.font_size;
    let count = coord.indicators.len();
    if count < 3 {
        return;
    }
    let series = series_index as u32;
    let line_height = measure.measure("0", size)[1];
    let widest = coord
        .indicators
        .iter()
        .map(|indicator| measure.measure(&indicator.name, size)[0])
        .fold(0.0, f32::max);
    let center = center_of(frame, coord.center);
    let basis = radius_basis(frame);
    let radius = coord
        .radius
        .resolve(basis)
        .min((frame[2] - frame[0]) * 0.5 - widest - LABEL_GAP)
        .min((frame[3] - frame[1]) * 0.5 - line_height - LABEL_GAP)
        .max(8.0);
    let angle_of = |i: usize| screen_angle(coord.start_angle + 360.0 * i as f32 / count as f32);
    // Each indicator's max: given, or a nice bound of the data.
    let maxima: Vec<(f64, f64)> = coord
        .indicators
        .iter()
        .enumerate()
        .map(|(i, indicator)| {
            let data_max = radar
                .data
                .iter()
                .filter_map(|item| item.values.get(i).copied())
                .filter(|v| v.is_finite())
                .fold(0.0, f64::max);
            let max = indicator
                .max
                .unwrap_or_else(|| scale::nice_linear(0.0, data_max, 5, Some(0.0), None).max);
            (indicator.min.unwrap_or(0.0), max.max(f64::EPSILON))
        })
        .collect();

    // The web: guides drawn only by the first radar series.
    let first_radar = input
        .option
        .series
        .iter()
        .position(|s| matches!(s, Series::Radar(_)))
        == Some(series_index);
    let splits = coord.split_number.max(1);
    if first_radar {
        let ring = |k: usize| radius * k as f32 / splits as f32;
        let guide = with_alpha(theme.grid, 1.0);
        for k in 1..=splits {
            let r = ring(k);
            match coord.shape {
                RadarShape::Polygon => {
                    let points: Vec<[f32; 2]> =
                        (0..count).map(|i| polar(center, r, angle_of(i))).collect();
                    if coord.split_area && k % 2 == 1 {
                        let bases: Vec<[f32; 2]> = (0..count)
                            .map(|i| polar(center, ring(k - 1), angle_of(i)))
                            .collect();
                        let area = builder.style(fill_style(theme.band, theme.band));
                        builder.polyline(
                            Polyline {
                                points: &points,
                                bases: Some(&bases),
                                from: None,
                                from_bases: None,
                            },
                            None,
                            Some(area),
                            0.0,
                            u32::MAX,
                            k as u32,
                            draw_flags::GUIDE | draw_flags::CLOSED | draw_flags::SOFT_BASE,
                            Along::X,
                        );
                    }
                    let style = builder.style(line_style(guide, 1.0, [0.0, 0.0]));
                    builder.polyline(
                        Polyline {
                            points: &points,
                            bases: None,
                            from: None,
                            from_bases: None,
                        },
                        Some(style),
                        None,
                        1.0,
                        u32::MAX,
                        100 + k as u32,
                        draw_flags::GUIDE | draw_flags::CLOSED,
                        Along::X,
                    );
                }
                RadarShape::Circle => {
                    let key = DrawKey {
                        series: u32::MAX,
                        part: DrawPart::Guide,
                        run: 200 + k as u32,
                    };
                    if coord.split_area && k % 2 == 1 {
                        let style = builder.style(fill_style(theme.band, theme.band));
                        let mut band = GpuShape::new(ShapeKind::Sector, 0, style, u32::MAX, 0);
                        band.to = [0.0, TAU, ring(k - 1), r];
                        band.from = band.to;
                        band.extra = [center[0], center[1], 0.0, 0.0];
                        builder.shape(band, key, 0);
                    }
                    let style = builder.style(fill_style(guide, guide));
                    let mut circle = GpuShape::new(ShapeKind::Sector, 0, style, u32::MAX, 0);
                    circle.to = [0.0, TAU, r - 0.5, r + 0.5];
                    circle.from = circle.to;
                    circle.extra = [center[0], center[1], 0.0, 0.0];
                    builder.shape(circle, key, 0);
                    builder.close_shapes();
                }
            }
        }
        let style = builder.style(line_style(guide, 1.0, [0.0, 0.0]));
        for i in 0..count {
            builder.polyline(
                Polyline {
                    points: &[center, polar(center, radius, angle_of(i))],
                    bases: None,
                    from: None,
                    from_bases: None,
                },
                Some(style),
                None,
                1.0,
                u32::MAX,
                300 + i as u32,
                draw_flags::GUIDE,
                Along::X,
            );
            let angle = angle_of(i);
            let anchor = polar(center, radius + LABEL_GAP, angle);
            let (cos, sin) = (angle.cos(), angle.sin());
            let align = if cos > 0.2 {
                TextAlign::Start
            } else if cos < -0.2 {
                TextAlign::End
            } else {
                TextAlign::Center
            };
            let vertical = if sin > 0.2 {
                TextAlign::Start
            } else if sin < -0.2 {
                TextAlign::End
            } else {
                TextAlign::Center
            };
            let name = &coord.indicators[i].name;
            texts.push(ChartText {
                rect: text_rect(measure, name, size, anchor, align, vertical),
                text: name.clone(),
                color: theme.muted,
                size,
                weight: None,
            });
        }
    }

    for (item_index, item) in radar.data.iter().enumerate() {
        if input.state.is_hidden(&item.name) {
            continue;
        }
        let color = item.color.map_or_else(
            || theme.series_color(series_index + item_index),
            |c| theme.resolve(c),
        );
        let points: Vec<[f32; 2]> = (0..count)
            .map(|i| {
                let (min, max) = maxima[i];
                let value = item.values.get(i).copied().unwrap_or(min);
                let ratio = if value.is_finite() {
                    ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                polar(center, radius * ratio, angle_of(i))
            })
            .collect();
        let bases = vec![center; count];
        let stroke = builder.style(line_style(color, radar.width, [0.0, 0.0]));
        let area = radar.area.map(|area| {
            let fill = with_alpha(area.color.map_or(color, |c| theme.resolve(c)), area.opacity);
            builder.style(fill_style(fill, fill))
        });
        builder.polyline(
            Polyline {
                points: &points,
                bases: Some(&bases),
                // Entry grows every vertex out of the centre.
                from: Some(&bases),
                from_bases: Some(&bases),
            },
            (radar.width > 0.0).then_some(stroke),
            area,
            radar.width,
            series,
            item_index as u32,
            draw_flags::CLOSED | draw_flags::EMPHASIS,
            Along::X,
        );
        if let Some(shape) = symbol_shape(radar.symbol) {
            let fill = if radar.symbol == Symbol::EmptyCircle {
                theme.surface
            } else {
                color
            };
            let border = if radar.symbol == Symbol::EmptyCircle {
                1.5
            } else {
                0.0
            };
            let style = builder.style(symbol_style(fill, color, border));
            for (i, point) in points.iter().enumerate() {
                builder.shape(
                    symbol_at(
                        shape,
                        *point,
                        center,
                        radar.symbol_size + border,
                        0.0,
                        (radar.symbol_size + border) * 1.6,
                        border,
                        style,
                        series,
                        item_index as u32,
                        SHAPE_EMPHASIS_GROW,
                    ),
                    DrawKey {
                        series,
                        part: DrawPart::Shapes,
                        run: item_index as u32,
                    },
                    0,
                );
                hit.items.push(ItemHit {
                    region: ItemRegion::Circle {
                        center: *point,
                        radius: 8.0,
                    },
                    series,
                    index: item_index as u32,
                    title: Some(item.name.clone()),
                    row: Some(coord.indicators[i].name.clone()),
                    value: item.values.get(i).copied().unwrap_or(f64::NAN),
                    color,
                });
            }
            builder.close_shapes();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_gauge(
    input: &LayoutInput<'_>,
    series_index: usize,
    gauge: &GaugeSeries,
    frame: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) {
    let theme = input.theme;
    let measure = input.measure;
    let size = theme.font_size;
    let series = series_index as u32;
    let center = center_of(frame, gauge.center);
    let radius = gauge.radius.resolve(radius_basis(frame)).max(8.0);
    let color = gauge
        .color
        .map_or_else(|| theme.series_color(series_index), |c| theme.resolve(c));
    let start = screen_angle(gauge.start_angle);
    let mut end = screen_angle(gauge.end_angle);
    while end <= start {
        end += TAU;
    }
    let span = (end - start).min(TAU);
    let end = start + span;
    let range = gauge.max - gauge.min;
    let fraction = if range.abs() > f64::EPSILON && gauge.value.is_finite() {
        ((gauge.value - gauge.min) / range).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let value_angle = start + span * fraction;
    let width = gauge.width.min(radius * 0.5);
    let key = |run| DrawKey {
        series,
        part: DrawPart::Shapes,
        run,
    };
    let round = width * 0.5;
    // Track and progress arcs, with rounded ends.
    let track_style = builder.style(fill_style(theme.band, theme.band));
    let mut track = GpuShape::new(ShapeKind::Sector, 0, track_style, u32::MAX, 0);
    track.to = [start, end, radius - width, radius];
    track.from = track.to;
    track.extra = [center[0], center[1], round, 0.0];
    builder.shape(track, key(0), 0);
    if gauge.progress {
        let style = builder.style(fill_style(color, color));
        let mut progress = GpuShape::new(ShapeKind::Sector, 0, style, series, 0);
        progress.to = [start, value_angle, radius - width, radius];
        progress.from = [start, start, radius - width, radius];
        progress.extra = [center[0], center[1], round, 0.0];
        builder.shape(progress, key(0), 0);
    }
    builder.close_shapes();
    hit.items.push(ItemHit {
        region: ItemRegion::Sector {
            center,
            start,
            end,
            inner: 0.0,
            outer: radius,
        },
        series,
        index: 0,
        title: Some(gauge.name.clone()),
        row: Some(gauge.name.clone()),
        value: gauge.value,
        color,
    });
    // Ticks and labels inside the track.
    let splits = gauge.split_number.max(1);
    let tick_style = builder.style(fill_style(theme.muted, theme.muted));
    let ticks = scale::nice_linear(
        gauge.min,
        gauge.max,
        splits,
        Some(gauge.min),
        Some(gauge.max),
    );
    if gauge.ticks {
        let minor_per = 5;
        for step in 0..=splits * minor_per {
            let t = step as f32 / (splits * minor_per) as f32;
            let angle = start + span * t;
            let major = step % minor_per == 0;
            let length = if major { 8.0 } else { 4.0 };
            let mut tick = GpuShape::new(ShapeKind::Needle, 0, tick_style, u32::MAX, 0);
            let offset = radius - width - 4.0 - length;
            tick.to = [center[0], center[1], angle, length];
            tick.from = tick.to;
            tick.extra = [if major { 2.0 } else { 1.0 }, offset, 0.5, 0.0];
            builder.shape(tick, key(1), 0);
        }
        builder.close_shapes();
    }
    if gauge.labels {
        let label_radius = radius - width - 26.0;
        let texts_at: Vec<(f32, Arc<str>)> = (0..=splits)
            .map(|step| {
                let t = step as f32 / splits as f32;
                let value = gauge.min + range * t as f64;
                let text: Arc<str> =
                    scale::format_number(value, ticks.step.max(range / splits as f64)).into();
                (start + span * t, text)
            })
            .collect();
        // Every label when the arc has room for them, else every other.
        let widest = texts_at
            .iter()
            .map(|(_, text)| measure.measure(text, size)[0])
            .fold(0.0, f32::max);
        let spacing = label_radius.max(1.0) * span / splits as f32;
        let every = if spacing >= widest + LABEL_GAP { 1 } else { 2 };
        for (step, (angle, text)) in texts_at.into_iter().enumerate() {
            if step % every != 0 && step != splits {
                continue;
            }
            let at = polar(center, label_radius, angle);
            texts.push(ChartText {
                rect: text_rect(
                    measure,
                    &text,
                    size,
                    at,
                    TextAlign::Center,
                    TextAlign::Center,
                ),
                text,
                color: theme.muted,
                size,
                weight: None,
            });
        }
    }
    if gauge.pointer {
        let style = builder.style(fill_style(color, color));
        let length = radius - width - 14.0;
        let mut needle = GpuShape::new(ShapeKind::Needle, 0, style, series, 0);
        needle.to = [center[0], center[1], value_angle, length + 8.0];
        needle.from = [center[0], center[1], start, length + 8.0];
        needle.extra = [5.0, -8.0, 2.5, 0.0];
        builder.shape(needle, key(2), 0);
        let anchor_style = builder.style(symbol_style(theme.surface, color, 3.0));
        builder.shape(
            symbol_at(
                SymbolShape::Circle,
                center,
                center,
                14.0,
                14.0,
                14.0,
                3.0,
                anchor_style,
                series,
                0,
                0,
            ),
            key(2),
            0,
        );
        builder.close_shapes();
    }
    let detail: Arc<str> = match &gauge.detail_formatter {
        Some(formatter) => formatter.format(gauge.value),
        None => scale::format_number(
            gauge.value,
            if gauge.value.fract() == 0.0 { 1.0 } else { 0.1 },
        )
        .into(),
    };
    let detail_size = nana_ui_core::type_scale::HEADING;
    let at = [center[0], center[1] + radius * 0.45];
    texts.push(ChartText {
        rect: text_rect(
            measure,
            &detail,
            detail_size,
            at,
            TextAlign::Center,
            TextAlign::Center,
        ),
        text: detail,
        color: theme.text,
        size: detail_size,
        weight: Some(nana_ui_core::type_scale::MEDIUM),
    });
}
