# Dock

`Dock` 是可以拆分、可以浮出的窗格树。窗格里的内容是子节点。它不接管编辑器页签的跨窗口语义，那是 `Tabs` 的合同。

控件表里没有 `<Dock>`。

## 基本用法

比例、浮动窗和命中条由框架算。

`Dock::new(root)` 接收一棵 `DockNode`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Dock, DockNode};

view! {
    <Widget of={Dock::new(DockNode::item("stage", None)).primary("stage").title("stage", "舞台")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dock, DockNode};

widget(Dock::new(DockNode::item("stage", None)).primary("stage").title("stage", "舞台"))
```

:::

比例和拖动落点由框架算，落下之后窗格里显示什么，仍是你绑进去的子节点。

## 节点

`DockNode::item(id, content)` 是一片，`content` 是可选的子节点 id。

`DockNode::tabs(tabs, active, contents)` 是一组页签。

`DockNode::split(axis, ratio, first, second)` 的 `axis` 是 `DockAxis::Horizontal` 或 `Vertical`，`ratio` 是第一片的份额，会被夹紧。

`.primary(id)` 标出不能隐藏的中心片。

`.title(id, title)` 给一片标题。

`.locked(true)` 锁住拖动。

`bind_content(id, node)` 把内容绑到已有的片上。

## 装配

`assemble_dock_panels` 是 `slot_assembler`，按树把面板装出来。

视图没有单独的内容槽方法。

手工建好节点、写进 `DockNode` 之后要自己调一次装配。

每次改标题不会自动重装。

## 浮动

浮动窗格经过 `DockWorkspaceEvent` 和 `runtime_dock_window_update`，变成 `WindowCommand::Open` 或 `Close`。

`nana_ui::dock` 里的 `DockController` 和 `DockAction` 是宿主适配器，把指针、停留和帧转成 mutation。

不要把它当成第二套 Dock。

## 显隐

`hide` 把一片从装配里拿开，`show` 再放回来。

`primary` 那一片不能隐藏。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `Dock::new(root)` | `DockNode` | 接收一棵 `DockNode` |
| `DockNode::item(id, content)` | — | 一片。`content` 是可选的子节点 id |
| `DockNode::tabs(tabs, active, contents)` | — | 一组页签 |
| `DockNode::split(axis, ratio, first, second)` | — | `axis` 是 `DockAxis::Horizontal` 或 `Vertical`。`ratio` 是第一片的份额，会被夹紧 |
| `.primary(id)` | — | 标出不能隐藏的中心片 |
| `.title(id, title)` | — | 给一片标题。每次改标题不会自动重装 |
| `.locked` | — | `.locked(true)` 锁住拖动 |
| `bind_content(id, node)` | — | 把内容绑到已有的片上 |
| `hide` | — | 把一片从装配里拿开。`primary` 那一片不能隐藏 |
| `show` | — | 再放回来 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `DockWorkspaceEvent` | — | 浮动窗格经过 `DockWorkspaceEvent` 和 `runtime_dock_window_update`，变成 `WindowCommand::Open` 或 `Close` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 视图没有单独的内容槽方法。窗格里的内容是子节点 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
