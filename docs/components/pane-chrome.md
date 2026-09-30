# PaneChrome

`PaneChrome` 是一块窗格顶部的标题行，加上下面填满的正文。

标题行里是页签和一排图标动作。

页签 id、脏标记和跨窗口移动仍由你拥有。

`PaneTree` 只是一次渲染用的轻量投影。

`PaneChrome::new()` 默认这一片是活动的。

`.tabs(id)` 是标题行里的页签。

`.body(id)` 是下面的内容。

`.header(id)` 换成你自己的整行标题，铬会给它样式，不改它的子节点。

`.active(false)` 把标题行背景换成淡一档。

`.actions` 是一列 `PaneChromeAction`。

`PaneChromeAction::new(kind, label)` 的 `kind` 是 `PaneChromeActionKind`：`Focus`、`SplitHorizontal`、`SplitVertical`、`MoveToWindow`、`MoveToNextPane`、`ClosePane`、`CloseItem`、`Custom`。

`.icon` 有图标时，装配把它画成图标按钮。

视图上 `.tabs(view)`、`.body(view)`、`.header(view)` 放槽。

`.action(action, view)` 按书写顺序追加动作，`view` 是那个按钮。

`assemble_pane_chrome` 是 `slot_assembler`。

视图提交时会跑。

手工放好槽之后要自己调一次。

每次改 `active` 不会自动重装。

控件表里没有 `<PaneChrome>`。`.action` 要动作和视图两个参数，不能写成 `<template #action>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{PaneChrome, PaneChromeAction, PaneChromeActionKind};

widget(PaneChrome::new())
    .tabs(view! { <Text>"main.rs"</Text> })
    .action(
        PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭"),
        view! { <Button>"关闭"</Button> },
    )
    .body(view! { <Text>"正文"</Text> })
```

```rust rust
use nana_ui::runtime::view::{button, text, widget};
use nana_ui::runtime::{PaneChrome, PaneChromeAction, PaneChromeActionKind};

widget(PaneChrome::new())
    .tabs(text("main.rs"))
    .action(
        PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭"),
        button("关闭"),
    )
    .body(text("正文"))
```

:::

动作点下去之后关不关页签、拆不拆窗口，在你的处理函数里做。

铬只把按钮装进标题行，并把活动片和闲置片的表面分开。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
