# AppShell

`AppShell` 是标题栏、填满的正文，以及可选的一层浮层。不需要侧栏、检查器和 Dock 时，用它就够。每个区域里放什么由你决定。

控件表里没有 `<AppShell>`。

## 基本用法

`AppShell::new()` 三个槽都是空的。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::AppShell;

view! {
    <Widget of={AppShell::new()}>
        <template #body>
            <Column />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::AppShell;

widget(AppShell::new()).body(column())
```

:::

## 装配

`assemble_app_shell` 是 `slot_assembler`。

视图提交时会跑，所以视图里不要再调一次。

手工 `create_component` 放好槽之后要自己调。

改字段的那一次写入不会自动重装。

没有标题栏槽、树上也没有标题栏子节点时，这次装配不新建标题栏。

槽在、或子节点已经是标题栏时，装配会用那一个，并接着调用 `assemble_app_title_bar`。

## 浮层

浮层槽绝对定位铺满；若它不是 `OverlayHost`，有内容时才接指针：至少一个子节点没有隐藏（`hidden` / `display: none`）。空着的时候，指针落到下面的正文上。

内容换了，接不接指针跟着重新判断：子节点增删（`when`、`dynamic`、`each` 直接作为槽位内容时换了分支或行，`keep_alive` 收起的分支也算移走），或者某个子节点显示、隐藏了，AppShell 都会重新投影这一层，不用重新装配。只看直接子节点：`when` 套在一层 `column()` 里时，外面那层 `column` 一直是内容，要把 `when` 直接交给 `.overlay(..)`。

浮层里的对话框走 `activate_overlay`，或者在对话框上声明 `.open(..)` / `.model(信号)`（见 [Dialog](dialog.md#声明式开合)）。

壳只负责把这一层叠在正文上。

标题文字、窗口按钮和拖动放在 `AppTitleBar` 上。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `AppShell::new()` | — | 三个槽都是空的 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

`.title_bar(id)`、`.body(id)`、`.overlay(id)` 写入已有节点。

视图上同名方法接收一段视图：`.title_bar(view)`、`.body(view)`、`.overlay(view)`。

正文槽占满剩余高度。

| 插槽 | 说明 |
| --- | --- |
| `.title_bar` | `.title_bar(id)` 写入已有节点。视图上 `.title_bar(view)` 接收一段视图 |
| `.body` | `.body(id)` 写入已有节点。视图上 `.body(view)` 接收一段视图。正文槽占满剩余高度 |
| `.overlay` | `.overlay(id)` 写入已有节点。视图上 `.overlay(view)` 接收一段视图。浮层槽绝对定位铺满；若它不是 `OverlayHost`，有没隐藏的子节点时才接指针，子节点增删、显隐时重新判断 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
