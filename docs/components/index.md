# 组件

这一页按族列出已经有的控件。你用它们往树上挂节点。Rust 入口是 `nana_ui::runtime`。名称和模板标签来自同一份控件表：标签写错、属性写错，在 `view!` 里是编译错误。

每一项都有自己的一页，页里有属性表和事件表。示例会跟着侧边栏顶部的开关切换。没有单独模板标签的控件，模板一侧写成 `<Widget of={...}>`，插槽写成 `<template #名字>`。行为和事件的合同仍以 [控件](../reference/components.md) 为准。

## 容易混淆

- [Select](select.md) 是单值下拉。[Dropdown](dropdown.md) 是单值或多值，和 Select 共用菜单。[SearchDropdown](search-dropdown.md) 的查询走已提交的输入框。
- [Dialog](dialog.md) 自带正文和底栏。[ConfirmDialog](confirm-dialog.md) 是确认和取消。[Drawer](drawer.md) 的角色也是对话框，从边上进入。
- [AppShell](app-shell.md) 是没有侧栏和 Dock 的壳。[DesktopShell](desktop-shell.md) 是带格子的桌面壳。[Workspace](workspace.md) 的内容是子节点，区域里放什么由应用决定，见 [工作区](../reference/workspace.md)。

## 操作与输入

- [Button](button.md) — 按下后发出 `Activate`。标签是可访问名称。
- [IconButton](icon-button.md) — 一枚图标。`label` 是可访问名称。激活不自己改 `selected`。
- [TextInput](text-input.md) — 一行文本。`model` 绑定 `value`。
- [TextArea](text-area.md) — 多行文本。有 `on_input`，没有 `on_submit`。
- [NumberInput](number-input.md) — 数字。`value` 的写入是 `assign`。
- [Checkbox](checkbox.md) — 勾选。`model` 绑定 `checked`。
- [Switch](switch.md) — 开关。比复选框多一个 `loading`。
- [RangeField](range-field.md) — 滑块。模板标签是 `<Slider>`，函数是 `slider(min, max, step)`。
- [TimelineEditor](timeline-editor.md) — 可编辑的时间底座：帧 / 秒标尺与播放头、可折叠轨道、关键点的选择与拖动（吸附到帧）、可移动和修剪的片段、贝塞尔曲线视图与切线手柄；内容归应用，编辑通过事件回报。
- [PropertyGrid](property-grid.md) — 检查器式两列属性面板：可折叠的组、共用且可拖动的标签列、混合值与带原因的禁用行；控件按行 id 放入。
- [ScrubField](scrub-field.md) — 在数字上左右拖动改值的属性字段，单击后可键入，支持混合值。
- [RangeSpanField](range-span-field.md) — 双滑块区间，可横可竖，轨道上可标实时值。模板标签是 `<RangeSpan>`，函数是 `range_span(min, max, step)`。
- [Select](select.md) — 单值下拉。
- [Dropdown](dropdown.md) — 单值或多值。没有 `<Dropdown>` 标签。
- [SearchDropdown](search-dropdown.md) — 查询走已提交的 `TextInput`。
- [FindReplaceBar](find-replace-bar.md) — 查找和替换工具条；搜索与修改由应用处理。
- [SegmentedControl](segmented-control.md) — 分段选择。选项是它的子节点。
- [Tabs](tabs.md) — 页签。选择、重排、关闭和跨条拖动从这里报出来。
- [XYPad](xy-pad.md) — 二维垫。按下和移动是 `Input`，抬起是 `Change`。
- [ColorField](color-field.md) — 色块、hex 和 HSV。装配由控件自己完成。
- [PathField](path-field.md) — 路径输入加一个图标按钮。装配由控件自己完成。
- [DatePicker](date-picker.md) — 表头和 6×7 的日期按钮。

## 布局与文本

- [Text](text.md) — 文本。带插值的子文本会跟着信号重算。
- [RichTextView](rich-text-view.md) — 带样式范围的文本：字体、颜色、描边、阴影、装饰线。值由应用持有，改色不重新排版。
- [RichTextEditor](rich-text-editor.md) — 就地编辑一份 `RichText`，和 RichTextView 断出同样的行。工具栏经 `cx.rich_edit` 发命令。
- [Column](column.md) — 纵向排列。间距是 `.gap`，单位是逻辑像素。
- [Row](row.md) — 横向排列。宽度随内容收缩，子项在交叉轴上居中。
- [Stack](stack.md) — `row` / `column` / `bar` / `spacer` / `layers` / `overlay_layer` 的容器。行列是 flex，`layers` 是叠层。
- [Divider](divider.md) — 分隔线。不带标签，也不参与操作。
- [IconGlyph](icon-glyph.md) — 装饰图标。不接收指针，也不能聚焦。
- [ScrollView](scroll-view.md) — 滚动。位置在 `ScrollOffset`，尺寸在 `ScrollMetrics`。

## 表格与树

- [Table](table.md) — 表（`TableRow` / `TableCell`）。它自己不比较数据。
- [TreeView](tree-view.md) — 树。不读路径，也不监视文件系统。
- [ReorderList](reorder-list.md) — 拖动排序。它报告被移动的值和它后面的值，自己不改顺序。

## 展示

- [Card](card.md) — 卡片。动作放在里面的控件上，卡片自己不充当按钮。
- [List](list.md) — 列表。读屏按列表来念。
- [ListItem](list-item.md) — 一行。选中不由控件自己翻。
- [FormField](form-field.md) — 表单行。控件是你的子节点。
- [DynamicForm](dynamic-form.md) — 稳定键驱动的动态表单；状态和校验由应用拥有。
- [InteractionRequestCard](interaction-request-card.md) — 中性的请求卡片；字段和动作由应用提供。
- [EmptyState](empty-state.md) — 空状态。标题下可以用 `.action` 放一个动作。
- [Progress](progress.md) — 进度。只有 `value` 和 `label`。
- [Skeleton](skeleton.md) — 内容还没到时占住尺寸。
- [Spinner](spinner.md) — 转圈。空标签就是圈，有标签时贴在文字左边。
- [StatusBadge](status-badge.md) — 状态徽章。只描述状态，不拥有动作。
- [Chip](chip.md) — 芯片。激活不自己改 `selected`。
- [Avatar](avatar.md) — 圆形头像。默认不参与命中。
- [Thumbnail](thumbnail.md) — 按宽高比显示的宿主纹理。命中归父行；`pickable` 时报图片内的指针位置。
- [Texture](texture.md) — 铺满自己盒子的宿主纹理。
- [Tooltip](tooltip.md) — 提示。角色是 tooltip，标签就是显示的文字。
- [ValidationMessage](validation-message.md) — 校验文案。和控件上的 `invalid`、`FormField` 上的 `error` 是三份显示。
- [QrCode](qr-code.md) — 二维码。不可聚焦，角色是图像。
- [ImageViewer](image-viewer.md) — 遮罩、表面、舞台和说明行。
- [NativeMarkdown](native-markdown.md) — Markdown。复制取的是它的选区快照。
- [CalendarHeatmap](calendar-heatmap.md) — 日历热力。日期、数值和标题由你提供。
- [Chart](chart.md) — 图表：折线、柱状、饼环、散点、雷达、仪表盘。option 和 ECharts 对应，数据和标签由你提供。
- [GraphCanvas](graph-canvas.md) — 图布。模型和持久化由你保存。
- [GraphMinimap](graph-minimap.md) — 按 `GraphModel::bounds` 画出节点和视口。

## 浮层

- [Dialog](dialog.md) — 标题、正文和底栏。没有 `<Dialog>` 标签。
- [ConfirmDialog](confirm-dialog.md) — 确认框。角色是警报对话框。
- [Drawer](drawer.md) — 抽屉。角色是对话框。
- [Popover](popover.md) — 弹出层。按下、焦点、Enter、空格和面板锚点都走触发器。
- [ActionMenu](action-menu.md) — 动作菜单。对齐改成起始边。
- [ContextMenu](context-menu.md) — 上下文菜单。角色是菜单。
- [AppMenuBar](app-menu-bar.md) — 画在应用界面里的菜单栏，跟随主题，子菜单级联弹出；选中项和原生菜单走同一个 id 队列。
- [CommandPalette](command-palette.md) — 命令面板。真正的 dispatch 和快捷键存盘由你做。
- [Toast](toast.md) — 被动通知。装配后带进度、一个动作和关闭按钮；移除由你做。
- [ToastStack](toast-stack.md) — 通知角。先量后显、排队、进退场；放在模态层之上时模态打开也可按。

浮层由框架放在窗口里。打开、关闭、锚点和焦点归还见 [控件](../reference/components.md)。

## 壳层

- [AppShell](app-shell.md) — 不需要侧栏、检查器和 Dock 时用它。
- [DesktopShell](desktop-shell.md) — 桌面壳。格子里的内容仍由你给。
- [AppTitleBar](app-title-bar.md) — 标题栏。高度是 `TITLE_BAR_HEIGHT`。
- [Toolbar](toolbar.md) — 工具条。里面的按钮由你放。
- [ToolGroup](tool-group.md) — 工具组。一组工具按钮放在同一条底槽上，横排或竖排，按单选、开关或操作读出。
- [StatusBar](status-bar.md) — 状态条。变化会念成 live status。
- [MediaTransportBar](media-transport-bar.md) — 播放、进度、音量、设置和全屏。
- [Workspace](workspace.md) — 工作区。内容是树上的子节点。
- [SidebarFrame](sidebar-frame.md) — 侧栏框。不内置产品导航。
- [SidebarSection](sidebar-section.md) — 侧栏分组。正文里的行由你给。
- [SidebarRow](sidebar-row.md) — 侧栏一行。复用列表项的槽来画。
- [SettingsRow](settings-row.md) — 设置行；`label_column` 改成检查器式的对齐标签列。具体的值仍由你的状态拥有。
- [Dock](dock.md) — 停靠。窗格里的内容是子节点。
- [AreaLayout](area-layout.md) — DCC 式区域切分：从角上拖动切开或合并，区域可最大化，头部和内容按键放入，布局可序列化。
- [SplitPane](split-pane.md) — 分割。尺寸来自 `SplitPaneModel`。
- [PaneChrome](pane-chrome.md) — 页签条和一排图标动作。

每个区域里放什么由你的应用决定，见 [工作区](../reference/workspace.md)。

## 表面

尺寸、颜色、字体和控件表面来自主题。深色和浅色切换的是颜色。单行控件有小、中、大三档高度。具体数值和换主题的方式见 [视觉](../reference/look.md)。

有些族需要单独的 Cargo feature，例如 `calendar`、`charts`、`graph-canvas`、`rich-text`。`components` 会启用全部可选族。表见 [应用 API](../reference/application-api.md)。
