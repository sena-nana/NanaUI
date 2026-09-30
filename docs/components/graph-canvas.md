# GraphCanvas

`GraphCanvas` 是一块受控的图画布。模型和持久化由你保存，控件持有视口、选择和这一下指针，并把结果发成事件。默认只画网格、节点框和边。

控件表里没有 `<GraphCanvas>`。

## 基本用法

`GraphCanvas::new(canvas_id, model)` 的模型是 `GraphModel`。

空图用 `GraphModel::empty()`。

`GraphModel::new(nodes, edges)` 会校验，失败时返回 `GraphModelError`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{GraphCanvas, GraphModel};

view! {
    <Widget of={GraphCanvas::new("board", GraphModel::empty()).label("节点图")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{GraphCanvas, GraphModel};

widget(GraphCanvas::new("board", GraphModel::empty()).label("节点图"))
```

:::

## 特性

这个类型在 Cargo feature `graph-canvas` 后面。

`components` 会打开它。

## 自定义绘制

`"graph-canvas"` 自定义 GPU renderer 不会自动挂上。

常量是 `GRAPH_CANVAS_RENDERER`。

默认场景画家拒绝未登记的这个键。

要直写 pass，宿主自己登记，再 `set_custom_render`。

`custom_render()` 只是拼出那份 `CustomRenderNode`，投影不会附上它。

## 装配

`assemble_graph_canvas_contents` 是槽位装配，负责节点内部的子节点。

它不是 GPU renderer。

视图提交时会跑；手工建好内部槽之后要自己调一次。

## 命中

右键仍然只发 `SecondaryPress`，坐标是窗口坐标。

`AppContext::graph_canvas_hit_at` 把它换成画布局部点和 `GraphCanvasHit { local, selection }`。

这是纯查询，不改选择、焦点或悬停。

菜单开不开、开什么，由你决定。

## 回写

禁用时几何查询仍然按盒子回答。

你把 `set_viewport` 写回事件里的视口，选择和连线是否进模型也由你提交。

## 属性

`.viewport`、`.selection`、`.label`、`.disabled` 写在画布上。

`.grid_spacing` 只接受有限且不小于 8 的间距。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `canvas_id` | — | `GraphCanvas::new(canvas_id, model)` 里的 `canvas_id` |
| `model` | `GraphModel` | `GraphCanvas::new` 的模型。空图用 `GraphModel::empty()`。`GraphModel::new(nodes, edges)` 会校验，失败时返回 `GraphModelError` |
| `.viewport` | — | 写在画布上 |
| `.selection` | — | 写在画布上 |
| `.label` | — | 写在画布上 |
| `.disabled` | — | 写在画布上。禁用时几何查询仍然按盒子回答 |
| `.grid_spacing` | — | 只接受有限且不小于 8 的间距 |
| `.node_content` | — | `.node_content(node, content)` 给某个节点绑一块内部区域或宿主纹理，未知 id 会留到 `set_model` 修剪 |

## 事件

事件是 `GraphCanvasEvent`：`SelectionChanged`、拖动中的 `NodePositionInput`、提交的 `NodePositionChanged`、`ConnectionRequested`、拖动中的 `ViewportInput`，以及抬起、滚轮或键盘产生的 `ViewportChanged`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `GraphCanvasEvent::SelectionChanged` | — | `GraphCanvasEvent` |
| `GraphCanvasEvent::NodePositionInput` | — | 拖动中 |
| `GraphCanvasEvent::NodePositionChanged` | — | 提交的 |
| `GraphCanvasEvent::ConnectionRequested` | — | `GraphCanvasEvent` |
| `GraphCanvasEvent::ViewportInput` | — | 拖动中 |
| `GraphCanvasEvent::ViewportChanged` | — | 抬起、滚轮或键盘产生 |
| `SecondaryPress` | — | 右键仍然只发这个，坐标是窗口坐标 |

## 插槽

节点内部由你往子节点里放。

`.node_content(node, content)` 给某个节点绑一块内部区域或宿主纹理，未知 id 会留到 `set_model` 修剪。

| 插槽 | 说明 |
| --- | --- |
| `.node_content` | 某个节点的内部区域或宿主纹理。`assemble_graph_canvas_contents` 负责这些子节点 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
