# 控件

你用现成控件往树上挂。不要每个按钮自己画。Rust 入口是 `nana_ui::runtime`。

```rust
let (_, save) = cx.mount_view_root(document_id, || {
    let save = entity_ref::<Button>();
    let view = button("保存").entity_ref(save).on(|_: &Activate| {
        // 应用自己的保存逻辑
    });
    with_refs(view, save)
})?;
```

迁 Vue 时，同一套控件从 `@nanaui/nanavue-components` 引入。它们进同一棵树：

```js
import { NanaButton, NanaInput, NanaDialog } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

名称对照和 props 见该包 README（含 `NanaScrollView`、`NanaNumberInput`、`NanaTable`、`NanaDesktopShell` 等与本目录一一对应的包装）。视觉与尺寸见 [视觉](look.md)。

## 目录

**操作与输入。** `Button`、`IconButton`、`TextInput`、`TextArea`、`NumberInput`、`Checkbox`、`Switch`、`RangeField`、`RangeSpanField`、`Select`、`Dropdown`、`SearchDropdown`、`SegmentedControl`、`Tabs`、`XYPad`、`ColorField`、`PathField`、`DatePicker`。`RangeField` 默认在轨道旁画当前值和单位。`.show_value(false)`（Vue `showValue`）只留轨道。读屏仍能读到数值。`.show_label(false)` 不画标签。轨道占满。标签仍是读屏名称。`.rail(粗细)` 只画一条横贯控件的细轨，圆点只在焦点可见时出现，控件高度就是命中区。`RangeSpanField`（`<RangeSpan>`）是两个滑块圈出的区间，可竖向（最小值在下）；两个滑块各是一个焦点停留点和读屏 `Slider`。`indicator` 是轨道上不吸附、不收输入的实时值，只改它是一次绘制更新。轨道两端从节点边缘内缩 `range_span_track_inset(size)`，旁边的自绘按它对齐。

**布局与文本基元。** `Text`、`Stack`（`row` / `column` / `bar` / `spacer` / `overlay_layer` 等预设）、`Divider`、`IconGlyph`、`ScrollView`。`Stack::spacer()` 是零宽 flex-grow。它把其后兄弟推到行尾。`Stack::overlay_layer()` 铺满已定位父级。它脱流。不命中。会裁剪。给舞台 HUD / 弹幕当容器。节点池仍由你的应用挂。`Divider` 默认交叉轴 `Fill` + `align_self: Stretch`。放进 `align_items: Start` 的列里仍能看见。

**表格与树。** `Table` / `TableRow` / `TableCell`、`TreeView`、`ReorderList`。列可 `sortable(true)`。表头激活走 `VirtualTableLayout::toggle_sort`（升序 → 降序 → 取消）。`move_column` 重排列。**排序本身仍由应用做。** 只有你知道数据怎么比。

**展示。** `Card`、`List` / `ListItem`、`FormField`、`EmptyState`、`Progress`、`Skeleton`、`Spinner`、`StatusBadge`、`Chip`、`Avatar`、`Tooltip`、`ValidationMessage`、`QrCode`、`ImageViewer`、`NativeMarkdown`、`CalendarHeatmap`、`Chart`、`GraphCanvas`、`GraphMinimap`。

**浮层。** `Dialog`、`ConfirmDialog`、`Drawer`、`Popover`、`ActionMenu`、`ContextMenu`、`CommandPalette`。浮层由框架放在窗口里。靠近边缘时收进视口。不要用 `position: fixed` 自己搭一层。`Popover` / `ActionMenu` 的触发器支持文本（`trigger`）与图标（`trigger_icon`）两种。图标触发器渲染为 28×28 方形按钮。图标在按钮内几何居中。可访问名由 `trigger_icon` 的 label 提供。裸符号（如 `+`）不要用文本触发器。要「图标 + 文字 + 计数」这类触发器（收藏 1800），用视图的 `.trigger(view)` 具名 slot（`Popover::trigger_content(id)`）。那段内容是触发器自己的子节点。它在触发器的外壳里横排。面板关着时照样显示。不算面板的条目。`trigger` 文本仍是可访问名。**触发器仍是 popover 自己。** 按下、焦点、Enter / 空格、面板的锚点都是它的。所以这段内容只做显示。里面不要放可按的控件。面板因按它的触发器、Escape、点外面或应用写 `open = false` 关上时，焦点若还在面板里的条目上，回到触发器。取舍：没有做成「任意元素 `anchor: NodeRef` 打开 popover」。那样面板要跟随另一个节点的盒子。布局里要多一条跨节点依赖（锚点挪了而 popover 没重新布局时面板会留在旧处）。还要额外接线打开、焦点归还和点外判定。slot 让锚点、命中、键盘和关闭沿用 popover 现有的一条路径。代价是触发器的外壳（默认 Subtle 底、控件高度）由 popover 给。`Popover::bare_trigger(true)`（`ActionMenu` 同样转发）去掉静止底色和描边，悬停、按下和打开时仍铺一层洗色。不传则保持浅底加描边。弹出表面是 viewport-fixed。**不进入父级 isolation group。** 卡内菜单会画到后面的兄弟卡之上。重叠处命中同一排序。应用不必给整张卡抬 `z_index`。所在面板的 overflow 裁剪也裁不到它。卡面和面板内容同一层。盖在触发器和页面后面的内容（包括 `z_index` 更高的后出现兄弟）之上。卡面空白处的按下留在面板里。不落到底下。也不开合面板。`Popover` / `ActionMenu` / `HoverCard` 的弹出层挂在触发器**显示的位置**上（经过祖先滚动与变换之后的盒子）。按 `placement` 的一侧放不下而对侧放得下时，翻到对侧。再沿两轴收进视口。两侧都放不下才会压住触发器。打开后滚动页面，弹出层留在原处，直到下一次布局。`DesktopShell` 有两层 `OverlayHost`。`overlay` 放对话框。`status` 放 toast。确认框打开时 toast 仍可显示。模态面板（对话框、抽屉）打开时，它的表面拿下落在上面的每一次按下。底下侧栏、分栏、Dock 的拖动手柄靠「几像素容差」抢按下的规则，只对没被模态盖住的手柄生效。光标也不会在面板上变成拖动样式。

`Chip`、`ColorField`、`PathField`、`FileTab` 这类**叶子复合件**在你写 props 的那一刻自己重建子节点。不需要再记一次 `assemble_*`。`Shell` / `Workspace` / `Dock` / `SplitPane` / `PaneSection` 不走这条。它们协调的是应用自己的槽位，而且不便宜。挂到每次写入会破坏「无变更不弄脏」的脏帧合同。这几个仍在装配好槽位后显式调用对应的 `assemble_*`。

`ConfirmDialog` 的确认 / 取消按钮由 `AppContext::assemble_confirm_dialog(dialog)` 建好，并接上 `ConfirmIntent`。不需要自己造两个按钮再拼槽位。按钮文案用 `confirm_label` / `cancel_label` 覆盖。`danger(true)` 让确认按钮走危险色，标题用危险口气（`Dialog::danger` 同一个口气）。需要次要动作、关闭钮或自定义正文时，仍用 `set_confirm_slots` 自己装配。这时 `assemble_confirm_dialog` 不会覆盖你已有的槽位。

`ContextMenu` 同样挂在 `OverlayHost` 下，并用 `activate_overlay` 打开。框架按 Menu 语义负责 Escape 与点击外部收起。应用不再自建点外判定。框架驱动的收起会同步组件自身的 `open`，并发出 `ContextMenuEvent::Dismiss`。与选中项收起走同一条回执。应用不需要事后对账两份状态。

`Toast::place_in(viewport, PopoverAlignment::Center, max_width, PanelInsets { .. })` 用与 `Panel::viewport` 相同的预留合同，把提示钉进空闲区域。`align` 决定它在剩余宽度里的分布。`max_width` 封顶。高度仍由内容决定，并贴住预留的底边。`viewport` 是定位宿主自身的盒子。未放置的 toast 仍然填满所在行。放置过的 toast 保留自己的宽度。空闲区域由应用给出。只有应用知道它开了哪些面板。

`Toast::timeout(Duration)` 让提示挂载满这么久后发一次 `ToastDismissed`，与按下关闭按钮的事件相同。计时器每个提示只启动一次，只在到期时唤醒宿主一次，不逐帧推进。不传则一直等用户关闭。框架不会自己移除提示，收到事件后由应用从列表里删掉。

`Spinner` 自己转。挂载后由 Motion IR 无限 timeline 驱动旋转相位。宿主不逐帧喂 phase。停用改为卸载或停放该节点。

`Panel` 是非模态任务面板。挂到独立 `OverlayHost` 的直接子节点。通过同一份 `activate_overlay` / `dismiss_overlay` 管理显示和退出动画。面板使用 Card 表面和具名 Region 无障碍语义。只有卡面命中。外部舞台和普通 Tab 顺序保持可用。不能用 Menu 或 Dialog 冒充非模态面板。可见标题、返回/关闭按钮和内容由应用装配为普通子节点。长内容使用 `ScrollView`。关闭按钮调用 `dismiss_overlay`。不直接删除节点。

`Panel::viewport(viewport, PanelEdge::Right, width, PanelInsets { .. })` 将宽高限制在应用预留标题栏、底栏等空间之后的视口内。也可用 `Panel::bounds` 获取同一布局合同的矩形。`viewport` 使用相对于宿主的逻辑坐标。宿主必须覆盖传入视口。`Panel::style` 可接入现有样式。没有第二套主题或绘制器。每个同时存在的面板使用独立 host。层级通过现有 `z_index` 控制。确认框应位于任务面板上方。

激活默认聚焦首个有效子控件。恢复固定面板时用 `focus_on_open(false)` 保留当前焦点。关闭时只有焦点仍在面板内才恢复有效入口。隐藏、卸载或失效入口不会被聚焦。`focus_first_in(document, root)` 使用 Runtime 的 Tab 候选规则聚焦子树。需要先返回业务详情的应用用 `close_on_escape(false)` 接管 Escape，再在路由根关闭面板。业务导航、固定偏好和窄窗时收起哪个面板由应用拥有。

同一 host 只激活一个直接子面板。其余子树保留内容和编辑状态。但不绘制。不命中。不参与 Tab。无需 park。固定/取消固定同一内容时，使用 `transfer_panel(panel, source_host, destination_host)` 原子移交到同文档中的另一 host。保留节点、焦点和动画。不先 dismiss 再 reparent。目标旧面板成为 inactive。两个 host 分别收到 `OverlayChanged`。不发送 `OverlayClosing`。关闭退场中的面板不可转移。

inactive overlay 与关闭菜单属于结构性隐藏。`ComputedStyle::box_visible` 沿整棵子树继承。子节点不能用 CSS `visibility: visible` 重新参与绘制。普通 CSS `visibility: hidden` 仍允许子节点显式覆盖。关闭保留的 Panel 不重置 `ScrollView` 滚动偏移。重新打开相同节点且内容与视口不变时，恢复原位置。内容或视口变化时仍遵循正常范围约束。`nana-ui-scene` 的 `RuntimeDocument` 行为回归通过实际增量场景图元检查关闭、重开和切换。并在完整布局刷新前后验证关闭重开的滚动偏移。不仅断言 world 显隐字段。

用户的关闭手势（Escape、点外面、关闭位）先在 `Dialog`、`ConfirmDialog`、`Drawer` 自己身上发 `DialogCloseRequested { trigger }`，再按 `close_policy` 决定框架关不关。策略不允许的手势只发请求，浮层留着。`DialogClosePolicy::requests_only()` 让框架一种都不关，开合全由应用决定。应用不必轮询无障碍树去发现「对话框被框架关了」。

`Dialog`、`ConfirmDialog`、`Drawer` 也可以声明开合：`.open(..)` / `.model(信号)`，挂在 `OverlayHost` 下时就是 `activate_overlay` / `dismiss_overlay`，宿主自己做的开合发 `DialogToggled` 并写回 `open`（见 [Dialog](../components/dialog.md#声明式开合)）。

`dismiss_overlay` 先关闭交互并恢复焦点。再保留菜单/对话框绘制到退出动画结束。宿主通过 `OverlayClosing { root }` 同步业务打开状态。通过 `OverlayChanged { active: None }` 处理最终释放。排队的关闭通知应在下一次投影前消费，并核对浮层身份，避免覆盖快速重开。退出期间保留父子关系和 `DesktopShell.overlays` 中的节点。直接 `remove_view` 会立即释放，并跳过退出动画。

**壳层。** `AppShell` / `DesktopShell`、`AppTitleBar`、`Toolbar`、`StatusBar`、`MediaTransportBar`、`Workspace`、`SidebarFrame` / `SidebarSection` / `SidebarRow`、设置行和设置页、`Dock`、`SplitPane`、`PaneChrome`。壳是通用桌面结构。每个区域里放什么由应用决定。见 [工作区](workspace.md)。主区域圆不圆角（外观设置里的「工作区圆角」）写在 `DesktopShell::workspace_corners` 上。壳装配时交给它建的 `Workspace`。不要去改框架建的 `Workspace` 节点。下一次装配会用壳上的值盖掉。

`SettingsRow::stack_below(480.0)` 可选开启按**该行实际布局宽度**的响应式排列。小于阈值时标签与控件上下排列。等于或大于时同行。默认未开启。`stacked(true)` 始终上下排列。无效阈值视为未开启。控件节点不重建。容器调整宽度由 Runtime 布局回流处理。应用不需要每帧扫描行。Vue 对应 `NanaSettingsRow` 的 `stackBelow` 属性。

设置行的标签给它放的控件起无障碍名字。`SettingsRow` 装配时（视图里的 `settings_row(..).control(..)`、`mount_settings_leaf_row`、`AppearanceSection` 的各行）用 `MutationQueue::set_labelled_by(控件, 标签节点)` 把两者关联起来。控件自己有非空的名字时用它自己的（`switch("静音")`）。没有时（`switch("")`、分段控件、滑块、下拉选择）用行标签。行标签改了，控件的无障碍节点跟着重新投影。这层关联归组合控件所有。存在 `UiWorld` 里。不是控件 `AccessibilityState` 的一部分。所以控件重新投影自己的状态不会冲掉它。任一端被销毁时关联一起去掉。`Select` 的无障碍名字因此不再取它显示的选项。没人给它起名时仍按显示的文字命名。显示的选项是它的值（`value`）。

设置行之外，视图里的任何元素都能声明同一层关联：`.labelled_by(标题)`（模板 `labelled_by={caption}`，`.vue` 里 `labelled-by="caption"`）。`标题` 是那段文字的 `NodeRef`、节点 id 或选出它的闭包。写在控件之前、之后或另一段视图里都行。标题重建（在 `when` 里）后关联跟到新节点上。字段标题写在旁边的下拉框、滑块、空标签开关都这样起名，不用挂载后再扫描文档。没有可见标题时，`Select` 写自己的 `label`（`aria-label`，不画出来），`RangeField` 写 `label` 加 `show_label(false)`。

`mount_settings_leaf_row` 保留标签、提示和控件槽。初始没有提示也可以随后通过 `update_component(row, |row, _| row.hint = Some(...))` 显示提示。设置 `None` 隐藏。`assemble_appearance_section` 刷新已有行时保留应用设置的 `stack_below` 与 `stacked`。


部分族需要 Cargo feature（`calendar`、`charts`、`graph-canvas`、`rich-text` 等）。见 [应用 API](application-api.md)。这些 feature 会转发到 `nana-ui-runtime` 和 `nana-ui-scene`。它们控制对应实现、几何投影和公开导出。`components` 启用全部可选控件族。精简宿主按需选择。Vue 标签也受相同功能开关约束。缺失时会报告组件不可用。

`DatePicker` 是月历网格。由现有控件组合而成（表头图标按钮 + 日期按钮）。`assemble_date_picker` 建好并复用这 6×7 个按钮。翻月只换标签，不重建。选中发 `DateChanged`。翻月发 `DateCursorMoved`。`range` 之外与非本月的日期不可选。**月份标题由应用给**（`month_label`）。月名是 locale 相关的。框架不带 locale 数据。日期类型是 `nana_ui_core::CivilDate`。只有年月日。不是日期时间库。

`Toolbar` / `StatusBar` 是两条横条容器。内容由应用放。相对裸 `Stack::bar` 多的是壳层表面和**无障碍角色**。读屏把工具栏播报成一组控件。状态栏播报成 live status 区域。普通布局盒表达不了这个。`chrome(false)` 用于已经自带表面的父容器。

两条横条放不下时会先收紧再溢出（Dynamic Layout，见 [布局](layout.md) 的「Dynamic Layout」）：横条自己的 gap 先收到四分之一，再收 `Button`、`Chip` 声明的左右 padding（各收到三分之一），都收完还放不下才溢出或交给 shrink。这是横条样式里 `adaptation: Some(AdaptationProfile::strip())` 带来的；`Button` / `Chip` 默认带 `AdaptationProfile::control()`，只是声明，放在不求解的容器里（普通 `Stack`、行、列、Dock、SplitPane）不读也不花代价。任何容器写 `solve_overflow: true` 都能这样求解；要让某个按钮不被收紧，把它的 `adaptation` 设成 `AdaptationProfile::RIGID`。`StatusBadge` 不声明：状态点画在左侧 padding 里，收紧会让文字压上状态点。

拖放：`set_drop_target(entity, DropAccepts::files())` 登记节点接受什么。`drop_target_at(document, x, y, kind)` 回答某点上最内层接受该载荷的节点（按布局盒匹配，不要求节点可点击）。**框架只回答落在哪里。** 落下之后做什么仍是应用的。和 `SecondaryPress` 一样。Tab / Dock / `ReorderList` 的拖动移动的是框架自己的结构。仍走各自的合同。

悬停显隐：给节点挂 `PointerHoverChanged` 处理函数，指针进入它的子树时收到 `hovered: true`，离开时收到 `false`。在子树内的两个后代之间移动不会重复发。只有挂了处理函数的节点收到，由内向外。`Stack` 默认不参与命中，指针停在行的空白处时命中不到它，要用 `.hittable()` 让整行接住悬停。框架只报告进出，显示哪些工具是应用的事。

键盘焦点用 `FocusWithinChanged` 报告，规则和悬停相同：焦点进入节点子树时收到 `focused: true`，离开时收到 `false`，在子树内移动不重复发。监听者正在 `update_component` 里时，事件等这次更新结束再送到。`VisibilitySpec::Hidden` 的节点不进 Tab 顺序，所以悬停才露出的工具要用 `opacity: 0` 收起，让 Tab 仍能落到它们上面，再在 `FocusWithinChanged` 里和悬停一起显出来。

选区位置：`UiWorld::text_selection_bounds(id)` 回答 `NativeMarkdown` / `SelectableRichText` 的选区在窗口里的外接矩形，已计入上层滚动和变换，没有选区时为 `None`。用来把浮动工具条锚在选区上：在改变选区的更新返回之后再问，滚动后重新问。配合 `resolve_popover_origin` 决定放在选区上方还是下方。

`ColorField` 是色块 + hex。`assemble_color_field` 挂 HSV 选择器。提交发 `ColorChanged`。拖动发 `ColorInput`。`PathField` 是路径 + 浏览按钮。浏览只发 `BrowseRequested`。由应用打开系统对话框。

### FindReplaceBar

`FindReplaceBar` 是可复用的查找 / 替换工具条（`nana.find-replace-bar`，别名 `text-search-bar`）。它只维护查找与替换草稿、展开状态和只读门控；文档搜索及修改由应用处理。调用 `assemble_find_replace_bar` 后会保留两个 `TextInput`、上一处 / 下一处 / 替换 / 全部替换按钮和反馈文本。`read_only(true)` 仍允许导航，但会禁用替换输入与动作。用户输入和动作通过 `FindReplaceEvent::{QueryChanged, ReplacementChanged, Previous, Next, Replace, ReplaceAll, Expanded}` 发出。

### DynamicForm

`DynamicForm`（`nana.dynamic-form`，别名 `property-surface`）按稳定字段键保留动态的 `Switch`、`Dropdown`、`TextInput`、`TextArea` 和动作控件。`DynamicFormField::Section` 使用 `SettingsCard` 分组，文本字段支持 `multiline`、`secure` 和 `binding_identity`。值、校验、产品标签和持久化仍由应用维护；`DynamicFormEvent` 只把键和值送回宿主。

### InteractionRequestCard

`InteractionRequestCard` 是中性的请求/审批表面，提供 `prompt`、`body`、`actions` 槽，并按 `InteractionRequestField::key` 保留 `FormField` 包装。字段控件和动作由应用创建并监听，卡片不携带 Pending、MCP 或导航语义。

`GraphCanvas` 默认只画网格、节点框和边（Scene Quad / Stroke）。节点内部内容由应用往子节点里放。`"graph-canvas"` 自定义 GPU renderer 不会自动挂上。要直写 pass，须宿主自己登记并 `set_custom_render`。右键仍然只发 `SecondaryPress`（窗口坐标）。`AppContext::graph_canvas_hit_at` 把它换成画布局部点与命中结果（`GraphCanvasHit`）。菜单开不开、开什么由应用决定。`NativeMarkdown` 解析 mermaid 与公式围栏并给出 presenter 槽。但**不渲染**图和公式。那两样由宿主自己画进槽里。

`GraphMinimap` 是图画布的概览小地图。按 `GraphModel::bounds` 等比缩放画节点矩形和视口指示框。点击/拖拽发 `GraphMinimapEvent::ViewportRequested`。由应用写回 `GraphCanvas::set_viewport`。它自己是普通组件。`canvas_size` 传图画布的可见尺寸。位置尺寸由应用布局给定（通常是画布角落的 `PositionSpec::Absolute`）。

### Chip

`Chip` 是紧凑可选 token（pill）。身份是 `nana.chip` / Vue `<nana-chip>`。叶子复合件：写 `dismissible` 时 `update_component` 自己跑 `assemble_chip`。不必再记一次。`selected` 走 Selected 底。否则 Subtle。

| 操作 | Runtime | Vue `NanaChip` |
| --- | --- | --- |
| 点本体 / Enter / 空格 | `Activate` | `@press` / `@click` |
| 点关闭钮 | `ChipDismissed` | `@dismiss` |
| `disabled` | 不发事件、不可焦点 | 吞掉 press 与 dismiss |

关闭是请求。移除由应用做。关闭命中不得再给 Chip 发 `Activate` / `press`。默认关闭无障碍名是「移除」（`close_label`）。AccessKit：本体一个可激活 Button（`selected` 映射 selected 状态）。关闭钮另一个独立可焦点的 Button。不是 list item。

相对 `Button`：Button 是动作。没有 token 的 `selected` / `dismissible`。相对 `StatusBadge`：Badge 只展示。不激活。不关闭。

### ImageViewer

`ImageViewer` 是整窗的大图浮层（`nana.image-viewer`）。它自己画遮罩、表面、舞台与说明行。图片由应用给（`HostTexture` / `CustomRender` 由查看器画在舞台里并按舞台裁剪。`Child` 是应用自己的子节点）。遮罩是主题的 `EffectTokens::media_scrim`。浅色主题下也是深色。图片总是放在深色上看。表面与说明行仍随主题。内置值是黑 0.99，而不是 CSS 里常写的 0.9。画家在线性光里合成。黑色 α 留下背后 `1 - α` 的线性亮度。0.99 在白底上约 `#1a1a1a`（CSS 的 0.9 那么深）。0.9 则是中灰 `#595959`。自定义主题写这个值时按同样的算法取。查看器的**操作是真控件**。关闭钮是 `assemble_image_viewer` 建的 `IconButton` 子节点。可焦点。有无障碍名（`close_label`，默认「关闭」）。有悬停与按下态。激活时查看器发 `ImageViewerEvent::Close`。这些控件总排在查看器所有其他子节点之后。所以 `Child` 内容铺满整个画面也盖不住它们。命中也先落在它们上面。内容晚于控件放进来时（子节点变化）查看器会重新装配，把控件挪回末尾。它是叶子复合件。视图建好时与每次写入后自动装配。`create_component` 手工建的查看器自己调一次 `assemble_image_viewer`。`ImageViewer::geometry(..).close` 仍给出关闭钮的位置，供应用给自己的内容留边。

图集：`gallery: Option<ImageViewerPosition>`（`.gallery(index, count)`，`index` 从 0 起）告诉查看器当前图在应用图集里的位置。多于一张时，舞台底部居中出现一块表面。上面是「上一张」「下一张」两个图标按钮（`previous_label` / `next_label`）和位置「3 / 9」。查看器的无障碍值也是这个位置。到头的那一侧按钮禁用。按钮激活，以及焦点在查看器或它的控件上时按 ← / →，查看器发 `ImageViewerEvent::Previous` / `Next`。到头时方向键不消费。**换图由应用做。** 收到事件后换内容，并把新位置写回 `gallery`（缩放与平移也由应用决定是否复位）。只有一张或没有 `gallery` 时不显示导航。Vue 的 `nana-image-viewer` 只做投影。不建控件。关闭由宿主负责。

### Avatar

`Avatar` 是圆形 Cover-fit `HostTexture` 槽（`nana.avatar` / `<nana-avatar>`，采样 `nana.host-texture`）。默认不参与命中。不可焦点。空 `resource`、宿主清空、加载失败都走 Subtle 占位。**不**自绘产品字母。加载失败由宿主把 `resource` 清成空。不改缺槽拒绝帧的 GPU 合同。有 `label` 时 AccessKit 为 Image 且有名。无名则为 Image 无 name。无点击事件。与 `Thumbnail` 的区分见 rustdoc（Cover、圆形、固定边长）。

`HoverCard::trigger_image` 与 `Avatar` / `Thumbnail` 一样走 generation / version。`replace_view(generation)` / `invalidate_content()` 推进 Scene revision。宿主纹理晚到时，画面才会从占位换成实图。

### OverlayVisibility

媒体/舞台 HUD 的自动隐藏用 `OverlayVisibility` **策略对象**。`MediaTransportBar::auto_hide(true)` 让运行时自己驱动它（路由过的指针、按键与指针离开窗口，加上运行时时钟），每次翻转发 `OverlayVisibilityChanged`；只放视频的窗口用这条，见 [MediaTransportBar](../components/media-transport-bar.md#自动收起)。下面是宿主自己驱动的写法。idle 超时隐藏。hover dwell 延迟显现。焦点 / 拖拽 / 菜单锁（`OverlayLocks`）保持可见。`active = false`（加载 / 暂停 / 空）保持可见。它不是叶子控件。不进 `register_component`。没有 Vue 标签。不参与布局或命中。`MediaTransportBar` 内持一份策略。用 `AppContext::sync_overlay_visibility(bar, now, active)` 从 world 收集锁（焦点 / capture 是否在条或其菜单内、任一子 `Popover.open`）、推进策略、返回 wakeup。画面上的指针活动用 `reveal_overlay`。祖先查询走公开的 `AppContext::is_descendant`。**空闲收起不写条的 `hidden`。** 条自己的 `hidden`（视图里 `.visible(..)` / `v-show`）只归应用。用来表达「传输不可用时整条不出现」。两者任一要藏，条就不出现（`MediaTransportBar::shown()` 读合起来的结果，投影出去的节点样式同样是它）。所以绑定重跑（播放进度每次刷新）不会把空闲收起的条又显出来。`reveal_overlay` 也不会显出应用藏起的条。

### MediaTransportBar

媒体播放条（`nana.media-transport-bar`）。框架只提供基础 chrome：播放、点播进度 / 直播 Progress、音量弹出、设置 `ActionMenu`、全屏。Stacked 密度把当前时间和总时长分置进度条两端，Regular / Compact 保留合并读数。场景控件挂到 `leading` / `trailing` / `secondary` 槽。设置菜单里的条目（剧场、独立窗口、停止播放）挂到 `settings` 槽（`settings_content`，视图里 `.settings(view)`）。放进条自己建的设置 `ActionMenu`。`secondary` 没有可见子节点时第二行自动收起。条变单行。`assemble_media_transport_bar` 建槽并接线。`sync_media_transport_bar` 写回播放态、进度与时间读数（`media_clock`：`m:ss` / `h:mm:ss`）并折叠空第二行。只写有变化的值。`update_component(bar, …)` 写完字段后已自动同步。播放 tick 不必再显式调用 sync。事件是 `MediaTransportEvent`（PlayPause / Seek / Volume / Fullscreen，以及下面的 SeekStarted / SeekEnded 和 MenuOpened / MenuClosed）。idle 隐藏走 `AppContext::sync_overlay_visibility`。不要在应用里再复制 descendant / hit-test 锁。条自己的设置菜单和音量弹出层打开、关上时，条发 `MenuOpened` / `MenuClosed`（不论是按触发器、Escape、点外面，还是设置钮被藏起时顺带关上）。宿主收到就调一次 `sync_overlay_visibility`。打开的菜单从这时起把条留住。关上后空闲计时从这时重新开始（否则 wakeup 为空，没有别的东西会叫醒宿主）。不必再在框架建的菜单节点上听 `PopoverToggled` / `PopoverClosed`。

同一控件有两个正交维度。改字段后下一次 `sync_media_transport_bar` 生效。不是第二套绘制：

- `density`：`Regular`（读数在进度上方，可开第二行）、`Compact`（单行紧凑，读数在进度旁；设置 / 全屏默认隐藏，`show_settings` / `show_fullscreen` 可显式打开，三个槽照常可用）、`Stacked`（按钮与 Compact 相同，读数与进度单独占上面一整行，窄表面如迷你播放器不必把进度挤在按钮之间）或 `Mini`（`.mini()`：顶边 2px 进度细轨带 16px 透明命中带，下面一行是播放、静音钮 `Mute(bool)`、无圆点音量细轨、`leading`、时间读数、`trailing`；没有音量弹出层。应用不再搬动条内部节点，见 [MediaTransportBar](../components/media-transport-bar.md#mini)）。
- `placement`：`Overlay`（Absolute 贴父级底边、`max_width` 封顶、外壳不命中）或 `Inline`（参与父级文档流，高度即 chrome 高度，横向填满父级，不用 `max_width`）。

播放钮在每种密度下都显示。`show_play = Some(false)`（`MediaTransportBar::new().show_play(false)`，视图里 `.bind(move |bar| bar.show_play = Some(ready.get()))`，模板 `<Widget of={MediaTransportBar::new().show_play(false)}>`）把它藏起。例如直播间还没准备好播放时。藏起的钮不占位。不能聚焦。焦点在它上面时会被清掉。

`seekable = false` 表示点播内容此刻还不能拖（仍在加载、时长未知）。进度留在原处置灰。不必由应用去改进度控件。直播仍换成进度表。进度控件的「进度」只作读屏名称。不占轨道宽度。

Compact + Overlay 适合分离窗底栏（单行加边距约 52px）。Compact + Inline 适合壳层迷你条。画面、封面与标题仍由应用放在条外。第二个 `RuntimeDocument` / 窗口直接 `assemble_media_transport_bar` 得到同一 chrome。标记里用 `density="compact"` / `density="stacked"`、`placement="inline"`、`show-play` / `show-settings` / `show-fullscreen` 布尔属性。重新绑定只更新这些配置。保留播放状态与已组装的 chrome。

进度拖拽只预览读数（宿主暂停、没有 tick 时也会跟随）。抬手才发一次 `Seek`。取消的拖拽不发。一次拖拽前后各有 `SeekStarted` / `SeekEnded`（结束在 `Seek` 之后；取消或原地松手时只有这一对）。宿主据此在拖动期间不回写播放位置。不必去监听条内部进度控件的 `RangeInput`。`RangeField` 自己的拖动起止是 `RangeDragging { dragging }`（键盘与无障碍步进不算拖动）。键盘 / 无障碍每一步都是提交。与原生 range 一致。音量跟随拖拽实时发 `Volume`。条在发 `Seek` / `Volume` 前先把目标写进 `position` / `volume`。宿主下一次写入仍是权威值。宿主若在 seek 完成前继续写旧位置，滑块会短暂回到旧位置。

`ReorderList` 可以挂 live 行子节点。按在行里的控件（按钮等能获得焦点的节点）上，不移动松开是控件的点击，移动超过拖动阈值才拖动这一行。`ReorderItem::tools` 标出绝不开始拖动的子树。没有子节点时仍按标签自绘行。`IconButton::with_tooltip` 用默认 `TooltipConfig`。

## 交互

可见的按下、输入、开关、选中都接到真实状态。用 `on` / `observe`，或 `update_component`。

典型事件：`Activate`（按钮）、`TextChanged`、`ToggleChanged`、`RangeInput` / `RangeChanged`、`TabsEvent`、`SearchDropdownEvent`、`ContextMenuEvent`。签名以 rustdoc 为准。组合控件的 `observe` 回调若改了派生子节点所依赖的字段，调用 `cx.reassemble()`。提交后会像 `update_component` 一样运行它的 assembler。不调用则不重组（例如正在输入的文本框不会被回写）。`RangeInput` 是每次可见取值（含拖拽中预览，适合实时预览）。`RangeChanged` 是提交：指针抬起且值变了、键盘步进、无障碍 `SetValue` 或 `set_range_value`。取消的拖拽不提交。

### 谁改状态

**内建控件一律自驱。** 用户操作后，控件自己更新可见状态。事件报告「发生了什么」，不是「请求做什么」。应用不需要把状态回写一遍才能看到变化。

| 控件 | 用户操作后 |
| --- | --- |
| `Checkbox` / `Switch` | 自己翻 `checked`，发 `ToggleChanged` |
| `Select` / `Dropdown` / `SearchDropdown` | 自己写选中值并收起菜单 |
| `Tabs` | 自己改 `selected` |
| `TreeView` | 自己应用展开 / 选中 |
| `Popover` / `ActionMenu` / `ContextMenu` | 自己开关 `open` |
| `SegmentedControl`（含 `radio_group()`） | 激活时自己改选中，发 `SegmentedSelectionRequested` |

要**否决或改写**用户的选择，在 handler 里把你要的状态写回去（`set_segmented_selection`、`update_component` 等）。重复写入当前已有的值是 no-op。不会多一次提交。

方向键在 `SegmentedControl` / `Tabs` 上只移焦点。不改选中（手动激活语义）。选中跟随 Enter / 空格 / 点击。这是读屏软件对 tablist / radiogroup 的预期。

右键（button 2）派发 `SecondaryPress`。从命中节点往上找到第一个注册了该事件的节点。事件里带命中节点与坐标。框架不开菜单。不塞默认项。要不要弹、弹什么，由应用在 handler 里决定（通常是 `ContextMenu`）。没人注册就什么都不发生。

平台文件拖放由宿主降级为 `InputPayload::FileDrag`（`FileDragKind::{Hover, Drop, Cancel}`）。经输入路由交给放置目标。命中目标发 `FileDropEvent::{Hovered, Dropped, Left}`，并画 hover chrome。Vue 用 `<nana-drop-target drop-accepts="files">` / `NanaDropTarget`。未登记节点不接收文件拖放。悬停与放下都带 `modifiers`。拖动期间键盘仍归拖动源。宿主在 Windows（`GetAsyncKeyState`）和 macOS（`NSEvent.modifierFlags`）上每次映射都采样系统状态。Linux 退回最近一次跟踪到的修饰键。Windows 的 OLE 循环在按键变化时也会补发位置。所以只按下 Ctrl 不动鼠标也会收到新的悬停。宿主接受 Copy。macOS 另接受 Link（Copy 优先）。因为 AppKit 会把按住 Control 的 Finder 拖动收窄为 Link。只接受 Copy 时系统会拒绝这次松手。松手后取路径失败时发 `Cancel`。拖动不会悬而不决。窗口失焦或关闭也会结束悬停。

`TextArea` 的行号、诊断沟、minimap、git gutter 是视图属性：`line_numbers` / `diagnostics` / `minimap` / `git_gutter`。buffer revision、LSP、git 状态仍由应用喂入。Vue `NanaTextarea` 对应 `lineNumbers`、`diagnostics`、`minimap`、`gitGutter`。

`TerminalView` 是保留式网格。应用喂 `TerminalScreen` 单元格与样式。框架发 `TerminalEvent::Input` / `Resize` / `SelectionChanged`。PTY、submit、interrupt 归应用（通常把 Enter / Ctrl+C 解释成对 PTY 的写入）。Vue tag 是 `nana-terminal`。`screen` JSON 与 Runtime `TerminalScreen` 同形。`cells` 可以是 grapheme 串（宽字符自动占两列），或字符串/对象数组（省略 `width` 时按第一个 grapheme 的显示宽度 0/1/2）。`foreground` / `background` 是 `[r,g,b]` 或 `[r,g,b,a]`：0–1。任一 RGB 通道 `> 1` 则按 0–255。省略 `screen` 时保留宿主 `sync_terminal_screen` 喂入的画面。高频 PTY 帧仍走那条 Runtime API。

`DiffView` 展示应用喂入的 hunk。`DiffEvent` 的接受/拒绝只是请求。不改 buffer。`assemble_diff_view` 用现有 `ScrollView` + `Button` + `Text` 装配。Vue tag 是 `nana-diff`。只读审阅（提交、工作区改动）用 `.review_actions(false)`：不装配接受/拒绝按钮。增删行以低透明度的成功/危险色铺底。

需要开窗、换 GPU、写盘时，在闭包里 `cx.dispatch_program(msg)`。下一帧进入 `RuntimeProgram::update`。不要在指针处理里做重活。

## 表单校验

每个控件自己带 `invalid`。`FormField` 带 `error`。`ValidationMessage` 单独显示。这三份是显示层。要在提交前问「这张表还有没有没填对的」，用 `AppContext::validity_of(root)`。它读控件已经发布的无障碍状态。按 document order 返回子树里所有 `invalid` 且未禁用的字段。

```rust
let validity = cx.validity_of(form.stable_id());
if let Some(first) = validity.first_invalid() {
    cx.scroll_into_view(scroll, first, 8.0)?;   // 滚到第一个错误
    cx.focus_node(document, first)?;
    return;
}
```

键盘把焦点移进滚动容器时，`focus_node`（Tab 走这条）按最小距离把焦点节点滚进每个祖先滚动视口。目标已经完全可见时容器不动。上面先调用 `scroll_into_view` 是为了留下边距；随后的聚焦看到目标已经在视口里，不会把边距再吃掉。指针按下的聚焦不滚动：目标本来就在指针下，滚开它会让松开落到别处。`focus_node_in_place` 也不滚动，相当于 Web 的 `focus({ preventScroll: true })`，给先恢复了滚动位置、再恢复焦点的应用用。能移动的轴把当前视口偏移和可达范围交给 AccessKit；通用容器因此投影为滚动视图，平台才能看到 ScrollPattern。百分比和视口比例由适配器计算。

禁用字段不计入。用户够不到的控件不该挡住提交。校验规则本身仍是应用的。`validity_of` 只报告树当前的说法。不定义什么算合法。

## 文本与列表

`TextInput` / `TextArea` 持有已提交的 UTF-8、选区和 IME preedit。它们是视图侧编辑模型。撤销由 Runtime 提供（见下）。文档 revision、冲突与持久化仍由应用拥有。可选 feature `syntax-highlighting` 在同一 `TextArea` 上启用名为 `"highlight"` 的 presenter。不另造一套编辑器。

Rust `TextArea::read_only(true)` 保留焦点、光标/选区、查找和复制。拒绝修改、替换、剪切及粘贴。它与 `disabled(true)` 不同。禁用控件不参与这些交互。运行时切换为只读会结束未提交的 IME 组合，并拒绝此前开始的文本拖放写入。程序仍可通过组件更新提供新的权威文本。`HostedTextarea` 转发同一属性。语义构造属性为 `readOnly`。应用显示只读文档时应使用此属性。而不是以禁用样式替代只读状态。

### 统一编辑入口与撤销

编辑器的文本**只有一条写路径**：`AppContext::commit_editor_edit(entity, origin, apply)`。键入、删除、粘贴、IME 提交、行变换、代码片段、应用设值，全部汇入这里。凡是「每次编辑都要做」的事只在这一处做：发变更事件、记撤销。新增一种编辑操作时，不要自己 `update_component` 写 `state.value`。走这个入口。否则它不会进撤销。

`TextEditOrigin` 说明这次编辑是什么。它决定这次是否与上一步合并：
- `Typing` / `Delete` —— 连续的同类操作合成**一个**撤销步。撤销不会一个字符一个字符往回走。移动光标会断开这一串。
- `Paste` / `Ime` / `Structural` —— 各自独立成步。一次输入法提交是一步。
- `Program` —— 应用自己写的值（如载入另一个文档）。它**清空**日志。撤销不会退回上一个文档的内容。
- `History` —— 撤销/重做自身。永不记录。

IME 预编辑存在 world 的 `ime` 槽里，不在编辑器的 `value` 里。所以日志天然看不到组合中间态。只看到提交。

`undo_focused_text` / `redo_focused_text` 作用于焦点编辑器。恢复编辑**开始时**的选区（含多光标）。`can_undo_text` / `can_redo_text` 供菜单置灰。输入路由已接 Ctrl/Cmd+Z 与 Ctrl/Cmd+Shift+Z。每个编辑器 200 步上限。节点销毁即释放。

剪贴板：Ctrl/Cmd + C / X / V / A 由输入路由接到焦点编辑器。位置在应用按键策略与终端之后。Runtime 只回答「选中的是什么」和「这次编辑做什么」（`focused_selected_text`、`cut_focused_text`、`select_all_focused_text`、`replace_focused_text`）。系统剪贴板由宿主持有。经 `HostServices::read_clipboard` / `write_clipboard` 访问。原生宿主整个进程共用一个 `OsClipboard`。headless 会话用 `HeadlessHostServices` 的私有剪贴板。后端忙时回答 `Busy`。不等待。没选中时 Ctrl+C 不清空剪贴板。剪贴板写失败时 Ctrl+X 不删文本。只读字段能复制。不能剪切粘贴。焦点在 `NativeMarkdown` / `SelectableRichText` 上时，Ctrl+C 取的是它的选区快照。普通 `Text` 在 `user-select: text | all | contain` 下可复制（文档级选区，不是第二套 TextInput）。`text` 拖选。`all` 单击即选中该节点全文。`contain` 选区不延伸到邻居。空选区同样不清剪贴板。Cut 不删这段只读文本。`user-select: auto` 默认不可选。`none` 不进选区。作者 `::selection` / `::-moz-selection` 的 `background` / `color` 画到选区高亮。未写时选中底用主题 `accent_soft`。Android 由 slot 的 `HostServices` 接 `AndroidClipboard`。

大列表、表格、树：Rust 用 `AppContext::materialize_virtual_*`。Vue 用 `NanaVirtualList` / `NanaVirtualTable` / `NanaVirtualTree`（host tag 是唯一的 `nana-scroll-view`）。两边同一份窗口几何（`VirtualListLayout::window`）。可见窗口外不建 live 节点。滚动不重排整棵布局。GPU 节点走同一张 `ComponentRegistry`：`nana-gpu` → `nana.gpu`，`nana-gpu-view` → `nana.gpu-view`。每个控件只保留一个 tag。等于 `ComponentTypeId` 去掉 `nana.` 前缀。

## 滚动与滚动条

`ScrollView` 是滚动容器。位置权威是 Runtime 的 `ScrollOffset`。尺寸权威是 `ScrollMetrics`（任何改动布局盒、子节点或样式的提交结束时由 Runtime 重新测量，偏移随之夹取；起点在右 / 下边的轴偏移为负，见 [布局](layout.md)）。滚动条不另存一份偏移。L1 `overflow: auto|scroll` 共用同一份 `ScrollOffset` 与 overflow clip（滚轮同样更新这份偏移）。但**不**再画一套 thumb。自定义滚动条铬只属于 `ScrollView`。

`ScrollView::scrollbars` 选三种。`AutoHide`（默认，指针进容器才现，overlay 式不占布局）。`Always`（能滚就常驻，画轨道底）。`Hidden`（不画，滚轮与 `scroll_to` 照常）。Vue 侧用 `scrollbars="always|hidden"`。

轨道与滑块几何在 `nana-ui-core` 的 `scrollbar` 模块。颜色默认取 `border_strong`（拖拽时 `muted`）与 `subtle`。`::-webkit-scrollbar` / `::-webkit-scrollbar-thumb` 可覆盖厚度和这些颜色。仍走同一份 `scrollbar_track` 与普通 Scene quad。不另做一套 thumb 几何引擎。滑块拖拽与轨道点击（按下即把滑块居中到落点）由 `AppContext::begin_scrollbar_drag` 一族处理。走指针 capture。

滚动条是 overlay。不占布局宽度。两轴都能滚时各让出末端一个轨道厚度。避免拐角重叠。

## 自己加一种控件

多数需求用现有控件组合即可。真要新增一种会参与排版、点击和绘制的控件：

1. 实现 Runtime 的 `ComponentView` / `RegisterableComponent`。`ComponentView` 要求 `PartialEq`。`project` 读的每个字段都要参与比较。相等的写入会被跳过。`project` 读共享的内部可变状态，或者往别的组件的节点上打补丁时，声明 `const ALWAYS_REPROJECT: bool = true;`。
2. 用 `UiExtension` + `ExtensionRegistrar::register_component` 登记。稳定身份是 `ComponentTypeId`（如 `nana.button`、`app.preview-card`）。
3. 若 Vue 也要用，登记的 tag 等于 `ComponentTypeId` 去掉 `nana.` 前缀。和 HTML 同语义就用原生标签（`button`、`table`/`tr`/`td`、`ul`/`li`、`details`）。语义不同就换名（`search-dropdown`，不是 HTML `<search>`）。Vue tag 和 Rust `create_component<C>` 解析同一张 `ComponentRegistry`。未登记、也不是已知 HTML 的 tag 会报错。不会当成布局盒。

只给 JavaScript 一组命令和属性白名单时，走 Vue 的 `NativeComponentRegistry`（`Nana.components.call`）。那张表**不会**让节点自动进入布局和命中。

工厂的 `command` / `unmount` 在 unwind 档下会捕获 panic，转成 `NativeComponentCommandError` 抛回 JS。发布用的 `dist` 档是 `panic = "abort"`（换掉展开表，桌面二进制省约 7 MB）。该隔离不存在。工厂 panic 直接终止进程。工厂不要把 panic 当成可恢复的错误路径。

实时画面不要做成「自己往窗口上画的控件」。走 [实时画面](gpu.md)。不支持动态加载 dylib 插件。

`GpuTextureView` 不加载网页。`<iframe>` 也不加载网页。需要应用内网页内容时，使用 `runtime::BrowserView` 的宿主原生内容例外。它不是 `GpuTextureView` 别名。当前只有 macOS `WKWebView` 实现。Windows 与 Linux 明确不可用。Gallery 不得把普通纹理或 iframe 当成浏览器。

`GpuTextureView`、`Thumbnail` 与 `Avatar` 默认只采样宿主纹理的第 0 层。宿主应按 `HostTextureRegistry::painted_extent` 准备尺寸。纹理带 mip 链时用 `.sampling(ImageSampling::Mipmap)` 切到三线性采样。`url()` 图片的默认重采样与 mip 选项见 [应用 API · 图片采样](application-api.md#图片采样)。

`Thumbnail` 默认维持控件高度 × aspect。显式 style 的宽高、约束与圆角（`style.radius` 档位或 `border_radius` 像素）优先。可用于响应式卡片封面。都没写时才取 `Xs`。`fit(ContentFit::Cover)` 保留封面裁切。默认仍是 Contain。空、加载、就绪与不可用共享布局尺寸。Loading 态的 spinner 居中绘制。边长 28（紧凑 `Spinner` 的两倍）。带标签的独立 `Spinner` 仍贴左，作为文字的前置槽。封面角标挂成 Thumbnail 的子节点。控件是 containing block（`position: relative`）并裁剪圆角。`Thumbnail::badge()` 给出右下角、不命中的实底徽章。Vue 的 `NanaThumbnail` 使用同一 `fit` 属性。

### 图表与带图标按钮

`Chart::new(option)` 画一张图。`ChartOption` 的结构对应 ECharts 的 `option`（`grid`、`x_axis`、`y_axis`、`series`、`tooltip`、`legend`、`data_zoom`），值是 Rust 类型，模型在 `nana_ui::runtime::chart`（`nana-ui-charts` crate）。系列有折线/面积、柱状、饼/环/玫瑰、散点、雷达、仪表盘。换一份 option 就是一次更新：元素从屏幕上正在显示的位置过渡到新位置。悬停按类目或按项触发，强调和轴指针由着色器按动效时钟画，提示是图表自己的浮层，带颜色点和右对齐的数值。离开、卸载或 park 后关闭。图例切换、缩放窗口存在 `Chart::view`，事件是 `ChartEvent`。数据、名称和数值格式仍由应用提供。见 [Chart](../components/chart.md)。

`Chart` 只走 Rust：option 是类型化数据，没有 Vue 标签。

`Button::icon(icon).icon_size(px).icon_gap(px)` 把图标与文字作为同一内容组量测、居中和裁剪。保持一个按钮的 Activate、焦点与禁用语义。loading 用 spinner 替换图标，而不叠加第二个槽位。仅 spinner 相位变化不触发布局。`trailing_icon(icon)` 在文字之后放第二个图标（选择器、菜单按钮的下拉箭头）。尺寸与间距沿用前置图标。宽度不够时先裁文字。两个图标保持原尺寸。

### HoverCard 编辑器焦点

`HoverCard::preserve_editor_focus(true)` 用于扫码等非模态辅助卡。默认关闭。指针命中触发器或卡内操作时，保留卡外编辑器焦点和选区。按钮仍正常激活。Tab/Shift+Tab 仍可进入可访问操作。键盘焦点从卡外进入后，关闭时恢复最近的有效外部焦点。卡片或当前操作隐藏、停放、禁用、删除时也执行恢复。显式焦点请求和作用域恢复优先。失效或不可见的恢复目标不会重新获得焦点。内容容器需要参与命中（例如 `Stack::column(...).hittable()`），使二维码和空白区域的鼠标事件归属该卡。`QrCode` 本身仍是不可聚焦的可访问 Image。
