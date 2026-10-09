# ReorderList

`ReorderList` 是纵向列表上的选择和拖动。它报告「被移动的值，以及它后面的值」，自己不改条目顺序。顺序、分组和存盘由你做。

## 基本用法

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

## 不跨窗口

它不跨窗口。

## 特性

这个类型在 Cargo feature `controls` 后面。`components` 会打开它。

## 条目

`ReorderList::new(items)` 接收 `ReorderItem`。`ReorderItem::new(value, label)` 默认可拖、可落、未选中、未禁用。

`.draggable(false)` 同时把 `drop_target` 写成 false；只要落点、不要起点时，再接 `.drop_target(true)`。

`.selected`、`.disabled` 写在行上。`.tools(id)` 标出一行里可点的子节点，命中这块区域不开始拖拽。

## 列表

`.spacing` 是行距，`.size` 默认小号，`.label` 是列表的可访问名。`.tree_drop(true)` 允许拖到树上。

## 自绘行

`.live_rows(true)` 表示行由你挂的子节点来画，列表仍用 `items` 做命中和拖动，不再按标签自绘。这个开关在构造时声明，不会在投影时从树上推断。

拖动时每行的盒子取自列表自己的直接子节点，所以行要直接挂在列表下面。用带键的 `each` 建行时，把列表交给 `.container(..)`，行就直接建在列表里，增删、重排时按键保留各行的节点：

```rust
use nana_ui::runtime::view::{each, widget};
use nana_ui::runtime::{ReorderList, ReorderListEvent};

each(ids, |id| id.clone(), entry_row).container(
    widget(ReorderList::new(items).live_rows(true).label("条目"))
        .on(|event: &ReorderListEvent| { /* 按 source 和 before 重排 */ }),
)
```

`items` 要和行一一对应：重排、增删时同步写回 `items`（例如绑 `fields::reorder_list::items`）。

## 手势

拖动要超过 4px 才算移动。

`selected_value` 读当前选中。`is_dragging` 告诉你手势还在不在。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `items` | `ReorderItem` | `ReorderList::new(items)`。收到 `Reorder` 后按 `source` 和 `before` 重排，再写回 |
| 条目的值和 `label` | — | `ReorderItem::new(value, label)`。默认可拖、可落、未选中、未禁用 |
| `.draggable` | — | `false` 同时把 `drop_target` 写成 false |
| `.drop_target` | — | 只要落点、不要起点时，在 `.draggable(false)` 后再接 `.drop_target(true)` |
| `.selected` | — | 写在行上。默认未选中 |
| `.disabled` | — | 写在行上。默认未禁用 |
| `.tools` | — | 参数是 `id`。标出一行里可点的子节点，命中这块区域不开始拖拽 |
| `.spacing` | — | 行距 |
| `.size` | — | 默认小号 |
| `.label` | — | 列表的可访问名 |
| `.tree_drop` | — | `true` 允许拖到树上 |
| `.live_rows` | — | `true` 时行由你挂的子节点来画。列表仍用 `items` 做命中和拖动，不再按标签自绘。构造时声明，不在投影时从树上推断 |

## 事件

事件是 `ReorderListEvent`：

- `Select(value)` 是点中一行。
- `Secondary { source, x, y }` 是行身上的右键。 `x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上。
- `Reorder { source, before }` 里 `before` 为 `None` 表示放到末尾。
- `TreeDrop { source, intent }` 的 `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After`。
- `Cancelled` 是取消。 Escape、失焦、触摸丢失要交给 `ReorderListPointer::Cancel`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `Select` | `value` | 点中一行 |
| `Secondary` | `source`、`x`、`y` | 行身上的右键。`x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上 |
| `Reorder` | `source`、`before` | `before` 为 `None` 表示放到末尾 |
| `TreeDrop` | `source`、`intent` | `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After` |
| `Cancelled` | — | 取消。Escape、失焦、触摸丢失要交给 `ReorderListPointer::Cancel` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 子节点 | `.live_rows(true)` 时由你挂来画。列表仍用 `items` 做命中和拖动，不再按标签自绘 |
| `.tools` | 一行里可点的子节点。命中这块区域不开始拖拽 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
