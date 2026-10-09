//! The cartesian grid: axes, split lines, and line, bar and scatter series.

use std::collections::HashMap;
use std::sync::Arc;

use super::builder::{Polyline, fill_style, line_style, symbol_style, with_alpha};
use super::{
    ChartText, DrawKey, DrawPart, LABEL_GAP, LayoutInput, MarkBuilder, SliderLayout, TextAlign,
    crisp, rects_overlap, text_rect,
};
use crate::hit::{AxisHit, AxisPoint, AxisSeries, BaseAxisHit, HitModel, ItemHit, ItemRegion};
use crate::marks::{
    GpuShape, SHAPE_CLIP_TO_PLOT, SHAPE_EMPHASIS_GROW, SHAPE_REVEAL, ShapeKind, SymbolShape,
    draw_flags,
};
use crate::option::{
    Axis, AxisBound, AxisKind, AxisPosition, BarSeries, DataZoomKind, Length, LineSeries, LineType,
    Sampling, ScatterSeries, Series, SeriesData, Symbol,
};
use crate::scale::{self, LinearTicks, TimeTick};
use crate::smooth::{self, Along};

/// Length of an axis tick mark.
const TICK: f32 = 4.0;
/// Height of the zoom slider.
const SLIDER_HEIGHT: f32 = 22.0;
/// Guides are hairlines.
const GUIDE_WIDTH: f32 = 1.0;
/// Longest segment of a smoothed curve, px.
const SMOOTH_SEGMENT: f32 = 4.0;

pub(super) struct CartesianOut {
    pub plot: [f32; 4],
    pub slider: Option<SliderLayout>,
}

/// A value axis' resolved scale.
#[derive(Debug, Clone)]
enum Scale {
    Linear(LinearTicks),
    Log {
        ticks: LinearTicks,
        base: f64,
    },
    Time {
        min: i64,
        max: i64,
        ticks: Vec<TimeTick>,
    },
    /// `first..first + count` categories; `gap` leaves half a band at the ends.
    Band {
        first: usize,
        count: usize,
        gap: bool,
    },
}

/// Maps axis values to px along one direction.
#[derive(Debug, Clone)]
pub(crate) struct AxisMap {
    scale: Scale,
    /// The px of the axis' start (min) and end (max).
    start: f32,
    end: f32,
}

impl AxisMap {
    fn new(scale: Scale, start: f32, end: f32, inverse: bool) -> Self {
        let (start, end) = if inverse { (end, start) } else { (start, end) };
        Self { scale, start, end }
    }

    fn unit(&self, value: f64) -> f64 {
        match &self.scale {
            Scale::Linear(ticks) => {
                let span = ticks.max - ticks.min;
                if span.abs() < f64::EPSILON {
                    0.5
                } else {
                    (value - ticks.min) / span
                }
            }
            Scale::Log { ticks, base } => {
                if value <= 0.0 {
                    return f64::NAN;
                }
                let (lo, hi) = (ticks.min.log(*base), ticks.max.log(*base));
                if (hi - lo).abs() < f64::EPSILON {
                    0.5
                } else {
                    (value.log(*base) - lo) / (hi - lo)
                }
            }
            Scale::Time { min, max, .. } => {
                let span = (*max - *min) as f64;
                if span <= 0.0 {
                    0.5
                } else {
                    (value - *min as f64) / span
                }
            }
            Scale::Band { first, count, gap } => {
                let index = value - *first as f64;
                if *gap {
                    (index + 0.5) / (*count).max(1) as f64
                } else if *count <= 1 {
                    0.5
                } else {
                    index / (*count - 1) as f64
                }
            }
        }
    }

    pub(crate) fn px(&self, value: f64) -> f32 {
        let unit = self.unit(value) as f32;
        self.start + (self.end - self.start) * unit
    }

    /// A band's width (categories), or zero.
    fn band(&self) -> f32 {
        match &self.scale {
            Scale::Band { count, .. } => (self.end - self.start).abs() / (*count).max(1) as f32,
            _ => 0.0,
        }
    }

    /// Where values fill from: zero when the axis shows it, else its min.
    fn baseline(&self) -> f32 {
        match &self.scale {
            Scale::Linear(ticks) => self.px(0.0_f64.clamp(ticks.min, ticks.max)),
            _ => self.start,
        }
    }

    fn lo_hi(&self) -> (f32, f32) {
        (self.start.min(self.end), self.start.max(self.end))
    }
}

/// One series placed on the grid.
struct Placed<'a> {
    index: usize,
    series: &'a Series,
    color: [f32; 4],
}

fn default_axis(series: &[Placed<'_>], base: bool) -> Axis {
    if !base {
        return Axis::value();
    }
    let points = series.iter().any(|placed| match placed.series {
        Series::Line(line) => matches!(line.data, SeriesData::Points(_)),
        Series::Bar(bar) => matches!(bar.data, SeriesData::Points(_)),
        Series::Scatter(_) => true,
        _ => false,
    });
    if points {
        Axis::value()
    } else {
        Axis::category(Vec::<Arc<str>>::new())
    }
}

fn series_data(series: &Series) -> Option<&SeriesData> {
    match series {
        Series::Line(line) => Some(&line.data),
        Series::Bar(bar) => Some(&bar.data),
        Series::Scatter(scatter) => Some(&scatter.data),
        _ => None,
    }
}

fn stack_of(series: &Series) -> Option<&Arc<str>> {
    match series {
        Series::Line(line) => line.stack.as_ref(),
        Series::Bar(bar) => bar.stack.as_ref(),
        _ => None,
    }
}

fn value_axis_index(series: &Series, base_x: bool) -> usize {
    let (x, y) = match series {
        Series::Line(line) => (line.x_axis_index, line.y_axis_index),
        Series::Bar(bar) => (bar.x_axis_index, bar.y_axis_index),
        Series::Scatter(scatter) => (scatter.x_axis_index, scatter.y_axis_index),
        _ => (0, 0),
    };
    if base_x { y } else { x }
}

/// Axis label text for a value.
fn value_label(axis: &Axis, value: f64, step: f64) -> Arc<str> {
    match &axis.axis_label.formatter {
        Some(formatter) => formatter.format(value),
        None => scale::format_number(value, step).into(),
    }
}

fn time_label(axis: &Axis, tick: TimeTick) -> Arc<str> {
    match &axis.axis_label.time_formatter {
        Some(formatter) => (formatter.0)(tick.value, tick.unit),
        None => scale::format_time(tick, 0).into(),
    }
}

/// A tick of either axis: where it is and what it says.
struct Tick {
    value: f64,
    label: Arc<str>,
}

fn axis_ticks(axis: &Axis, map: &AxisMap, labels: &[Arc<str>]) -> Vec<Tick> {
    match &map.scale {
        Scale::Linear(ticks) => ticks
            .ticks
            .iter()
            .map(|value| Tick {
                value: *value,
                label: value_label(axis, *value, ticks.step),
            })
            .collect(),
        Scale::Log { ticks, .. } => ticks
            .ticks
            .iter()
            .map(|value| Tick {
                value: *value,
                label: value_label(axis, *value, 1.0),
            })
            .collect(),
        Scale::Time { ticks, .. } => ticks
            .iter()
            .map(|tick| Tick {
                value: tick.value as f64,
                label: time_label(axis, *tick),
            })
            .collect(),
        Scale::Band { first, count, .. } => (*first..*first + *count)
            .map(|index| Tick {
                value: index as f64,
                label: labels
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| (index + 1).to_string().into()),
            })
            .collect(),
    }
}

pub(super) fn layout(
    input: &LayoutInput<'_>,
    frame: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) -> CartesianOut {
    let option = input.option;
    let theme = input.theme;
    let placed: Vec<Placed<'_>> = option
        .series
        .iter()
        .enumerate()
        .filter(|(_, series)| series.is_cartesian() && !input.state.is_hidden(series.name()))
        .map(|(index, series)| Placed {
            index,
            series,
            color: series
                .explicit_color()
                .map_or_else(|| theme.series_color(index), |color| theme.resolve(color)),
        })
        .collect();

    let x_axis = option
        .x_axis
        .first()
        .cloned()
        .unwrap_or_else(|| default_axis(&placed, true));
    // The base axis is the one series advance along: a category or time
    // axis. Bars and lines turn horizontal when it is the y axis.
    let y_first = option.y_axis.first().cloned();
    let base_x = !matches!(
        (x_axis.kind, y_first.as_ref().map(|axis| axis.kind)),
        (
            AxisKind::Value | AxisKind::Log,
            Some(AxisKind::Category | AxisKind::Time)
        )
    );
    let along = if base_x { Along::X } else { Along::Y };
    let base_axis = if base_x {
        x_axis.clone()
    } else {
        y_first
            .clone()
            .unwrap_or_else(|| default_axis(&placed, true))
    };
    let value_axes: Vec<Axis> = {
        let list = if base_x {
            &option.y_axis
        } else {
            &option.x_axis
        };
        if list.is_empty() {
            vec![Axis::value()]
        } else {
            list.clone()
        }
    };

    // The base axis' full extent, then the zoom window over it.
    let category_count = placed
        .iter()
        .filter_map(|placed| series_data(placed.series))
        .filter(|data| matches!(data, SeriesData::Values(_)))
        .map(SeriesData::len)
        .max()
        .unwrap_or(0)
        .max(base_axis.data.len());
    let mut base_lo = f64::INFINITY;
    let mut base_hi = f64::NEG_INFINITY;
    for placed in &placed {
        let Some(data) = series_data(placed.series) else {
            continue;
        };
        for index in 0..data.len() {
            if let Some((x, _)) = data.get(index)
                && x.is_finite()
            {
                base_lo = base_lo.min(x);
                base_hi = base_hi.max(x);
            }
        }
    }
    let zoom = option
        .data_zoom
        .iter()
        .enumerate()
        .find(|(_, zoom)| zoom.x_axis_index == 0)
        .and_then(|(index, _)| {
            input
                .state
                .zoom_window(option, index)
                .map(|window| (index, window))
        });
    let slider_zoom =
        zoom.filter(|(index, _)| option.data_zoom[*index].kind == DataZoomKind::Slider);
    let (start_pct, end_pct) = zoom.map_or((0.0, 100.0), |(_, window)| window);

    let is_category = base_axis.kind == AxisKind::Category;
    let wants_gap = base_axis.boundary_gap.unwrap_or_else(|| {
        placed
            .iter()
            .any(|placed| matches!(placed.series, Series::Bar(_)))
    });
    let (window_first, window_count) = if is_category {
        let total = category_count.max(1);
        let first = ((start_pct / 100.0) * total as f64).floor() as usize;
        let last = (((end_pct / 100.0) * total as f64).ceil() as usize).clamp(first + 1, total);
        (first.min(total - 1), last - first.min(total - 1))
    } else {
        (0, 0)
    };
    let (window_lo, window_hi) = if is_category || !base_lo.is_finite() {
        (
            window_first as f64,
            (window_first + window_count) as f64 - 1.0,
        )
    } else {
        let span = base_hi - base_lo;
        (
            base_lo + span * start_pct / 100.0,
            base_lo + span * end_pct / 100.0,
        )
    };
    let in_window = |x: f64| x >= window_lo - 1e-9 && x <= window_hi + 1e-9;

    // Stacks: `(stack name, value axis)` → series order inside the stack.
    let mut stacks: HashMap<(Arc<str>, usize), Vec<usize>> = HashMap::new();
    for (slot, placed) in placed.iter().enumerate() {
        if let Some(stack) = stack_of(placed.series)
            && matches!(series_data(placed.series), Some(SeriesData::Values(_)))
        {
            stacks
                .entry((stack.clone(), value_axis_index(placed.series, base_x)))
                .or_default()
                .push(slot);
        }
    }
    // Per placed series and index: `(base, top)` in value units.
    let mut stacked: HashMap<usize, Vec<(f64, f64)>> = HashMap::new();
    for members in stacks.values() {
        let columns: Vec<&[f64]> = members
            .iter()
            .map(|slot| match series_data(placed[*slot].series) {
                Some(SeriesData::Values(values)) => &values[..],
                _ => &[][..],
            })
            .collect();
        for (slot, result) in members.iter().zip(crate::stack::stack_same_sign(&columns)) {
            stacked.insert(*slot, result);
        }
    }
    let value_at = |slot: usize, index: usize| -> Option<(f64, f64, f64)> {
        let data = series_data(placed[slot].series)?;
        let (x, y) = data.get(index)?;
        if !y.is_finite() {
            return None;
        }
        let (base, top) = stacked
            .get(&slot)
            .and_then(|column| column.get(index).copied())
            .unwrap_or((0.0, y));
        Some((x, base, top))
    };

    // Value extents per value axis over the window.
    let mut extents: Vec<(f64, f64)> = vec![(f64::INFINITY, f64::NEG_INFINITY); value_axes.len()];
    for (slot, placed_series) in placed.iter().enumerate() {
        let axis = value_axis_index(placed_series.series, base_x).min(value_axes.len() - 1);
        let Some(data) = series_data(placed_series.series) else {
            continue;
        };
        let bars = matches!(placed_series.series, Series::Bar(_));
        let stacked_series = stacked.contains_key(&slot);
        for index in 0..data.len() {
            let Some((x, base, top)) = value_at(slot, index) else {
                continue;
            };
            if !in_window(x) {
                continue;
            }
            let extent = &mut extents[axis];
            extent.0 = extent.0.min(top);
            extent.1 = extent.1.max(top);
            if bars || stacked_series {
                extent.0 = extent.0.min(base);
                extent.1 = extent.1.max(base);
            }
        }
    }

    let value_scales: Vec<Scale> = value_axes
        .iter()
        .zip(&extents)
        .map(|(axis, (lo, hi))| value_scale(axis, *lo, *hi))
        .collect();
    let base_scale = if is_category {
        Scale::Band {
            first: window_first,
            count: window_count.max(1),
            gap: wants_gap,
        }
    } else {
        let (lo, hi) = if window_lo.is_finite() {
            (window_lo, window_hi)
        } else {
            (0.0, 1.0)
        };
        match base_axis.kind {
            AxisKind::Time => {
                let (min, max) = (lo.floor() as i64, hi.ceil() as i64);
                // About one tick per label's width of the frame.
                let length = if base_x {
                    frame[2] - frame[0]
                } else {
                    frame[3] - frame[1]
                };
                let widest =
                    input.measure.measure("00-00 00:00", theme.font_size)[0] + LABEL_GAP * 3.0;
                let count = ((length / widest).floor() as usize).clamp(2, 12);
                Scale::Time {
                    min,
                    max,
                    ticks: scale::time_ticks(min, max, count, 0),
                }
            }
            _ => {
                // A zoomed continuous axis keeps the window's exact ends.
                let zoomed = zoom.is_some() && (start_pct > 0.0 || end_pct < 100.0);
                let pinned = |bound| match bound {
                    AxisBound::Value(v) => Some(v),
                    _ => None,
                };
                let mut ticks = scale::nice_linear(
                    lo,
                    hi,
                    base_axis.split_number,
                    pinned(base_axis.min).or(zoomed.then_some(lo)),
                    pinned(base_axis.max).or(zoomed.then_some(hi)),
                );
                if !base_axis.scale && base_axis.kind == AxisKind::Value && !zoomed && lo > 0.0 {
                    ticks = scale::nice_linear(0.0, hi, base_axis.split_number, None, None);
                }
                Scale::Linear(ticks)
            }
        }
    };

    // Measure labels to size the margins.
    let size = theme.font_size;
    let measure = input.measure;
    let measure_ticks = |axis: &Axis, scale: &Scale| -> Vec<[f32; 2]> {
        if !axis.show || !axis.axis_label.show {
            return Vec::new();
        }
        let probe = AxisMap::new(scale.clone(), 0.0, 1.0, false);
        axis_ticks(axis, &probe, &axis.data)
            .iter()
            .map(|tick| measure.measure(&tick.label, size))
            .collect()
    };
    let base_sizes = measure_ticks(&base_axis, &base_scale);
    let value_sizes: Vec<Vec<[f32; 2]>> = value_axes
        .iter()
        .zip(&value_scales)
        .map(|(axis, scale)| measure_ticks(axis, scale))
        .collect();
    let line_height = measure.measure("0", size)[1];

    let [fx0, fy0, fx1, fy1] = frame;
    let mut margin = [0.0_f32; 4]; // left, top, right, bottom

    // In px of the margin side an axis sits on.
    let mut add_margin = |position: AxisPosition, across: f32| {
        let slot = match position {
            AxisPosition::Left => 0,
            AxisPosition::Top => 1,
            AxisPosition::Right => 2,
            AxisPosition::Bottom => 3,
        };
        margin[slot] = margin[slot].max(across);
    };
    let base_position = if base_x {
        base_axis.position.unwrap_or(AxisPosition::Bottom)
    } else {
        base_axis.position.unwrap_or(AxisPosition::Left)
    };
    let base_across = if base_x {
        line_height
    } else {
        base_sizes.iter().map(|s| s[0]).fold(0.0, f32::max)
    };
    if base_axis.show && option.grid.contain_label {
        add_margin(base_position, base_across + LABEL_GAP + TICK);
    }
    let value_positions: Vec<AxisPosition> = value_axes
        .iter()
        .enumerate()
        .map(|(index, axis)| {
            axis.position.unwrap_or(match (base_x, index) {
                (false, _) => AxisPosition::Bottom,
                (true, 0) => AxisPosition::Left,
                (true, _) => AxisPosition::Right,
            })
        })
        .collect();
    for ((axis, sizes), position) in value_axes.iter().zip(&value_sizes).zip(&value_positions) {
        if !axis.show || !option.grid.contain_label {
            continue;
        }
        let across = if base_x {
            sizes.iter().map(|s| s[0]).fold(0.0, f32::max)
        } else {
            line_height
        };
        add_margin(*position, across + LABEL_GAP);
        if axis.name.is_some() {
            let slot = if base_x { AxisPosition::Top } else { *position };
            add_margin(slot, line_height + LABEL_GAP);
        }
    }
    // Half a label of room at the ends of the axis that runs across the
    // bottom, so its first and last labels are not cut; half a line at the
    // top for the highest label of the axis that runs up.
    let horizontal_sizes = if base_x {
        (!(is_category && wants_gap)).then_some(&base_sizes)
    } else {
        value_sizes.first()
    };
    if let Some(sizes) = horizontal_sizes {
        margin[0] = margin[0].max(sizes.first().map_or(0.0, |s| s[0] * 0.5));
        margin[2] = margin[2].max(sizes.last().map_or(0.0, |s| s[0] * 0.5));
    }
    margin[1] = margin[1].max(line_height * 0.5);
    let slider_space = if slider_zoom.is_some() {
        SLIDER_HEIGHT + LABEL_GAP * 2.0
    } else {
        0.0
    };
    margin[3] += slider_space;
    let resolve = |length: Option<Length>, reference: f32, fallback: f32| {
        length.map_or(fallback, |length| length.resolve(reference))
    };
    let width = fx1 - fx0;
    let height = fy1 - fy0;
    let (x0, y0) = (
        fx0 + resolve(option.grid.left, width, margin[0]),
        fy0 + resolve(option.grid.top, height, margin[1]),
    );
    let plot = [
        x0,
        y0,
        (fx1 - resolve(option.grid.right, width, margin[2])).max(x0 + 1.0),
        (fy1 - resolve(option.grid.bottom, height, margin[3])).max(y0 + 1.0),
    ];

    let base_map = if base_x {
        AxisMap::new(base_scale.clone(), plot[0], plot[2], base_axis.inverse)
    } else {
        AxisMap::new(base_scale.clone(), plot[3], plot[1], base_axis.inverse)
    };
    let value_maps: Vec<AxisMap> = value_axes
        .iter()
        .zip(&value_scales)
        .map(|(axis, scale)| {
            if base_x {
                AxisMap::new(scale.clone(), plot[3], plot[1], axis.inverse)
            } else {
                AxisMap::new(scale.clone(), plot[0], plot[2], axis.inverse)
            }
        })
        .collect();

    // Guides are axis-aligned hairlines: one-pixel rects, so a whole axis of
    // them is one draw.
    let guide = |builder: &mut MarkBuilder, a: [f32; 2], b: [f32; 2], color: [f32; 4]| {
        let style = builder.style(fill_style(color, color));
        let half = GUIDE_WIDTH * 0.5;
        let rect = if (a[0] - b[0]).abs() < f32::EPSILON {
            [a[0] - half, a[1].min(b[1]), a[0] + half, a[1].max(b[1])]
        } else {
            [a[0].min(b[0]), a[1] - half, a[0].max(b[0]), a[1] + half]
        };
        let mut shape = GpuShape::new(ShapeKind::Rect, 0, style, u32::MAX, 0);
        shape.to = rect;
        shape.from = rect;
        builder.shape(
            shape,
            DrawKey {
                series: u32::MAX,
                part: DrawPart::Guide,
                run: 0,
            },
            0,
        );
    };

    // Split lines of the first value axis, across the plot.
    if let (Some(axis), Some(map)) = (value_axes.first(), value_maps.first())
        && axis.show
        && axis.split_line.unwrap_or(true)
    {
        for tick in axis_ticks(axis, map, &axis.data) {
            let at = crisp(map.px(tick.value));
            if base_x {
                guide(builder, [plot[0], at], [plot[2], at], theme.grid);
            } else {
                guide(builder, [at, plot[1]], [at, plot[3]], theme.grid);
            }
        }
    }
    if base_axis.split_line.unwrap_or(false) {
        for tick in axis_ticks(&base_axis, &base_map, &base_axis.data) {
            let at = crisp(base_map.px(tick.value));
            if base_x {
                guide(builder, [at, plot[1]], [at, plot[3]], theme.grid);
            } else {
                guide(builder, [plot[0], at], [plot[2], at], theme.grid);
            }
        }
    }

    // Bar backgrounds and the axis-pointer band go under the series; the
    // series follow; the base axis line goes on top of the bars' feet.
    let bars = layout_bars(
        input,
        &placed,
        &stacked,
        &base_map,
        &value_maps,
        base_x,
        plot,
        builder,
        texts,
        hit,
    );
    let mut axis_series: Vec<AxisSeries> = bars;
    for (slot, placed_series) in placed.iter().enumerate() {
        let value_map =
            &value_maps[value_axis_index(placed_series.series, base_x).min(value_maps.len() - 1)];
        match placed_series.series {
            Series::Line(line) => axis_series.push(layout_line(
                input,
                line,
                placed_series,
                slot,
                &value_at,
                &stacked,
                &base_map,
                value_map,
                base_x,
                plot,
                (window_lo, window_hi),
                builder,
            )),
            Series::Scatter(scatter) => layout_scatter(
                input,
                scatter,
                placed_series,
                &base_map,
                value_map,
                base_x,
                plot,
                builder,
                hit,
            ),
            _ => {}
        }
    }

    // Axis lines, ticks and labels.
    let label_color = theme.muted;
    if base_axis.show {
        let at = match base_position {
            AxisPosition::Bottom => crisp(plot[3]),
            AxisPosition::Top => crisp(plot[1]),
            AxisPosition::Left => crisp(plot[0]),
            AxisPosition::Right => crisp(plot[2]),
        };
        // The base axis sits on the value axis' zero when it shows one.
        let at = value_maps
            .first()
            .filter(|map| matches!(map.scale, Scale::Linear(ref t) if t.min < 0.0 && t.max > 0.0))
            .map_or(at, |map| crisp(map.px(0.0)));
        if base_axis.axis_line {
            if base_x {
                guide(builder, [plot[0], at], [plot[2], at], theme.axis);
            } else {
                guide(builder, [at, plot[1]], [at, plot[3]], theme.axis);
            }
        }
        let ticks = axis_ticks(&base_axis, &base_map, &base_axis.data);
        let band = base_map.band();
        // Category labels: every n-th one so neighbours do not touch.
        let interval = if is_category {
            base_axis.axis_label.interval.unwrap_or_else(|| {
                let widest = if base_x {
                    base_sizes.iter().map(|s| s[0]).fold(0.0, f32::max)
                } else {
                    line_height
                };
                if band <= 0.0 {
                    1
                } else {
                    ((widest + LABEL_GAP * 2.0) / band).ceil().max(1.0) as usize
                }
            })
        } else {
            1
        };
        let label_edge = match base_position {
            AxisPosition::Bottom => plot[3] + TICK + LABEL_GAP * 0.5,
            AxisPosition::Top => plot[1] - TICK - LABEL_GAP * 0.5,
            AxisPosition::Left => plot[0] - TICK - LABEL_GAP * 0.5,
            AxisPosition::Right => plot[2] + TICK + LABEL_GAP * 0.5,
        };
        let mut last_rect: Option<[f32; 4]> = None;
        for tick in &ticks {
            let center = base_map.px(tick.value);
            let index = tick.value as usize;
            let labelled =
                !is_category || (index - window_first.min(index)).is_multiple_of(interval);
            // Category ticks follow the labels shown.
            if base_axis.axis_tick && labelled {
                // Category ticks sit between bands when there is a gap.
                let tick_at = if is_category && wants_gap {
                    crisp(center - band * 0.5)
                } else {
                    crisp(center)
                };
                if base_x {
                    guide(builder, [tick_at, at], [tick_at, at + TICK], theme.axis);
                } else {
                    guide(builder, [at - TICK, tick_at], [at, tick_at], theme.axis);
                }
            }
            if !labelled {
                continue;
            }
            let label = &tick.label;
            if !base_axis.axis_label.show {
                continue;
            }
            let rect = if base_x {
                text_rect(
                    measure,
                    label,
                    size,
                    [center, label_edge],
                    TextAlign::Center,
                    if base_position == AxisPosition::Top {
                        TextAlign::End
                    } else {
                        TextAlign::Start
                    },
                )
            } else {
                text_rect(
                    measure,
                    label,
                    size,
                    [label_edge, center],
                    if base_position == AxisPosition::Right {
                        TextAlign::Start
                    } else {
                        TextAlign::End
                    },
                    TextAlign::Center,
                )
            };
            if !is_category
                && base_axis.axis_label.hide_overlap
                && last_rect.is_some_and(|last| rects_overlap(last, rect, LABEL_GAP))
            {
                continue;
            }
            last_rect = Some(rect);
            texts.push(ChartText {
                rect,
                text: label.clone(),
                color: label_color,
                size,
                weight: None,
            });
        }
    }
    for ((axis, map), position) in value_axes.iter().zip(&value_maps).zip(&value_positions) {
        if !axis.show {
            continue;
        }
        let at = match position {
            AxisPosition::Left => plot[0],
            AxisPosition::Right => plot[2],
            AxisPosition::Top => plot[1],
            AxisPosition::Bottom => plot[3],
        };
        if axis.axis_line && !axis.split_line.unwrap_or(true) {
            let line = crisp(at);
            if base_x {
                guide(builder, [line, plot[1]], [line, plot[3]], theme.axis);
            } else {
                guide(builder, [plot[0], line], [plot[2], line], theme.axis);
            }
        }
        if !axis.axis_label.show {
            continue;
        }
        let mut last_rect: Option<[f32; 4]> = None;
        for tick in axis_ticks(axis, map, &axis.data) {
            let label = tick.label;
            let center = map.px(tick.value);
            let (anchor, align, vertical) = match position {
                AxisPosition::Left => ([at - LABEL_GAP, center], TextAlign::End, TextAlign::Center),
                AxisPosition::Right => (
                    [at + LABEL_GAP, center],
                    TextAlign::Start,
                    TextAlign::Center,
                ),
                AxisPosition::Bottom => (
                    [center, at + LABEL_GAP],
                    TextAlign::Center,
                    TextAlign::Start,
                ),
                AxisPosition::Top => ([center, at - LABEL_GAP], TextAlign::Center, TextAlign::End),
            };
            let rect = text_rect(measure, &label, size, anchor, align, vertical);
            if axis.axis_label.hide_overlap
                && last_rect.is_some_and(|last| rects_overlap(last, rect, 2.0))
            {
                continue;
            }
            last_rect = Some(rect);
            texts.push(ChartText {
                rect,
                text: label,
                color: label_color,
                size,
                weight: None,
            });
        }
        if let Some(name) = &axis.name {
            let rect = if base_x {
                let anchor = if *position == AxisPosition::Right {
                    [plot[2], plot[1] - LABEL_GAP]
                } else {
                    [plot[0], plot[1] - LABEL_GAP]
                };
                text_rect(
                    measure,
                    name,
                    size,
                    anchor,
                    if *position == AxisPosition::Right {
                        TextAlign::End
                    } else {
                        TextAlign::Start
                    },
                    TextAlign::End,
                )
            } else {
                text_rect(
                    measure,
                    name,
                    size,
                    [plot[2], at + LABEL_GAP + line_height],
                    TextAlign::End,
                    TextAlign::Start,
                )
            };
            texts.push(ChartText {
                rect,
                text: name.clone(),
                color: label_color,
                size,
                weight: None,
            });
        }
    }

    let slider = slider_zoom.map(|(zoom_index, (start, end))| {
        let track = [plot[0], fy1 - SLIDER_HEIGHT, plot[2], fy1];
        let span = track[2] - track[0];
        let window = [
            track[0] + span * (start / 100.0) as f32,
            track[1],
            track[0] + span * (end / 100.0) as f32,
            track[3],
        ];
        layout_slider(input, &placed, track, window, builder);
        SliderLayout {
            zoom_index,
            track,
            window,
        }
    });

    let titles: Arc<[Arc<str>]> = if is_category {
        (0..category_count)
            .map(|index| {
                base_axis
                    .data
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| (index + 1).to_string().into())
            })
            .collect()
    } else {
        Arc::from([])
    };
    let base_hit = if is_category {
        BaseAxisHit::Category {
            first: window_first,
            count: window_count.max(1),
            band: base_map.band(),
            gap: wants_gap,
        }
    } else {
        BaseAxisHit::Continuous
    };
    axis_series.sort_by_key(|series| series.series);
    hit.axis = Some(AxisHit {
        along,
        plot,
        base: base_hit,
        base_start: base_map.start,
        base_end: base_map.end,
        titles,
        time: base_axis.kind == AxisKind::Time,
        time_formatter: base_axis.axis_label.time_formatter.clone(),
        series: axis_series,
        has_bars: placed.iter().any(|p| matches!(p.series, Series::Bar(_))),
    });
    CartesianOut { plot, slider }
}

fn value_scale(axis: &Axis, lo: f64, hi: f64) -> Scale {
    let (lo, hi) = if lo.is_finite() { (lo, hi) } else { (0.0, 1.0) };
    let bound = |bound: AxisBound, data: f64| match bound {
        AxisBound::Auto => None,
        AxisBound::Value(value) => Some(value),
        AxisBound::DataMin | AxisBound::DataMax => Some(data),
    };
    match axis.kind {
        AxisKind::Log => Scale::Log {
            ticks: scale::nice_log(lo, hi, axis.log_base, axis.split_number),
            base: axis.log_base,
        },
        _ => {
            let fixed_min = bound(axis.min, lo);
            let fixed_max = bound(axis.max, hi);
            // Zero belongs on a value axis unless it is asked to fit the data.
            let (lo, hi) = if axis.scale {
                (lo, hi)
            } else {
                (lo.min(0.0), hi.max(0.0))
            };
            Scale::Linear(scale::nice_linear(
                lo,
                hi,
                axis.split_number,
                fixed_min,
                fixed_max,
            ))
        }
    }
}

fn dash_of(line_type: LineType, width: f32) -> [f32; 2] {
    match line_type {
        LineType::Solid => [0.0, 0.0],
        LineType::Dashed => [width * 4.0, width * 3.0],
        LineType::Dotted => [width.max(1.0), width * 2.0],
    }
}

pub(crate) fn symbol_shape(symbol: Symbol) -> Option<SymbolShape> {
    Some(match symbol {
        Symbol::EmptyCircle | Symbol::Circle => SymbolShape::Circle,
        Symbol::Rect => SymbolShape::Rect,
        Symbol::RoundRect => SymbolShape::RoundRect,
        Symbol::Triangle => SymbolShape::Triangle,
        Symbol::Diamond => SymbolShape::Diamond,
        Symbol::Pin => SymbolShape::Pin,
        Symbol::None => return None,
    })
}

/// A symbol shape at `center`, entering from `from`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn symbol_at(
    shape: SymbolShape,
    center: [f32; 2],
    from: [f32; 2],
    size: f32,
    from_size: f32,
    hover_size: f32,
    border: f32,
    style: u32,
    series: u32,
    index: u32,
    flags: u32,
) -> GpuShape {
    let mut gpu = GpuShape::new(ShapeKind::Symbol, flags, style, series, index);
    gpu.to = [center[0], center[1], size, 0.0];
    gpu.from = [from[0], from[1], from_size, 0.0];
    gpu.extra = [border, shape as u32 as f32, hover_size, 0.0];
    gpu
}

/// A data point of a line run: `(data index, point, base)`.
type RunPoint = (usize, [f32; 2], [f32; 2]);

#[allow(clippy::too_many_arguments)]
fn layout_line(
    input: &LayoutInput<'_>,
    line: &LineSeries,
    placed: &Placed<'_>,
    slot: usize,
    value_at: &dyn Fn(usize, usize) -> Option<(f64, f64, f64)>,
    stacked: &HashMap<usize, Vec<(f64, f64)>>,
    base_map: &AxisMap,
    value_map: &AxisMap,
    base_x: bool,
    plot: [f32; 4],
    window: (f64, f64),
    builder: &mut MarkBuilder,
) -> AxisSeries {
    let theme = input.theme;
    let series = placed.index as u32;
    let at = |base: f32, value: f32| if base_x { [base, value] } else { [value, base] };
    let along = if base_x { Along::X } else { Along::Y };
    let baseline = value_map.baseline();
    let data_len = line.data.len();
    // One point before and after the window so the line runs to the
    // plot's edge (where it is clipped).
    let mut runs: Vec<Vec<RunPoint>> = vec![Vec::new()];
    let mut axis_points = Vec::new();
    let in_window = |x: f64| x >= window.0 && x <= window.1;
    for index in 0..data_len {
        let Some((x, base, top)) = value_at(slot, index) else {
            if !line.connect_nulls && !runs.last().is_some_and(Vec::is_empty) {
                runs.push(Vec::new());
            }
            continue;
        };
        let near_window = in_window(x)
            || line
                .data
                .get(index + 1)
                .is_some_and(|(next, _)| in_window(next) && x < window.0)
            || index
                .checked_sub(1)
                .and_then(|prev| line.data.get(prev))
                .is_some_and(|(prev, _)| in_window(prev) && x > window.1);
        if !near_window {
            continue;
        }
        let base_px = base_map.px(x);
        let point = at(base_px, value_map.px(top));
        let base_point = if stacked.contains_key(&slot) {
            at(base_px, value_map.px(base))
        } else {
            at(base_px, baseline)
        };
        if in_window(x) {
            axis_points.push(AxisPoint {
                base: base_px,
                cross: if base_x { point[1] } else { point[0] },
                index: index as u32,
                key: x,
                value: line.data.get(index).map_or(top, |(_, y)| y),
            });
        }
        runs.last_mut().unwrap().push((index, point, base_point));
    }
    runs.retain(|run| !run.is_empty());

    let width = line.width;
    let stroke = builder.style(line_style(
        placed.color,
        width,
        dash_of(line.line_type, width),
    ));
    let area = line.area.map(|area| {
        let color = area.color.map_or(placed.color, |c| theme.resolve(c));
        let top = with_alpha(color, area.opacity);
        let end = if area.gradient {
            with_alpha(color, 0.0)
        } else {
            top
        };
        let mut style = fill_style(top, end);
        // The gradient runs over the plot, from the far edge to the base.
        style.params = if base_x {
            [plot[1], baseline, 0.0, 0.0]
        } else {
            [plot[2], baseline, 1.0, 0.0]
        };
        builder.style(style)
    });
    let plot_length = if base_x {
        plot[2] - plot[0]
    } else {
        plot[3] - plot[1]
    };
    let visible: usize = runs.iter().map(Vec::len).sum();
    let show_symbols = line.symbol != Symbol::None
        && line.show_symbol.unwrap_or(
            (visible as f32) * (line.symbol_size + 6.0) <= plot_length
                && visible <= input.option.animation.threshold,
        );
    let mut flags = draw_flags::CLIP_TO_PLOT | draw_flags::REVEAL | draw_flags::EMPHASIS;
    if stacked.contains_key(&slot) {
        flags |= draw_flags::SOFT_BASE;
    }
    for (run_index, run) in runs.iter().enumerate() {
        let mut points: Vec<[f32; 2]> = run.iter().map(|(_, p, _)| *p).collect();
        let mut bases: Vec<[f32; 2]> = run.iter().map(|(_, _, b)| *b).collect();
        // Long series come down to what the plot can show first.
        if line.sampling != Sampling::None && points.len() as f32 > plot_length * 1.5 {
            let keep = match line.sampling {
                Sampling::Lttb => {
                    let data: Vec<[f64; 2]> = points
                        .iter()
                        .map(|p| {
                            let (b, v) = if base_x { (p[0], p[1]) } else { (p[1], p[0]) };
                            [b as f64, v as f64]
                        })
                        .collect();
                    crate::sample::lttb(&data, plot_length.max(2.0) as usize)
                }
                _ => {
                    let data: Vec<[f32; 2]> = points
                        .iter()
                        .map(|p| if base_x { *p } else { [p[1], p[0]] })
                        .collect();
                    crate::sample::min_max(&data, 1.0)
                }
            };
            points = keep.iter().map(|i| points[*i]).collect();
            bases = keep.iter().map(|i| bases[*i]).collect();
        }
        let (curve, curve_bases) = if let Some(step) = line.step {
            let (curve, _) = smooth::step(&points, along, step);
            let (base_curve, _) = smooth::step(&bases, along, step);
            (curve, base_curve)
        } else if line.smooth && points.len() >= 3 {
            let (curve, _) = smooth::smooth(&points, along, SMOOTH_SEGMENT);
            let base_curve = if stacked.contains_key(&slot) {
                let (smoothed, _) = smooth::smooth(&bases, along, SMOOTH_SEGMENT);
                curve
                    .iter()
                    .map(|p| sample_at(&smoothed, along, if base_x { p[0] } else { p[1] }))
                    .collect()
            } else {
                curve
                    .iter()
                    .map(|p| {
                        if base_x {
                            [p[0], baseline]
                        } else {
                            [baseline, p[1]]
                        }
                    })
                    .collect()
            };
            (curve, base_curve)
        } else {
            (points.clone(), bases.clone())
        };
        if curve.len() >= 2 {
            builder.polyline(
                Polyline {
                    points: &curve,
                    bases: Some(&curve_bases),
                    // Entry is the reveal; updates rise from the base.
                    from: Some(&curve_bases),
                    from_bases: Some(&curve_bases),
                },
                (width > 0.0).then_some(stroke),
                area,
                width,
                series,
                run_index as u32,
                flags,
                along,
            );
        }
        // Symbols on the data points. Hidden ones still grow under the
        // pointer, as ECharts shows the hovered point.
        if let Some(shape) = symbol_shape(line.symbol)
            && visible <= input.option.animation.threshold
        {
            let fill = if line.symbol == Symbol::EmptyCircle {
                theme.surface
            } else {
                placed.color
            };
            let border = if line.symbol == Symbol::EmptyCircle {
                (width * 0.75).max(1.5)
            } else {
                0.0
            };
            let style = builder.style(symbol_style(fill, placed.color, border));
            let size = if show_symbols {
                line.symbol_size + border
            } else {
                0.0
            };
            let hover = (line.symbol_size + border).max(6.0) * 1.6;
            for (index, point, base) in run {
                let symbol = symbol_at(
                    shape,
                    *point,
                    *base,
                    size,
                    0.0,
                    hover,
                    border,
                    style,
                    series,
                    *index as u32,
                    SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW | SHAPE_REVEAL,
                );
                builder.shape(
                    symbol,
                    DrawKey {
                        series,
                        part: DrawPart::Shapes,
                        run: run_index as u32,
                    },
                    0,
                );
            }
            builder.close_shapes();
        }
    }
    AxisSeries {
        series,
        name: line.name.clone(),
        color: placed.color,
        points: axis_points,
    }
}

/// The point of `polyline` (advancing along `along`) at `coord` along it,
/// clamped to its ends.
pub(crate) fn sample_at(polyline: &[[f32; 2]], along: Along, coord: f32) -> [f32; 2] {
    let key = |p: &[f32; 2]| if along == Along::X { p[0] } else { p[1] };
    if polyline.is_empty() {
        return [0.0, 0.0];
    }
    let ascending = polyline.len() < 2 || key(&polyline[0]) <= key(&polyline[polyline.len() - 1]);
    let position = polyline.partition_point(|p| {
        if ascending {
            key(p) < coord
        } else {
            key(p) > coord
        }
    });
    if position == 0 {
        return polyline[0];
    }
    if position >= polyline.len() {
        return polyline[polyline.len() - 1];
    }
    let (a, b) = (polyline[position - 1], polyline[position]);
    let span = key(&b) - key(&a);
    let t = if span.abs() < f32::EPSILON {
        0.0
    } else {
        (coord - key(&a)) / span
    };
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Bars of every bar series, grouped per category. Returns their axis-hit
/// series.
#[allow(clippy::too_many_arguments)]
fn layout_bars(
    input: &LayoutInput<'_>,
    placed: &[Placed<'_>],
    stacked: &HashMap<usize, Vec<(f64, f64)>>,
    base_map: &AxisMap,
    value_maps: &[AxisMap],
    base_x: bool,
    plot: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) -> Vec<AxisSeries> {
    let theme = input.theme;
    let bar_slots: Vec<(usize, &BarSeries)> = placed
        .iter()
        .enumerate()
        .filter_map(|(slot, p)| match p.series {
            Series::Bar(bar) => Some((slot, bar)),
            _ => None,
        })
        .collect();
    if bar_slots.is_empty() {
        return Vec::new();
    }
    // Series that stack share a slot within the band.
    let mut groups: Vec<Option<Arc<str>>> = Vec::new();
    let mut group_of = Vec::new();
    for (_, bar) in &bar_slots {
        let key = bar.stack.clone();
        let group = match &key {
            Some(_) => groups.iter().position(|g| *g == key),
            None => None,
        }
        .unwrap_or_else(|| {
            groups.push(key);
            groups.len() - 1
        });
        group_of.push(group);
    }
    let groups = groups.len().max(1) as f32;
    let first = bar_slots[0].1;
    let band = if base_map.band() > 0.0 {
        base_map.band()
    } else {
        // A bar on a continuous axis gets the narrowest spacing between
        // its points.
        let mut xs: Vec<f32> = bar_slots
            .iter()
            .flat_map(|(_, bar)| (0..bar.data.len()).filter_map(|i| bar.data.get(i)))
            .map(|(x, _)| base_map.px(x))
            .collect();
        xs.sort_by(f32::total_cmp);
        xs.windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| *d > 0.5)
            .fold(f32::INFINITY, f32::min)
            .min((plot[2] - plot[0]).abs() * 0.2)
            .max(2.0)
    };
    let usable = band * (1.0 - first.bar_category_gap.clamp(0.0, 0.95));
    let gap = first.bar_gap.clamp(-1.0, 1.0);
    let mut bar_width = first
        .bar_width
        .map(|w| w.resolve(band))
        .unwrap_or(usable / (groups + (groups - 1.0) * gap));
    if let Some(max) = first.bar_max_width {
        bar_width = bar_width.min(max);
    }
    let bar_width = bar_width.max(1.0);
    let total = bar_width * groups + bar_width * gap * (groups - 1.0);
    let mut out = Vec::new();
    for ((slot, bar), group) in bar_slots.iter().zip(&group_of) {
        let p = &placed[*slot];
        let series = p.index as u32;
        let value_map = &value_maps[value_axis_index(p.series, base_x).min(value_maps.len() - 1)];
        let baseline = value_map.baseline();
        let offset = -total * 0.5 + *group as f32 * bar_width * (1.0 + gap);
        let color_style = |builder: &mut MarkBuilder, index: usize| {
            let color = if bar.item_colors.is_empty() {
                p.color
            } else {
                theme.resolve(bar.item_colors[index % bar.item_colors.len()])
            };
            (
                builder.style(super::builder::fill_style(color, color)),
                color,
            )
        };
        let background = bar
            .show_background
            .then(|| builder.style(fill_style(theme.band, theme.band)));
        let (value_lo, value_hi) = value_map.lo_hi();
        let mut points = Vec::new();
        for index in 0..bar.data.len() {
            let Some((x, y)) = bar.data.get(index) else {
                continue;
            };
            if !y.is_finite() {
                continue;
            }
            let (base_value, top_value) = stacked
                .get(slot)
                .and_then(|column| column.get(index).copied())
                .unwrap_or((0.0, y));
            let center = base_map.px(x);
            let (lo_edge, hi_edge) = (center + offset, center + offset + bar_width);
            let (plot_lo, plot_hi) = if base_x {
                (plot[0], plot[2])
            } else {
                (plot[1], plot[3])
            };
            if hi_edge < plot_lo || lo_edge > plot_hi {
                continue;
            }
            let to_px = value_map.px(top_value);
            // A bar stacked on another reaches half a pixel into it, so the
            // two antialiased edges do not leave a seam of background.
            let from_px = if stacked.contains_key(slot) && base_value != 0.0 {
                let base_px = value_map.px(base_value);
                base_px + (base_px - to_px).signum() * 0.5
            } else {
                baseline
            };
            let rect_of = |a: f32, b: f32| {
                if base_x {
                    [lo_edge, a.min(b), hi_edge, a.max(b)]
                } else {
                    [a.min(b), lo_edge, a.max(b), hi_edge]
                }
            };
            if let Some(style) = background {
                let mut shape = GpuShape::new(ShapeKind::Rect, 0, style, u32::MAX, index as u32);
                shape.to = rect_of(value_lo, value_hi);
                shape.from = shape.to;
                shape.extra = radii(bar.border_radius, true, base_x);
                builder.shape(
                    shape,
                    DrawKey {
                        series,
                        part: DrawPart::Guide,
                        run: 0,
                    },
                    0,
                );
            }
            // Radii go on the end away from the base.
            let grows_forward = if base_x {
                to_px <= from_px
            } else {
                to_px >= from_px
            };
            let (style, color) = color_style(builder, index);
            let mut shape = GpuShape::new(
                ShapeKind::Rect,
                SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW,
                style,
                series,
                index as u32,
            );
            shape.to = rect_of(from_px, to_px);
            shape.from = rect_of(from_px, from_px);
            shape.extra = radii(bar.border_radius, grows_forward, base_x);
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
                region: ItemRegion::Rect(shape.to),
                series,
                index: index as u32,
                title: None,
                row: None,
                value: y,
                color,
            });
            points.push(AxisPoint {
                base: center,
                cross: to_px,
                index: index as u32,
                key: x,
                value: y,
            });
            if bar.label.show {
                let text: Arc<str> = match &bar.label.formatter {
                    Some(formatter) => formatter.format(y),
                    None => {
                        scale::format_number(y, if y.fract() == 0.0 { 1.0 } else { 0.01 }).into()
                    }
                };
                let anchor = if base_x {
                    [center + offset + bar_width * 0.5, to_px - LABEL_GAP * 0.5]
                } else {
                    [to_px + LABEL_GAP * 0.5, center + offset + bar_width * 0.5]
                };
                let rect = if base_x {
                    text_rect(
                        input.measure,
                        &text,
                        theme.font_size,
                        anchor,
                        TextAlign::Center,
                        if grows_forward {
                            TextAlign::End
                        } else {
                            TextAlign::Start
                        },
                    )
                } else {
                    text_rect(
                        input.measure,
                        &text,
                        theme.font_size,
                        anchor,
                        if grows_forward {
                            TextAlign::Start
                        } else {
                            TextAlign::End
                        },
                        TextAlign::Center,
                    )
                };
                texts.push(ChartText {
                    rect,
                    text,
                    color: theme.muted,
                    size: theme.font_size,
                    weight: None,
                });
            }
        }
        builder.close_shapes();
        out.push(AxisSeries {
            series,
            name: bar.name.clone(),
            color: p.color,
            points,
        });
    }
    out
}

/// `[tl, tr, br, bl]` of a bar whose rounded end faces away from its base.
fn radii(radius: [f32; 4], forward: bool, base_x: bool) -> [f32; 4] {
    let [a, b, c, d] = radius;
    match (base_x, forward) {
        // Upward bar: rounded top.
        (true, true) => [a, b, c, d],
        // Downward bar: mirrored vertically.
        (true, false) => [d, c, b, a],
        // Rightward bar: the "top" is the right end.
        (false, true) => [d, a, b, c],
        (false, false) => [a, d, c, b],
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_scatter(
    input: &LayoutInput<'_>,
    scatter: &ScatterSeries,
    placed: &Placed<'_>,
    base_map: &AxisMap,
    value_map: &AxisMap,
    base_x: bool,
    plot: [f32; 4],
    builder: &mut MarkBuilder,
    hit: &mut HitModel,
) {
    let Some(shape) = symbol_shape(scatter.symbol) else {
        return;
    };
    let series = placed.index as u32;
    let fill = with_alpha(placed.color, scatter.opacity);
    let style = builder.style(symbol_style(fill, placed.color, 0.0));
    let animate = scatter.data.len() <= input.option.animation.threshold;
    let mut grid = Vec::with_capacity(scatter.data.len());
    for index in 0..scatter.data.len() {
        let Some((x, y)) = scatter.data.get(index) else {
            continue;
        };
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        let center = if base_x {
            [base_map.px(x), value_map.px(y)]
        } else {
            [value_map.px(y), base_map.px(x)]
        };
        if center[0] < plot[0] - 50.0
            || center[0] > plot[2] + 50.0
            || center[1] < plot[1] - 50.0
            || center[1] > plot[3] + 50.0
        {
            continue;
        }
        let size = scatter
            .sizes
            .as_ref()
            .and_then(|sizes| sizes.get(index).copied())
            .unwrap_or(scatter.symbol_size)
            .max(0.0);
        let symbol = symbol_at(
            shape,
            center,
            center,
            size,
            if animate { 0.0 } else { size },
            size * 1.4,
            0.0,
            style,
            series,
            index as u32,
            SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW,
        );
        builder.shape(
            symbol,
            DrawKey {
                series,
                part: DrawPart::Shapes,
                run: 0,
            },
            0,
        );
        grid.push((center, size * 0.5, index as u32, if base_x { y } else { x }));
    }
    builder.close_shapes();
    for (center, radius, index, value) in grid {
        hit.items.push(ItemHit {
            region: ItemRegion::Circle {
                center,
                radius: radius.max(3.0) + 2.0,
            },
            series,
            index,
            title: None,
            row: None,
            value,
            color: placed.color,
        });
    }
}

/// The slider: its track, a faint preview of the first series, the window
/// and its two handles.
fn layout_slider(
    input: &LayoutInput<'_>,
    placed: &[Placed<'_>],
    track: [f32; 4],
    window: [f32; 4],
    builder: &mut MarkBuilder,
) {
    let theme = input.theme;
    let key = DrawKey {
        series: u32::MAX,
        part: DrawPart::Guide,
        run: 0,
    };
    let track_style = builder.style(fill_style(theme.band, theme.band));
    let mut shape = GpuShape::new(ShapeKind::Rect, 0, track_style, u32::MAX, 0);
    shape.to = track;
    shape.from = track;
    shape.extra = [4.0; 4];
    builder.shape(shape, key, 0);
    builder.close_shapes();
    // The first series over its whole extent.
    if let Some(data) = placed.first().and_then(|p| series_data(p.series)) {
        let count = data.len();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut xlo, mut xhi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..count {
            if let Some((x, y)) = data.get(i)
                && y.is_finite()
                && x.is_finite()
            {
                lo = lo.min(y);
                hi = hi.max(y);
                xlo = xlo.min(x);
                xhi = xhi.max(x);
            }
        }
        if lo.is_finite() && count >= 2 {
            let span = (hi - lo).max(f64::EPSILON);
            let xspan = (xhi - xlo).max(f64::EPSILON);
            let inset = 3.0;
            let mut points: Vec<[f32; 2]> = (0..count)
                .filter_map(|i| data.get(i))
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .map(|(x, y)| {
                    [
                        track[0] + (track[2] - track[0]) * ((x - xlo) / xspan) as f32,
                        track[3]
                            - inset
                            - (track[3] - track[1] - inset * 2.0) * ((y - lo) / span) as f32,
                    ]
                })
                .collect();
            if points.len() as f32 > (track[2] - track[0]) * 1.5 {
                let keep = crate::sample::min_max(&points, 1.0);
                points = keep.iter().map(|i| points[*i]).collect();
            }
            let bases: Vec<[f32; 2]> = points.iter().map(|p| [p[0], track[3]]).collect();
            let stroke = builder.style(line_style(theme.pointer, 1.0, [0.0, 0.0]));
            let area = builder.style(fill_style(
                with_alpha(theme.pointer, 0.25),
                with_alpha(theme.pointer, 0.25),
            ));
            builder.polyline(
                Polyline {
                    points: &points,
                    bases: Some(&bases),
                    from: None,
                    from_bases: None,
                },
                Some(stroke),
                Some(area),
                1.0,
                u32::MAX,
                1,
                draw_flags::GUIDE,
                Along::X,
            );
        }
    }
    let accent = theme.palette.first().copied().unwrap_or(theme.text);
    let window_style = builder.style(fill_style(
        with_alpha(accent, 0.18),
        with_alpha(accent, 0.18),
    ));
    let mut selected = GpuShape::new(ShapeKind::Rect, 0, window_style, u32::MAX, 0);
    selected.to = window;
    selected.from = window;
    builder.shape(selected, key, 0);
    let handle_style = builder.style(symbol_style(theme.surface, theme.pointer, 1.0));
    for x in [window[0], window[2]] {
        let center = [x, (window[1] + window[3]) * 0.5];
        let mut handle = GpuShape::new(ShapeKind::Symbol, 0, handle_style, u32::MAX, 0);
        let size = (window[3] - window[1]) * 0.7;
        handle.to = [center[0], center[1], size, 0.0];
        handle.from = handle.to;
        handle.extra = [1.0, SymbolShape::RoundRect as u32 as f32, size, 0.0];
        builder.shape(handle, key, 0);
    }
    builder.close_shapes();
}
