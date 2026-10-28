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

`.spacing` 是行距，`.size` 默认小号，`.label` 是列表的可访问名。`.tree_drop(true)` 允许拖到树上。`.reorder(false)` 让列表只能点选：按下后怎么移动都不会拖起，松手发 `Select`。

## 树

一棵树按深度优先展开成行，每行用 `.depth(n)` 写它的深度（顶层是 0）：一行拥有它下面紧挨着的、更深的那些行。收起的子行不要放进 `items`。

- 拖动一行就带着它拥有的整棵子树：被拖的描边框住整块，子树里的位置不能落下。
- `.kind("…")` 写这一行是什么。`.accepts_inside(..)` 写哪些类型能放进这一行，`.accepts_sibling(..)` 写哪些类型能放在它前后（成为它的兄弟），都用 `ReorderAccepts`：`All`（默认）、`Nothing`，或 `ReorderAccepts::kinds([..])`。没写 `kind` 的行只被 `All` 接受。
- 放在一行前后，还要它的父行（顶层看列表的 `.accepts_root(..)`）接受这种类型。
- 不接受的地方不画插入线，松手也不发事件，所以列表报出的落点都是你声明过能接受的。

树拖放的落点：

- 行的上四分之一在它前面，中间放进它，下四分之一在它后面；这一行不接受被拖的类型时，按上下两半算前后。
- 子行展开着的行，下面那一段是它第一个子行的前面。
- 在一串嵌套的最后一行下面，指针的横向位置选深度：每往左一个 `.indent`，就退出一层，落在那一层的祖先后面。
- 插入线按落点的深度缩进 `depth × indent`，默认缩进 `space::LG`，和你自己画的行缩进对齐时用 `.indent(px)` 改。
- 放下后位置不变的地方（紧挨着的前后兄弟）不画线、不发事件。`Inside` 由应用决定放在子行里的哪一头。

## 键盘

列表或行里的控件有焦点时，Alt 加方向键移动这一行（焦点在行里就是那一行，否则是选中行），连同子树：

| 按键 | 移动 | 事件 |
| --- | --- | --- |
| Alt+↑ | 到前一个兄弟前面 | `TreeDrop` `Before`；非树列表是 `Reorder` |
| Alt+↓ | 到后一个兄弟后面 | `TreeDrop` `After`；非树列表是 `Reorder` |
| Alt+→ | 放进前一个兄弟，做它最后一个子行 | `TreeDrop`：它有可见子行时是最后一个子行的 `After`，否则 `Inside` |
| Alt+← | 移出父行，放在父行后面 | `TreeDrop` `After` |

键盘移动和拖动守同样的规则：不接受、没有兄弟或不改变位置时什么也不发。`ReorderList::key_move(value, ReorderKeyMove)` 直接算出这个事件。

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

活行的普通拖动（`.live_rows(true)`、没开 `.tree_drop`）：

- 被拖的行（连同它拥有的子行）跟着指针走，叠在别的行上面，但在打开的菜单下面（层级是 `REORDER_PREVIEW_Z_INDEX`，低于 `MENU_OVERLAY_Z_INDEX`）。
- 松手会落下的位置，其它行平滑让出一个被拖块的高度（含行距）。落点不变或者不接受的位置，所有行回到原处。
- 这些都只在呈现层上：行的样式、布局盒和落点判定都不动，落点仍按行原来的布局盒算。动画由框架推进，应用不用逐帧驱动；减少动态效果时让位直接到位，预览照样跟手。
- 列表不再画描边和插入线，让出的空位就是落点。
- 松手（落下或没落下）、Escape 和 `Cancel` 时，预览和让位立刻全部收回。收到 `Reorder` 后在同一帧里写回 `items`，行就停在它让出的位置上。

树拖放和自绘行：移动后列表给被拖的行描一圈强调色边，并在松手会落下的位置画插入线；树拖放放进某一行时改为给那一行描边并铺一层淡强调色。放下不改变顺序的位置不画线。活行时这些都按子节点的布局盒放，并画在行的上面（选中行的底色不会盖住它们），行自己不用画拖动态。

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
| `.reorder` | `true` | `false` 时只点选、不拖动 |
| `.depth` | `u16` | 写在行上。树里的深度，顶层 0，默认 0 |
| `.kind` | — | 写在行上。行的类型，供 `ReorderAccepts` 判断 |
| `.accepts_inside` | `ReorderAccepts` | 写在行上。哪些类型能放进这一行，默认 `All`；`Nothing` 时这一行按上下两半算前后 |
| `.accepts_sibling` | `ReorderAccepts` | 写在行上。哪些类型能放在它前后，默认 `All`；还要父行接受 |
| `.accepts_root` | `ReorderAccepts` | 哪些类型能放在顶层，默认 `All` |
| `.indent` | — | 每层深度的缩进，插入线按它缩进，指针按它选深度。默认 `space::LG` |
| `.live_rows` | — | `true` 时行由你挂的子节点来画。列表仍用 `items` 做命中和拖动，不再按标签自绘。构造时声明，不在投影时从树上推断 |

## 事件

事件是 `ReorderListEvent`：

- `Select(value)` 是点中一行。
- `Secondary { source, x, y }` 是行身上的右键。 `x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上。
- `Reorder { source, before }` 里 `before` 为 `None` 表示放到末尾。
- `TreeDrop { source, intent }` 的 `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After`。`source` 带着它的子树一起移动。键盘移动也发这个事件。
- `Cancelled` 是取消。拖动中按 Escape 由框架取消，并释放捕获和按下记录，随后的松开不再落下、也不点击起手的行内控件。自己驱动 `apply_pointer` 时，失焦、触摸丢失要交给 `ReorderListPointer::Cancel`。

指针坐标先按列表的布局坐标反投影再命中行，所以列表放在 `ScrollView` 里或祖先带变换时，按下、拖动、落点和右键找的都是指针下的那一行；`Secondary` 的 `x`、`y` 仍是窗口坐标。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `Select` | `value` | 点中一行 |
| `Secondary` | `source`、`x`、`y` | 行身上的右键。`x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上 |
| `Reorder` | `source`、`before` | `before` 为 `None` 表示放到末尾 |
| `TreeDrop` | `source`、`intent` | `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After` |
| `Cancelled` | — | 取消。拖动中的 Escape 由框架处理；自己驱动指针时失焦、触摸丢失交给 `ReorderListPointer::Cancel` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 子节点 | `.live_rows(true)` 时由你挂来画。列表仍用 `items` 做命中和拖动，不再按标签自绘 |
| `.tools` | 一行里绝不开始拖动的子节点。按在这里移动也不拖 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
