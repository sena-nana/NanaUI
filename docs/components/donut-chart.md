# DonutChart

`DonutChart` 用主题语义色画环形组成。

分组、Top N 和数值格式由你提供，控件按你给的扇区画。

这个类型在 Cargo feature `charts` 后面。

`components` 会打开它。

`DonutChart::new(slices)` 接收 `DonutSlice { value, color }`。

`.labels` 提供各项名称，和扇区按下标对应。

`.cutout` 是中心孔的比例，默认 0.62。

`.label` 是整张图的可访问名，默认 “Donut chart”。

无效或负的 `value` 不占面积，原来的下标仍留着。

`active` 是指针正常悬停命中的那一项。

中心孔和分隔缝不会被选中。

分隔缝默认 2 逻辑像素。

图表内部用普通 tooltip。

离开、卸载或停放之后提示会关上。

合法的 live reparent 保留当前状态。

控件表里没有 `<DonutChart>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{DonutChart, DonutSlice, SemanticColorRole};

view! {
    <Widget
        of={DonutChart::new([
            DonutSlice {
                value: 3.0,
                color: SemanticColorRole::Accent,
            },
            DonutSlice {
                value: 1.0,
                color: SemanticColorRole::Success,
            },
        ])
        .labels(["完成", "剩余"])
        .cutout(0.62)
        .label("进度")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{DonutChart, DonutSlice, SemanticColorRole};

widget(
    DonutChart::new([
        DonutSlice {
            value: 3.0,
            color: SemanticColorRole::Accent,
        },
        DonutSlice {
            value: 1.0,
            color: SemanticColorRole::Success,
        },
    ])
    .labels(["完成", "剩余"])
    .cutout(0.62)
    .label("进度"),
)
```

:::

颜色用 `SemanticColorRole` 的名字，例如 `Accent`、`Success`，和 Vue 的 `slices` / `labels` /`cutout` 进同一条绘制路径。

你改扇区之后，悬停下标仍按保留下来的原下标对应。

[总览](index.md) · [控件合同](../reference/components.md)
