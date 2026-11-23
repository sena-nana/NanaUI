# Chart

`Chart` draws charts: line, area, bar, pie and donut, rose, scatter, radar and gauge. You give it a `ChartOption` whose structure matches ECharts' `option` (`grid`, `x_axis`, `y_axis`, `series`, `tooltip`, `legend`, `data_zoom`), with Rust types as values. You provide the data, names and number formats; the control handles geometry, animation and interaction.

This type is behind the Cargo feature `charts`. `components` turns it on. There is no `<Chart>` in the control table: the option is typed data and goes through Rust only.

## Basic usage

:::api

```rust view
use nana_ui::runtime::chart::{Axis, BarSeries, ChartOption, Legend, LineSeries};
use nana_ui::runtime::view;
use nana_ui::runtime::Chart;

let option = ChartOption::new()
    .legend(Legend::default())
    .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
    .y_axis(Axis::value())
    .series(BarSeries::new("Visits", [120.0, 200.0, 150.0, 80.0, 70.0]))
    .series(LineSeries::new("Conversions", [80.0, 132.0, 101.0, 134.0, 90.0]).smooth(true));

view! {
    <Widget of={Chart::new(option).label("Visits this week")} />
}
```

```rust rust
use nana_ui::runtime::chart::{Axis, BarSeries, ChartOption, Legend, LineSeries};
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Chart;

let option = ChartOption::new()
    .legend(Legend::default())
    .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
    .y_axis(Axis::value())
    .series(BarSeries::new("Visits", [120.0, 200.0, 150.0, 80.0, 70.0]))
    .series(LineSeries::new("Conversions", [80.0, 132.0, 101.0, 134.0, 90.0]).smooth(true));

widget(Chart::new(option).label("Visits this week"))
```

:::

The width fills the parent by default and the height defaults to 300px. If you set a size, your size is used.

## Series

| Series | Use | Description |
| --- | --- | --- |
| `LineSeries` | Line and area | `.smooth(true)` is monotone cubic interpolation and never overshoots the data. `.step(..)`, `.area(AreaStyle)`, `.stack(..)`, `.line_type(..)`, `.symbol(..)`, `.sampling(..)` |
| `BarSeries` | Bar | Side by side within a category; series with the same `.stack(..)` stack together (positive and negative stack separately). The `.border_radius(..)` corners are on the end away from the baseline. Horizontal when the y axis is the category axis |
| `PieSeries` | Pie, donut, rose | `.ring(inner, outer)`, `.rose(..)`, `.pad(px)`, `.corner_radius(px)`, `.label(..)`. Outer labels have leader lines, and the left and right sides avoid collisions independently |
| `ScatterSeries` | Scatter | `[x, y]` points. `.sizes(..)` sets per-point size. Even tens of thousands of points are one draw |
| `RadarSeries` | Radar | Needs `ChartOption::radar(RadarCoord)` to supply the indicators |
| `GaugeSeries` | Gauge | `.range(..)`, `.angles(..)`, progress arc, ticks and pointer |

Data columns use `SeriesData`: a set of values matched to category indices (`[f64; N]`, `Vec<f64>`), or `[x, y]` point pairs (`Vec<[f64; 2]>`) for value and time axes. `NaN` is a gap; lines break at gaps (`.connect_nulls(true)` joins them).

## Axes

`Axis::category(names)`, `Axis::value()`, `Axis::time()` (Unix milliseconds), `Axis::log()`. Value axes round to readable ticks following ECharts' rules and include 0 by default; `.scale(true)` fits the data only. `.min(..)` / `.max(..)` fix one end. When category labels do not fit, every few are shown; when value labels overlap, the later one is hidden. `.formatter(..)` and `.time_formatter(..)` are where you localize.

`Grid` gives the margins around the plot area, measured by default to the outer edge of the labels (`contain_label`).

## Tooltip, axis pointer and legend

On line and bar charts the pointer triggers by category: all series of the same category are listed together in the tooltip, and a vertical line (for bars, a category-wide shade) marks the position. Pie, scatter, radar and gauge trigger on the item under the pointer. `Tooltip::trigger(..)`, `.axis_pointer(..)`, `.value_formatter(..)` and `.formatter(..)` change these behaviors, and `Tooltip::hidden()` turns the tooltip off.

The hovered item is emphasized: points grow, pie sectors move outward, bars brighten; when a series sets `.focus(EmphasisFocus::Series)`, the other series fade. The emphasis transition is sampled in the shader on the motion clock, with no relayout.

`Legend` lists the series (pie charts list sectors). A click toggles visibility, and the remaining series relay out with a transition.

## Zoom

`DataZoom::inside()`: wheel zoom inside the plot area (centered on the pointer), drag to pan. `DataZoom::slider()`: a track with a preview; drag the window or its ends. `DataZoom::select()`: drag a box in the plot area and release to zoom to it; a double click goes back to the initial window. `.range(start, end)` is the initial window (percent). When the window moves the chart redraws immediately, without a transition.

Zoom works on the x axis by default; `.y_axis(0)` moves it to the y axis, counted by position on screen like ECharts' `yAxisIndex` (on a horizontal bar chart the x axis is the value axis). A y axis' slider sits to the right of the chart. Several zoom entries on the same axis share one window, so the wheel and the slider always agree. With a `DataZoom::select()` on both x and y, box selection zooms both directions:

```rust
use nana_ui::runtime::chart::{Axis, ChartOption, DataZoom, LineSeries};

let option = ChartOption::new()
    .x_axis(Axis::category(["00:00", "00:05", "00:10", "00:15", "00:20"]))
    .y_axis(Axis::value())
    .data_zoom(DataZoom::select())
    .data_zoom(DataZoom::select().y_axis(0))
    .series(LineSeries::new("Latency", [12.0, 18.0, 95.0, 22.0, 15.0]));
```

## Mark lines and mark areas

Line, bar and scatter series can carry `mark_line(MarkLine)` and `mark_area(MarkArea)`: alert thresholds, average lines, normal ranges, incident windows. Mark areas are drawn under the series and mark lines over them, in the series color by default; mark lines are 1px dashes.

```rust
use nana_ui::runtime::chart::{
    Axis, ChartOption, LineSeries, MarkArea, MarkAreaItem, MarkLine, MarkLineItem,
};

let option = ChartOption::new()
    .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
    .y_axis(Axis::value())
    .series(
        LineSeries::new("CPU", [32.0, 48.0, 71.0, 55.0, 40.0])
            .mark_line(MarkLine::new([
                MarkLineItem::y(90.0).name("Alert"),
                MarkLineItem::average(),
            ]))
            .mark_area(MarkArea::new([MarkAreaItem::y(30.0, 60.0).name("Normal")])),
    );
```

- `MarkLineItem::x(..)` / `::y(..)` are vertical and horizontal lines (category indices on a category axis), `min()` / `max()` / `average()` are statistic lines over the series' values, and `segment(..)` is a segment between two points.
- `MarkAreaItem::y(a, b)` is a horizontal band, `::x(a, b)` a vertical band, `::rect(..)` a rectangle.
- The value axis takes in thresholds and band edges, so a threshold above the data is still visible.
- A mark line writes its name at its end, or its value when it has no name; a mark area writes its name in its top-left corner.
- Near a mark line the tooltip shows its name and value and the line thickens; inside a mark area the area deepens, and an axis tooltip lists the areas it is in after the series.

## Animation

On first appearance, lines unfold from left to right, bars grow from the baseline, sectors sweep from the start angle, and scatter and radar grow from the center. After that, each time the option changes, elements move from where they are currently shown on screen (mid-transition counts too) to their new positions; newly appearing elements enter with their entrance animation. `Animation` sets duration and easing; the defaults are 1 second for entrance and 0.3 seconds for updates, both cubic-out. When the number of data items exceeds `threshold` (default 2000), there is no animation. `Animation::disabled()` turns it off.

For live curves that update many times per second (frame time, levels), turn animation off: each option change restarts an update transition.

## Events

`ChartEvent::Click { series, index }`: pressed on an item and released in place. `ChartEvent::LegendSelect { name, selected }`: a legend toggle. `ChartEvent::DataZoom { index, start, end }`: the zoom window changed (once for each entry on that axis). Receive them with `.on(|e: &ChartEvent| ..)`.

## Colors

Series colors default to the chart palette: the first is the theme's accent color, and the rest change hue at the same perceptual lightness and chroma, a little brighter in the dark theme. `ChartColor::Role(..)` uses a theme role, `ChartColor::Palette(i)` uses the i-th palette color, and `ChartColor::Rgba(..)` hard-codes a color. Grid lines, axis lines and text follow the theme.

## Performance

Layout runs once only when the option, legend/zoom state, size or theme changes, and the result is cached by those four. Pointer movement only queries the existing layout. Mark arrays (points, shapes, styles) stay resident on the GPU by version: frames where the picture does not change upload nothing, and animation and emphasis rely only on the motion clock, with no per-frame CPU work. Lines can first be downsampled to pixel width with `Sampling::Lttb` / `Sampling::MinMax`.

## Accessibility

The role is image. `.label(..)` is the accessible name; write a one-sentence summary, for example "Visits this week, highest on Tuesday".

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `option` | `Arc<ChartOption>` | The whole chart. Swapping in a new one is an update |
| `.label` | `Arc<str>` | Accessible name |
| `.style` | `NodeStyle` | Layout style such as size |
| `view` | `ChartViewState` | Series hidden by the legend and the zoom window: state after user interaction |

## Events

| Event | Description |
| --- | --- |
| `ChartEvent` | Click, legend toggle, zoom |

## Slots

There are no slots.

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md) · [Chart model and rendering](../reference/charts.md)
