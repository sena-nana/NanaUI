# 控件

用现成控件往树上挂，不要每个按钮自己画。Rust 入口是 `nana_ui::runtime`。

```rust
let button = cx.create_component(document_id, Button::new("保存"))?;
cx.on(button, move |_button, _event: &Activate, _cx| {
    // 应用自己的保存逻辑
})?;
```

迁 Vue 时，同一套控件从 `@nanaui/nanavue-components` 引入，进同一棵树：

```js
import { NanaButton, NanaInput, NanaDialog } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

名称对照和 props 见该包 README。视觉与尺寸见 [视觉](look.md)。

## 目录

**操作与输入。** `Button`、`IconButton`、`TextInput`、`TextArea`、`Checkbox`、`Switch`、`Slider` / `RangeField`、`Select`、`Dropdown`、`SearchDropdown`、`SegmentedControl`、`Tabs`、`XYPad`。

**展示。** `Card`、`List` / `ListItem`、`FormField`、`EmptyState`、`Progress`、`Skeleton`、`Spinner`、`StatusBadge`、`Tooltip`、`ValidationMessage`、`QrCode`、`ImageViewer`、`NativeMarkdown`、`CalendarHeatmap`、`TimeSeriesChart`、`GraphCanvas`。

**浮层。** `Dialog`、`ConfirmDialog`、`Drawer`、`Popover`、`Menu` / `ActionMenu`、`ContextMenu`、`CommandPalette`。浮层由框架放在窗口里，靠近边缘时收进视口；不要用 `position: fixed` 自己搭一层。

**壳层。** `AppShell` / `DesktopShell`、`AppTitleBar`、`Workspace`、`SidebarFrame` / `SidebarSection` / `SidebarRow`、设置行和设置页、`Dock`、`SplitPane`、`PaneChrome`。壳是通用桌面结构；每个区域里放什么由应用决定，见 [工作区](workspace.md)。

部分族需要 Cargo feature（`calendar`、`charts`、`overlays`、`rich-text` 等），见 [应用 API](application-api.md)。未启用的模块不会编进你的二进制。

## 交互

可见的按下、输入、开关、选中都接到真实状态：`on` / `observe`，或 `update_component`。

典型事件：`Activate`（按钮）、`TextChanged`、`ToggleChanged`、`SliderChanged`、`TabsEvent`、`SearchDropdownEvent`、`ContextMenuEvent`。签名以 rustdoc 为准。

需要开窗、换 GPU、写盘时，在闭包里 `cx.dispatch_program(msg)`，下一帧进入 `RuntimeProgram::update`。不要在指针处理里做重活。

## 文本与列表

`TextInput` / `TextArea` 持有已提交的 UTF-8、选区和 IME preedit。它们是视图侧编辑模型：文档 revision、撤销、冲突、持久化仍由应用拥有。可选 feature `syntax-highlighting` 在同一 `TextArea` 上启用名为 `"highlight"` 的 presenter，不另造一套编辑器。

大列表、表格、树用 `AppContext::materialize_virtual_*`。可见窗口外不建 live 节点；滚动不重排整棵布局。

## 自己加一种控件

多数需求用现有控件组合即可。真要新增一种会参与排版、点击和绘制的控件：

1. 实现 Runtime 的 `ComponentView` / `RegisterableComponent`。
2. 用 `UiExtension` + `ExtensionRegistrar::register_component` 登记。稳定身份是 `ComponentTypeId`（如 `nana.button`、`app.preview-card`）。
3. 若 Vue 也要用，提供对应的 `nana-*` 标签。Vue tag 和 Rust `create_component<C>` 解析同一张 `ComponentRegistry`。

只给 JavaScript 一组命令和属性白名单时，走 Vue 的 `NativeComponentRegistry`（`Nana.components.call`）。那张表**不会**让节点自动进入布局和命中。

实时画面不要做成「自己往窗口上画的控件」，走 [实时画面](gpu.md)。不支持动态加载 dylib 插件。
