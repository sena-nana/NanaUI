# 视图迁移补缺（二）：图片查看器的控件与遮罩、对话框长高、换行行、富触发器

LiliaBilibili 迁到声明式视图时发现的第二批框架缺口。说明见 [控件](../../docs/reference/components.md)、[声明式视图](../../docs/reference/reactive-view.md)、[布局](../../docs/reference/layout.md)、[窗口](../../docs/reference/window.md)、[主题](../../docs/reference/theme.md)。

## 需要改的地方

- **`ImageViewerEvent` 多了 `Previous`、`Next`，`MediaTransportEvent` 多了 `SeekStarted`、`SeekEnded`。** 穷举匹配它们的代码要加分支。
- **`ComponentGeometry::ImageViewer` 去掉了 `close` 字段**：关闭钮不再是查看器自己画的一块，而是它装配的 `IconButton` 子节点。关闭钮的位置仍在 `ImageViewer::geometry(..).close`。
- **公开结构体多了字段**：`ImageViewer` 的 `gallery`、`close_label`、`previous_label`、`next_label`；`EffectTokens` 的 `media_scrim`；`TriggeredMenuOverlay` 的 `trigger_content`；`Popover` 的 `trigger_content`；`DesktopShell` 的 `workspace_corners`。用结构体字面量构造它们的代码要补上（用 `new()` / `for_mode(..)` 的不受影响）。
- **`create_component` 手工建的 `ImageViewer` 要调一次 `assemble_image_viewer(viewer)`** 才有关闭钮（视图里建的、以及之后每次写入都会自动装配）。
- 应用里为绕开缺口写的代码可以删掉：
  - 给关闭钮留边的舞台几何（`attachment_geometry` 那种把舞台往下挪的写法）——内容铺满也盖不住关闭钮了。
  - 自己处理图集的 ← / → 和写在 `metadata` 里的「3 / 9 · ← → 切换图片」提示：设 `gallery`，听 `Previous` / `Next`，换图后把新位置写回。应用自己的键盘策略先于框架拿到按键，两边都处理会走两步。
  - 对话框正文区设成占满可给高度来绕开「长高后按旧高度裁剪与命中」。
  - `FormField` 的 `.child_slot(control, FormField::control_child)` 可以写成 `.control(control)`（钉版 3afe5fa 里已经有，不是这批新增）。
  - 用 `flex-wrap` 时为了「放得下就并排」而写的按宽度切换排法的信号；`flex: 1 1 320px` 这样的写法现在就能换行。
  - 读布局盒来选响应式排法的代码：改听 `SizeChanged`。
  - 在框架建的 `Workspace` 节点上写 `workspace_corners`：写到 `DesktopShell::workspace_corners` 上，否则下一次装配会用壳上的值盖掉。
  - 挂载后往条的设置 `ActionMenu` 里追加条目、在条内部进度控件上听 `RangeInput` 判断拖动开始：用 `.settings(view)` 和 `SeekStarted` / `SeekEnded`。
  - 在 `Popover` 外面自己拼「图标 + 文字 + 计数」按钮再手动开合：用 `.trigger(view)`。

## 行为变化

- **图片查看器的关闭钮是真控件**，排在查看器所有子节点之后：`Child` 内容盖不住它、按不到它下面；它可焦点、有名字（默认「关闭」）、有悬停和按下态。仍把指针事件转发给 `image_viewer_pointer_down` 的宿主，在关闭钮上按下时会先从转发收到一次 `Close`、松手时再从按钮收到一次；关闭本来就应当是幂等的，要去重就在关闭处理里判断当前是否还开着。Vue 的 `nana-image-viewer` 只做投影，不再画那个不能按的「×」。
- **图片查看器的遮罩在浅色主题下也是深色**（主题的 `EffectTokens::media_scrim`，默认黑 0.9）；深色主题下由原来的背景色 0.94 换成它，几乎看不出差别。
- **对话框正文长高**（异步内容到了）后，面板按新高度裁剪、命中和计算无障碍边界。
- **换行的行按项目长大之前的尺寸分行**：带 `flex-grow` 的项目不再独占一行；`flex-basis` / 主轴尺寸 / 内容，受 `min-*` / `max-*` 约束。依赖旧行为（`flex: 1` 的项目在换行行里一项一行）的布局会变成并排，要一项一行就给它们 `flex-basis: 100%` 或最小宽度。
- **flex 子项的 `min-width` / `min-height` 写 `min-content` / `max-content` / `fit-content` 时生效**（以前按 0 处理），下限是量出来的内容尺寸。
- **标题栏放不下时中间列让位**：右侧列（应用按钮 + 窗口按钮）至少保有内容宽度，中间列先挪、再收窄（可收到零、内容省略），左侧列会被挤窄；窗口按钮不再被裁。组件库里 360 px 的弹窗标题栏因此右移了 10 px 的窗口按钮回到窗内。
- **弹出层关上时，焦点若还在它的条目上，回到触发器**（`Popover`、`ActionMenu`，不论是按触发器、Escape、点外面还是应用写 `open = false` 关的）。
- **模态面板打开时，底下的拖动手柄不再靠几像素容差抢按下**，光标也不在面板上变成拖动样式。
- **`RangeField` 的指针拖动前后各发一次 `RangeDragging { dragging }`**。

## 新增

- `ImageViewer::gallery(index, count)`、`ImageViewerPosition`、`previous_label` / `next_label` / `close_label`，`AppContext::assemble_image_viewer`。
- `EffectTokens::media_scrim`。
- `SizeChanged { width, height }`：监听它的节点在布局后尺寸变了时收到。
- `teleport(..)` 的 `.class(..)`、`.class_when(..)`、`.css(..)`，模板 `<Teleport to={..} class="..">`：给锚点。
- `Popover::trigger_content(id)`、视图 `El<Popover>::trigger(view)` / `El<ActionMenu>::trigger(view)`（模板 `<template #trigger>`）。
- `MediaTransportBar::settings_content(id)`、视图 `.settings(view)`（模板 `<template #settings>`）；`MediaTransportEvent::SeekStarted` / `SeekEnded`；`RangeDragging`。
- `DesktopShell::workspace_corners(bool)`。
- `El<AppTitleBar>` 的 `.visible(..)`、`.class(..)`、`.css(..)`。
