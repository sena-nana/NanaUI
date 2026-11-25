# Chart model and rendering (nana-ui-charts)

The `Chart` control has three layers behind it: `nana-ui-charts` lays a `ChartOption` out into labels, hit data and GPU mark arrays; the `Chart` in `nana-ui-runtime` owns interaction state and hands the marks to the Scene; the chart pipelines in `nana-ui` draw them with `chart.wgsl`. This page is the contract between the three. For using the control, see [Chart](../components/chart.md).

## Pipeline

| Stage | Where | Input | Output | Recomputed when |
| --- | --- | --- | --- | --- |
| Layout | `nana_ui_charts::layout` | option, `ChartViewState` (legend, zoom), size, `ChartTheme`, `LabelMeasure` | `ChartLayout`: text, `HitModel`, `ChartMarks`, zoom maps | any of the four inputs changes |
| Transition | `nana_ui_charts::transition` | the new layout, the layout shown before, the motion clock | every element's `from` | a new option or a legend change; zoom lands at once |
| Interaction | `Chart` and `framework/charts.rs` in `nana-ui-runtime` | pointer, wheel, the finished layout | hover, zoom windows, the selection box, `ChartEvent` | every pointer event, without relayout |
| Paint | `scene_paint/chart.rs` + `chart.wgsl` in `nana-ui` | `ChartGpuPayload` (marks and hover) | GPU draws | uploads only when the marks' `revision` changes |

`nana-ui-charts` depends only on `nana-ui-core`: it does not measure text (the caller passes a `LabelMeasure`) and touches neither the retained tree nor the GPU.

## Mark arrays (the GPU contract)

The layouts in `marks.rs` are the storage-buffer layouts of `chart.wgsl`; a field changes in both places or neither.

| Type | Size | Contents |
| --- | --- | --- |
| `GpuPoint` | 48 B | A line point and its base point, both ends `to` / `from`, arc length (dashes) |
| `GpuShape` | 64 B | Rect, sector, symbol, needle; `meta = [kind | flags, style, series, data index]` |
| `GpuStyle` | 64 B | Fill, gradient end, stroke (straight-alpha sRGB, made linear on upload), line width and dashes |
| `MarkDraw` | — | One draw: `pass`, range, style, `series`, `flags`, `neighbours`, `index` |

On the GPU a `MarkDraw` is two `vec4<u32>`: `a = [first, count, style, flags]`, `b = [neighbours, series, pass, index]`.

| Flag | Bit | Effect |
| --- | --- | --- |
| `draw_flags::CLIP_TO_PLOT` | 1 | Cut to the plot |
| `draw_flags::REVEAL` | 2 | Uncovered left to right on entry |
| `draw_flags::EMPHASIS` | 16 | The line widens while its series is hovered |
| `draw_flags::GUIDE` | 32 | Grid and axis lines: never faded with a series |
| `draw_flags::MARK` | 64 | A mark line: widens by 2px only when its own `(series, index)` is hovered |
| `SHAPE_EMPHASIS_GROW` | 512 | Hover grows symbols, pushes sectors out, brightens bars |
| `SHAPE_EMPHASIS_FILL` | 2048 | Hover deepens a rect's fill (mark areas) |

Hover matches on `(series, data index)`. `MARK_LINE_INDEX + k` (from `0x4000_0000`) is a series' k-th mark line and `MARK_AREA_INDEX + k` (from `0x6000_0000`) its k-th mark area; neither meets a data index or the category index the axis trigger uses.

A change to `chart.wgsl` needs the precompiled artifacts regenerated, or release builds on Apple refuse the mismatched stages:

```bash
cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1
```

```bash
cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1
```

## Zoom

A `DataZoom` names its axis by where it sits on screen, as ECharts' `xAxisIndex` / `yAxisIndex` do: `.x_axis(i)` (0 by default) or `.y_axis(i)`, and `target()` returns `(ZoomAxis, index)`. On a horizontal bar chart the x axis is the value axis and the y axis the categories; on a heatmap both axes can be category axes.

- Windows are percent. Every entry on one axis shares a window: layout reads the first entry's, and when the runtime moves a window it sets every entry on that axis to it and emits one `ChartEvent::DataZoom` for each entry that changed.
- On a category/base axis (categories, time, or the value axis series advance along) percent is of the full data extent; category `i` is centred at `(i + 0.5) / count`. A category y axis (such as `HeatmapSeries`) slices its categories by the same rule. On a value axis percent is of the nice-rounded scale over **all** the data, independent of the x window, so a selection box converts on each side on its own. A value axis with a zoom entry no longer fits itself to the x window; with a window other than 0–100 its ends are exactly the window's, not rounded. Log axes do not zoom on their values.
- `ChartLayout::zoom_axes` maps px to percent for every zoomable axis shown (`ZoomAxisLayout::percent_at` / `per_px`); the wheel, drag panning and box selection convert only through it, with no relayout.
- `DataZoomKind::Slider`: an x axis' track sits under the plot (`ChartLayout::slider`, previewing the first series when it is the base axis), a y axis' to the right of it (`ChartLayout::y_slider`, its window growing up from the bottom).
- `DataZoomKind::Inside`: the wheel zooms every axis with `zoom_on_wheel` around the pointer; dragging pans every axis with `move_on_drag`.
- Box selection: when an inside entry sets `select_on_drag` (`DataZoom::select()`), dragging in the plot draws a box, and the release zooms every axis that has a selecting entry to it. With only x zoomable the box spans the plot's height, and the other way round. A box under 8px on every zoomed side is a click. A double click restores the option's initial windows. The runtime draws the box as an ordinary rect (slot 14), not through the mark arrays.

## Mark lines and mark areas

`LineSeries`, `BarSeries`, `ScatterSeries`, `CandlestickSeries`, `BoxplotSeries` and `HeatmapSeries` take `mark_line(MarkLine)` and `mark_area(MarkArea)`, drawn on that series' axes and in its color by default.

| Item | Where |
| --- | --- |
| `MarkLineItem::x(v)` / `::y(v)` | A vertical / horizontal line; on a category axis `v` is a category index |
| `MarkLineItem::min()` / `max()` / `average()` / `stat(MarkStat::Median)` | A statistic of the series' values (NaN left out), across the plot on the value axis (`low`/`high`/`close` for candlestick, `min`/`max`/`median` for boxplot) |
| `MarkLineItem::segment([x, y], [x, y])` | A segment between two points, clipped to the plot |
| `MarkAreaItem::y(a, b)` / `::x(a, b)` / `::rect(..)` | A horizontal band / vertical band / rectangle; a missing side spans the plot |

- Drawing: mark areas go under every series (after the grid lines), mark lines over every series; both are cut to the plot. Mark lines default to 1px dashes. On bars, candlesticks, boxplots and heatmaps (categories with a gap) a mark area covers its first and last categories whole; on a line's category points it runs between them.
- Axis ranges include the fixed values of mark lines and areas (`x` / `y` / segment ends / band edges), so a threshold beyond the data stays in view; statistics are inside the data range anyway.
- Labels: a mark line writes its name at its end, else its value (through `formatter` when given); a mark area writes its name in its top-left corner, and nothing when it does not fit.
- Hit testing: within 4px of a mark line the line wins, with the series name as the tooltip title and one row of its name and value; then the data itself (candlesticks expand to `O/C/L/H` rows and boxplots to `min/Q1/median/Q3/max` by default, overridable via `value_labels`); when the pointer is over no data item, the mark area under it answers. With the axis trigger the tooltip appends the named mark areas the pointer is in after the series rows. Hovering a mark line widens it and hovering a mark area deepens it, both eased in the shader on the motion clock.
- Transitions: mark lines are matched to the previous layout by `(series, MARK_LINE_INDEX + k)`, so a changed threshold moves from where it was; mark areas by `(series, MARK_AREA_INDEX + k)`.

## Tests

```bash
cargo test -p nana-ui-charts
```

```bash
cargo test -p nana-ui-runtime --features charts --lib charts
```

```bash
cargo test -p nana-ui --features hosted,gpu,charts --lib -- scene_paint::wgsl scene_paint::msl_precompiled scene_paint::naga_ir chart --test-threads=1
```

The first covers layout, candlestick/boxplot/heatmap geometry and multi-row tooltips, the placement and hit testing of mark lines and areas, value-axis and category y-axis zoom, and transition matching; the second box selection, double-click restore, markLine/markArea tooltips, and the y-axis wheel and slider; the third validates the shader, the precompiled artifacts, and GPU drawing (including markLine hover widening, markArea draw order/deepening, and the new series).
