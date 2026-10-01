# 工作区

桌面产品通常不是一张卡片。它是侧栏，加上主区，再加上检查器。有时还有底栏，以及可以拆出去的 Dock。

NanaUI 提供这些结构。每个格子里放什么，仍由你的应用决定。

不需要 IDE 式壳时，用 `List` 或 `AppShell` 即可。你不必上 `Workspace`。

## 区域

`WorkspaceLayout` 按注册顺序持有 `RegionState`。一块区域包含：

| 字段 | 作用 |
| --- | --- |
| `id` | 内建名或 `Custom(String)`，稳定身份 |
| `role` | 导航、资源、主区、检查器、时间线、控制台、工具 |
| `placement` | start / primary / end / top / bottom |
| `scope` | workspace 或 primary |
| `size` / min / max / default | 尺寸约束 |
| `fill_priority` | 多个 primary 怎么分剩余空间 |
| `collapsible` / `resizable` | 能否折叠、拖动 |
| `narrow_behavior` | 变窄时 shrink / collapse / overlay / none |

内建 `RegionId` 覆盖常见工作区。`Custom(String)` 可以再加任意的起止区域和上下区域。`register` 拒绝重复 ID。标准六区是便捷构造，不是上限。

产品状态是 `WorkspaceModel` 加上 `WorkspaceMutation`。折叠、尺寸、可见性和 JSON 恢复都走这里。

`WorkspaceController` 只把指针和时钟转成 mutation。不要在适配器里再存一份 Region 状态。

`Workspace` 控件把你的内容绑到 Region：`WorkspaceRegionSlot { id, content }`。内容是树上的子节点。

分隔条的命中区盖在区域边缘，宽 8px。它不占 grid track。

折叠或隐藏的区域不绘制，也不响应 resize，并在几何里让出空间。overlay 区域会显示，但不占 primary。

折叠带 260ms 过渡。只有动画在跑时才要帧。

`WorkspaceGeometry` 把同一份布局映射成逻辑矩形和物理矩形，供 GPU 视口使用。它不创建窗口，也不创建 GPU 资源。

## 侧栏与设置

`SidebarFrame`、`SidebarSection`、`SidebarRow` 是通用导航结构。

链接、路由和选中项由你的应用提供。NanaUI 不内置产品导航。

`SettingsModel` 和 `SettingsState` 维护稳定 Tab，以及恢复规则。主题由应用拥有的
`ThemeRegistry` 管理，设置页接收 `ThemeChoice` 列表并持久化 `ThemeId`；其他具体
设置值仍由你的应用状态拥有，例如账号和路径。

设置页用独立的 `WorkspaceController` 时，进入设置不会覆盖主工作区的尺寸和折叠。

全窗画面上仍要一块可交互的任务区域时，用独立的 `OverlayHost` 加 `Panel`。它不改变主区尺寸。它不锁外部指针，也不锁 Tab。

你提供业务页面、详情返回状态和 viewport insets。NanaUI 负责面板表面、关闭生命周期，以及有效焦点的恢复。

不要把业务路由塞进 SettingsState。也不要为非模态面板再写一套挂载、卸载和焦点控制。

## Dock

产品 Dock 是 Runtime 的 `Dock` 和 `DockWorkspace`。crate 根会再导出它们。

pane 里的内容是子节点。split 比例、浮动窗和命中条由框架计算。

浮动 pane 通过 `DockWorkspaceEvent` 和 `runtime_dock_window_update`，变成 `WindowCommand::Open` 或 `Close`。对照 `examples/runtime-host-fixture`。

`nana_ui::dock::*`（`DockController`、`DockAction`）是宿主适配器。它把指针、dwell 和帧转成 mutation。

不要把它当成第二套 Dock。也不要再写一套 split 算法。

`Dock` 不会隐式接管编辑器 Tab 的跨窗口语义。Tab 重排见下一节。

## Tab、列表拖放、动作

`Tabs::on_reorder` 用「被移动的值 + 其后的值」描述结果。`None` 表示末尾。

跨窗口拖 Tab 用 `TabDragGroup` 和 `TabDragSurface`。选择和顺序仍由你的应用持有。你可以用 `DockWorkspace::save_to_store` 写进 `ViewStateStore`。

接收方可以用 `accepts_external_drop(false)`，只允许拖出，拒绝外部落入。

`ReorderList` 是纵向列表的同一套 before-value 合同。它不跨窗口。

树拖放回传稳定节点，以及 `before`、`inside` 或 `after`。它不推断文件系统语义。

`ActionRegistry` 登记稳定的 Action ID，以及快捷键上下文。`CommandPalette` 只呈现你提供的、当前上下文里可用的 Action。

真正的 dispatch，以及 keymap 的存盘，由你的应用来做。

`TreeView` 把你给出的稳定节点 ID、展开事实和层级，画成 disclosure row。它不读路径，也不 watch 文件系统。

`PaneTree` 和 `PaneChrome` 是一次渲染用的轻量投影。Pane ID、dirty 和跨窗口移动仍由你的应用拥有。
