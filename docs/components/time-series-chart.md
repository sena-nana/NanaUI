# TimeSeriesChart

`TimeSeriesChart` 画一条时间序列。

数值和本地化标签由你提供。

没有图层时它是原来的 sparkline。

加上图层之后，在总量曲线下面画分类堆叠柱。

这个类型在 Cargo feature `charts` 后面。

`components` 会打开它。

`TimeSeriesChart::new(values)` 的 `values` 是总量，决定样本个数。

`.stacked(layers)` 接上 `TimeSeriesLayer`。

每一层用 `TimeSeriesLayer::new(label, values, color)`，颜色是 `SemanticColorRole`。

某一层少了的项按 0，多出来的项不增加日期。

`.axis_labels` 和 `.tooltip_details` 由你提供本地化日期和补充说明。

悬停时 `active`、标记和提示一起更新。

你写了高度就保留这个高度。

`TimeSeriesChart::from_samples` 接收 `(Unix 毫秒, Option<f64>)`，按真实时间间隔定位。

缺失或非有限的样本把线断开。

有时间戳数据时，图层不参与定位。

`.unit` 是单位，`.time_labels(start, end)` 是两端的本地化文字，格式和时区仍由你负责。

`.label` 是可访问名，空字符串回到 “Time series”。

控件表里没有 `<TimeSeriesChart>`。

`stacked` 是 `new` 之后的方法，不是单独的构造函数。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{SemanticColorRole, TimeSeriesChart, TimeSeriesLayer};

view! {
    <Widget
        of={TimeSeriesChart::new([3.0, 5.0, 4.0])
            .stacked([
                TimeSeriesLayer::new("读", [1.0, 2.0, 1.0], SemanticColorRole::Accent),
                TimeSeriesLayer::new("写", [2.0, 3.0, 3.0], SemanticColorRole::Success),
            ])
            .axis_labels(["一", "二", "三"])
            .label("流量")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{SemanticColorRole, TimeSeriesChart, TimeSeriesLayer};

widget(
    TimeSeriesChart::new([3.0, 5.0, 4.0])
        .stacked([
            TimeSeriesLayer::new("读", [1.0, 2.0, 1.0], SemanticColorRole::Accent),
            TimeSeriesLayer::new("写", [2.0, 3.0, 3.0], SemanticColorRole::Success),
        ])
        .axis_labels(["一", "二", "三"])
        .label("流量"),
)
```

:::

图表内部用普通 tooltip。

指针离开、节点卸载或停放之后，提示会关上。

业务上的分组、Top N 和数值格式仍由你给。

[总览](index.md) · [控件合同](../reference/components.md)
