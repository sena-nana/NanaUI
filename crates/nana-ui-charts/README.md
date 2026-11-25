# nana-ui-charts

English · [简体中文](README.zh-CN.md)

NanaUI's chart model, with an option shape modeled on ECharts.

`ChartOption` describes axes, series, legend, tooltip, and zoom with Rust types. `layout` computes it on the CPU together with the container size and theme into a `ChartLayout`, which holds labels, hit data for hovering, and the `ChartMarks` that the chart shaders read directly. Marks carry both a start state and a target state, and transitions are interpolated by the shaders; hover queries run against the finished layout, so pointer movement never triggers relayout. Supported today: line (including area, stacked, and smooth), bar, pie, scatter, radar, gauge, candlestick, boxplot, and heatmap, with four axis types: category, value, time, and log. Cartesian series can carry mark lines (thresholds, statistics, segments) and mark areas (bands), and zoom works on either axis by wheel, drag, slider, or a selection box.

This crate depends only on `nana-ui-core` and does not measure text; label widths are supplied by the caller through `LabelMeasure`. The `Chart` component is in [`nana-ui-runtime`](../nana-ui-runtime), and the paint pipelines are in [`nana-ui`](../nana-ui).

## Example

```rust
use nana_ui_charts::{
    ApproximateMeasure, Axis, ChartOption, ChartTheme, ChartViewState, LayoutInput, LineSeries,
    layout,
};
use nana_ui_core::SemanticPalette;

let option = ChartOption::new()
    .x_axis(Axis::category(["Mon", "Tue", "Wed", "Thu", "Fri"]))
    .y_axis(Axis::value())
    .series(LineSeries::new("visits", [820.0, 932.0, 901.0, 934.0, 1290.0]));
let palette = SemanticPalette::light();
let theme = ChartTheme::new(&palette, &option, |_| [0.5, 0.5, 0.5, 1.0]);
let chart = layout(&LayoutInput {
    option: &option,
    size: [480.0, 300.0],
    theme: &theme,
    measure: &ApproximateMeasure,
    state: &ChartViewState::default(),
});
```

Applications usually do not call `layout` directly; they hand the `ChartOption` to `nana_ui::runtime::Chart`.

## Tests

```bash
cargo test -p nana-ui-charts
```

See also [Chart component](../../docs/en/components/chart.md), [Chart model and rendering](../../docs/en/reference/charts.md) and [Supporting crates](../../docs/en/learn/supporting-crates.md).
