# List

`List` 是一组条目的容器。

它的无障碍角色是列表，读屏按列表来念。

里面放什么条目由你决定。

条目控件是 `ListItem`，那一页在总览里单独说明。

`List::new()` 没有子节点。

`.label` 给整张列表一个可访问名。

`.style` 整份替换节点样式。

列表本身不接指针，也不能聚焦；点击落在你放进去的条目上。

不需要 IDE 式工作区时，一列 `List` 或一个 `AppShell` 就够。

区域、停靠和拆分留给 `Workspace`。

控件表里没有 `<List>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::List;

view! {
    <Widget of={List::new().label("最近文件")}>
        <ListItem>"main.rs"</ListItem>
        <ListItem>"lib.rs"</ListItem>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{list_item, widget};
use nana_ui::runtime::List;

widget(List::new().label("最近文件")).children((
    list_item("main.rs"),
    list_item("lib.rs"),
))
```

:::

`list_item` 是条目的视图函数，标签既是构造参数，也是显示文字。

选中、禁用和行尾补充写在条目上，不写在这张列表上。

列表不排序，也不虚拟化；长列表外面套 `ScrollView`，或者用虚拟列表自己管理窗口。

默认样式是一份空的 `NodeStyle`。

你要间距、内边距或背景，写在 `.style` 里，或者把列表放进 `Stack::column`。

列表的可访问名来自 `.label`；不写时，读屏只按角色念「列表」。

[总览](index.md) · [控件合同](../reference/components.md)
