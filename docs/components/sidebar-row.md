# SidebarRow

`SidebarRow` 是侧栏里的一行导航。

它复用列表项的槽来画，身份仍是侧栏行。

链接、路由和选中由你提供，NanaUI 不内置产品导航。

`SidebarRow::new(label)` 默认空闲、深度 0、小号。

`.state` 用 `SidebarRowState`：`Idle`、`Active`、`AncestorActive`、`Disabled`。

当前页用 `Active`，走到它下面的页时，祖先行用 `AncestorActive`。

`.depth` 是树里的缩进，`sidebar_row_depth_inset` 把深度换成左侧留白。

`.disclosure(expanded)` 在行首放一个展开标记。

`.tone` 是 `SidebarRowTone`：`Default`、`Warning`、`Error`。

`.tools(id)` 是行尾的工具节点。

激活走 `activate_sidebar_row`。

点下去之后去哪一个路由，在你的处理函数里做。

控件不保存历史。

控件表里没有 `<SidebarRow>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{SidebarRow, SidebarRowState};

view! {
    <Widget of={SidebarRow::new("概览").state(SidebarRowState::Active).depth(0)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{SidebarRow, SidebarRowState};

widget(
    SidebarRow::new("概览")
        .state(SidebarRowState::Active)
        .depth(0),
)
```

:::

同一节里可以有多行 `Active` 的视觉，哪一行代表当前页由你写进 `state`。

禁用行用 `SidebarRowState::Disabled`。

深度从 0 起，每一层按侧栏的缩进步进。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
