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

`.selected`、`.disabled` 写在行上。`.tools(id)` 标出一行里绝不开始拖动的子节点：按在这块区域里，移动多远都不拖。行里的按钮不必登记成 `.tools` 也能收到点击，见[手势](#手势)。

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

拖动要超过 4px 才算移动。移动后列表给被拖的行描一圈强调色边，并在松手会落下的位置画插入线（树拖放时是目标行的线）；放下不改变顺序的位置不画线。活行时两者都按子节点的布局盒放，行自己不用画拖动态。

按在哪里决定这次按下归谁：

- 按在行上、不在控件上（自绘的行；`.live_rows(true)` 时行这个节点本身，和行里不能获得焦点的部分）：列表立刻接过指针。不移动就松开是 `Select`，移动超过 4px 就拖动这一行。
- 按在行里的控件上（行下面能获得焦点的节点，比如按钮）：按下照常归这个控件。不移动就松开是控件自己的点击，列表不发事件；移动超过 4px 时列表才接过指针、拖动这一行，控件这次收不到点击。和 HTML5 拖放一样，整行都能拖，从按钮上按下拖动也会拖。
- 按在 `.tools` 区域里：列表不管，移动也不拖。

以前为了让按钮收到点击而把它们包成一段登记成 `.tools` 的，升级后可以去掉 `.tools`：按钮照常点击，从按钮上拖动会拖动这一行。保留 `.tools` 就还是原来的行为，这块区域按下不拖。

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
| `.tools` | — | 参数是 `id`。标出一行里绝不开始拖动的子节点。行里的按钮不必登记也能收到点击 |
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
| `.tools` | 一行里绝不开始拖动的子节点。按在这里移动也不拖 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
