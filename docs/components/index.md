# 组件

这一页按族列出已经有的控件。你用它们往树上挂节点。Rust 入口是 `nana_ui::runtime`。名称和模板标签来自同一份控件表：标签写错、属性写错，在 `view!` 里是编译错误。

每一项都有自己的一页。页里的示例会跟着侧边栏顶部的开关切换。没有单独模板标签的控件，模板一侧写成 `<Widget of={...}>`，插槽写成 `<template #名字>`。控件表里仍然没有 `<Dialog>`、`<Dropdown>` 这种标签。行为和事件的合同仍以 [控件](../reference/components.md) 为准。

## 操作与输入

[Button](button.md)、[IconButton](icon-button.md)、[TextInput](text-input.md)、[TextArea](text-area.md)、[NumberInput](number-input.md)、[Checkbox](checkbox.md)、[Switch](switch.md)、[RangeField](range-field.md)、[Select](select.md)、[Dropdown](dropdown.md)、[SearchDropdown](search-dropdown.md)、[SegmentedControl](segmented-control.md)、[Tabs](tabs.md)、[XYPad](xy-pad.md)、[ColorField](color-field.md)、[PathField](path-field.md)、[DatePicker](date-picker.md)。

## 布局与文本

[Text](text.md)、[Stack](stack.md)（[row](row.md) / [column](column.md) / `bar` / `spacer` / `overlay_layer`）、[Divider](divider.md)、[IconGlyph](icon-glyph.md)、[ScrollView](scroll-view.md)。

## 表格与树

[Table](table.md)（`TableRow` / `TableCell`）、[TreeView](tree-view.md)、[ReorderList](reorder-list.md)。排序时怎么比较，由你的应用决定。

## 展示

[Card](card.md)、[List](list.md) / [ListItem](list-item.md)、[FormField](form-field.md)、[EmptyState](empty-state.md)、[Progress](progress.md)、[Skeleton](skeleton.md)、[Spinner](spinner.md)、[StatusBadge](status-badge.md)、[Chip](chip.md)、[Avatar](avatar.md)、[Thumbnail](thumbnail.md)、[Texture](texture.md)、[Tooltip](tooltip.md)、[ValidationMessage](validation-message.md)、[QrCode](qr-code.md)、[ImageViewer](image-viewer.md)、[NativeMarkdown](native-markdown.md)、[CalendarHeatmap](calendar-heatmap.md)、[TimeSeriesChart](time-series-chart.md)、[DonutChart](donut-chart.md)、[GraphCanvas](graph-canvas.md)、[GraphMinimap](graph-minimap.md)。

## 浮层

[Dialog](dialog.md)、[ConfirmDialog](confirm-dialog.md)、[Drawer](drawer.md)、[Popover](popover.md)、[ActionMenu](action-menu.md)、[ContextMenu](context-menu.md)、[CommandPalette](command-palette.md)。浮层由框架放在窗口里。打开、关闭、锚点和焦点归还见 [控件](../reference/components.md)。

## 壳层

[AppShell](app-shell.md) / [DesktopShell](desktop-shell.md)、[AppTitleBar](app-title-bar.md)、[Toolbar](toolbar.md)、[StatusBar](status-bar.md)、[MediaTransportBar](media-transport-bar.md)、[Workspace](workspace.md)、[SidebarFrame](sidebar-frame.md) / [SidebarSection](sidebar-section.md) / [SidebarRow](sidebar-row.md)、[设置行](settings-row.md)、[Dock](dock.md)、[SplitPane](split-pane.md)、[PaneChrome](pane-chrome.md)。每个区域里放什么由你的应用决定，见 [工作区](../reference/workspace.md)。

## 表面

尺寸、颜色、字体和控件表面来自主题。深色和浅色切换的是颜色。单行控件有小、中、大三档高度。具体数值和换主题的方式见 [视觉](../reference/look.md)。

有些族需要单独的 Cargo feature，例如 `calendar`、`charts`、`graph-canvas`、`rich-text`。`components` 会启用全部可选族。表见 [应用 API](../reference/application-api.md)。
