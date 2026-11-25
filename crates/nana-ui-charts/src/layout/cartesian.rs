//! The cartesian grid: axes, split lines, and line, bar and scatter series.

use nana_ui_core::sort::CompactSort as _;
use std::collections::HashMap;
use std::sync::Arc;

use super::builder::{Polyline, fill_style, line_style, symbol_style, with_alpha};
use super::{
    ChartText, DrawKey, DrawPart, LABEL_GAP, LayoutInput, MarkBuilder, SliderLayout, TextAlign,
    ZoomAxisLayout, crisp, rects_overlap, text_rect,
};
use crate::hit::{
    AxisHit, AxisPoint, AxisSeries, BaseAxisHit, HitModel, ItemHit, ItemRegion, MarkHit, MarkRegion,
};
use crate::marks::{
    GpuShape, MARK_AREA_INDEX, MARK_LINE_INDEX, SHAPE_CLIP_TO_PLOT, SHAPE_EMPHASIS_FILL,
    SHAPE_EMPHASIS_GROW, SHAPE_REVEAL, ShapeKind, SymbolShape, draw_flags,
};
use crate::option::{
    Axis, AxisBound, AxisKind, AxisPosition, BarSeries, BoxplotSeries, CandlestickSeries,
    DataZoomKind, HeatmapSeries, Length, LineSeries, LineType, MarkLineKind, MarkStat, Sampling,
    ScatterSeries, Series, SeriesData, Symbol, ZoomAxis,
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
    pub y_slider: Option<SliderLayout>,
    pub zoom_axes: Vec<ZoomAxisLayout>,
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
        Series::Candlestick(candle) => (candle.x_axis_index, candle.y_axis_index),
        Series::Boxplot(boxplot) => (boxplot.x_axis_index, boxplot.y_axis_index),
        Series::Heatmap(heatmap) => (heatmap.x_axis_index, heatmap.y_axis_index),
        _ => (0, 0),
    };
    if base_x { y } else { x }
}

fn series_category_len(series: &Series) -> usize {
    match series {
        Series::Line(line) if matches!(line.data, SeriesData::Values(_)) => line.data.len(),
        Series::Bar(bar) if matches!(bar.data, SeriesData::Values(_)) => bar.data.len(),
        Series::Candlestick(candle) => candle.data.len(),
        Series::Boxplot(boxplot) => boxplot.data.len(),
        Series::Heatmap(heatmap) => heatmap
            .data
            .iter()
            .filter(|cell| cell.x.is_finite() && cell.x >= 0.0)
            .map(|cell| cell.x.floor() as usize + 1)
            .max()
            .unwrap_or(0),
        _ => 0,
    }
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
    let has_heatmap = placed
        .iter()
        .any(|placed| matches!(placed.series, Series::Heatmap(_)));
    let value_axes: Vec<Axis> = {
        let list = if base_x {
            &option.y_axis
        } else {
            &option.x_axis
        };
        if list.is_empty() {
            if has_heatmap {
                vec![Axis::category(Vec::<Arc<str>>::new())]
            } else {
                vec![Axis::value()]
            }
        } else {
            list.clone()
        }
    };

    // Zoom entries name an axis by its side; the base axis is the x axis
    // unless the chart is horizontal. The first entry on an axis holds its
    // window (the runtime keeps every entry on it the same).
    let (base_side, value_side) = if base_x {
        (ZoomAxis::X, ZoomAxis::Y)
    } else {
        (ZoomAxis::Y, ZoomAxis::X)
    };
    // The base axis' full extent, then the zoom window over it.
    let category_count = placed
        .iter()
        .map(|placed| series_category_len(placed.series))
        .max()
        .unwrap_or(0)
        .max(base_axis.data.len());
    let mut base_lo = f64::INFINITY;
    let mut base_hi = f64::NEG_INFINITY;
    for placed in &placed {
        match placed.series {
            Series::Candlestick(candle) => {
                if !candle.data.is_empty() {
                    base_lo = base_lo.min(0.0);
                    base_hi = base_hi.max((candle.data.len() - 1) as f64);
                }
            }
            Series::Boxplot(boxplot) => {
                if !boxplot.data.is_empty() {
                    base_lo = base_lo.min(0.0);
                    base_hi = base_hi.max((boxplot.data.len() - 1) as f64);
                }
            }
            Series::Heatmap(heatmap) => {
                for cell in heatmap.data.iter() {
                    if cell.x.is_finite() {
                        base_lo = base_lo.min(cell.x);
                        base_hi = base_hi.max(cell.x);
                    }
                }
            }
            _ => {
                if let Some(data) = series_data(placed.series) {
                    for index in 0..data.len() {
                        if let Some((x, _)) = data.get(index)
                            && x.is_finite()
                        {
                            base_lo = base_lo.min(x);
                            base_hi = base_hi.max(x);
                        }
                    }
                }
            }
        }
        if base_axis.kind == AxisKind::Value {
            for value in mark_values(placed.series, base_side) {
                base_lo = base_lo.min(value);
                base_hi = base_hi.max(value);
            }
        }
    }
    let zoom_on = |side: ZoomAxis, index: usize| -> Option<(f64, f64)> {
        let entry = option
            .data_zoom
            .iter()
            .position(|zoom| zoom.target() == (side, index))?;
        input.state.zoom_window(option, entry)
    };
    let slider_on = |side: ZoomAxis| -> Option<(usize, (f64, f64))> {
        let entry = option
            .data_zoom
            .iter()
            .position(|zoom| zoom.target() == (side, 0) && zoom.kind == DataZoomKind::Slider)?;
        Some((entry, zoom_on(side, 0)?))
    };
    let zoom = zoom_on(base_side, 0);
    let (start_pct, end_pct) = zoom.unwrap_or((0.0, 100.0));

    let is_category = base_axis.kind == AxisKind::Category;
    let wants_gap = base_axis.boundary_gap.unwrap_or_else(|| {
        placed.iter().any(|placed| {
            matches!(
                placed.series,
                Series::Bar(_) | Series::Candlestick(_) | Series::Boxplot(_) | Series::Heatmap(_)
            )
        })
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

    // Value extents per value axis over the window, and over all the data
    // (what a value-axis zoom window is a percent of).
    let mut extents: Vec<(f64, f64)> = vec![(f64::INFINITY, f64::NEG_INFINITY); value_axes.len()];
    let mut full_extents = extents.clone();
    let grow = |extent: &mut (f64, f64), value: f64| {
        extent.0 = extent.0.min(value);
        extent.1 = extent.1.max(value);
    };
    for (slot, placed_series) in placed.iter().enumerate() {
        let axis = value_axis_index(placed_series.series, base_x).min(value_axes.len() - 1);
        match placed_series.series {
            Series::Candlestick(candle) => {
                for (index, ohlc) in candle.data.iter().enumerate() {
                    for value in [ohlc.low, ohlc.high, ohlc.open, ohlc.close] {
                        if value.is_finite() {
                            grow(&mut full_extents[axis], value);
                            if in_window(index as f64) {
                                grow(&mut extents[axis], value);
                            }
                        }
                    }
                }
            }
            Series::Boxplot(boxplot) => {
                for (index, bw) in boxplot.data.iter().enumerate() {
                    for value in [bw.min, bw.q1, bw.median, bw.q3, bw.max] {
                        if value.is_finite() {
                            grow(&mut full_extents[axis], value);
                            if in_window(index as f64) {
                                grow(&mut extents[axis], value);
                            }
                        }
                    }
                }
            }
            Series::Heatmap(heatmap) => {
                for cell in heatmap.data.iter() {
                    if cell.y.is_finite() {
                        grow(&mut full_extents[axis], cell.y);
                        if in_window(cell.x) {
                            grow(&mut extents[axis], cell.y);
                        }
                    }
                }
            }
            _ => {
                if let Some(data) = series_data(placed_series.series) {
                    let bars = matches!(placed_series.series, Series::Bar(_));
                    let stacked_series = stacked.contains_key(&slot);
                    for index in 0..data.len() {
                        let Some((x, base, top)) = value_at(slot, index) else {
                            continue;
                        };
                        let both = bars || stacked_series;
                        grow(&mut full_extents[axis], top);
                        if both {
                            grow(&mut full_extents[axis], base);
                        }
                        if !in_window(x) {
                            continue;
                        }
                        grow(&mut extents[axis], top);
                        if both {
                            grow(&mut extents[axis], base);
                        }
                    }
                }
            }
        }
        // A threshold or band at a fixed value stays in view.
        for value in mark_values(placed_series.series, value_side) {
            grow(&mut extents[axis], value);
            grow(&mut full_extents[axis], value);
        }
    }

    let value_category_totals: Vec<usize> = value_axes
        .iter()
        .enumerate()
        .map(|(index, axis)| {
            if axis.kind != AxisKind::Category {
                return 0;
            }
            let data_max = placed
                .iter()
                .filter(|placed| value_axis_index(placed.series, base_x) == index)
                .filter_map(|placed| match placed.series {
                    Series::Heatmap(heatmap) => heatmap
                        .data
                        .iter()
                        .filter(|cell| cell.y.is_finite() && cell.y >= 0.0)
                        .map(|cell| cell.y.floor() as usize + 1)
                        .max(),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            axis.data.len().max(data_max).max(1)
        })
        .collect();
    let category_slice = |total: usize, (s, e): (f64, f64)| -> (usize, usize) {
        let total = total.max(1);
        let first = ((s / 100.0) * total as f64).floor() as usize;
        let last = (((e / 100.0) * total as f64).ceil() as usize).clamp(first + 1, total);
        let first = first.min(total - 1);
        (first, last - first)
    };
    let value_scales: Vec<Scale> = value_axes
        .iter()
        .enumerate()
        .map(
            |(index, axis)| match (axis.kind, zoom_on(value_side, index)) {
                (AxisKind::Category, zoom) => {
                    let (first, count) =
                        category_slice(value_category_totals[index], zoom.unwrap_or((0.0, 100.0)));
                    Scale::Band {
                        first,
                        count: count.max(1),
                        gap: axis.boundary_gap.unwrap_or(true),
                    }
                }
                (AxisKind::Value, Some(window)) => {
                    let (lo, hi) = full_extents[index];
                    zoomed_value_scale(axis, lo, hi, window)
                }
                _ => value_scale(axis, extents[index].0, extents[index].1),
            },
        )
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
    let x_slider = slider_on(ZoomAxis::X);
    let y_slider = slider_on(ZoomAxis::Y);
    if x_slider.is_some() {
        margin[3] += SLIDER_HEIGHT + LABEL_GAP * 2.0;
    }
    if y_slider.is_some() {
        margin[2] += SLIDER_HEIGHT + LABEL_GAP * 2.0;
    }
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
        && axis.split_line.unwrap_or(axis.kind != AxisKind::Category)
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

    let marks = MarkContext {
        input,
        placed: &placed,
        base_axis: &base_axis,
        value_axes: &value_axes,
        base_map: &base_map,
        value_maps: &value_maps,
        base_x,
        plot,
    };
    // Mark areas lie under every series.
    marks.areas(builder, texts, hit);

    // Heatmaps, bars, candlesticks, boxplots, then lines and scatter.
    let mut axis_series = Vec::new();
    for placed_series in &placed {
        if let Series::Heatmap(heatmap) = placed_series.series {
            let value_index =
                value_axis_index(placed_series.series, base_x).min(value_maps.len() - 1);
            layout_heatmap(
                input,
                heatmap,
                placed_series,
                &base_axis,
                &value_axes[value_index],
                &base_map,
                &value_maps[value_index],
                base_x,
                plot,
                builder,
                texts,
                hit,
            );
        }
    }
    axis_series.extend(layout_bars(
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
    ));
    for placed_series in &placed {
        let value_map =
            &value_maps[value_axis_index(placed_series.series, base_x).min(value_maps.len() - 1)];
        match placed_series.series {
            Series::Candlestick(candle) => axis_series.push(layout_candlestick(
                input,
                candle,
                placed_series,
                &base_map,
                value_map,
                base_x,
                plot,
                builder,
                hit,
            )),
            Series::Boxplot(boxplot) => axis_series.push(layout_boxplot(
                input,
                boxplot,
                placed_series,
                &base_map,
                value_map,
                base_x,
                plot,
                builder,
                hit,
            )),
            Series::Line(line) => axis_series.push(layout_line(
                input,
                line,
                placed_series,
                placed
                    .iter()
                    .position(|p| p.index == placed_series.index)
                    .unwrap_or(0),
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
    // Mark lines go over every series.
    marks.lines(builder, texts, hit);

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
        if axis.axis_line && !axis.split_line.unwrap_or(axis.kind != AxisKind::Category) {
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

    let slider = x_slider.map(|(zoom_index, (start, end))| {
        let track = [plot[0], fy1 - SLIDER_HEIGHT, plot[2], fy1];
        let span = track[2] - track[0];
        let window = [
            track[0] + span * (start / 100.0) as f32,
            track[1],
            track[0] + span * (end / 100.0) as f32,
            track[3],
        ];
        // The preview shows the data along the base axis.
        layout_slider(
            input,
            base_x.then_some(&placed[..]),
            track,
            window,
            false,
            builder,
        );
        SliderLayout {
            zoom_index,
            track,
            window,
        }
    });
    let y_slider = y_slider.map(|(zoom_index, (start, end))| {
        let track = [fx1 - SLIDER_HEIGHT, plot[1], fx1, plot[3]];
        let span = track[3] - track[1];
        // The window grows upward, as the axis does.
        let window = [
            track[0],
            track[3] - span * (end / 100.0) as f32,
            track[2],
            track[3] - span * (start / 100.0) as f32,
        ];
        layout_slider(input, None, track, window, true, builder);
        SliderLayout {
            zoom_index,
            track,
            window,
        }
    });

    // How px map to each zoomable axis' window.
    let mut zoom_axes = Vec::new();
    let zoomable = |side: ZoomAxis, index: usize| {
        option
            .data_zoom
            .iter()
            .any(|zoom| zoom.target() == (side, index))
    };
    if zoomable(base_side, 0) {
        // Two base values and their percent of the full extent: category
        // `i` is centred at `(i + 0.5) / total`.
        let anchors = if is_category {
            let total = category_count.max(1) as f64;
            let first = window_first as f64;
            Some((
                [first, first + 1.0],
                [(first + 0.5) / total * 100.0, (first + 1.5) / total * 100.0],
            ))
        } else {
            (base_lo.is_finite() && base_hi > base_lo).then_some(([base_lo, base_hi], [0.0, 100.0]))
        };
        if let Some((values, percent)) = anchors {
            zoom_axes.push(ZoomAxisLayout {
                axis: base_side,
                index: 0,
                vertical: !base_x,
                px: [base_map.px(values[0]), base_map.px(values[1])],
                percent,
            });
        }
    }
    for (index, ((axis, map), scale)) in value_axes
        .iter()
        .zip(&value_maps)
        .zip(&value_scales)
        .enumerate()
    {
        if !zoomable(value_side, index) {
            continue;
        }
        match (axis.kind, scale) {
            (AxisKind::Category, Scale::Band { first, .. }) => {
                let total = value_category_totals[index].max(1) as f64;
                let first = *first as f64;
                zoom_axes.push(ZoomAxisLayout {
                    axis: value_side,
                    index,
                    vertical: base_x,
                    px: [map.px(first), map.px(first + 1.0)],
                    percent: [(first + 0.5) / total * 100.0, (first + 1.5) / total * 100.0],
                });
            }
            (AxisKind::Value, _) if zoom_on(value_side, index).is_some() => {
                let (start, end) = zoom_on(value_side, index).unwrap();
                zoom_axes.push(ZoomAxisLayout {
                    axis: value_side,
                    index,
                    vertical: base_x,
                    px: [map.start, map.end],
                    percent: [start, end],
                });
            }
            _ => {}
        }
    }

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
    axis_series.compact_sort_by_key(|series| series.series);
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
    CartesianOut {
        plot,
        slider,
        y_slider,
        zoom_axes,
    }
}

/// A value axis zoomed to `window` (percent) of the scale it has over all
/// the data, keeping the window's exact ends.
fn zoomed_value_scale(axis: &Axis, lo: f64, hi: f64, (start, end): (f64, f64)) -> Scale {
    let reference = value_scale(axis, lo, hi);
    let Scale::Linear(ticks) = &reference else {
        return reference;
    };
    if start <= 0.0 && end >= 100.0 {
        return reference;
    }
    let span = ticks.max - ticks.min;
    let (lo, hi) = (
        ticks.min + span * start / 100.0,
        ticks.min + span * end / 100.0,
    );
    Scale::Linear(scale::nice_linear(
        lo,
        hi,
        axis.split_number,
        Some(lo),
        Some(hi),
    ))
}

/// Fixed values a series' marks put on its value axis (`value_side`).
fn mark_values(series: &Series, value_side: ZoomAxis) -> Vec<f64> {
    let (line, area) = series.marks();
    let mut values = Vec::new();
    let on_y = value_side == ZoomAxis::Y;
    for item in line.iter().flat_map(|line| &line.data) {
        match item.kind {
            MarkLineKind::Y(value) if on_y => values.push(value),
            MarkLineKind::X(value) if !on_y => values.push(value),
            MarkLineKind::Segment(a, b) => {
                let side = usize::from(on_y);
                values.extend([a[side], b[side]]);
            }
            _ => {}
        }
    }
    for item in area.iter().flat_map(|area| &area.data) {
        if let Some(range) = if on_y { item.y } else { item.x } {
            values.extend(range);
        }
    }
    values.retain(|value| value.is_finite());
    values
}

/// A statistic of a series' values (NaN gaps left out).
fn series_stat(series: &Series, stat: MarkStat) -> Option<f64> {
    let mut values: Vec<f64> = match series {
        Series::Candlestick(candle) => match stat {
            MarkStat::Min => candle
                .data
                .iter()
                .map(|item| item.low)
                .filter(|v| v.is_finite())
                .collect(),
            MarkStat::Max => candle
                .data
                .iter()
                .map(|item| item.high)
                .filter(|v| v.is_finite())
                .collect(),
            MarkStat::Average | MarkStat::Median => candle
                .data
                .iter()
                .map(|item| item.close)
                .filter(|v| v.is_finite())
                .collect(),
        },
        Series::Boxplot(boxplot) => match stat {
            MarkStat::Min => boxplot
                .data
                .iter()
                .map(|item| item.min)
                .filter(|v| v.is_finite())
                .collect(),
            MarkStat::Max => boxplot
                .data
                .iter()
                .map(|item| item.max)
                .filter(|v| v.is_finite())
                .collect(),
            MarkStat::Average | MarkStat::Median => boxplot
                .data
                .iter()
                .map(|item| item.median)
                .filter(|v| v.is_finite())
                .collect(),
        },
        Series::Heatmap(heatmap) => heatmap
            .data
            .iter()
            .map(|cell| cell.value)
            .filter(|v| v.is_finite())
            .collect(),
        _ => {
            let data = series_data(series)?;
            (0..data.len())
                .filter_map(|index| data.get(index))
                .map(|(_, value)| value)
                .filter(|value| value.is_finite())
                .collect()
        }
    };
    if values.is_empty() {
        return None;
    }
    Some(match stat {
        MarkStat::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
        MarkStat::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        MarkStat::Average => values.iter().sum::<f64>() / values.len() as f64,
        MarkStat::Median => {
            values.compact_sort_by(f64::total_cmp);
            let middle = values.len() / 2;
            if values.len().is_multiple_of(2) {
                (values[middle - 1] + values[middle]) * 0.5
            } else {
                values[middle]
            }
        }
    })
}

/// Clips a segment to `rect` via Liang-Barsky, returning `None` when it
/// lies entirely outside.
fn clip_segment_to_rect(a: [f32; 2], b: [f32; 2], rect: [f32; 4]) -> Option<[[f32; 2]; 2]> {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    for (p, q) in [
        (-dx, a[0] - rect[0]),
        (dx, rect[2] - a[0]),
        (-dy, a[1] - rect[1]),
        (dy, rect[3] - a[1]),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    Some([
        [a[0] + dx * t0, a[1] + dy * t0],
        [a[0] + dx * t1, a[1] + dy * t1],
    ])
}

fn axis_value_text(
    axis: &Axis,
    value: f64,
    option: &crate::option::ChartOption,
    formatter: Option<&crate::option::ValueFormatter>,
) -> Arc<str> {
    if axis.kind == AxisKind::Category {
        let index = value.round();
        if index >= 0.0
            && let Some(name) = axis.data.get(index as usize)
        {
            return name.clone();
        }
    }
    if axis.kind == AxisKind::Time {
        return time_label(
            axis,
            TimeTick {
                value: value.round() as i64,
                unit: crate::option::TimeUnit::Second,
                boundary: false,
            },
        );
    }
    match formatter {
        Some(formatter) => formatter.format(value),
        None => crate::hit::format_value(option, value),
    }
}

/// What mark lines and areas are placed against.
struct MarkContext<'a> {
    input: &'a LayoutInput<'a>,
    placed: &'a [Placed<'a>],
    base_axis: &'a Axis,
    value_axes: &'a [Axis],
    base_map: &'a AxisMap,
    value_maps: &'a [AxisMap],
    base_x: bool,
    plot: [f32; 4],
}

impl MarkContext<'_> {
    /// The maps of the x and y axes a series is drawn against.
    fn maps(&self, series: &Series) -> (&AxisMap, &AxisMap) {
        let value =
            &self.value_maps[value_axis_index(series, self.base_x).min(self.value_maps.len() - 1)];
        if self.base_x {
            (self.base_map, value)
        } else {
            (value, self.base_map)
        }
    }

    /// A value on the x (`on_x`) or y axis as text: a category's name, a
    /// time, or a number.
    fn value_text(
        &self,
        series: &Series,
        on_x: bool,
        value: f64,
        formatter: Option<&crate::option::ValueFormatter>,
    ) -> Arc<str> {
        let on_base = on_x == self.base_x;
        let axis = if on_base {
            self.base_axis
        } else {
            &self.value_axes[value_axis_index(series, self.base_x).min(self.value_axes.len() - 1)]
        };
        axis_value_text(axis, value, self.input.option, formatter)
    }

    fn areas(&self, builder: &mut MarkBuilder, texts: &mut Vec<ChartText>, hit: &mut HitModel) {
        let theme = self.input.theme;
        let plot = self.plot;
        for placed in self.placed {
            let (_, Some(area)) = placed.series.marks() else {
                continue;
            };
            let series = placed.index as u32;
            let (x_map, y_map) = self.maps(placed.series);
            // A range's px; on bands (bars) it covers its categories whole,
            // on category points (lines) it runs between them.
            let span = |map: &AxisMap, range: Option<[f64; 2]>, lo: f32, hi: f32| match range {
                None => Some((lo, hi)),
                Some([a, b]) => {
                    let (a, b) = (map.px(a), map.px(b));
                    let half = if matches!(map.scale, Scale::Band { gap: true, .. }) {
                        map.band() * 0.5
                    } else {
                        0.0
                    };
                    (a.is_finite() && b.is_finite()).then(|| (a.min(b) - half, a.max(b) + half))
                }
            };
            for (k, item) in area.data.iter().enumerate() {
                let (Some((x0, x1)), Some((y0, y1))) = (
                    span(x_map, item.x, plot[0], plot[2]),
                    span(y_map, item.y, plot[1], plot[3]),
                ) else {
                    continue;
                };
                let rect = [
                    x0.max(plot[0]),
                    y0.max(plot[1]),
                    x1.min(plot[2]),
                    y1.min(plot[3]),
                ];
                if rect[2] <= rect[0] || rect[3] <= rect[1] {
                    continue;
                }
                let color = item
                    .color
                    .or(area.color)
                    .map_or(placed.color, |color| theme.resolve(color));
                let fill = with_alpha(color, area.opacity);
                let style = builder.style(fill_style(fill, fill));
                let index = MARK_AREA_INDEX + k as u32;
                let mut shape = GpuShape::new(
                    ShapeKind::Rect,
                    SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_FILL,
                    style,
                    series,
                    index,
                );
                shape.to = rect;
                shape.from = rect;
                builder.shape(
                    shape,
                    DrawKey {
                        series,
                        part: DrawPart::MarkArea,
                        run: 0,
                    },
                    0,
                );
                let name = item.name.clone().unwrap_or_else(|| Arc::from(""));
                if area.label && !name.is_empty() {
                    let size = theme.font_size;
                    let label = text_rect(
                        self.input.measure,
                        &name,
                        size,
                        [rect[0] + LABEL_GAP * 0.5, rect[1] + LABEL_GAP * 0.5],
                        TextAlign::Start,
                        TextAlign::Start,
                    );
                    if label[2] <= rect[2] && label[3] <= rect[3] {
                        texts.push(ChartText {
                            rect: label,
                            text: name.clone(),
                            color: theme.muted,
                            size,
                            weight: None,
                        });
                    }
                }
                let range = |on_x: bool, [a, b]: [f64; 2]| -> Arc<str> {
                    format!(
                        "{} – {}",
                        self.value_text(placed.series, on_x, a.min(b), None),
                        self.value_text(placed.series, on_x, a.max(b), None)
                    )
                    .into()
                };
                let value = match (item.y, item.x) {
                    (Some(y), _) => range(false, y),
                    (None, Some(x)) => range(true, x),
                    (None, None) => Arc::from(""),
                };
                hit.marks.push(MarkHit {
                    region: MarkRegion::Area(rect),
                    series,
                    index,
                    name,
                    value,
                    color,
                });
            }
            builder.close_shapes();
        }
    }

    fn lines(&self, builder: &mut MarkBuilder, texts: &mut Vec<ChartText>, hit: &mut HitModel) {
        let theme = self.input.theme;
        let plot = self.plot;
        let inside = |lo: f32, hi: f32, at: f32| at.is_finite() && at >= lo - 0.5 && at <= hi + 0.5;
        for placed in self.placed {
            let (Some(line), _) = placed.series.marks() else {
                continue;
            };
            let series = placed.index as u32;
            let (x_map, y_map) = self.maps(placed.series);
            let formatter = line.formatter.as_ref();
            for (k, item) in line.data.iter().enumerate() {
                // `(ends, the value it marks, on the x axis, vertical)`.
                let vertical = |value: f64| {
                    let x = x_map.px(value);
                    inside(plot[0], plot[2], x).then_some((
                        [[x, plot[1]], [x, plot[3]]],
                        value,
                        true,
                        true,
                    ))
                };
                let horizontal = |value: f64| {
                    let y = y_map.px(value);
                    inside(plot[1], plot[3], y).then_some((
                        [[plot[0], y], [plot[2], y]],
                        value,
                        false,
                        false,
                    ))
                };
                let placed_line = match item.kind {
                    MarkLineKind::X(value) => vertical(value),
                    MarkLineKind::Y(value) => horizontal(value),
                    MarkLineKind::Stat(stat) => {
                        series_stat(placed.series, stat).and_then(|value| {
                            if self.base_x {
                                horizontal(value)
                            } else {
                                vertical(value)
                            }
                        })
                    }
                    MarkLineKind::Segment(a, b) => {
                        let a_px = [x_map.px(a[0]), y_map.px(a[1])];
                        let b_px = [x_map.px(b[0]), y_map.px(b[1])];
                        if !a_px.iter().chain(&b_px).all(|v| v.is_finite()) {
                            None
                        } else {
                            clip_segment_to_rect(a_px, b_px, plot).map(|ends| {
                                // A segment reads as its end's value.
                                let (value, on_x) = if self.base_x {
                                    (b[1], false)
                                } else {
                                    (b[0], true)
                                };
                                (ends, value, on_x, false)
                            })
                        }
                    }
                };
                let Some((ends, value, on_x, is_vertical)) = placed_line else {
                    continue;
                };
                let color = item
                    .color
                    .or(line.color)
                    .map_or(placed.color, |color| theme.resolve(color));
                let style = builder.style(line_style(
                    color,
                    line.width,
                    dash_of(line.line_type, line.width),
                ));
                let index = MARK_LINE_INDEX + k as u32;
                builder.mark_line(
                    ends,
                    style,
                    line.width,
                    series,
                    index,
                    if is_vertical { Along::Y } else { Along::X },
                );
                let value_text = self.value_text(placed.series, on_x, value, formatter);
                let name = item.name.clone().unwrap_or_else(|| Arc::from(""));
                if line.label {
                    let text = if name.is_empty() {
                        value_text.clone()
                    } else {
                        name.clone()
                    };
                    let size = theme.font_size;
                    let gap = LABEL_GAP * 0.5;
                    let rect = match item.kind {
                        MarkLineKind::Segment(..) => text_rect(
                            self.input.measure,
                            &text,
                            size,
                            [ends[1][0] + gap, ends[1][1] - gap],
                            TextAlign::Start,
                            TextAlign::End,
                        ),
                        _ if is_vertical => text_rect(
                            self.input.measure,
                            &text,
                            size,
                            [ends[0][0] + gap, plot[1] + gap],
                            TextAlign::Start,
                            TextAlign::Start,
                        ),
                        _ => text_rect(
                            self.input.measure,
                            &text,
                            size,
                            [plot[2] - gap, ends[0][1] - gap * 0.5],
                            TextAlign::End,
                            TextAlign::End,
                        ),
                    };
                    texts.push(ChartText {
                        rect,
                        text,
                        color,
                        size,
                        weight: None,
                    });
                }
                hit.marks.push(MarkHit {
                    region: MarkRegion::Line {
                        a: ends[0],
                        b: ends[1],
                    },
                    series,
                    index,
                    name,
                    value: value_text,
                    color,
                });
            }
        }
    }
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
        xs.compact_sort_by(f32::total_cmp);
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
                details: Vec::new(),
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
            details: Vec::new(),
        });
    }
}

fn mix_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

fn continuous_cell_span(coords: impl Iterator<Item = f32>, plot_span: f32) -> f32 {
    let mut sorted: Vec<f32> = coords.filter(|v| v.is_finite()).collect();
    sorted.compact_sort_by(f32::total_cmp);
    sorted
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.5)
        .fold(f32::INFINITY, f32::min)
        .min(plot_span.abs() * 0.25)
        .max(2.0)
}

#[allow(clippy::too_many_arguments)]
fn layout_candlestick(
    input: &LayoutInput<'_>,
    candle: &CandlestickSeries,
    placed: &Placed<'_>,
    base_map: &AxisMap,
    value_map: &AxisMap,
    base_x: bool,
    plot: [f32; 4],
    builder: &mut MarkBuilder,
    hit: &mut HitModel,
) -> AxisSeries {
    let theme = input.theme;
    let series = placed.index as u32;
    let up_color = candle.up_color.map_or(theme.positive, |c| theme.resolve(c));
    let down_color = candle
        .down_color
        .map_or(theme.negative, |c| theme.resolve(c));
    let band = if base_map.band() > 0.0 {
        base_map.band()
    } else {
        continuous_cell_span(
            (0..candle.data.len()).map(|i| base_map.px(i as f64)),
            if base_x {
                plot[2] - plot[0]
            } else {
                plot[3] - plot[1]
            },
        )
    };
    let mut width = candle
        .bar_width
        .map(|w| w.resolve(band))
        .unwrap_or(band * 0.6);
    if let Some(max) = candle.bar_max_width {
        width = width.min(max);
    }
    let width = width.max(2.0);
    let animate = candle.data.len() <= input.option.animation.threshold;
    let labels = candle.value_labels.clone().unwrap_or_else(|| {
        [
            Arc::from("O"),
            Arc::from("C"),
            Arc::from("L"),
            Arc::from("H"),
        ]
    });
    let up_style = builder.style(fill_style(up_color, up_color));
    let down_style = builder.style(fill_style(down_color, down_color));
    let (plot_lo, plot_hi) = if base_x {
        (plot[0], plot[2])
    } else {
        (plot[1], plot[3])
    };
    let rect_of = |a_lo: f32, a_hi: f32, v_lo: f32, v_hi: f32| {
        if base_x {
            [a_lo, v_lo, a_hi, v_hi]
        } else {
            [v_lo, a_lo, v_hi, a_hi]
        }
    };
    let mut points = Vec::new();
    for (index, ohlc) in candle.data.iter().enumerate() {
        if ![ohlc.open, ohlc.close, ohlc.low, ohlc.high]
            .into_iter()
            .all(f64::is_finite)
        {
            continue;
        }
        let center = base_map.px(index as f64);
        let (lo_edge, hi_edge) = (center - width * 0.5, center + width * 0.5);
        if hi_edge < plot_lo || lo_edge > plot_hi {
            continue;
        }
        let open_px = value_map.px(ohlc.open);
        let close_px = value_map.px(ohlc.close);
        let low_px = value_map.px(ohlc.low);
        let high_px = value_map.px(ohlc.high);
        let mid_px = (open_px + close_px) * 0.5;
        let (body_lo, body_hi) = if (close_px - open_px).abs() < 1.0 {
            (mid_px - 0.5, mid_px + 0.5)
        } else {
            (open_px.min(close_px), open_px.max(close_px))
        };
        let wick_lo = low_px.min(high_px).min(body_lo);
        let wick_hi = low_px.max(high_px).max(body_hi);
        let rising = ohlc.close >= ohlc.open;
        let (style, color) = if rising {
            (up_style, up_color)
        } else {
            (down_style, down_color)
        };
        let key = DrawKey {
            series,
            part: DrawPart::Shapes,
            run: 0,
        };
        let flags = SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW;
        let mut wick = GpuShape::new(ShapeKind::Rect, flags, style, series, index as u32);
        wick.to = rect_of(center - 0.75, center + 0.75, wick_lo, wick_hi);
        wick.from = if animate {
            rect_of(center - 0.75, center + 0.75, mid_px, mid_px)
        } else {
            wick.to
        };
        builder.shape(wick, key, 0);

        let mut body = GpuShape::new(ShapeKind::Rect, flags, style, series, index as u32);
        body.to = rect_of(lo_edge, hi_edge, body_lo, body_hi);
        body.from = if animate {
            rect_of(lo_edge, hi_edge, mid_px, mid_px)
        } else {
            body.to
        };
        builder.shape(body, key, 0);

        let half_hit = (width * 0.5).max(3.0);
        hit.items.push(ItemHit {
            region: ItemRegion::Rect(rect_of(
                center - half_hit,
                center + half_hit,
                wick_lo,
                wick_hi,
            )),
            series,
            index: index as u32,
            title: None,
            row: None,
            value: ohlc.close,
            color,
            details: vec![
                (labels[0].clone(), ohlc.open),
                (labels[1].clone(), ohlc.close),
                (labels[2].clone(), ohlc.low),
                (labels[3].clone(), ohlc.high),
            ],
        });
        points.push(AxisPoint {
            base: center,
            cross: close_px,
            index: index as u32,
            key: index as f64,
            value: ohlc.close,
        });
    }
    builder.close_shapes();
    AxisSeries {
        series,
        name: candle.name.clone(),
        color: up_color,
        points,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_boxplot(
    input: &LayoutInput<'_>,
    boxplot: &BoxplotSeries,
    placed: &Placed<'_>,
    base_map: &AxisMap,
    value_map: &AxisMap,
    base_x: bool,
    plot: [f32; 4],
    builder: &mut MarkBuilder,
    hit: &mut HitModel,
) -> AxisSeries {
    let series = placed.index as u32;
    let color = placed.color;
    let fill_style_id = builder.style(fill_style(with_alpha(color, 0.22), with_alpha(color, 0.22)));
    let stroke_style_id = builder.style(fill_style(color, color));
    let band = if base_map.band() > 0.0 {
        base_map.band()
    } else {
        continuous_cell_span(
            (0..boxplot.data.len()).map(|i| base_map.px(i as f64)),
            if base_x {
                plot[2] - plot[0]
            } else {
                plot[3] - plot[1]
            },
        )
    };
    let mut width = boxplot
        .box_width
        .map(|w| w.resolve(band))
        .unwrap_or(band * 0.55);
    if let Some(max) = boxplot.box_max_width {
        width = width.min(max);
    }
    let width = width.max(4.0);
    let animate = boxplot.data.len() <= input.option.animation.threshold;
    let labels = boxplot.value_labels.clone().unwrap_or_else(|| {
        [
            Arc::from("min"),
            Arc::from("Q1"),
            Arc::from("median"),
            Arc::from("Q3"),
            Arc::from("max"),
        ]
    });
    let (plot_lo, plot_hi) = if base_x {
        (plot[0], plot[2])
    } else {
        (plot[1], plot[3])
    };
    let rect_of = |a_lo: f32, a_hi: f32, v_lo: f32, v_hi: f32| {
        if base_x {
            [a_lo, v_lo, a_hi, v_hi]
        } else {
            [v_lo, a_lo, v_hi, a_hi]
        }
    };
    let key = DrawKey {
        series,
        part: DrawPart::Shapes,
        run: 0,
    };
    let flags = SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW;
    let mut points = Vec::new();
    for (index, bw) in boxplot.data.iter().enumerate() {
        if ![bw.min, bw.q1, bw.median, bw.q3, bw.max]
            .into_iter()
            .all(f64::is_finite)
        {
            continue;
        }
        let center = base_map.px(index as f64);
        let (lo_edge, hi_edge) = (center - width * 0.5, center + width * 0.5);
        if hi_edge < plot_lo || lo_edge > plot_hi {
            continue;
        }
        let min_px = value_map.px(bw.min);
        let q1_px = value_map.px(bw.q1);
        let med_px = value_map.px(bw.median);
        let q3_px = value_map.px(bw.q3);
        let max_px = value_map.px(bw.max);
        let (iqr_lo, iqr_hi) = if (q3_px - q1_px).abs() < 2.0 {
            (med_px - 1.0, med_px + 1.0)
        } else {
            (q1_px.min(q3_px), q1_px.max(q3_px))
        };
        let whisker_lo = min_px.min(max_px).min(iqr_lo);
        let whisker_hi = min_px.max(max_px).max(iqr_hi);
        let mut emit = |a_lo: f32, a_hi: f32, v_lo: f32, v_hi: f32, style: u32| {
            if v_hi <= v_lo || a_hi <= a_lo {
                return;
            }
            let mut shape = GpuShape::new(ShapeKind::Rect, flags, style, series, index as u32);
            shape.to = rect_of(a_lo, a_hi, v_lo, v_hi);
            shape.from = if animate {
                rect_of(a_lo, a_hi, med_px, med_px)
            } else {
                shape.to
            };
            builder.shape(shape, key, 0);
        };
        // Whisker stems and caps.
        let cap_half = (width * 0.25).max(1.5);
        emit(
            center - 0.75,
            center + 0.75,
            whisker_lo,
            iqr_lo,
            stroke_style_id,
        );
        emit(
            center - 0.75,
            center + 0.75,
            iqr_hi,
            whisker_hi,
            stroke_style_id,
        );
        emit(
            center - cap_half,
            center + cap_half,
            whisker_lo - 0.75,
            whisker_lo + 0.75,
            stroke_style_id,
        );
        emit(
            center - cap_half,
            center + cap_half,
            whisker_hi - 0.75,
            whisker_hi + 0.75,
            stroke_style_id,
        );
        // IQR box fill and outline.
        emit(lo_edge, hi_edge, iqr_lo, iqr_hi, fill_style_id);
        let stroke_w = 1.5_f32
            .min((hi_edge - lo_edge) * 0.25)
            .min((iqr_hi - iqr_lo) * 0.25);
        emit(lo_edge, hi_edge, iqr_lo, iqr_lo + stroke_w, stroke_style_id);
        emit(lo_edge, hi_edge, iqr_hi - stroke_w, iqr_hi, stroke_style_id);
        emit(lo_edge, lo_edge + stroke_w, iqr_lo, iqr_hi, stroke_style_id);
        emit(hi_edge - stroke_w, hi_edge, iqr_lo, iqr_hi, stroke_style_id);
        // Median bar.
        emit(
            lo_edge,
            hi_edge,
            med_px - 1.0,
            med_px + 1.0,
            stroke_style_id,
        );

        hit.items.push(ItemHit {
            region: ItemRegion::Rect(rect_of(lo_edge, hi_edge, whisker_lo, whisker_hi)),
            series,
            index: index as u32,
            title: None,
            row: None,
            value: bw.median,
            color,
            details: vec![
                (labels[0].clone(), bw.min),
                (labels[1].clone(), bw.q1),
                (labels[2].clone(), bw.median),
                (labels[3].clone(), bw.q3),
                (labels[4].clone(), bw.max),
            ],
        });
        points.push(AxisPoint {
            base: center,
            cross: med_px,
            index: index as u32,
            key: index as f64,
            value: bw.median,
        });
    }
    builder.close_shapes();
    AxisSeries {
        series,
        name: boxplot.name.clone(),
        color,
        points,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_heatmap(
    input: &LayoutInput<'_>,
    heatmap: &HeatmapSeries,
    placed: &Placed<'_>,
    base_axis: &Axis,
    cross_axis: &Axis,
    base_map: &AxisMap,
    cross_map: &AxisMap,
    base_x: bool,
    plot: [f32; 4],
    builder: &mut MarkBuilder,
    texts: &mut Vec<ChartText>,
    hit: &mut HitModel,
) {
    let theme = input.theme;
    let series = placed.index as u32;
    let (x_axis, y_axis) = if base_x {
        (base_axis, cross_axis)
    } else {
        (cross_axis, base_axis)
    };
    let (x_map, y_map) = if base_x {
        (base_map, cross_map)
    } else {
        (cross_map, base_map)
    };
    let mut data_min = f64::INFINITY;
    let mut data_max = f64::NEG_INFINITY;
    for cell in heatmap.data.iter() {
        if cell.value.is_finite() {
            data_min = data_min.min(cell.value);
            data_max = data_max.max(cell.value);
        }
    }
    let min = heatmap
        .min
        .unwrap_or(if data_min.is_finite() { data_min } else { 0.0 });
    let max = heatmap
        .max
        .unwrap_or(if data_max.is_finite() { data_max } else { 1.0 });
    let span = max - min;
    let stops: Vec<[f32; 4]> = if heatmap.colors.is_empty() {
        let accent = placed.color;
        vec![
            mix_color(theme.surface, accent, 0.15),
            accent,
            theme.warning,
        ]
    } else {
        heatmap.colors.iter().map(|c| theme.resolve(*c)).collect()
    };
    let ramp = |t: f32| -> [f32; 4] {
        if stops.len() == 1 {
            return stops[0];
        }
        let scaled = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
        let idx = (scaled.floor() as usize).min(stops.len() - 2);
        mix_color(stops[idx], stops[idx + 1], scaled - idx as f32)
    };
    let cell_w = if x_map.band() > 0.0 {
        x_map.band()
    } else {
        continuous_cell_span(
            heatmap
                .data
                .iter()
                .map(|c| x_map.px(if base_x { c.x } else { c.y })),
            plot[2] - plot[0],
        )
    };
    let cell_h = if y_map.band() > 0.0 {
        y_map.band()
    } else {
        continuous_cell_span(
            heatmap
                .data
                .iter()
                .map(|c| y_map.px(if base_x { c.y } else { c.x })),
            plot[3] - plot[1],
        )
    };
    let inset = heatmap.gap * 0.5;
    let animate = heatmap.data.len() <= input.option.animation.threshold;
    let key = DrawKey {
        series,
        part: DrawPart::Shapes,
        run: 0,
    };
    let flags = SHAPE_CLIP_TO_PLOT | SHAPE_EMPHASIS_GROW;
    for (index, cell) in heatmap.data.iter().enumerate() {
        if !cell.x.is_finite() || !cell.y.is_finite() || !cell.value.is_finite() {
            continue;
        }
        let (x_val, y_val) = if base_x {
            (cell.x, cell.y)
        } else {
            (cell.y, cell.x)
        };
        let cx = x_map.px(x_val);
        let cy = y_map.px(y_val);
        let (x0, x1) = (cx - cell_w * 0.5 + inset, cx + cell_w * 0.5 - inset);
        let (y0, y1) = (cy - cell_h * 0.5 + inset, cy + cell_h * 0.5 - inset);
        if x1 <= plot[0] || x0 >= plot[2] || y1 <= plot[1] || y0 >= plot[3] || x1 <= x0 || y1 <= y0
        {
            continue;
        }
        let t = if span.abs() <= f64::EPSILON {
            0.5
        } else {
            ((cell.value - min) / span).clamp(0.0, 1.0) as f32
        };
        let fill = ramp(t);
        let style = builder.style(fill_style(fill, fill));
        let mut shape = GpuShape::new(ShapeKind::Rect, flags, style, series, index as u32);
        shape.to = [x0, y0, x1, y1];
        shape.from = if animate { [cx, cy, cx, cy] } else { shape.to };
        shape.extra = [heatmap.border_radius; 4];
        builder.shape(shape, key, 0);

        if heatmap.label.show {
            let text: Arc<str> = match &heatmap.label.formatter {
                Some(formatter) => formatter.format(cell.value),
                None => scale::format_number(
                    cell.value,
                    if cell.value.fract() == 0.0 { 1.0 } else { 0.01 },
                )
                .into(),
            };
            let rect = text_rect(
                input.measure,
                &text,
                theme.font_size,
                [cx, cy],
                TextAlign::Center,
                TextAlign::Center,
            );
            let clipped = [
                x0.max(plot[0]),
                y0.max(plot[1]),
                x1.min(plot[2]),
                y1.min(plot[3]),
            ];
            if rect[0] >= clipped[0]
                && rect[2] <= clipped[2]
                && rect[1] >= clipped[1]
                && rect[3] <= clipped[3]
            {
                let lum = 0.2126 * fill[0] + 0.7152 * fill[1] + 0.0722 * fill[2];
                texts.push(ChartText {
                    rect,
                    text,
                    color: if lum < 0.45 {
                        theme.on_series
                    } else {
                        theme.text
                    },
                    size: theme.font_size,
                    weight: None,
                });
            }
        }
        let x_name = axis_value_text(x_axis, x_val, input.option, None);
        let y_name = axis_value_text(y_axis, y_val, input.option, None);
        hit.items.push(ItemHit {
            region: ItemRegion::Rect([
                x0.max(plot[0]),
                y0.max(plot[1]),
                x1.min(plot[2]),
                y1.min(plot[3]),
            ]),
            series,
            index: index as u32,
            title: Some(format!("{x_name} / {y_name}").into()),
            row: Some(heatmap.name.clone()),
            value: cell.value,
            color: fill,
            details: Vec::new(),
        });
    }
    builder.close_shapes();
}

/// The slider: its track, a faint preview of the first series, the window
/// and its two handles.
fn layout_slider(
    input: &LayoutInput<'_>,
    preview: Option<&[Placed<'_>]>,
    track: [f32; 4],
    window: [f32; 4],
    vertical: bool,
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
    if let Some(data) = preview
        .and_then(|placed| placed.first())
        .and_then(|p| series_data(p.series))
    {
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
    let ends = if vertical {
        let x = (window[0] + window[2]) * 0.5;
        [[x, window[1]], [x, window[3]]]
    } else {
        let y = (window[1] + window[3]) * 0.5;
        [[window[0], y], [window[2], y]]
    };
    for center in ends {
        let mut handle = GpuShape::new(ShapeKind::Symbol, 0, handle_style, u32::MAX, 0);
        let size = if vertical {
            window[2] - window[0]
        } else {
            window[3] - window[1]
        } * 0.7;
        handle.to = [center[0], center[1], size, 0.0];
        handle.from = handle.to;
        handle.extra = [1.0, SymbolShape::RoundRect as u32 as f32, size, 0.0];
        builder.shape(handle, key, 0);
    }
    builder.close_shapes();
}
