# GraphMinimap

`GraphMinimap` 是图画布的概览。它按 `GraphModel::bounds` 等比缩放，画出节点矩形和视口指示框。点击和拖拽发出新的视口，由你写回 `GraphCanvas::set_viewport`。

控件表里没有 `<GraphMinimap>`。

## 基本用法

它不改画布。

`GraphMinimap::new(model)` 从空视口和空的画布尺寸开始。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{GraphMinimap, GraphModel, GraphSize};

view! {
    <Widget
        of={GraphMinimap::new(GraphModel::empty())
            .canvas_size(GraphSize::new(640.0, 480.0))
            .label("概览")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{GraphMinimap, GraphModel, GraphSize};

widget(
    GraphMinimap::new(GraphModel::empty())
        .canvas_size(GraphSize::new(640.0, 480.0))
        .label("概览"),
)
```

:::

收到 `ViewportRequested` 之后，对那块 `GraphCanvas` 调用 `set_viewport`，再把同一份视口写回小地图，指示框才跟手。

模型换了也要 `set_model`，小地图不会自己去读画布节点。

## 特性

这个类型和 `GraphCanvas` 一样，在 Cargo feature `graph-canvas` 后面。

`components` 会打开它。

## 投影

投影在小地图自己的盒子里居中，并保持节点的宽高比。

位置和尺寸由你的布局给定，常见做法是放在画布角落，用 `PositionSpec::Absolute`。

## 拖拽

取消拖拽时会按按下时记下的视口再请求一次。

## 属性

`.canvas_size` 传入图画布的可见尺寸，用来算指示框和导航偏移；非法尺寸会被留下原值。

`.viewport` 跟画布当前视口保持一致。

`.node_fill` 可以换成别的语义色。

`.disabled` 停掉导航。

`.label` 是可访问名，空字符串回到 “Graph minimap”。

`.set_model` 和 `.set_viewport` 在已有节点上更新。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `model` | `GraphModel` | `GraphMinimap::new(model)` 从空视口和空的画布尺寸开始 |
| `.canvas_size` | — | 图画布的可见尺寸，用来算指示框和导航偏移；非法尺寸会被留下原值 |
| `.viewport` | — | 跟画布当前视口保持一致 |
| `.node_fill` | — | 可以换成别的语义色 |
| `.disabled` | — | 停掉导航 |
| `.label` | — | 可访问名，空字符串回到 “Graph minimap” |
| `.set_model` | — | 在已有节点上更新。模型换了也要调用，小地图不会自己去读画布节点 |
| `.set_viewport` | — | 在已有节点上更新 |

## 事件

事件只有 `GraphMinimapEvent::ViewportRequested(viewport)`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `GraphMinimapEvent::ViewportRequested` | `viewport` | 点击和拖拽发出新的视口，由你写回 `GraphCanvas::set_viewport`。取消拖拽时会按按下时记下的视口再请求一次 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
