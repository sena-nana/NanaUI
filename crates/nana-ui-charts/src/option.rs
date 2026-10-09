//! The chart option: what a chart shows, shaped after ECharts' `option`.
//!
//! The structure and names follow ECharts (`grid`, `xAxis`, `series`,
//! `tooltip`, `legend`, `dataZoom`) so an ECharts option reads across, but
//! the values are Rust types: lengths are [`Length`], colors are
//! [`ChartColor`] (theme roles by default), formatters are functions and data
//! columns are shared slices. Applications own the values and every label;
//! the chart owns geometry, motion and interaction.

use std::fmt;
use std::sync::Arc;

use nana_ui_core::{Easing, SemanticColorRole};

/// A value that is either absolute logical px or a fraction of a reference
/// (ECharts `'50%'`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    Px(f32),
    /// `0.0..=1.0` of the reference length.
    Percent(f32),
}

impl Length {
    pub fn resolve(self, reference: f32) -> f32 {
        match self {
            Self::Px(px) => px,
            Self::Percent(fraction) => fraction * reference,
        }
    }
}

/// A color: a theme role, the chart's categorical palette, or explicit sRGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChartColor {
    Role(SemanticColorRole),
    /// Entry of the chart palette (wraps around).
    Palette(usize),
    /// Straight-alpha sRGB, `0.0..=1.0`.
    Rgba([f32; 4]),
}

/// Shared formatter for numbers on axes, labels and tooltips.
#[derive(Clone)]
pub struct ValueFormatter(pub Arc<dyn Fn(f64) -> Arc<str> + Send + Sync>);

impl ValueFormatter {
    pub fn new(format: impl Fn(f64) -> Arc<str> + Send + Sync + 'static) -> Self {
        Self(Arc::new(format))
    }

    pub fn format(&self, value: f64) -> Arc<str> {
        (self.0)(value)
    }
}

impl fmt::Debug for ValueFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ValueFormatter(..)")
    }
}

impl PartialEq for ValueFormatter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Formats a time-axis instant (Unix milliseconds) for the given tick level.
#[derive(Clone)]
pub struct TimeFormatter(pub Arc<dyn Fn(i64, TimeUnit) -> Arc<str> + Send + Sync>);

impl TimeFormatter {
    pub fn new(format: impl Fn(i64, TimeUnit) -> Arc<str> + Send + Sync + 'static) -> Self {
        Self(Arc::new(format))
    }
}

impl fmt::Debug for TimeFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TimeFormatter(..)")
    }
}

impl PartialEq for TimeFormatter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The calendar step a time tick falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeUnit {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
}

/// The whole chart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartOption {
    pub grid: Grid,
    pub x_axis: Vec<Axis>,
    pub y_axis: Vec<Axis>,
    pub radar: Option<RadarCoord>,
    pub legend: Option<Legend>,
    pub tooltip: Tooltip,
    pub data_zoom: Vec<DataZoom>,
    /// The categorical palette. Empty uses the theme-derived one.
    pub color: Vec<ChartColor>,
    pub animation: Animation,
    pub series: Vec<Series>,
}

impl ChartOption {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grid(mut self, grid: Grid) -> Self {
        self.grid = grid;
        self
    }

    pub fn x_axis(mut self, axis: Axis) -> Self {
        self.x_axis.push(axis);
        self
    }

    pub fn y_axis(mut self, axis: Axis) -> Self {
        self.y_axis.push(axis);
        self
    }

    pub fn radar(mut self, radar: RadarCoord) -> Self {
        self.radar = Some(radar);
        self
    }

    pub fn legend(mut self, legend: Legend) -> Self {
        self.legend = Some(legend);
        self
    }

    pub fn tooltip(mut self, tooltip: Tooltip) -> Self {
        self.tooltip = tooltip;
        self
    }

    pub fn data_zoom(mut self, zoom: DataZoom) -> Self {
        self.data_zoom.push(zoom);
        self
    }

    pub fn color(mut self, palette: impl IntoIterator<Item = ChartColor>) -> Self {
        self.color = palette.into_iter().collect();
        self
    }

    pub fn animation(mut self, animation: Animation) -> Self {
        self.animation = animation;
        self
    }

    pub fn series(mut self, series: impl Into<Series>) -> Self {
        self.series.push(series.into());
        self
    }

    /// Every theme role a color in this option names.
    pub fn color_roles(&self) -> Vec<SemanticColorRole> {
        let mut roles = Vec::new();
        let mut add = |color: Option<ChartColor>| {
            if let Some(ChartColor::Role(role)) = color
                && !roles.contains(&role)
            {
                roles.push(role);
            }
        };
        for color in &self.color {
            add(Some(*color));
        }
        for series in &self.series {
            add(series.explicit_color());
            match series {
                Series::Line(line) => add(line.area.and_then(|area| area.color)),
                Series::Bar(bar) => bar.item_colors.iter().for_each(|c| add(Some(*c))),
                Series::Pie(pie) => pie.data.iter().for_each(|item| add(item.color)),
                Series::Radar(radar) => radar.data.iter().for_each(|item| add(item.color)),
                Series::Scatter(_) | Series::Gauge(_) => {}
            }
        }
        roles
    }
}

/// The cartesian plot's margins inside the chart box (ECharts `grid`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub left: Option<Length>,
    pub right: Option<Length>,
    pub top: Option<Length>,
    pub bottom: Option<Length>,
    /// Margins are measured to the axis labels' outer edge rather than to
    /// the axis line (ECharts `containLabel`).
    pub contain_label: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            left: None,
            right: None,
            top: None,
            bottom: None,
            contain_label: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisKind {
    /// Discrete, evenly spaced bands named by [`Axis::data`].
    #[default]
    Category,
    Value,
    /// Unix milliseconds.
    Time,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisPosition {
    Bottom,
    Top,
    Left,
    Right,
}

/// An end of a value axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AxisBound {
    /// Nice-rounded from the data (and from 0 unless [`Axis::scale`]).
    #[default]
    Auto,
    Value(f64),
    DataMin,
    DataMax,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisLabel {
    pub show: bool,
    pub formatter: Option<ValueFormatter>,
    pub time_formatter: Option<TimeFormatter>,
    /// Show every n-th category label. `None` picks the interval that keeps
    /// labels from overlapping.
    pub interval: Option<usize>,
    /// Drop labels that would overlap a neighbour (value and time axes).
    pub hide_overlap: bool,
}

impl Default for AxisLabel {
    fn default() -> Self {
        Self {
            show: true,
            formatter: None,
            time_formatter: None,
            interval: None,
            hide_overlap: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub kind: AxisKind,
    /// Category names, in order.
    pub data: Arc<[Arc<str>]>,
    pub position: Option<AxisPosition>,
    pub min: AxisBound,
    pub max: AxisBound,
    /// Do not force zero into a value axis (ECharts `scale`).
    pub scale: bool,
    /// Leave half a band of room at each end of a category axis. `None`
    /// uses the series' preference: bars want it, lines do not.
    pub boundary_gap: Option<bool>,
    /// The number of intervals the axis aims for.
    pub split_number: usize,
    pub inverse: bool,
    pub log_base: f64,
    pub name: Option<Arc<str>>,
    pub axis_label: AxisLabel,
    /// `None`: value axes draw split lines, category axes do not.
    pub split_line: Option<bool>,
    pub axis_line: bool,
    pub axis_tick: bool,
    pub show: bool,
}

impl Default for Axis {
    fn default() -> Self {
        Self {
            kind: AxisKind::Category,
            data: Arc::from([]),
            position: None,
            min: AxisBound::Auto,
            max: AxisBound::Auto,
            scale: false,
            boundary_gap: None,
            split_number: 5,
            inverse: false,
            log_base: 10.0,
            name: None,
            axis_label: AxisLabel::default(),
            split_line: None,
            axis_line: true,
            axis_tick: true,
            show: true,
        }
    }
}

impl Axis {
    pub fn category(data: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        Self {
            kind: AxisKind::Category,
            data: data.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    pub fn value() -> Self {
        Self {
            kind: AxisKind::Value,
            ..Self::default()
        }
    }

    pub fn time() -> Self {
        Self {
            kind: AxisKind::Time,
            ..Self::default()
        }
    }

    pub fn log() -> Self {
        Self {
            kind: AxisKind::Log,
            ..Self::default()
        }
    }

    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn min(mut self, bound: AxisBound) -> Self {
        self.min = bound;
        self
    }

    pub fn max(mut self, bound: AxisBound) -> Self {
        self.max = bound;
        self
    }

    pub fn scale(mut self, scale: bool) -> Self {
        self.scale = scale;
        self
    }

    pub fn boundary_gap(mut self, gap: bool) -> Self {
        self.boundary_gap = Some(gap);
        self
    }

    pub fn split_number(mut self, count: usize) -> Self {
        self.split_number = count.max(1);
        self
    }

    pub fn inverse(mut self, inverse: bool) -> Self {
        self.inverse = inverse;
        self
    }

    pub fn name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn formatter(mut self, formatter: ValueFormatter) -> Self {
        self.axis_label.formatter = Some(formatter);
        self
    }

    pub fn time_formatter(mut self, formatter: TimeFormatter) -> Self {
        self.axis_label.time_formatter = Some(formatter);
        self
    }

    pub fn split_line(mut self, show: bool) -> Self {
        self.split_line = Some(show);
        self
    }

    pub fn show(mut self, show: bool) -> Self {
        self.show = show;
        self
    }
}

/// One radar indicator: a spoke with its own range.
#[derive(Debug, Clone, PartialEq)]
pub struct RadarIndicator {
    pub name: Arc<str>,
    pub max: Option<f64>,
    pub min: Option<f64>,
}

impl RadarIndicator {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            max: None,
            min: None,
        }
    }

    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadarShape {
    #[default]
    Polygon,
    Circle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadarCoord {
    pub indicators: Vec<RadarIndicator>,
    pub center: [Length; 2],
    pub radius: Length,
    pub start_angle: f32,
    pub split_number: usize,
    pub shape: RadarShape,
    /// Alternate the rings' fill (ECharts `splitArea`).
    pub split_area: bool,
}

impl Default for RadarCoord {
    fn default() -> Self {
        Self {
            indicators: Vec::new(),
            center: [Length::Percent(0.5), Length::Percent(0.5)],
            radius: Length::Percent(0.75),
            start_angle: 90.0,
            split_number: 5,
            shape: RadarShape::Polygon,
            split_area: true,
        }
    }
}

impl RadarCoord {
    pub fn new(indicators: impl IntoIterator<Item = RadarIndicator>) -> Self {
        Self {
            indicators: indicators.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn shape(mut self, shape: RadarShape) -> Self {
        self.shape = shape;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegendPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Legend {
    pub show: bool,
    pub position: LegendPosition,
    /// Clicking an item toggles its series (pie: its slice).
    pub selectable: bool,
}

impl Default for Legend {
    fn default() -> Self {
        Self {
            show: true,
            position: LegendPosition::Top,
            selectable: true,
        }
    }
}

impl Legend {
    pub fn position(mut self, position: LegendPosition) -> Self {
        self.position = position;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipTrigger {
    /// The item under the pointer (default for pie, scatter, radar, gauge).
    Item,
    /// Every series at the hovered category / x (default for line and bar).
    Axis,
    None,
    /// Axis for cartesian-only charts, item otherwise.
    #[default]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisPointer {
    /// A line for value axes, a band shadow for bar categories.
    #[default]
    Auto,
    Line,
    Shadow,
    Cross,
    None,
}

/// What a tooltip formatter is given.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipItem {
    pub series_index: usize,
    pub series_name: Arc<str>,
    pub data_index: usize,
    /// The category or item name.
    pub name: Arc<str>,
    pub value: f64,
    pub color: [f32; 4],
}

/// A tooltip formatter's function: hovered items to `(title, rows)`.
pub type TooltipFormat = dyn Fn(&[TooltipItem]) -> (Arc<str>, Vec<Arc<str>>) + Send + Sync;

/// Builds a tooltip's text from the hovered items. Returns `(title, rows)`.
#[derive(Clone)]
pub struct TooltipFormatter(pub Arc<TooltipFormat>);

impl TooltipFormatter {
    pub fn new(
        format: impl Fn(&[TooltipItem]) -> (Arc<str>, Vec<Arc<str>>) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(format))
    }
}

impl fmt::Debug for TooltipFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TooltipFormatter(..)")
    }
}

impl PartialEq for TooltipFormatter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tooltip {
    pub show: Option<bool>,
    pub trigger: TooltipTrigger,
    pub axis_pointer: AxisPointer,
    pub value_formatter: Option<ValueFormatter>,
    pub formatter: Option<TooltipFormatter>,
}

impl Tooltip {
    pub fn trigger(mut self, trigger: TooltipTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn axis_pointer(mut self, pointer: AxisPointer) -> Self {
        self.axis_pointer = pointer;
        self
    }

    pub fn value_formatter(mut self, formatter: ValueFormatter) -> Self {
        self.value_formatter = Some(formatter);
        self
    }

    pub fn formatter(mut self, formatter: TooltipFormatter) -> Self {
        self.formatter = Some(formatter);
        self
    }

    pub fn hidden() -> Self {
        Self {
            show: Some(false),
            trigger: TooltipTrigger::None,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataZoomKind {
    /// Wheel zooms and drag pans inside the plot.
    #[default]
    Inside,
    /// A range bar under the plot with draggable handles.
    Slider,
}

/// A window over an axis, in percent of the full data extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DataZoom {
    pub kind: DataZoomKind,
    pub x_axis_index: usize,
    /// `0.0..=100.0`.
    pub start: f64,
    pub end: f64,
    pub zoom_on_wheel: bool,
    pub move_on_drag: bool,
}

impl Default for DataZoom {
    fn default() -> Self {
        Self {
            kind: DataZoomKind::Inside,
            x_axis_index: 0,
            start: 0.0,
            end: 100.0,
            zoom_on_wheel: true,
            move_on_drag: true,
        }
    }
}

impl DataZoom {
    pub fn inside() -> Self {
        Self::default()
    }

    pub fn slider() -> Self {
        Self {
            kind: DataZoomKind::Slider,
            ..Self::default()
        }
    }

    pub fn range(mut self, start: f64, end: f64) -> Self {
        self.start = start.clamp(0.0, 100.0);
        self.end = end.clamp(self.start, 100.0);
        self
    }
}

/// Entry and update motion (ECharts `animation*`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    pub enabled: bool,
    /// Entry duration, seconds.
    pub duration: f32,
    pub easing: Easing,
    /// Data-update duration, seconds.
    pub duration_update: f32,
    pub easing_update: Easing,
    /// Above this many drawn elements a chart does not animate.
    pub threshold: usize,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            enabled: true,
            duration: 1.0,
            easing: Easing::EaseOutCubic,
            duration_update: 0.3,
            easing_update: Easing::EaseOutCubic,
            threshold: 2000,
        }
    }
}

impl Animation {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

/// Mark shapes for points, legend icons and scatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Symbol {
    /// A ring filled with the chart's surface color: ECharts' line default.
    #[default]
    EmptyCircle,
    Circle,
    Rect,
    RoundRect,
    Triangle,
    Diamond,
    Pin,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineType {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Start,
    Middle,
    End,
}

/// Fill under a line (ECharts `areaStyle`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaStyle {
    pub color: Option<ChartColor>,
    pub opacity: f32,
    /// Fade the fill to transparent towards the base.
    pub gradient: bool,
}

impl Default for AreaStyle {
    fn default() -> Self {
        Self {
            color: None,
            opacity: 0.25,
            gradient: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sampling {
    #[default]
    None,
    /// Largest-Triangle-Three-Buckets down to about one point per px.
    Lttb,
    /// Each px column keeps its minimum and maximum.
    MinMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmphasisFocus {
    /// Highlight the hovered item only.
    #[default]
    None,
    /// Fade every other series.
    Series,
}

/// A data column for cartesian series.
#[derive(Debug, Clone)]
pub enum SeriesData {
    /// One value per category (or per index on a value x axis). NaN is a gap.
    Values(Arc<[f64]>),
    /// `(x, y)` pairs for value and time x axes. NaN y is a gap.
    Points(Arc<[[f64; 2]]>),
}

impl PartialEq for SeriesData {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Values(a), Self::Values(b)) => Arc::ptr_eq(a, b) || a == b,
            (Self::Points(a), Self::Points(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
    }
}

impl Default for SeriesData {
    fn default() -> Self {
        Self::Values(Arc::from([]))
    }
}

impl SeriesData {
    pub fn len(&self) -> usize {
        match self {
            Self::Values(values) => values.len(),
            Self::Points(points) => points.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `(x, y)` of item `index`; for [`Self::Values`] x is the index.
    pub fn get(&self, index: usize) -> Option<(f64, f64)> {
        match self {
            Self::Values(values) => values.get(index).map(|y| (index as f64, *y)),
            Self::Points(points) => points.get(index).map(|[x, y]| (*x, *y)),
        }
    }
}

impl From<Vec<f64>> for SeriesData {
    fn from(values: Vec<f64>) -> Self {
        Self::Values(values.into())
    }
}

impl<const N: usize> From<[f64; N]> for SeriesData {
    fn from(values: [f64; N]) -> Self {
        Self::Values(Arc::from(values.as_slice()))
    }
}

impl From<Vec<[f64; 2]>> for SeriesData {
    fn from(points: Vec<[f64; 2]>) -> Self {
        Self::Points(points.into())
    }
}

/// Labels drawn on the marks themselves.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkLabel {
    pub show: bool,
    pub formatter: Option<ValueFormatter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineSeries {
    pub name: Arc<str>,
    pub data: SeriesData,
    pub color: Option<ChartColor>,
    pub x_axis_index: usize,
    pub y_axis_index: usize,
    pub stack: Option<Arc<str>>,
    /// Monotone cubic interpolation (ECharts `smooth` + `smoothMonotone`).
    pub smooth: bool,
    pub step: Option<Step>,
    pub width: f32,
    pub line_type: LineType,
    pub area: Option<AreaStyle>,
    pub symbol: Symbol,
    pub symbol_size: f32,
    /// `None` shows symbols while there is room for them.
    pub show_symbol: Option<bool>,
    pub connect_nulls: bool,
    pub sampling: Sampling,
    pub label: MarkLabel,
    pub focus: EmphasisFocus,
}

impl LineSeries {
    pub fn new(name: impl Into<Arc<str>>, data: impl Into<SeriesData>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            color: None,
            x_axis_index: 0,
            y_axis_index: 0,
            stack: None,
            smooth: false,
            step: None,
            width: 2.0,
            line_type: LineType::Solid,
            area: None,
            symbol: Symbol::EmptyCircle,
            symbol_size: 4.0,
            show_symbol: None,
            connect_nulls: false,
            sampling: Sampling::None,
            label: MarkLabel::default(),
            focus: EmphasisFocus::None,
        }
    }

    pub fn color(mut self, color: ChartColor) -> Self {
        self.color = Some(color);
        self
    }

    pub fn stack(mut self, stack: impl Into<Arc<str>>) -> Self {
        self.stack = Some(stack.into());
        self
    }

    pub fn smooth(mut self, smooth: bool) -> Self {
        self.smooth = smooth;
        self
    }

    pub fn step(mut self, step: Step) -> Self {
        self.step = Some(step);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    pub fn line_type(mut self, line_type: LineType) -> Self {
        self.line_type = line_type;
        self
    }

    pub fn area(mut self, area: AreaStyle) -> Self {
        self.area = Some(area);
        self
    }

    pub fn symbol(mut self, symbol: Symbol, size: f32) -> Self {
        self.symbol = symbol;
        self.symbol_size = size.max(0.0);
        self
    }

    pub fn show_symbol(mut self, show: bool) -> Self {
        self.show_symbol = Some(show);
        self
    }

    pub fn sampling(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    pub fn y_axis_index(mut self, index: usize) -> Self {
        self.y_axis_index = index;
        self
    }

    pub fn focus(mut self, focus: EmphasisFocus) -> Self {
        self.focus = focus;
        self
    }

    pub fn connect_nulls(mut self, connect: bool) -> Self {
        self.connect_nulls = connect;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BarSeries {
    pub name: Arc<str>,
    pub data: SeriesData,
    pub color: Option<ChartColor>,
    /// Per-item colors (wraps around). Empty uses the series color.
    pub item_colors: Vec<ChartColor>,
    pub x_axis_index: usize,
    pub y_axis_index: usize,
    pub stack: Option<Arc<str>>,
    pub bar_width: Option<Length>,
    pub bar_max_width: Option<f32>,
    /// Between bars of one category, as a fraction of a bar's width.
    pub bar_gap: f32,
    /// Between categories, as a fraction of the band.
    pub bar_category_gap: f32,
    /// Corner radii, `[top-left, top-right, bottom-right, bottom-left]` of
    /// a bar growing upward (mirrored for negative and horizontal bars).
    pub border_radius: [f32; 4],
    pub show_background: bool,
    pub label: MarkLabel,
    pub focus: EmphasisFocus,
}

impl BarSeries {
    pub fn new(name: impl Into<Arc<str>>, data: impl Into<SeriesData>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            color: None,
            item_colors: Vec::new(),
            x_axis_index: 0,
            y_axis_index: 0,
            stack: None,
            bar_width: None,
            bar_max_width: None,
            bar_gap: 0.3,
            bar_category_gap: 0.2,
            border_radius: [0.0; 4],
            show_background: false,
            label: MarkLabel::default(),
            focus: EmphasisFocus::None,
        }
    }

    pub fn color(mut self, color: ChartColor) -> Self {
        self.color = Some(color);
        self
    }

    pub fn item_colors(mut self, colors: impl IntoIterator<Item = ChartColor>) -> Self {
        self.item_colors = colors.into_iter().collect();
        self
    }

    pub fn stack(mut self, stack: impl Into<Arc<str>>) -> Self {
        self.stack = Some(stack.into());
        self
    }

    pub fn bar_width(mut self, width: Length) -> Self {
        self.bar_width = Some(width);
        self
    }

    pub fn bar_max_width(mut self, width: f32) -> Self {
        self.bar_max_width = Some(width);
        self
    }

    pub fn border_radius(mut self, radius: [f32; 4]) -> Self {
        self.border_radius = radius;
        self
    }

    pub fn show_background(mut self, show: bool) -> Self {
        self.show_background = show;
        self
    }

    pub fn label(mut self, label: MarkLabel) -> Self {
        self.label = label;
        self
    }

    pub fn focus(mut self, focus: EmphasisFocus) -> Self {
        self.focus = focus;
        self
    }

    pub fn y_axis_index(mut self, index: usize) -> Self {
        self.y_axis_index = index;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PieItem {
    pub name: Arc<str>,
    pub value: f64,
    pub color: Option<ChartColor>,
}

impl PieItem {
    pub fn new(name: impl Into<Arc<str>>, value: f64) -> Self {
        Self {
            name: name.into(),
            value,
            color: None,
        }
    }

    pub fn color(mut self, color: ChartColor) -> Self {
        self.color = Some(color);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoseType {
    /// Angle by value, radius by value.
    Radius,
    /// Equal angles, radius by value.
    Area,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PieLabelPosition {
    /// Outside the ring with a guide line.
    #[default]
    Outside,
    Inside,
    /// The hovered slice's name in the hole.
    Center,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PieSeries {
    pub name: Arc<str>,
    pub data: Arc<[PieItem]>,
    pub center: [Length; 2],
    /// Inner and outer radius. A zero inner radius is a pie, else a ring.
    pub radius: [Length; 2],
    pub rose: Option<RoseType>,
    /// Degrees, counter-clockwise from 3 o'clock (ECharts).
    pub start_angle: f32,
    pub clockwise: bool,
    /// Gap between slices, logical px.
    pub pad: f32,
    pub corner_radius: f32,
    pub min_angle: f32,
    pub label: PieLabelPosition,
    pub label_formatter: Option<ValueFormatter>,
}

impl PieSeries {
    pub fn new(name: impl Into<Arc<str>>, data: impl IntoIterator<Item = PieItem>) -> Self {
        Self {
            name: name.into(),
            data: data.into_iter().collect(),
            center: [Length::Percent(0.5), Length::Percent(0.5)],
            radius: [Length::Px(0.0), Length::Percent(0.75)],
            rose: None,
            start_angle: 90.0,
            clockwise: true,
            pad: 2.0,
            corner_radius: 0.0,
            min_angle: 0.0,
            label: PieLabelPosition::Outside,
            label_formatter: None,
        }
    }

    /// Inner and outer radius as fractions of half the smaller side.
    pub fn ring(mut self, inner: f32, outer: f32) -> Self {
        self.radius = [Length::Percent(inner), Length::Percent(outer)];
        self
    }

    pub fn rose(mut self, rose: RoseType) -> Self {
        self.rose = Some(rose);
        self
    }

    pub fn center(mut self, x: Length, y: Length) -> Self {
        self.center = [x, y];
        self
    }

    pub fn pad(mut self, pad: f32) -> Self {
        self.pad = pad.max(0.0);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius.max(0.0);
        self
    }

    pub fn start_angle(mut self, degrees: f32) -> Self {
        self.start_angle = degrees;
        self
    }

    pub fn label(mut self, position: PieLabelPosition) -> Self {
        self.label = position;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScatterSeries {
    pub name: Arc<str>,
    pub data: SeriesData,
    pub color: Option<ChartColor>,
    pub x_axis_index: usize,
    pub y_axis_index: usize,
    pub symbol: Symbol,
    pub symbol_size: f32,
    /// Per-point size from a third value (`[x, y, size]`), when given.
    pub sizes: Option<Arc<[f32]>>,
    pub opacity: f32,
    pub focus: EmphasisFocus,
}

impl ScatterSeries {
    pub fn new(name: impl Into<Arc<str>>, data: impl Into<SeriesData>) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            color: None,
            x_axis_index: 0,
            y_axis_index: 0,
            symbol: Symbol::Circle,
            symbol_size: 10.0,
            sizes: None,
            opacity: 0.8,
            focus: EmphasisFocus::None,
        }
    }

    pub fn color(mut self, color: ChartColor) -> Self {
        self.color = Some(color);
        self
    }

    pub fn symbol(mut self, symbol: Symbol, size: f32) -> Self {
        self.symbol = symbol;
        self.symbol_size = size.max(0.0);
        self
    }

    pub fn sizes(mut self, sizes: impl Into<Arc<[f32]>>) -> Self {
        self.sizes = Some(sizes.into());
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadarItem {
    pub name: Arc<str>,
    pub values: Arc<[f64]>,
    pub color: Option<ChartColor>,
}

impl RadarItem {
    pub fn new(name: impl Into<Arc<str>>, values: impl Into<Arc<[f64]>>) -> Self {
        Self {
            name: name.into(),
            values: values.into(),
            color: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadarSeries {
    pub name: Arc<str>,
    pub data: Vec<RadarItem>,
    pub area: Option<AreaStyle>,
    pub width: f32,
    pub symbol: Symbol,
    pub symbol_size: f32,
}

impl RadarSeries {
    pub fn new(name: impl Into<Arc<str>>, data: impl IntoIterator<Item = RadarItem>) -> Self {
        Self {
            name: name.into(),
            data: data.into_iter().collect(),
            area: None,
            width: 2.0,
            symbol: Symbol::EmptyCircle,
            symbol_size: 4.0,
        }
    }

    pub fn area(mut self, area: AreaStyle) -> Self {
        self.area = Some(area);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaugeSeries {
    pub name: Arc<str>,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub center: [Length; 2],
    pub radius: Length,
    /// Degrees, counter-clockwise from 3 o'clock.
    pub start_angle: f32,
    pub end_angle: f32,
    pub split_number: usize,
    /// Track and progress width, logical px.
    pub width: f32,
    pub color: Option<ChartColor>,
    pub progress: bool,
    pub pointer: bool,
    pub ticks: bool,
    pub labels: bool,
    pub detail_formatter: Option<ValueFormatter>,
}

impl GaugeSeries {
    pub fn new(name: impl Into<Arc<str>>, value: f64) -> Self {
        Self {
            name: name.into(),
            value,
            min: 0.0,
            max: 100.0,
            center: [Length::Percent(0.5), Length::Percent(0.55)],
            radius: Length::Percent(0.75),
            start_angle: 225.0,
            end_angle: -45.0,
            split_number: 10,
            width: 10.0,
            color: None,
            progress: true,
            pointer: true,
            ticks: true,
            labels: true,
            detail_formatter: None,
        }
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn angles(mut self, start: f32, end: f32) -> Self {
        self.start_angle = start;
        self.end_angle = end;
        self
    }

    pub fn color(mut self, color: ChartColor) -> Self {
        self.color = Some(color);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    pub fn pointer(mut self, show: bool) -> Self {
        self.pointer = show;
        self
    }

    pub fn detail_formatter(mut self, formatter: ValueFormatter) -> Self {
        self.detail_formatter = Some(formatter);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Series {
    Line(LineSeries),
    Bar(BarSeries),
    Pie(PieSeries),
    Scatter(ScatterSeries),
    Radar(RadarSeries),
    Gauge(GaugeSeries),
}

impl Series {
    pub fn name(&self) -> &Arc<str> {
        match self {
            Self::Line(series) => &series.name,
            Self::Bar(series) => &series.name,
            Self::Pie(series) => &series.name,
            Self::Scatter(series) => &series.name,
            Self::Radar(series) => &series.name,
            Self::Gauge(series) => &series.name,
        }
    }

    pub fn is_cartesian(&self) -> bool {
        matches!(self, Self::Line(_) | Self::Bar(_) | Self::Scatter(_))
    }

    pub(crate) fn explicit_color(&self) -> Option<ChartColor> {
        match self {
            Self::Line(series) => series.color,
            Self::Bar(series) => series.color,
            Self::Scatter(series) => series.color,
            Self::Gauge(series) => series.color,
            Self::Pie(_) | Self::Radar(_) => None,
        }
    }
}

impl From<LineSeries> for Series {
    fn from(series: LineSeries) -> Self {
        Self::Line(series)
    }
}

impl From<BarSeries> for Series {
    fn from(series: BarSeries) -> Self {
        Self::Bar(series)
    }
}

impl From<PieSeries> for Series {
    fn from(series: PieSeries) -> Self {
        Self::Pie(series)
    }
}

impl From<ScatterSeries> for Series {
    fn from(series: ScatterSeries) -> Self {
        Self::Scatter(series)
    }
}

impl From<RadarSeries> for Series {
    fn from(series: RadarSeries) -> Self {
        Self::Radar(series)
    }
}

impl From<GaugeSeries> for Series {
    fn from(series: GaugeSeries) -> Self {
        Self::Gauge(series)
    }
}
