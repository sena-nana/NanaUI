# Table

`Table` 是一张保留在树上的表。

它自己不比较数据。

列头上的排序指示、列的顺序，由旁边的 `VirtualTableLayout` 记录；行的先后由你排，因为只有你知道两行该怎么比。

可见控件是三个类型。

`Table::new()` 的无障碍角色是表，`.label` 给整张表一个名字。

`TableRow::new()` 是一行，`.selected` 标出当前行。

`TableCell::new(value)` 是一格，文本就是这个值。

`.column_header(true)` 把格子投影成列表头，否则是单元格。

格子默认可点、可聚焦。

表和行本身不接指针。

几何在 `nana_ui::runtime::VirtualTableLayout`。

`VirtualTableLayout::new(row_extents, columns)` 接收每一行的高度，以及一组 `nana_ui::TableColumn`。

`TableColumn::new(key, extent)` 给出列键和宽度。

`.sortable(true)` 把这一列的表头标成排序控件。

`.limits(min, max)` 限制宽度，`.resizable` 允许改宽。

表头激活走 `toggle_sort(column)`：升序，然后降序，然后取消。

第三次激活清掉排序，而不是回到升序。

不可排序的列、或表里没有的列，会被拒绝。

`set_sort` 直接发布指示；`None` 表示没有排序。

`move_column(from, to)` 重排列，宽度和总宽度保持，`to` 是抽出之后的目标下标。

控件表里没有 `<Table>`。

排序比较由你做。

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

你在表头激活时调用 `toggle_sort`，读到现在的 `TableSort`，再按这个列键和方向重排自己的数据，然后改表格子节点的顺序。

`VirtualTableLayout` 只回答哪一列在指示排序，以及列的几何。

[总览](index.md) · [控件合同](../reference/components.md)
