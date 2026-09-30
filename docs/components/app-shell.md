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

浮层槽绝对定位铺满；若它不是 `OverlayHost`，有子节点时才接指针。

浮层里的对话框仍走 `activate_overlay`。

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
| `.overlay` | `.overlay(id)` 写入已有节点。视图上 `.overlay(view)` 接收一段视图。浮层槽绝对定位铺满；若它不是 `OverlayHost`，有子节点时才接指针 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
