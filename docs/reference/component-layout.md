# 组件内容布局与迁移审计

组件保留语义、交互和 chrome。普通盒子排列交给 Layout Foundation；`ComponentGeometry` 是已发布部件的绘制投影。本文记录 #205 的迁移边界，未迁移行不能据此宣称验收完成。

## 当前产品路径

Button、Chip 和 ActionMenuItem 的旧 API 保持不变。组件内容声明为轻量 Flex parts，调用 `layout_engine::parts`；其主轴分配、收缩和对齐复用 retained Flex 的求解函数。部件没有 `StableNodeId`，不会产生 wrapper、组件生命周期或另一棵树。

`UiWorld` 在同一次布局发布中把 `TextContent`、`LeadingContent`、`TrailingContent`、`Hint`、`Spinner` 写入 `LayoutResult.parts`。文本片段使用实际的 `TextContent` 盒子。Scene 的组件几何只读取这些部件并附加颜色、文字、图标等绘制数据。发布期间的兼容查询调用同一个部件求解器。

纯文字 Button 和 Chip 沿用原来的两个部件（文本和内容），不增加 retained 节点或部件。loading、内容对齐、菜单 hint 与图标占位的变化失效布局输入；hover、disabled 颜色和 spinner 相位保留布局结果。极窄的内容盒会裁限部件范围，避免图标或 hint 产生负宽度。

一个控件把内容交给部件，就只留一份部件声明：intrinsic measure 取其中除文字以外的宽度与间距（`parts::chrome_extent`），段落预算是可用宽度减去同一份（`parts::text_budget`），placement 用 Flex 求解它们，绘制读发布的结果。Checkbox 按这条规则迁移：指示框与标签是 `LeadingContent` / `TextContent` 两个部件，一个 Checkbox 仍是一个实体、三个 parts（文字、内容、指示框）；勾选、悬停只改绘制，零布局。指示框按内容盒取正方形，内容盒比指示框还矮时标签跟着左移，不再空出原来那段固定宽度。Switch 的文字不是节点文字，标签和 hint 画在部件里：一行是 track 与文字列，文字列是嵌套的 Flex 列，带 hint 的开关的高度由测量这份声明给出，样式不再写 `min_height`。track 与间隔不出现在开关自己的样式里，主题改动它们时开关照样重新测量（`visual_parts_read_theme`）。

标签被清空的控件（Button、Chip、Checkbox 以及任何自带文字的盒子）交出原文字的度量，和冷布局时没有文字的同一控件相同。此前没有文字的盒子不再进文字工作，旧度量留着，清空标签的 Button 仍保持原来的宽度。

已继续接入 [InlineContent](inline-content.md)：Text/Button/Chip/ActionMenuItem 接受 canonical 内容，普通 retained child 通过 nana-text object placement 混排，typed fragments 携带 source range；高级控件文字复用 Runtime retained layout，atomic Text 子项独立绘制。`StandardVisual` 的兼容 label/icon 字段仍在；ListItem 的 fallback label/note/detail 已继续迁移为 Foundation 的 TextContent/Note/Detail 部件，复用 Flex 分配与 RTL placement；leading/content/trailing 仍为普通 retained slots，读取已有 child placements。SidebarRow 与 FileTab 复用这一路径；纯 label 不额外测量 note/detail。其余组件和完整 chrome/content 拆分继续按下表迁移。

## mini-layout 审计

分类沿用 #205：**1** 应由 Flex/Grid/Inline/Overlay 表达；**2** 应由 Layout Part/Fragment 表达；**3** 组件语义几何（交互、数据行、文本编辑）；**4** 非盒式视觉几何（图表、路径、画布）。只迁移 1/2，3/4 保留领域算法，外部盒子照样读 `LayoutResult`。下表覆盖 `UiWorld::derive_component_geometry` 的全部分支（2026-10-29 逐项核对源码）。

| 组件或几何 | 分类 | 当前状态与迁移路径 |
| --- | --- | --- |
| Button / Chip | 2 | 已迁移：文字、前后图标和 spinner 是 Flex parts；intrinsic measure 与段落预算读同一组部件（`chrome_extent` / `text_budget`）；绘制保留 chrome 与语义 |
| ActionMenuItem | 2 | 已迁移 icon / label / hint；hint 更新复用原节点 |
| Checkbox | 2 | 已迁移：指示框是 `LeadingContent`，标签是 `TextContent`，标签取余下宽度。intrinsic measure、段落预算、placement 与绘制都读 `UiWorld::checkbox_parts`，原先 measure、`text_shape_constraints` 与 geometry 各算一遍指示框的占位，现已删除；Scene 不再自带指示框和标签的让位。TextRun 片段落在标签部件里（此前落在整个内容盒，与指示框重叠）。RTL 与以前一样不镜像 |
| ListItem / SidebarRow / FileTab | 1 + 2 | retained slots 使用已有 child placements；fallback label/note/detail 已迁移，隐藏 slot 不占文字预算 |
| FormField / LabeledValue | 1 + 2 | LabeledValue 已发布 Label/Value，action 是普通 child；FormField 已发布 Label/Hint/LeadingContent，control 是普通 child |
| Card | 2 | 已发布 Header/Spinner，body 使用普通 children 与 padding |
| Panel | 1 | 本来就是普通 Flex 列（`Stack::fill_column`），标题、动作、正文都是子节点，没有组件几何，无需迁移 |
| Select trigger | 2 | 已发布 TextContent/Hint/TrailingContent，Geometry 不再重算 trigger fallback |
| Select / Dropdown / SearchDropdown 菜单 | 3 | 打开的菜单是字段的数据行，不是布局盒子：行几何在 `select_menu_geometry`，摆放走 #200 的锚点解析，label/hint 分割与 ContextMenu 共用 `menu_option_text_boxes`；无障碍选项读同一份 `SelectMenuGeometry`。可用空间 `select_menu_room` 原先自己沿祖先走一遍、自己判断逃出和 `fixed`，现改为沿 `UiWorld::clip_ancestors`（发布的 clip 链）；`fixed` 的字段因此取整个窗口，不再被它所在的滚动容器截 |
| ContextMenu 行 | 3 | 同为数据行。剩余重复：`apply_anchor` 按默认度量写的 `context_menu_height` 会被投影里按主题算的 `context_menu_height_in` 覆盖，待删 |
| 组件铬裁剪（EmptyState、ModalFrame） | 3 | 命中索引按 `clip_parent` 挂条目；无障碍的铬裁剪原先自己沿祖先走，现与菜单空间一样沿 `clip_ancestors`，`fixed` 盒子不再被祖先的铬裁剪截 |
| Switch | 2 | 已迁移：一行是 track 与文字列，文字列是嵌套的 Flex 列（标签 `Label` 在上、`Hint` 在下）；`control_position: End` 是同一行的 reverse，track 是 `TrailingContent`，否则是 `LeadingContent`。`UiWorld::switch_inputs` 从主题读一次，intrinsic measure（`SwitchInputs::extent`）与 placement（`parts::switch_parts`）都读它。删除：`project` 按行高写的 `min_height`（改为 `ControlHeight::Min` 下限加测量）、`text_shape_constraints` 与 `text_visual_key` 的 track 占位、geometry 的手算、Scene 的 `SWITCH_TEXT_INSET` 和无几何时自画的 30×16 轨道。标签文字不计入宽度（与迁移前相同）。RTL 不镜像 |
| StatusBadge / ValidationMessage | 2，待迁移 | 指示点的占位写成 padding，geometry 再按同一组常量算一遍指示点和标签 |
| SelectionOption（radio 行、分段项） | 2，待迁移 | ring、icon、label 手算，占位写成 `radio_lead` 等 padding |
| EmptyState | 2，待迁移 | icon/title/message 手算；action 是普通 child |
| Toast | 2，待迁移 | indicator/title/description/dismiss 手算 |
| MenuSurface 触发器 | 2，待迁移 | `menu_surface_geometry` 手算 trigger 文字与图标 |
| Range / Progress / LevelMeter | 2 + 3 | label/value/unit/cancel 文字为 2（待迁移）；track、fill、thumb 为交互几何 |
| ModalFrame | 1 + 2 | slots 已走 Overlay context（#199）；title/description/body 文本仍由 `ModalChrome::measure_in` 排 |
| PathText | 2 | 只读已发布的内容盒做中间省略，没有自己的位置计算 |
| TextInput / 编辑器 caret、selection、IME、gutter | 3 | 由 `nana-text` 和 Runtime 文本布局提供 |
| TreeView / CommandPalette / ReorderList 行、Scrollbar、XYPad、resize grip、KeyCapture/Keymap 徽标 | 3 | 数据行、虚拟行或交互几何；外部盒子读 LayoutResult |
| Chart / ChartTooltip / CalendarHeatmap / QrCode / GraphCanvas / GraphMinimap / NativeMarkdown / ImageViewer 舞台 | 4 | 保留领域算法 |

## 验证入口

- `cargo test --locked -p nana-ui-runtime --lib issue205`：部件与绘制投影一致、hint/loading 更新、a11y、hover 零布局、1k/10k 单实体和 retained parts 数量；Checkbox 的部件、measure、预算与冷布局对照；清空标签后与冷布局相同；菜单空间与铬裁剪沿 clip 链。
- `cargo test --locked -p nana-ui-runtime --lib issue199`：1k 实际组件跨 context / 正常流与脱流切换，保留状态、焦点和资源身份。
- `cargo test --locked -p nana-ui-devtools --features runtime-agent --test issue205_component_pixels`：实际 GPU 的组件图像和 hover；设置 `NANA_COMPONENT_REQUIRE_GPU=1` 时无 GPU 必须失败。

Card 标题复用 Runtime 的 retained paragraph，具有独立的单行字形预算与字号/字重；这些文字样式不继承到 body。Header baseline 与所有 context 的 intrinsic baseline 共用同一部件，标题高度只由 band 保留一次。spinner 相位为 paint-only；窄标题的溢出字形裁剪到 Header，不能占用 spinner 间隔。

CPU p95 和 allocator/RSS 的历史比例需要固定机器上的前后报告。实体数和 parts 数量门禁只证明结构成本，不替代 #188 的 CPU ≤5% / memory ≤8% 比例验收。

参见 [布局结果](layout-result.md)、[布局](layout.md)、[组件](components.md)。
