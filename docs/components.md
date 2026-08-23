# 控件

不要每个按钮自己画。用现成控件往树上挂。新应用在 Rust 里写：

```rust
let button = cx.create_component(document_id, Button::new("保存"))?;
cx.on(button, move |_button, _event: &Activate, _cx| {
    // 写你的保存逻辑
})?;
```

能看见的按下、输入、开关、选中，都要改到你自己的状态上。不要放点了没反应的入口。

迁 Vue 时，同一套控件从 `@nanaui/nanavue-components` 引入，进的是同一棵树：

```js
import { NanaButton, NanaInput, NanaDialog } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

## 有什么

**操作与输入。** 按钮、图标按钮、输入、多行文本、复选框、开关、滑杆、选择、下拉、搜索下拉、分段控件、Tab、XYPad。

**展示。** 卡片、列表行、表单行、空状态、进度、骨架屏、加载、状态徽章、提示、校验信息、二维码、图片查看、Markdown、日历热力、图表画布。

**浮层。** 对话框、确认框、抽屉、气泡、菜单、右键菜单、命令面板、Tooltip。浮层由框架放在窗口里，靠近边缘时自己收进视口，应用不必算坐标。

**壳层。** 标题栏、应用壳、工作区、侧栏、设置行和设置页、Dock、分栏。壳层是通用桌面结构；每个区域里放什么仍由应用决定。

视觉和尺寸见 [视觉](look.md)。

## 自己加控件

常见需求用组合现有控件就能做。真的要新增一种会参与排版、点击和绘制的控件时：

- 在 Rust 里注册成界面控件，并提供对应的 `nana-*` Vue 标签，这样它和按钮一样走进布局和点击。
- 如果只是给 JavaScript 一组命令和属性白名单，走宿主组件表，不要指望它自动变成可点的界面节点。
- 实时画面用 [实时画面](gpu.md) 那条路，不要在控件里直接往窗口上画。

不要为每个业务控件发明一套私有绘制。动态加载二进制插件不是这条路。

## 内部如何工作

公开只读目录是 `component_catalog()` / `component_support()`。默认绘制路径由同一份声明推导，不另维护一份名单。当前目录里的控件都走 Runtime。

内建与插件共用 `AppContext` 上的 `ComponentRegistry`。稳定身份是 `ComponentTypeId`（例如 `nana.button`）。Vue 的 `nana-*` 标签和 Rust 的 `create_component<C>` 都解析到这张表。`bind` 只把通用 UI 投影进 `UiWorld`；业务 state 留在 `AppContext.views`。

两张注册表不是同一条 ABI：

| 你要 | 注册到 |
| --- | --- |
| 进入布局、点击、Scene 的新控件 | `ExtensionRegistrar::register_component` + Vue `nana-*` |
| 仅 JS 的 props / 事件 / 命令白名单 | `NativeComponentRegistry` + `Nana.components.call` |
| GPU 内容 | `CustomRenderNode` + 宿主 `HostTexture` |

只注册其中一张，另一条路径不会自动生效。不要靠扩展 `WidgetKind` 来加业务控件。

浮层的 exclusive active 和焦点恢复只存在 `UiWorld`。切到另一个浮层时，不活跃的子树退出布局、点击和绘制；模态会限制焦点范围。

大列表、表格的窗口几何在 `nana-ui-core`：可见范围外不建 live entity，滚动不重排整棵布局，只让受影响的命中和绘制失效。
