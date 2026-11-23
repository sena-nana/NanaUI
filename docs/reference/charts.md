# 图表模型与渲染（nana-ui-charts）

`Chart` 控件背后是三层：`nana-ui-charts` 把 `ChartOption` 布局成标签、命中数据和 GPU 标记数组；`nana-ui-runtime` 的 `Chart` 管交互状态并把标记交给 Scene；`nana-ui` 的图表管线用 `chart.wgsl` 画出来。本页是这三层之间的合同。控件用法见 [Chart](../components/chart.md)。

## 管线

| 阶段 | 位置 | 输入 | 输出 | 何时重算 |
| --- | --- | --- | --- | --- |
| 布局 | `nana_ui_charts::layout` | option、`ChartViewState`（图例、缩放）、尺寸、`ChartTheme`、`LabelMeasure` | `ChartLayout`：文字、`HitModel`、`ChartMarks`、缩放映射 | 四项输入任一变化 |
| 过渡 | `nana_ui_charts::transition` | 新布局、上一份正在显示的布局、动效时钟 | 每个元素的 `from` | 新的 option 或图例变化；缩放立即落位 |
| 交互 | `nana-ui-runtime` `Chart` 与 `framework/charts.rs` | 指针、滚轮、已完成的布局 | 悬停、缩放窗口、选择框、`ChartEvent` | 每个指针事件，不重新布局 |
| 绘制 | `nana-ui` `scene_paint/chart.rs` + `chart.wgsl` | `ChartGpuPayload`（标记与悬停） | GPU 绘制 | 标记 `revision` 变化才上传 |

`nana-ui-charts` 只依赖 `nana-ui-core`：不测量文字（由调用方给 `LabelMeasure`），不碰保留树，也不碰 GPU。

## 标记数组（GPU 合同）

`marks.rs` 里的布局就是 `chart.wgsl` 的存储缓冲布局，字段改动必须两边一起改。

| 类型 | 大小 | 内容 |
| --- | --- | --- |
| `GpuPoint` | 48 B | 折线点与它的底边点，`to` / `from` 两端，弧长（虚线） |
| `GpuShape` | 64 B | 矩形、扇形、符号、指针；`meta = [kind | flags, style, series, data index]` |
| `GpuStyle` | 64 B | 填充、渐变终点、描边（sRGB 直通 alpha，上传时转线性），线宽与虚线 |
| `MarkDraw` | — | 一次绘制：`pass`、范围、样式、`series`、`flags`、`neighbours`、`index` |

`MarkDraw` 在 GPU 上是两个 `vec4<u32>`：`a = [first, count, style, flags]`，`b = [neighbours, series, pass, index]`。

| 标志 | 位 | 作用 |
| --- | --- | --- |
| `draw_flags::CLIP_TO_PLOT` | 1 | 裁到绘图区 |
| `draw_flags::REVEAL` | 2 | 入场时从左到右揭开 |
| `draw_flags::EMPHASIS` | 16 | 悬停所在系列时线变宽 |
| `draw_flags::GUIDE` | 32 | 网格、轴线等，不随系列淡出 |
| `draw_flags::MARK` | 64 | 标线：只有悬停 `(series, index)` 正是它时才加宽 2px |
| `SHAPE_EMPHASIS_GROW` | 512 | 悬停时符号变大、扇形外扩、柱子提亮 |
| `SHAPE_EMPHASIS_FILL` | 2048 | 悬停时矩形填充加深（标域） |

悬停用 `(series, data index)` 匹配。`MARK_LINE_INDEX + k`（`0x4000_0000` 起）是某系列第 k 条标线，`MARK_AREA_INDEX + k`（`0x6000_0000` 起）是第 k 个标域，与数据下标和轴触发的类目下标不相交。

改了 `chart.wgsl` 必须重新生成预编译产物，否则 Apple 上的发布构建会拒绝不匹配的阶段：

```bash
cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1
```

```bash
cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1
```

## 缩放

`DataZoom` 按屏幕上的方位指定坐标轴，和 ECharts 的 `xAxisIndex` / `yAxisIndex` 一致：`.x_axis(i)`（默认 0）或 `.y_axis(i)`，`target()` 返回 `(ZoomAxis, index)`。横向柱状图上 x 轴是数值轴、y 轴是类目轴。

- 窗口是百分比。同一坐标轴上的所有条目共用一个窗口：布局读第一个条目的窗口，运行时移动窗口时把该轴上每个条目都设成同一个值，并为每个变化的条目发一次 `ChartEvent::DataZoom`。
- 基轴（类目、时间或沿着前进的数值轴）的百分比相对全部数据范围；类目 `i` 的中心在 `(i + 0.5) / 总数`。数值轴的百分比相对它在**全部数据**上取整后的刻度范围，与 x 窗口无关，所以框选时两个方向可以各自换算。数值轴有缩放条目时不再按 x 窗口自适应；窗口不是 0–100 时两端精确等于窗口，不再取整。对数轴不做数值缩放。
- `ChartLayout::zoom_axes` 给出每个显示中的可缩放轴的像素到百分比映射（`ZoomAxisLayout::percent_at` / `per_px`），滚轮、拖动平移和框选都只用它换算，不重新布局。
- `DataZoomKind::Slider`：x 轴的滑轨在绘图区下方（`ChartLayout::slider`，基轴时带第一条系列的预览），y 轴的滑轨在右侧（`ChartLayout::y_slider`，窗口从底部向上增长）。
- `DataZoomKind::Inside`：滚轮以指针为中心缩放每个开了 `zoom_on_wheel` 的轴；拖动平移每个开了 `move_on_drag` 的轴。
- 框选：任一 inside 条目设了 `select_on_drag`（`DataZoom::select()`）时，在绘图区内拖动画选择框，松开后把每个有框选条目的轴缩放到框内；只有 x 方向可缩放时选择框占满高度，反之占满宽度。两个方向都比 8px 窄的框算点击。双击恢复 option 里的初始窗口。选择框由 Runtime 以普通矩形绘制（槽位 14），不经过标记数组。

## 标线与标域

`LineSeries`、`BarSeries`、`ScatterSeries` 可带 `mark_line(MarkLine)` 和 `mark_area(MarkArea)`，画在该系列的坐标轴上，默认用系列颜色。

| 条目 | 位置 |
| --- | --- |
| `MarkLineItem::x(v)` / `::y(v)` | 竖线 / 横线；类目轴上 `v` 是类目下标 |
| `MarkLineItem::min()` / `max()` / `average()` / `stat(MarkStat::Median)` | 系列数值（去掉 NaN）的统计值，横跨绘图区，落在数值轴上 |
| `MarkLineItem::segment([x, y], [x, y])` | 两点之间的线段 |
| `MarkAreaItem::y(a, b)` / `::x(a, b)` / `::rect(..)` | 横向带 / 纵向带 / 矩形；缺的方向占满绘图区 |

- 画法：标域在所有系列之下（网格线之后），标线在所有系列之上；都裁到绘图区。标线默认 1px 虚线。柱状图（类目带有间隙）上的标域把首尾类目整格包进去，折线的类目点上则在两点之间。
- 数值轴范围包含标线和标域的固定值（`x` / `y` / 线段端点 / 带的边界），阈值在数据之外也看得见；统计值本来就在数据范围内。
- 标签：标线在末端写名字，没有名字写数值（可用 `formatter`）；标域在左上角写名字，放不下就不写。
- 命中：指针在标线 4px 以内时标线优先，提示的标题是系列名，一行是名字和数值；然后是数据本身；指针不在任何数据项上时，所在的标域给出提示。轴触发时，提示在系列各行之后追加指针所在的、有名字的标域。悬停标线让它加宽，悬停标域让它加深，都在着色器里按动效时钟过渡。
- 过渡：标线按 `(series, MARK_LINE_INDEX + k)` 与上一份布局匹配，阈值改动时从原来的位置移过去；标域按 `(series, MARK_AREA_INDEX + k)` 匹配。

## 测试

```bash
cargo test -p nana-ui-charts
```

```bash
cargo test -p nana-ui-runtime --features charts --lib charts
```

```bash
cargo test -p nana-ui --features hosted,gpu,charts --lib -- scene_paint::wgsl scene_paint::msl_precompiled scene_paint::naga_ir chart --test-threads=1
```

第一条覆盖布局、标线标域的位置与命中、数值轴缩放和过渡匹配；第二条覆盖框选、双击恢复、y 轴滚轮与滑轨；第三条校验着色器、预编译产物和 GPU 绘制。
