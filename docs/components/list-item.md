# ListItem

`list_item(label)` 创建一行列表项。`label` 既是构造参数，也是可读的主文本。

模板里的标签是 `<ListItem>`。子文本和 `label` 都写成这一个字段。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <ListItem detail="外观" selected={current} @activate={open}>"设置"</ListItem>
}
```

```rust rust
use nana_ui::runtime::view::list_item;

list_item("设置").detail("外观").selected(current).on_activate(open)
```

:::

`@activate={open}` 在 `open` 已经是函数时，直接把函数传进去。写成一段表达式时，展开成 `.on_activate(move || { … })`。`disabled` 为真时不发 `Activate`。

## 行尾

`detail` 非空、又没有 content 槽时，画在行尾：右对齐、小字号、muted，不改变行高。过长的主文本截断省略，不折行撑破行高。

`detail` 空字符串表示不画行尾补充。它不增加行高。

## 选中

`selected` 由应用写。激活只报告点了这一行，不改选中。无障碍角色是 ListItem，`selected` 映射选中，`disabled` 映射禁用。

不在列表里的行（Dock 的入口、一排胶囊）用 `role` 说明自己是什么，外观不变：`ListItemRole::Button` 报成按钮（画成选中时仍报选中）；`ListItemRole::ToggleButton` 报成切换按钮，`selected` 报成"按下"而不是"选中"，例如打开着的任务卡片的入口。按下的切换按钮照样画成选中。

点了会弹出一列项的行（Dock 里能升起一列胶囊的入口）再写 `has_popup(true)`：读屏报成菜单按钮。焦点在这一行时，ArrowUp / ArrowDown、`ContextMenu` 键和 Shift+F10 都发 `keyboard: true` 的 `SecondaryPress`，你在处理函数里打开那一列并把焦点移进去。那一列用 `.roving_focus(RovingFocusGroup::vertical())` 让方向键在项之间走，越过两端时收到 `RovingFocusEdge`。见 [控件](../reference/components.md) 的菜单按钮和方向键焦点组。

## 要改控件

`@activate={open}` 已经在上面的例子里。函数不接收参数。需要 `ViewContext` 时用 `.on_cx(|_item, _event: &Activate, cx| …)`，模板里写成三个参数的 `on:Activate={…}`。

## 跟着数据

要跟着数据变的一整列，用 `each`，见 [视图写法](../guide/essentials/view.md)。个数写死的几行放进 `column().children((…))`。行高、间距、`auto_height` 和 `pill_bleed` 在组件 `ListItem` 上，不在字段表里。

槽是这一行自己的子节点。根上写了 `key` 的，可以用装配路径再找到。没写 key 的按位置命名。

## 属性

字段是 `label: String`、`detail: String`、`selected: bool`、`disabled: bool`、`role: ListItemRole`、`has_popup: bool`。没有 `model`。选中不由控件自己翻。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | `String` | 构造参数，也是可读的主文本。子文本写入同一字段 |
| `detail` | `String` | 空字符串表示不画行尾补充，不增加行高。非空、又没有 content 槽时画在行尾 |
| `selected` | `bool` | 由应用写。选中不由控件自己翻。无障碍映射选中 |
| `disabled` | `bool` | 为真时不发 `Activate`。无障碍映射禁用 |
| `role` | `ListItemRole` | 默认 `ListItem`；`Button`、`ToggleButton`（`selected` 报成按下）。只改无障碍，不改外观 |
| `has_popup` | `bool` | 这一行会弹出菜单。读屏报成菜单按钮，ArrowUp / ArrowDown 发 `keyboard: true` 的 `SecondaryPress` |

## 事件

事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_activate`，模板 `@activate` | 不接收参数 | 类型是 `Activate`。函数直接传入；表达式包成 `.on_activate(move \|\| { … })`。`disabled` 为真时不发 |
| `.on_cx`，模板里三个参数的 `on:Activate` | `&Activate` 和 `ViewContext` | 需要 `ViewContext` 时 |

## 插槽

三个槽是这一行自己的子节点，顺序是 leading、content、trailing。`.leading(…)` 放在标签前，`.content(…)` 代替标签，`.trailing(…)` 放在标签后。模板里写成 `<template #leading>`、`#content`、`#trailing`。挂了 content 之后，`detail` 不再占行尾。

| 插槽 | 说明 |
| --- | --- |
| `#leading` / `.leading` | 放在标签前 |
| `#content` / `.content` | 代替标签。挂了之后，`detail` 不再占行尾 |
| `#trailing` / `.trailing` | 放在标签后 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
