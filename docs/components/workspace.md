# Workspace

`Workspace` 把你的内容绑到区域上。

内容是树上的子节点。

折叠、尺寸、可见性和恢复走 `nana_ui::WorkspaceModel` 与 `WorkspaceMutation`。

不需要 IDE 式壳时，用 `List` 或 `AppShell`。

`Workspace::new()` 等于 `from_model(&WorkspaceModel::new(), [])`。

`from_model(model, slots)` 带上已有的 `WorkspaceRegionSlot`。

`.slot(id, content)` 按 `nana_ui::RegionId` 绑定一块内容，同一 id 再写会替换。

视图上是 `.region(id, view)`。

内建 id 有 `GlobalNavigation`、`SectionNavigation`、`Resources`、`PrimaryToolbar`、`Primary`、`Inspector`、`Diagnostics`，以及 `Custom(String)`。

`register` 拒绝重复 id。

字段 `workspace_corners` 决定主区域圆不圆角，默认是圆的。

`DesktopShell` 装配时把自己的 `workspace_corners` 抄到它建的这份工作区上。

你改框架建出来的那个节点，下一次壳装配会盖掉。

`apply(mutation, now)` 把 mutation 推进模型。

`refresh_from_model` 只更新由模型推导的字段，不换你的槽。

分隔条命中区盖在区域边缘，宽 8px，不占 grid track。

折叠带 260ms 过渡。

`assemble_workspace` 是 `slot_assembler`。

视图提交时会跑。

手工放好槽之后要自己调一次。

每次写入不会自动重装。

控件表里没有 `<Workspace>`。`.region` 要区域 id 和视图两个参数，不能写成 `<template #region>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Workspace;
use nana_ui::RegionId;

widget(Workspace::new()).region(RegionId::Primary, view! { <Column /> })
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::Workspace;
use nana_ui::RegionId;

widget(Workspace::new()).region(RegionId::Primary, column())
```

:::

`WorkspaceGeometry` 把同一份布局映射成逻辑矩形和物理矩形，供 GPU 视口使用。

它不创建窗口，也不创建 GPU 资源。

`WorkspaceController` 只把指针和时钟转成 mutation，不要在适配器里再存一份区域状态。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
