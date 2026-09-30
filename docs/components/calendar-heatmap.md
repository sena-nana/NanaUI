# CalendarHeatmap

`CalendarHeatmap` 按周列画出一格一格的热力。日期、数值、标题和含义都由你提供。几何和等级映射由 Runtime 管。

控件表里没有 `<CalendarHeatmap>`。

## 基本用法

这个类型在 Cargo feature `calendar` 后面。

`components` 会打开它。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{CalendarHeatmap, CalendarHeatmapDatum};

view! {
    <Widget
        of={CalendarHeatmap::new([
            CalendarHeatmapDatum::new("2026-09-01", 1.0),
            CalendarHeatmapDatum::new("2026-09-02", 4.0),
        ])
        .week_starts_on(1)
        .label("提交")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{CalendarHeatmap, CalendarHeatmapDatum};

widget(
    CalendarHeatmap::new([
        CalendarHeatmapDatum::new("2026-09-01", 1.0),
        CalendarHeatmapDatum::new("2026-09-02", 4.0),
    ])
    .week_starts_on(1)
    .label("提交"),
)
```

:::

## 数据

`CalendarHeatmap::new(data)` 接收 `CalendarHeatmapDatum`。

`CalendarHeatmapDatum::new(date, value)` 的日期是字符串，`.data` 可以附上你自己的载荷。

控件不解释日期字符串的时区，也不替你补没有数据的日子。

## 周与名字

`.week_starts_on(day)` 决定一周从哪天起。

`.label` 是可访问名，空字符串会回到默认的 “Calendar heatmap”。

## 等级

`.level_strategy` 换成你的分档。

等级 0 用主题的 `subtle`，其余等级把强调色混进 subtle。

## 格子

单元格边长常量是 `CELL_SIZE` 11、间距 `CELL_GAP` 3。

活动格记在 `active` 上。

悬停时读 `CellEnter` 和 `CellMove` 里的日期、数值和标题，自己决定提示怎么写。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `CalendarHeatmap::new(data)` | `CalendarHeatmapDatum` | 接收这些数据 |
| `CalendarHeatmapDatum::new(date, value)` | — | 日期是字符串 |
| `.data` | — | 可以附上你自己的载荷 |
| `.week_starts_on(day)` | — | 决定一周从哪天起 |
| `.label` | — | 可访问名。空字符串会回到默认的 “Calendar heatmap” |
| `.level_strategy` | — | 换成你的分档 |
| `active` | — | 活动格记在这里 |
| `CELL_SIZE` | — | 单元格边长常量是 11 |
| `CELL_GAP` | — | 间距常量是 3 |

## 事件

事件是 `CalendarHeatmapEvent`：`CellEnter`、`CellMove` 带着 `CalendarHeatmapActiveCell`，`CellLeave` 表示指针离开。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `CalendarHeatmapEvent::CellEnter` | `CalendarHeatmapActiveCell` | 悬停时读这里的日期、数值和标题，自己决定提示怎么写 |
| `CalendarHeatmapEvent::CellMove` | `CalendarHeatmapActiveCell` | 悬停时读这里的日期、数值和标题，自己决定提示怎么写 |
| `CalendarHeatmapEvent::CellLeave` | — | 表示指针离开 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
