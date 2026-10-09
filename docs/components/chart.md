# Chart

`Chart` 画图表：折线、面积、柱状、饼和环、玫瑰、散点、雷达、仪表盘。你给一份 `ChartOption`，结构和 ECharts 的 `option` 对应（`grid`、`x_axis`、`y_axis`、`series`、`tooltip`、`legend`、`data_zoom`），值是 Rust 类型。数据、名称和数值格式由你提供，几何、动画和交互由控件管。

这个类型在 Cargo feature `charts` 后面。`components` 会打开它。控件表里没有 `<Chart>`：option 是类型化的数据，只走 Rust。

## 基本用法

:::api

```rust view
use nana_ui::runtime::chart::{Axis, BarSeries, ChartOption, Legend, LineSeries};
use nana_ui::runtime::view;
use nana_ui::runtime::Chart;

let option = ChartOption::new()
    .legend(Legend::default())
    .x_axis(Axis::category(["周一", "周二", "周三", "周四", "周五"]))
    .y_axis(Axis::value())
    .series(BarSeries::new("访问", [120.0, 200.0, 150.0, 80.0, 70.0]))
    .series(LineSeries::new("转化", [80.0, 132.0, 101.0, 134.0, 90.0]).smooth(true));

view! {
    <Widget of={Chart::new(option).label("本周访问")} />
}
```

```rust rust
use nana_ui::runtime::chart::{Axis, BarSeries, ChartOption, Legend, LineSeries};
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Chart;

let option = ChartOption::new()
    .legend(Legend::default())
    .x_axis(Axis::category(["周一", "周二", "周三", "周四", "周五"]))
    .y_axis(Axis::value())
    .series(BarSeries::new("访问", [120.0, 200.0, 150.0, 80.0, 70.0]))
    .series(LineSeries::new("转化", [80.0, 132.0, 101.0, 134.0, 90.0]).smooth(true));

widget(Chart::new(option).label("本周访问"))
```

:::

宽度默认铺满，高度默认 300px。你写了尺寸就用你的尺寸。

## 系列

| 系列 | 用法 | 说明 |
| --- | --- | --- |
| `LineSeries` | 折线与面积 | `.smooth(true)` 是单调三次插值，不会冲过数据。`.step(..)`、`.area(AreaStyle)`、`.stack(..)`、`.line_type(..)`、`.symbol(..)`、`.sampling(..)` |
| `BarSeries` | 柱状 | 同一类目内并列，`.stack(..)` 相同的叠在一起（正负分开叠）。`.border_radius(..)` 圆角在远离基线的一端。y 轴是类目轴时横向 |
| `PieSeries` | 饼、环、玫瑰 | `.ring(内, 外)`、`.rose(..)`、`.pad(px)`、`.corner_radius(px)`、`.label(..)`。外侧标签带引导线，左右两侧各自避让 |
| `ScatterSeries` | 散点 | `[x, y]` 点。`.sizes(..)` 逐点大小。上万个点也是一次绘制 |
| `RadarSeries` | 雷达 | 需要 `ChartOption::radar(RadarCoord)` 给指标 |
| `GaugeSeries` | 仪表盘 | `.range(..)`、`.angles(..)`、进度弧、刻度和指针 |

数据列用 `SeriesData`：一组值按类目下标对应（`[f64; N]`、`Vec<f64>`），或者 `[x, y]` 点对（`Vec<[f64; 2]>`）用于数值轴和时间轴。`NaN` 是缺口，折线在缺口处断开（`.connect_nulls(true)` 连上）。

## 坐标轴

`Axis::category(名称)`、`Axis::value()`、`Axis::time()`（Unix 毫秒）、`Axis::log()`。数值轴按 ECharts 的规则取整到好读的刻度，默认包含 0；`.scale(true)` 只贴合数据。`.min(..)` / `.max(..)` 固定一端。类目标签放不下时隔几个显示一个，数值标签重叠时隐藏后一个。`.formatter(..)`、`.time_formatter(..)` 由你做本地化。

`Grid` 给出绘图区四周的留白，默认量到标签外沿（`contain_label`）。

## 提示、轴指针和图例

指针在折线和柱状图上按类目触发：同一类目的所有系列一起列进提示，竖线（柱状是一条类目宽的阴影）标出位置。饼、散点、雷达、仪表盘按指针下的那一项触发。`Tooltip::trigger(..)`、`.axis_pointer(..)`、`.value_formatter(..)`、`.formatter(..)` 改这些行为，`Tooltip::hidden()` 关掉提示。

悬停的那一项会被强调：点变大、饼的扇区外扩、柱子提亮；系列设了 `.focus(EmphasisFocus::Series)` 时其余系列变淡。强调的过渡在着色器里按动效时钟采样，不重新布局。

`Legend` 列出系列（饼图列出扇区）。点一下切换显示，其余系列重新排布并带过渡。

## 缩放

`DataZoom::inside()`：在绘图区里滚轮缩放（以指针为中心），拖动平移。`DataZoom::slider()`：图下方一条带预览的滑轨，拖窗口或两端。`.range(start, end)` 是初始窗口（百分比）。窗口移动时立即重画，不做过渡。

## 动画

第一次出现时，折线从左到右展开，柱子从基线长出，扇区从起始角扫开，散点和雷达从中心长出。之后每次换 option，元素从屏幕上正在显示的位置（过渡中途也算）移到新位置；新出现的元素按入场方式出现。`Animation` 设时长和缓动，默认入场 1 秒、更新 0.3 秒，都是 cubic-out。数据项超过 `threshold`（默认 2000）时不做动画。`Animation::disabled()` 关掉。

每秒更新很多次的实时曲线（帧时间、电平）建议关掉动画：每次换 option 都会重新开始一段更新过渡。

## 事件

`ChartEvent::Click { series, index }`：在一项上按下并原地抬起。`ChartEvent::LegendSelect { name, selected }`：图例切换。`ChartEvent::DataZoom { index, start, end }`：缩放窗口变了。用 `.on(|e: &ChartEvent| ..)` 接收。

## 颜色

系列颜色默认取图表调色板：第一个是主题的强调色，其余在同样的感知亮度和彩度下换色相，暗色主题更亮一些。`ChartColor::Role(..)` 用主题角色，`ChartColor::Palette(i)` 用调色板第 i 个，`ChartColor::Rgba(..)` 写死颜色。网格线、轴线和文字跟随主题。

## 性能

布局只在 option、图例/缩放状态、尺寸或主题变化时做一次，结果按这四样缓存。指针移动只查已有的布局。标记数组（点、形状、样式）按版本常驻 GPU：画面不变的帧不上传，动画和强调只靠动效时钟，每帧没有 CPU 工作。折线可以用 `Sampling::Lttb` / `Sampling::MinMax` 先降到像素宽度。

## 无障碍

角色是图像。`.label(..)` 是可访问名，写一句概括，例如「本周访问，周二最高」。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `option` | `Arc<ChartOption>` | 整张图。换一份新的就是一次更新 |
| `.label` | `Arc<str>` | 可访问名 |
| `.style` | `NodeStyle` | 尺寸等布局样式 |
| `view` | `ChartViewState` | 图例关掉的系列和缩放窗口，用户操作后的状态 |

## 事件

| 事件 | 说明 |
| --- | --- |
| `ChartEvent` | 点击、图例切换、缩放 |

## 插槽

没有插槽。

## 参见

[总览](index.md) · [控件](../reference/components.md)
