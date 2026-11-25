# Table

`Table` 是一张保留在树上的表。它自己不比较数据。列头上的排序指示、列的顺序，由旁边的 `VirtualTableLayout` 记录；行的先后由你排，因为只有你知道两行该怎么比。

## 基本用法

可见控件是三个类型。`Table::new()` 的无障碍角色是表，`.label` 给整张表一个名字。`TableRow::new()` 是一行，`.selected` 标出当前行。`TableCell::new(value)` 是一格，文本就是这个值。`.column_header(true)` 把格子投影成列表头，否则是单元格。格子默认可点、可聚焦。表和行本身不接指针。

控件表里没有 `<Table>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Table, TableCell, TableRow};

view! {
    <Widget of={Table::new().label("文件")}>
        <Widget of={TableRow::new()}>
            <Widget of={TableCell::new("名称").column_header(true)} />
            <Widget of={TableCell::new("大小").column_header(true)} />
        </Widget>
        <Widget of={TableRow::new()}>
            <Widget of={TableCell::new("main.rs")} />
            <Widget of={TableCell::new("12 KB")} />
        </Widget>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Table, TableCell, TableRow};

widget(Table::new().label("文件")).children((
    widget(TableRow::new()).children((
        widget(TableCell::new("名称").column_header(true)),
        widget(TableCell::new("大小").column_header(true)),
    )),
    widget(TableRow::new()).children((
        widget(TableCell::new("main.rs")),
        widget(TableCell::new("12 KB")),
    )),
))
```

:::

## 几何

几何在 `nana_ui::runtime::VirtualTableLayout`。`VirtualTableLayout::new(row_extents, columns)` 接收每一行的高度，以及一组 `nana_ui::runtime::TableColumn`。`TableColumn::new(key, extent)` 给出列键和宽度。`.sortable(true)` 把这一列的表头标成排序控件。`.limits(min, max)` 限制宽度，`.resizable` 允许改宽。

`move_column(from, to)` 重排列，宽度和总宽度保持，`to` 是抽出之后的目标下标。`VirtualTableLayout` 只回答哪一列在指示排序，以及列的几何。

## 排序

排序比较由你做。表头激活走 `toggle_sort(column)`：升序，然后降序，然后取消。第三次激活清掉排序，而不是回到升序。不可排序的列、或表里没有的列，会被拒绝。`set_sort` 直接发布指示；`None` 表示没有排序。

你在表头激活时调用 `toggle_sort`，读到现在的 `TableSort`，再按这个列键和方向重排自己的数据，然后改表格子节点的顺序。

## 改列宽

把表头格子建成 `TableCell::new(标题).header_for(&column)`：它带上列键、是否可改宽，以及 `.limits` 给的宽度范围。指针落在可改宽的表头右边缘 5 px（`TABLE_RESIZE_HANDLE`）内时显示左右拉伸光标，按住拖动就在 `Table` 上发 `TableColumnResized { column, extent, finished }`：拖动中每次移动发一次 `finished: false`，松开再发一次 `finished: true`；宽度已按范围截好。按 `Escape` 或指针被取消时拖动会恢复起始宽度并用 `finished: true` 收尾。

表格不改自己的几何。你在收到时调用 `layout.resize_column_by_key(&column, extent)` 再同步一次（`sync_virtual_table_retained_in`）：`VirtualTableLayout::revision()` 会推动窗口重新布局，表头和表体的格子从同一份列宽放置，冻结的列和表头照样对齐（表放在滚动容器其它兄弟节点下方时也会按其在容器内的偏移对齐视口）。只想在松开时改，就只处理 `finished: true`。

## 选择

选中哪些行是你的状态，通常是一份 `SelectionState<行键>`（见下文）。表格只报告请求：

- 在表体格子上按下，发 `TableSelectionRequested { row, cell, gesture: SelectionGesture::Press(修饰键) }`。Ctrl（macOS 上是 Cmd）是切换，Shift 是从锚点连成一段，两者同按是追加一段。
- 焦点在表体格子上时，方向键、Home / End、Page Up / Down 移动焦点并发 `Move`；带 Shift 是从锚点扩展，Ctrl+Shift 保留既有选中并追加一段；Ctrl 加上下方向键只移动焦点、不改选中；Ctrl+Space 发 `Toggle`；Ctrl+A 发 `SelectAll`。
- 表头行不发选择请求。

`row` 是行节点，`VirtualTableItems::row_key_of(&cx, row)` 换成你的行键。把请求交给 `SelectionState::apply(gesture, 行键, &顺序)`，再调用 `cx.sync_virtual_table_selection(&items, |键| 选择.is_selected(键))`：已挂载的行和它们的格子得到 `selected`，画主题的选中底色，并对辅助技术报告已选中；没挂载的行不用管，滚进来时由 `build_row` 用同一个判断建出。选择没变时这一步什么也不写，也不分配。`Table::multiselectable(true)` 告诉辅助技术可以多选。

## SelectionState

`nana_ui::runtime::SelectionState<K>` 是不依附任何控件的纯数据选择状态，`each_virtual`、`List`、网格、虚拟表格和 `TreeView`（见 [TreeView 的选择](https://github.com/sena-nana/NanaUI/blob/main/docs/components/tree-view.md#选择)）都能共用。它只存你的业务键：选中集合、当前项（键盘所在）和锚点（Shift 连选的起点），不存位置、行或节点，所以滚出视口、没有建出来的行照样保持选中。

需要顺序的操作（Shift 连选、方向键、全选、数据变化后的清理）从你传入的 `SelectionOrder` 读顺序：一个键的切片，或者 `keyed_order(&items, |item| item.id)`，用 `.selectable(..)` 排除禁用项。顺序不会被复制进状态。

- `SelectionState::single()` 最多选一项；`multiple()` 可多选。
- `apply(gesture, key, &order)` 是指针和键盘的同一个入口；`navigate(step, modifiers, &order)` 按 `SelectionStep` 走一步并应用。
- `reconcile(&order)` 在数据增删、重排之后调用：不在顺序里的键被去掉；当前项被删时落到原位置附近的可选项；重排什么也不丢。
- `is_selected`、`selected_in(&order)`（按顺序列出）、`current`、`anchor`、`revision`（每次改变递增）。

在响应式视图里把它放进 `Signal<SelectionState<K>>`，行上用 `.selected(sel.map(move |s| s.is_selected(&key)))`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.label` | — | `Table::new()` 上，给整张表一个名字。无障碍角色是表 |
| `.multiselectable` | `bool` | `Table::new()` 上，告诉辅助技术可以选多行 |
| `.selected` | — | `TableRow::new()` 的一行上，标出当前行 |
| `value` | — | `TableCell::new(value)` 的文本就是这个值 |
| `.column_header` | — | `true` 把格子投影成列表头，否则是单元格 |
| `.header_for(&column)` | `&TableColumn` | 表头格子：列键、可否改宽、宽度范围 |
| `.column` / `.resizable` | — | 单独设列键、可否拖右边缘改宽 |
| `row_extents` | — | `VirtualTableLayout::new` 接收的每一行高度 |
| `columns` | 一组 `nana_ui::runtime::TableColumn` | 同上 |
| `key` | — | `TableColumn::new(key, extent)` 的列键 |
| `extent` | — | `TableColumn::new(key, extent)` 的宽度 |
| `.sortable` | — | `true` 把这一列的表头标成排序控件 |
| `.limits(min, max)` | — | 限制宽度 |
| `.resizable` | — | 允许改宽 |

## 事件

| 事件 | 发在 | 说明 |
| --- | --- | --- |
| `TableCellFocused` | `Table` | 方向键把焦点移到了某一格 |
| `TableColumnResized` | `Table` | 表头拖动改列宽，见“改列宽” |
| `TableSelectionRequested` | `Table` | 按下或按键请求改变选择，见“选择” |

表头上的排序由你调用 `toggle_sort` 或 `set_sort`，见上文。

## 插槽

没有具名插槽。行和格子是子节点，见基本用法。

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
