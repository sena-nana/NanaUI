# ToolGroup

`ToolGroup` 把一组工具按钮放在同一条底槽上：绘图工具栏里的一组工具、画布的显示开关、面板自己的几个操作。按钮由你放，通常是 `IconButton`；哪一枚选中仍由你的状态决定，和单独用图标按钮时一样。相对 `Stack::row`，它多了两样东西：一条把按钮读成一组的底槽，以及让读屏把按钮念成一组的无障碍角色。

## 基本用法

`ToolGroup::new()` 是横排，`ToolGroup::vertical()` 是竖排，例如画布侧边的工具架。`.label` 是这组的可访问名，例如「选择工具」。`.mode` 说明按钮之间的关系。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Icon, ToolGroup};

view! {
    <Widget of={ToolGroup::new().label("图层操作")}>
        <IconButton icon={Icon::Add}>"新建图层"</IconButton>
        <IconButton icon={Icon::Settings}>"图层设置"</IconButton>
        <IconButton icon={Icon::Close}>"删除图层"</IconButton>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{icon_button, widget};
use nana_ui::runtime::{Icon, ToolGroup};

widget(ToolGroup::new().label("图层操作")).children((
    icon_button(Icon::Add, "新建图层"),
    icon_button(Icon::Settings, "图层设置"),
    icon_button(Icon::Close, "删除图层"),
))
```

:::

竖排的当前工具组写成 `ToolGroup::vertical().mode(ToolGroupMode::Exclusive)`，当前那一枚 `.selected(true)`。

组本身不接指针，点击落在按钮上。没有装配函数，子节点就是内容。控件表里没有 `<ToolGroup>`。

## 模式

| 模式 | 用途 | 读屏 |
|---|---|---|
| `Actions`（默认） | 每按一下做一件事，没有常亮的：新建、删除、上移 | 工具栏 |
| `Exclusive` | 其中一枚是当前工具，由你标 `selected` | 单选组 |
| `Toggles` | 每枚各自开或关，例如显示开关 | 工具栏，每枚报告自己的 `selected` |

模式不改按钮的外观，每枚按钮照自己的 `selected` 画。几组共用一个当前工具时（例如工具栏分成选择、变形、制作三组），你在持有它的那一组里标 `selected`，其余组都不标。

## 底槽和排列

底槽用 `Subtle` 底色、`Md` 圆角，按钮之间和到底槽边缘都隔 `space::XXS`，按钮自己的 `Sm` 圆角正好嵌在里面。方向跟 `orientation` 走：横排是行，竖排是列，子项居中，不伸缩。

相邻的组之间留间距就行，不要再加 `Divider`：每条底槽已经收住了自己那一组。

要用方向键在按钮之间走，给组加上 `.roving_focus(RovingFocusGroup::vertical())`（横排用 `horizontal()`）。见 [控件](../reference/components.md) 的方向键焦点组。

## 属性

| 属性 | 类型 | 说明 |
|---|---|---|
| `label` | `Option<Arc<str>>` | 组的可访问名 |
| `orientation` | `SelectionOrientation` | `Horizontal`（默认）或 `Vertical`，决定排列方向和读屏报告的方向 |
| `mode` | `ToolGroupMode` | 按钮之间的关系，见上表 |
| `style` | `NodeStyle` | 整体替换样式；方向仍由 `orientation` 决定 |
