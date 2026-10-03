# ContextMenu

`ContextMenu` 是钉在一个坐标上的菜单。无障碍角色是菜单。它挂在 `OverlayHost` 下面，用 `activate_overlay` 打开。

控件表里没有 `<ContextMenu>`。

## 基本用法

`ContextMenu::new(anchor_x, anchor_y)` 的锚点是逻辑坐标，构造时 `open` 为真。

Escape 和点外面由框架按菜单语义收起，你不用再写一套点外判定。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ContextMenu, ContextMenuItem};

view! {
    <Widget
        of={ContextMenu::new(120.0, 80.0)
            .searchable(true)
            .items([
                ContextMenuItem::new("rename", "重命名"),
                ContextMenuItem::new("delete", "删除").danger(true),
            ])}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ContextMenu, ContextMenuItem};

widget(
    ContextMenu::new(120.0, 80.0)
        .searchable(true)
        .items([
            ContextMenuItem::new("rename", "重命名"),
            ContextMenuItem::new("delete", "删除").danger(true),
        ]),
)
```

:::

## 条目

`.items` 接收 `ContextMenuItem::new(value, label)`。

`.hint`、`.icon`、`.disabled` 和 `.danger` 写在条目上。

斜杠分开的值（`parent/child`）是一层树。

## 搜索

`.searchable(true)` 加上过滤框，查询存在已提交的 `TextInputState` 里。

`.query(...)` 和 `set_query` 改这串文字；读取时使用 `query_text()`。

空查询显示当前层；有查询时显示匹配的叶子。

## 位置

`place_in(viewport)` 让表面留在视口里。

过滤和钻进子菜单会重新锚定，这份边界一直有效。

不调用时，锚点按原坐标放置。

## 收起

事件是 `ContextMenuEvent::Select(value)`、`Search` 和 `Dismiss`。

框架收起时会把 `open` 写成 `false`，并发出 `Dismiss`，和选中之后的收起是同一条回执。

右键本身只发 `SecondaryPress`。

你在处理函数里决定要不要弹出这个菜单。

`activate_overlay(host, menu)` 要求菜单是这个宿主的直接子节点。

`dismiss_overlay(host)` 先停交互、恢复焦点，再把绘制留到退出动画结束。

退出动画结束前，节点还在树上。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `ContextMenu::new(anchor_x, anchor_y)` | — | 锚点是逻辑坐标，构造时 `open` 为真 |
| `.items` | — | 接收 `ContextMenuItem::new(value, label)` |
| `.hint` | — | 写在条目上 |
| `.icon` | — | 写在条目上 |
| `.disabled` | — | 写在条目上 |
| `.danger` | — | 写在条目上 |
| `.searchable` | — | `.searchable(true)` 加上过滤框，查询存在已提交的 `TextInputState` 里 |
| `.query(...)` | — | 和 `set_query` 改这串文字；`query_text()` 读取当前值 |
| `set_query` | — | 改这串文字 |
| `open` | — | 构造时为真。框架收起时写成 `false` |
| `place_in(viewport)` | — | 让表面留在视口里。不调用时，锚点按原坐标放置 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `ContextMenuEvent::Select(value)` | `value` | 事件是 `ContextMenuEvent::Select(value)`、`Search` 和 `Dismiss` |
| `Search` | — | 事件是 `ContextMenuEvent::Select(value)`、`Search` 和 `Dismiss` |
| `Dismiss` | — | 框架收起时会把 `open` 写成 `false`，并发出 `Dismiss`，和选中之后的收起是同一条回执 |
| `SecondaryPress` | — | 右键本身只发这个。你在处理函数里决定要不要弹出这个菜单 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
