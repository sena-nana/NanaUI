# 控件

用现成控件往树上挂，不要每个按钮自己画。Rust 入口是 `nana_ui::runtime`。

```rust
let button = cx.build(document_id, |ui| {
    let button = ui.child("save", Button::new("保存"));
    ui.on(button, move |_button, _event: &Activate, _cx| {
        // 应用自己的保存逻辑
    });
    button
})?;
```

迁 Vue 时，同一套控件从 `@nanaui/nanavue-components` 引入，进同一棵树：

```js
import { NanaButton, NanaInput, NanaDialog } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

名称对照和 props 见该包 README（含 `NanaScrollView`、`NanaNumberInput`、`NanaTable`、`NanaDesktopShell` 等与本目录一一对应的包装）。视觉与尺寸见 [视觉](look.md)。

## 目录

**操作与输入。** `Button`、`IconButton`、`TextInput`、`TextArea`、`NumberInput`、`Checkbox`、`Switch`、`RangeField`、`Select`、`Dropdown`、`SearchDropdown`、`SegmentedControl`、`Tabs`、`XYPad`、`ColorField`、`PathField`。

**展示。** `Card`、`List` / `ListItem`、`FormField`、`EmptyState`、`Progress`、`Skeleton`、`Spinner`、`StatusBadge`、`Tooltip`、`ValidationMessage`、`QrCode`、`ImageViewer`、`NativeMarkdown`、`CalendarHeatmap`、`TimeSeriesChart`、`GraphCanvas`。

**浮层。** `Dialog`、`ConfirmDialog`、`Drawer`、`Popover`、`ActionMenu`、`ContextMenu`、`CommandPalette`。浮层由框架放在窗口里，靠近边缘时收进视口；不要用 `position: fixed` 自己搭一层。`DesktopShell` 有两层 `OverlayHost`：`overlay` 放对话框，`status` 放 toast，确认框打开时 toast 仍可显示。

**壳层。** `AppShell` / `DesktopShell`、`AppTitleBar`、`Workspace`、`SidebarFrame` / `SidebarSection` / `SidebarRow`、设置行和设置页、`Dock`、`SplitPane`、`PaneChrome`。壳是通用桌面结构；每个区域里放什么由应用决定，见 [工作区](workspace.md)。

部分族需要 Cargo feature（`calendar`、`charts`、`overlays`、`rich-text` 等），见 [应用 API](application-api.md)。这些 feature 只控制 `nana-ui` 的再导出路径和 `ComponentSupport` 的 `compiled` 标记；控件实现都在 `nana-ui-runtime` 里，不启用也照样编译，仍可从 `nana_ui::runtime` 取到。要真正裁二进制得在 `nana-ui-runtime` 一侧做，目前没有。

`ColorField` 是色块 + hex，`assemble_color_field` 挂 HSV 选择器；提交发 `ColorChanged`，拖动发 `ColorInput`。`PathField` 是路径 + 浏览按钮，浏览只发 `BrowseRequested`，由应用打开系统对话框。

`GraphCanvas` 默认只画网格、节点框和边（Scene Quad / Stroke），节点内部内容由应用往子节点里放。`"graph-canvas"` 自定义 GPU renderer 不会自动挂上；要直写 pass 须宿主自己登记并 `set_custom_render`。`NativeMarkdown` 解析 mermaid 与公式围栏并给出 presenter 槽，但**不渲染**图和公式——那两样由宿主自己画进槽里。

`ReorderList` 可以挂 live 行子节点。`ReorderItem::tools` 标出行内可点控件；命中该子树不开始拖拽。没有子节点时仍按标签自绘行。`IconButton::with_tooltip` 用默认 `TooltipConfig`。

## 交互

可见的按下、输入、开关、选中都接到真实状态：`on` / `observe`，或 `update_component`。

典型事件：`Activate`（按钮）、`TextChanged`、`ToggleChanged`、`RangeChanged`、`TabsEvent`、`SearchDropdownEvent`、`ContextMenuEvent`。签名以 rustdoc 为准。

右键（button 2）派发 `SecondaryPress`，从命中节点往上找到第一个注册了该事件的节点，事件里带命中节点与坐标。框架不开菜单、不塞默认项：要不要弹、弹什么，由应用在 handler 里决定（通常是 `ContextMenu`）。没人注册就什么都不发生。

需要开窗、换 GPU、写盘时，在闭包里 `cx.dispatch_program(msg)`，下一帧进入 `RuntimeProgram::update`。不要在指针处理里做重活。

## 文本与列表

`TextInput` / `TextArea` 持有已提交的 UTF-8、选区和 IME preedit。它们是视图侧编辑模型：文档 revision、撤销、冲突、持久化仍由应用拥有。可选 feature `syntax-highlighting` 在同一 `TextArea` 上启用名为 `"highlight"` 的 presenter，不另造一套编辑器。

剪贴板：Ctrl/Cmd + C / X / V / A 由 `RuntimeInputAdapter` 接到焦点编辑器。Runtime 只回答「选中的是什么」和「这次编辑做什么」（`focused_selected_text`、`cut_focused_text`、`select_all_focused_text`、`replace_focused_text`），系统剪贴板由宿主持有：默认是进程级 `OsClipboard`，宿主可用 `RuntimeInputAdapter::with_clipboard` 换掉。没选中时 Ctrl+C 不清空剪贴板；剪贴板写失败时 Ctrl+X 不删文本；只读字段能复制、不能剪切粘贴。焦点在 `NativeMarkdown` / `SelectableRichText` 上时，Ctrl+C 取的是它的选区快照。

大列表、表格、树：Rust 用 `AppContext::materialize_virtual_*`；Vue 用 `NanaVirtualList` / `NanaVirtualTable` / `NanaVirtualTree`（host tag 是唯一的 `nana-scroll-view`）。两边同一份窗口几何（`VirtualListLayout::window`），可见窗口外不建 live 节点；滚动不重排整棵布局。GPU 节点走同一张 `ComponentRegistry`：`nana-gpu` → `nana.gpu`，`nana-gpu-view` → `nana.gpu-view`。每个控件只保留一个 tag，等于 `ComponentTypeId` 去掉 `nana.` 前缀。

## 滚动与滚动条

`ScrollView` 是滚动容器。位置权威是 Runtime 的 `ScrollOffset`，尺寸权威是布局后发布的 `ScrollMetrics`；滚动条不另存一份偏移。L1 `overflow: auto|scroll` 共用同一份 `ScrollOffset` 与 overflow clip（滚轮同样更新这份偏移），但**不**再画一套 thumb：自定义滚动条铬只属于 `ScrollView`。

`ScrollView::scrollbars` 选三种：`AutoHide`（默认，指针进容器才现，overlay 式不占布局）、`Always`（能滚就常驻，画轨道底）、`Hidden`（不画，滚轮与 `scroll_to` 照常）。Vue 侧用 `scrollbars="always|hidden"`。

轨道与滑块几何在 `nana-ui-core` 的 `scrollbar` 模块，颜色默认取 `border_strong`（拖拽时 `muted`）与 `subtle`。`::-webkit-scrollbar` / `::-webkit-scrollbar-thumb` 可覆盖厚度和这些颜色，仍走同一份 `scrollbar_track` 与普通 Scene quad，不另做一套 thumb 几何引擎。滑块拖拽与轨道点击（按下即把滑块居中到落点）由 `AppContext::begin_scrollbar_drag` 一族处理，走指针 capture。

滚动条是 overlay，不占布局宽度；两轴都能滚时各让出末端一个轨道厚度，避免拐角重叠。

## 自己加一种控件

多数需求用现有控件组合即可。真要新增一种会参与排版、点击和绘制的控件：

1. 实现 Runtime 的 `ComponentView` / `RegisterableComponent`。
2. 用 `UiExtension` + `ExtensionRegistrar::register_component` 登记。稳定身份是 `ComponentTypeId`（如 `nana.button`、`app.preview-card`）。
3. 若 Vue 也要用，登记的 tag 等于 `ComponentTypeId` 去掉 `nana.` 前缀。和 HTML 同语义就用原生标签（`button`、`table`/`tr`/`td`、`ul`/`li`、`details`）。语义不同就换名（`search-dropdown`，不是 HTML `<search>`）。Vue tag 和 Rust `create_component<C>` 解析同一张 `ComponentRegistry`。未登记、也不是已知 HTML 的 tag 会报错，不会当成布局盒。

只给 JavaScript 一组命令和属性白名单时，走 Vue 的 `NativeComponentRegistry`（`Nana.components.call`）。那张表**不会**让节点自动进入布局和命中。

实时画面不要做成「自己往窗口上画的控件」，走 [实时画面](gpu.md)。不支持动态加载 dylib 插件。

没有应用内浏览器控件。`GpuTextureView` / `<iframe>` 都不加载网页；拟议的 `WebView`（`nana.webview`）见 [应用内浏览器](gpu.md#应用内浏览器)，目前未实现，Gallery 不得摆假浏览。
