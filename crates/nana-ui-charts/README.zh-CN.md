# nana-ui-charts

[English](README.md) · 简体中文

NanaUI 的图表模型，option 的形状参照 ECharts。

`ChartOption` 用 Rust 类型描述坐标轴、系列、图例、提示和缩放。`layout` 在 CPU 上把它和容器尺寸、主题一起算成 `ChartLayout`，里面有标签、悬停用的命中数据，以及图表着色器直接读取的 `ChartMarks`。标记同时带起始态和目标态，过渡由着色器插值；悬停查询在已完成的布局上进行，指针移动不会触发重新布局。目前支持折线（含面积、堆叠、平滑）、柱状、饼图、散点、雷达和仪表盘，坐标轴有类目、数值、时间和对数四种。直角坐标系列可以带标线（阈值、统计值、线段）和标域（区间带），缩放可以作用在任一根轴上，用滚轮、拖动、滑轨或框选。

本 crate 只依赖 `nana-ui-core`，不测量文字，标签宽度由调用方经 `LabelMeasure` 提供。`Chart` 组件在 [`nana-ui-runtime`](../nana-ui-runtime)，绘制管线在 [`nana-ui`](../nana-ui)。

## 示例

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

应用通常不直接调用 `layout`，而是把 `ChartOption` 交给 `nana_ui::runtime::Chart`。

## 测试

```bash
cargo test -p nana-ui-charts
```

更多内容见 [Chart 组件](../../docs/components/chart.md)、[图表模型与渲染](../../docs/reference/charts.md) 和 [支撑 crate](../../docs/learn/supporting-crates.md)。
