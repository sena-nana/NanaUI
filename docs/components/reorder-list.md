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

## 跨列表拖动

同一个文档里，`.transfer_group("…")` 相同的列表互相接收拖动。默认没有组，拖动只在自己的列表里。

- 从列表 A 拖出的行移到同组的列表 B 上时，由 B 按自己的规则算落点：被拖行的 `kind`，B 各行的 `accepts_inside`、`accepts_sibling`，B 的 `accepts_root`、`.tree_drop` 和深度，和在 B 里拖动一样，只是 B 里没有跟着走的行。用 `.drag_preview(ReorderDragPreview::LiveGap)` 时，B 也会在落点让出空位并画虚线框；默认模式下 B 画插入线或放进某一行的标记，A 不再画自己的落点，被拖行的描边和跟手照旧。指针回到 A 上，A 恢复原样。
- 给 B 设 `.drop_zone(Some(node_id))` 可把跨列表命中范围扩展到外包容器（例如整张卡片与其内边距）；空的普通列表也能接进外框并显示落位框，而隐藏或收起状态的列表（包含其 `drop_zone`）不会截获同组或外部拖动。
- 源列表开启 `.drag_preview(ReorderDragPreview::LiveGap)` 后，每次活跃拖动还会发出 `ReorderListEvent::Drag { source, phase, x, y, inside_group }`（窗口坐标与指针是否落在同组任一列表内），方便外层在拖出组外时接力做跨容器／跨窗口预览。
- 从同组外的来源（如另一个窗口或候选抽屉）向列表注入预览时，调用 `AppContext::set_reorder_external_drag(list_id, Some(ReorderExternalDrag { value, kind, x, y, size }))`（清空传 `None`），松手时调用 `AppContext::finish_reorder_external_drag(list_id)` 提交并清理。
- 在 B 上松手、落点有效时，事件发在 B 上：树列表是 `TreeDrop { source, intent }`，普通列表是 `Reorder { source, before }`。`source` 是被拖行在 A 里的值，`intent`、`before` 指向 B 的行；同组列表的行值要能互相区分。A 这次不发事件。B 不接受的地方不画标记，松手两边都不发事件。
- 树列表要指在 B 的某一行上才有落点，空的树列表接不到；普通列表按纵向位置算，空列表也能放到末尾。
- 指针一直由 A 捕获。拖动中按 Escape 或 `Cancel`，A 发 `Cancelled`，B 的标记一起收回。
- 键盘移动不跨列表。不同文档（窗口）的列表不互相接收。

## 特性

这个类型在 Cargo feature `controls` 后面。`components` 会打开它。

## 条目

`ReorderList::new(items)` 接收 `ReorderItem`。`ReorderItem::new(value, label)` 默认可拖、可落、未选中、未禁用。

`.draggable(false)` 同时把 `drop_target` 写成 false；只要落点、不要起点时，再接 `.drop_target(true)`。

`.selected`、`.disabled` 写在行上。`.tools(id)` 标出一行里绝不开始拖动的子节点：按在这块区域里，移动多远都不拖。行里的按钮不必登记成 `.tools` 也能收到点击，见[手势](#手势)。

## 列表

`.spacing` 是行距（网格模式下也是行间垂直间距），`.column_gap` 是二维网格列距，`.layout_mode(ReorderLayout::Column | ReorderLayout::Grid)` 选择纵向单列（默认）或从左到右自动换行的二维网格，`.size` 默认小号，`.label` 是列表的可访问名。`.drag_preview(ReorderDragPreview::Auto | ReorderDragPreview::LiveGap | ReorderDragPreview::Marks)` 选拖动预览方式（`Auto` 时平铺活行让位、树拖放保留插线；`LiveGap` 对单列、二维网格、树拖放、跨组与外部拖入统一开启实时让位与虚线落位框；`Marks` 始终只画描边和插入线）。`.drop_zone(Some(id))` 扩大跨列表/外部拖动的容器命中区，`.tree_drop(true)` 允许拖到树上。`.reorder(false)` 让列表只能点选：按下后怎么移动都不会拖起，松手发 `Select`。只让某几行不动时，给那几行 `.movable(false)`：它们照样点选、照样能接别的行落在旁边。

## 树

一棵树按深度优先展开成行，每行用 `.depth(n)` 写它的深度（顶层是 0）：一行拥有它下面紧挨着的、更深的那些行。收起的子行不要放进 `items`，并在父行上标 `.folded(true)`。

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

`items` 要和行一一对应：重排、增删时同步写回 `items`（例如绑 `fields::reorder_list::items`；同模块还提供 `drop_zone`、`transfer_group`、`accepts_root`、`reorder` 字段写绑定）。

## 手势

拖动要超过 4px 才算移动。

实时让位拖动（`.drag_preview(ReorderDragPreview::LiveGap)`，以及 `.drag_preview(ReorderDragPreview::Auto)` 下没开 `.tree_drop` 的活行）：

- 被拖的行（连同它拥有的子行）跟着指针走，叠在别的行上面，但在打开的菜单下面（层级是 `REORDER_PREVIEW_Z_INDEX`，低于 `MENU_OVERLAY_Z_INDEX`）。
- 松手会落下的位置，其它行平滑让出一个被拖块的占位空间（单列与树沿纵向让位；`ReorderLayout::Grid` 在网格槽位间按二维 `[dx, dy]` 让位）；`LiveGap` 在让出的空位里画出强调色虚线圆角框（树拖放进入嵌套子行时同时高亮目标父行并画缩进 L 形连接线）。落点不变或者不接受的位置，所有行回到原处。
- 这些都只在呈现层上：行的样式、布局盒和落点判定都不动，落点仍按行原来的布局盒算。动画由框架推进，应用不用逐帧驱动；减少动态效果时让位直接到位，预览照样跟手。
- 让出的空位与虚线框就是落点，不再额外画单条插入横线。
- 松手（落下或没落下）、Escape 和 `Cancel` 时，预览、虚线框和让位立刻全部收回。收到 `Reorder` / `TreeDrop` 后在同一帧里写回 `items`，行就停在它让出的位置上。

树拖放默认态（`Auto`）与 `Marks` 或自绘行：移动后列表给被拖的行描一圈强调色边，并在松手会落下的位置画插入线；树拖放放进某一行时改为给那一行描边并铺一层淡强调色。放下不改变顺序的位置不画线。活行时这些都按子节点的布局盒放，并画在行的上面（选中行的底色不会盖住它们），行自己不用画拖动态。

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
| `.folded` | `bool` | 写在行上。标记树里当前收起的父行，默认 `false` |
| `.tools` | — | 参数是 `id`。标出一行里绝不开始拖动的子节点。行里的按钮不必登记也能收到点击 |
| `.spacing` | — | 行距（网格模式下同时是行间垂直间距） |
| `.column_gap` | — | 二维网格列距；未设时回退到 `.spacing` |
| `.layout_mode` | `ReorderLayout` | `Column`（默认纵向）或 `Grid`（左到右自动换行二维网格） |
| `.drag_preview` | `ReorderDragPreview` | `Auto`（默认）、`LiveGap`（统一让位+虚线落位框）或 `Marks`（仅插入线/描边） |
| `.drop_zone` | `Option<StableNodeId>` | 跨组/外部拖入的扩展容器命中节点（隐藏时不生效） |
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
| `.transfer_group` | — | 同一文档里组相同的列表互相接收拖动，落点和事件归接收的列表。默认没有组 |

## 事件

事件是 `ReorderListEvent`：

- `Select(value)` 是点中一行。
- `Secondary { source, x, y }` 是行身上的右键。 `x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上。
- `Drag { source, phase, x, y, inside_group }` 在开启 `LiveGap` 的源列表拖动期间随生命周期发出，`x`、`y` 是窗口坐标，`inside_group` 表示指针是否仍在同组列表（含其 `drop_zone`）内。
- `Reorder { source, before }` 里 `before` 为 `None` 表示放到末尾。
- `TreeDrop { source, intent }` 的 `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After`。`source` 带着它的子树一起移动。键盘移动也发这个事件。
- 从同组列表拖进来的行，在这个列表上发 `TreeDrop`（普通列表是 `Reorder`），`source` 是它在原列表里的值，见[跨列表拖动](#跨列表拖动)。
- `Cancelled` 是取消。拖动中按 Escape 由框架取消，并释放捕获和按下记录，随后的松开不再落下、也不点击起手的行内控件。自己驱动 `apply_pointer` 时，失焦、触摸丢失要交给 `ReorderListPointer::Cancel`。

指针坐标先按列表的布局坐标反投影再命中行，所以列表放在 `ScrollView` 里或祖先带变换时，按下、拖动、落点和右键找的都是指针下的那一行；`Secondary` 的 `x`、`y` 仍是窗口坐标。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `Select` | `value` | 点中一行 |
| `Secondary` | `source`、`x`、`y` | 行身上的右键。`x`、`y` 是窗口坐标，用来锚住上下文菜单。行表面不接指针，所以这个事件发在列表上 |
| `Drag` | `source`、`phase`、`x`、`y`、`inside_group` | 启用 `LiveGap` 时发出源列表拖动生命周期、窗口坐标与组内命中状态 |
| `Reorder` | `source`、`before` | `before` 为 `None` 表示放到末尾 |
| `TreeDrop` | `source`、`intent` | `intent` 带目标 id 和 `TreeDropPosition`：`Before`、`Inside` 或 `After` |
| `Cancelled` | — | 取消。拖动中的 Escape 由框架处理；自己驱动指针时失焦、触摸丢失交给 `ReorderListPointer::Cancel` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 子节点 | `.live_rows(true)` 时由你挂来画。列表仍用 `items` 做命中和拖动，不再按标签自绘 |
| `.tools` | 一行里绝不开始拖动的子节点。按在这里移动也不拖 |

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
