# ReorderList

`ReorderList` 是纵向列表上的选择和拖动。

它报告「被移动的值，以及它后面的值」，自己不改条目顺序。

顺序、分组和存盘由你做。

它不跨窗口。

这个类型在 Cargo feature `controls` 后面。

`components` 会打开它。

`ReorderList::new(items)` 接收 `ReorderItem`。

`ReorderItem::new(value, label)` 默认可拖、可落、未选中、未禁用。

`.draggable(false)` 同时把 `drop_target` 写成 false；只要落点、不要起点时，再接 `.drop_target(true)`。

`.selected`、`.disabled` 写在行上。

`.tools(id)` 标出一行里可点的子节点，命中这块区域不开始拖拽。

`.spacing` 是行距，`.size` 默认小号，`.label` 是列表的可访问名。

`.tree_drop(true)` 允许拖到树上。

`.live_rows(true)` 表示行由你挂的子节点来画，列表仍用 `items` 做命中和拖动，不再按标签自绘。

这个开关在构造时声明，不会在投影时从树上推断。

事件是 `ReorderListEvent`：

- `Select(value)` 是点中一行。
- `Secondary { source, x, y }` 是行身上的右键。 `x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上。
- `Reorder { source, before }` 里 `before` 为 `None` 表示放到末尾。
- `TreeDrop { source, intent }` 的 `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After`。
- `Cancelled` 是取消。 Escape、失焦、触摸丢失要交给 `ReorderListPointer::Cancel`。

拖动要超过 4px 才算移动。

控件表里没有 `<ReorderList>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ReorderItem, ReorderList};

view! {
    <Widget
        of={ReorderList::new([
            ReorderItem::new("a", "第一项").selected(true),
            ReorderItem::new("b", "第二项"),
        ])
        .label("顺序")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ReorderItem, ReorderList};

widget(
    ReorderList::new([
        ReorderItem::new("a", "第一项").selected(true),
        ReorderItem::new("b", "第二项"),
    ])
    .label("顺序"),
)
```

:::

收到 `Reorder` 之后，按 `source` 和 `before` 重排你的列表，再写回 `items`。

`selected_value` 读当前选中。

`is_dragging` 告诉你手势还在不在。

[总览](index.md) · [控件合同](../reference/components.md)
