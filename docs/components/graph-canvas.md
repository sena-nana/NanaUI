# GraphCanvas

`GraphCanvas` 是一块受控的图画布。

模型和持久化由你保存，控件持有视口、选择和这一下指针，并把结果发成事件。

默认只画网格、节点框和边。

节点内部由你往子节点里放。

这个类型在 Cargo feature `graph-canvas` 后面。

`components` 会打开它。

`GraphCanvas::new(canvas_id, model)` 的模型是 `GraphModel`。

空图用 `GraphModel::empty()`。

`GraphModel::new(nodes, edges)` 会校验，失败时返回 `GraphModelError`。

`.viewport`、`.selection`、`.label`、`.disabled` 写在画布上。

`.grid_spacing` 只接受有限且不小于 8 的间距。

`.node_content(node, content)` 给某个节点绑一块内部区域或宿主纹理，未知 id 会留到 `set_model` 修剪。

`"graph-canvas"` 自定义 GPU renderer 不会自动挂上。

常量是 `GRAPH_CANVAS_RENDERER`。

默认场景画家拒绝未登记的这个键。

要直写 pass，宿主自己登记，再 `set_custom_render`。

`custom_render()` 只是拼出那份 `CustomRenderNode`，投影不会附上它。

`assemble_graph_canvas_contents` 是槽位装配，负责节点内部的子节点。

它不是 GPU renderer。

视图提交时会跑；手工建好内部槽之后要自己调一次。

事件是 `GraphCanvasEvent`：`SelectionChanged`、拖动中的 `NodePositionInput`、提交的 `NodePositionChanged`、`ConnectionRequested`、拖动中的 `ViewportInput`，以及抬起、滚轮或键盘产生的 `ViewportChanged`。

右键仍然只发 `SecondaryPress`，坐标是窗口坐标。

`AppContext::graph_canvas_hit_at` 把它换成画布局部点和 `GraphCanvasHit { local, selection }`。

这是纯查询，不改选择、焦点或悬停。

菜单开不开、开什么，由你决定。

控件表里没有 `<GraphCanvas>`。

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

禁用时几何查询仍然按盒子回答。

你把 `set_viewport` 写回事件里的视口，选择和连线是否进模型也由你提交。

[总览](index.md) · [控件合同](../reference/components.md)
