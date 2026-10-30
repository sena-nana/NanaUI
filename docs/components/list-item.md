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

## 紧跟主文本的次要文字

`note` 画在主文本后面，和 `detail` 同字号、同取色（随行的状态变，键盘聚焦时跟着主文本换色），同样不改变行高。适合主文本是译名、`note` 写原文这类成对的名字：`list_item("角度X").note("ParamAngleX")`。

宽度先给主文本和行尾的 `detail`，`note` 只用两者之间剩下的地方，放不下时省略；剩下的连一小段都不够时不画。挂了 content 槽时和 `detail` 一样不画。

无障碍名依次是 `label`、`note`、`detail`，画不下的 `note` 也照样读出。

## 只显示的行

只用来显示信息的行（房间里的成员、一条已绑定的记录）写 `read_only(true)`。它不接指针、不随悬停高亮、Tab 不停在它上面，也不发 `Activate`；文字照常取色，不画成禁用灰。无障碍角色仍是 ListItem，不报禁用，也不提供 `Click` / `Focus` 动作。能操作但暂时不可用的行才用 `disabled`。

```rust
list_item("Hiyori").detail("房主").read_only(true)
```

## 选中

`selected` 由应用写。激活只报告点了这一行，不改选中。无障碍角色是 ListItem，`selected` 映射选中，`disabled` 映射禁用。

不在列表里的行（Dock 的入口、一排胶囊）用 `role` 说明自己是什么，外观不变：`ListItemRole::Button` 报成按钮（画成选中时仍报选中）；`ListItemRole::ToggleButton` 报成切换按钮，`selected` 报成"按下"而不是"选中"，例如打开着的任务卡片的入口。按下的切换按钮照样画成选中。

点了会弹出一列项的行（Dock 里能升起一列胶囊的入口）再写 `has_popup(true)`：读屏报成菜单按钮。焦点在这一行时，ArrowUp / ArrowDown、`ContextMenu` 键和 Shift+F10 都发 `keyboard: true` 的 `SecondaryPress`（ArrowDown 的 `focus` 是 `Some(RovingEdge::Start)`，ArrowUp 是 `Some(RovingEdge::End)`，菜单键是 `None`），你在处理函数里打开那一列，再用 `AppContext::focus_roving_edge` 把焦点移进去。那一列用 `.roving_focus(RovingFocusGroup::vertical())` 让方向键在项之间走，越过两端时收到 `RovingFocusEdge`。见 [控件](../reference/components.md) 的菜单按钮和方向键焦点组。

## 行内工具

只在需要时出现的行尾工具（固定、关闭）写 `.tools(…)`，代替 `.trailing(…)`。指针在这一行上，或键盘焦点在这一行或行内时显出，所以 Tab 先停在行上、再停到工具上。`keep_tools` 为真时平时也显示（已打开的固定）。

收起时怎么占位由组件的 `tools_rest` 决定：默认 `RowToolsRest::Yield` 退出布局，标签用满整行，显出时这一行重排；Shift+Tab 从下面回来时跳过工具、落在行上，要再按 Tab 才到工具。`RowToolsRest::Reserve` 保留宽度、只是不画，显出时不重排，工具一直在 Tab 顺序里。

```rust
widget(ListItem::new("模型").tools_rest(RowToolsRest::Reserve))
    .keep_tools(pinned)
    .tools(icon_button(icons_tabler::PIN, "固定"))
```

## 要改控件

`@activate={open}` 已经在上面的例子里。函数不接收参数。需要 `ViewContext` 时用 `.on_cx(|_item, _event: &Activate, cx| …)`，模板里写成三个参数的 `on:Activate={…}`。

## 跟着数据

要跟着数据变的一整列，用 `each`，见 [视图写法](../guide/essentials/view.md)。个数写死的几行放进 `column().children((…))`。行高、间距、`auto_height`、`trailing_align`、`pill_bleed` 和 `tools_rest` 在组件 `ListItem` 上，不在字段表里。

槽是这一行自己的子节点。根上写了 `key` 的，可以用装配路径再找到。没写 key 的按位置命名。

## 属性

字段是 `label: String`、`note: String`、`detail: String`、`selected: bool`、`disabled: bool`、`read_only: bool`、`role: ListItemRole`、`has_popup: bool`、`keep_tools: bool`。没有 `model`。选中不由控件自己翻。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | `String` | 构造参数，也是可读的主文本。子文本写入同一字段 |
| `note` | `String` | 空字符串表示没有。非空、又没有 content 槽时紧跟主文本画成次要文字，放不下时省略或不画，无障碍名照样带上 |
| `detail` | `String` | 空字符串表示不画行尾补充，不增加行高。非空、又没有 content 槽时画在行尾 |
| `selected` | `bool` | 由应用写。选中不由控件自己翻。无障碍映射选中 |
| `disabled` | `bool` | 为真时不发 `Activate`。无障碍映射禁用 |
| `read_only` | `bool` | 只显示：不接指针与焦点、不悬停高亮、不发 `Activate`，正常文字色。无障碍不报禁用，也没有 `Click` / `Focus` 动作 |
| `role` | `ListItemRole` | 默认 `ListItem`；`Button`、`ToggleButton`（`selected` 报成按下）。只改无障碍，不改外观 |
| `has_popup` | `bool` | 这一行会弹出菜单。读屏报成菜单按钮，ArrowUp / ArrowDown 发 `keyboard: true` 的 `SecondaryPress` |
| `keep_tools` | `bool` | `.tools` 的工具平时也显示 |

## 事件

事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_activate`，模板 `@activate` | 不接收参数 | 类型是 `Activate`。函数直接传入；表达式包成 `.on_activate(move \|\| { … })`。`disabled` 为真时不发 |
| `.on_cx`，模板里三个参数的 `on:Activate` | `&Activate` 和 `ViewContext` | 需要 `ViewContext` 时 |

## 插槽

三个槽是这一行自己的子节点，顺序是 leading、content、trailing。`.leading(…)` 放在标签前，`.content(…)` 代替标签，`.trailing(…)` 放在标签后。模板里写成 `<template #leading>`、`#content`、`#trailing`。挂了 content 之后，`detail` 不再占行尾。

前导槽或尾随槽里的子节点隐藏（`hidden`，或 `display: none`）时，这一行按没有这个槽来排：前导槽隐藏后，标签从内边距起排，尾随槽仍在行尾；再显示时恢复原来的排法，不必重建这一行。

| 插槽 | 说明 |
| --- | --- |
| `#leading` / `.leading` | 放在标签前 |
| `#content` / `.content` | 代替标签。挂了之后，`detail` 不再占行尾 |
| `#trailing` / `.trailing` | 放在标签后 |
| `.tools` | 占 trailing 的位置，悬停或焦点在行内时显出，见上面的行内工具 |


## 多行的行

名称下面带说明的行，行首行尾要对着名称那一行，而不是对着两行整体居中。`ListItem::trailing_align(TrailingAlign::FirstLine)` 这样放：行把第一行垫到单行行的位置（上下各 `(带高 − 行盒) / 2 − 细边框`；带高是行 `style.control_height` 那一档的控件高，没有就按 `size`，和单行行一样把边框算在内），行首行尾放在这条带子里居中，所以单行行和多行行混排时行尾在同一条线上。`auto_height(true)` 的行默认就是 `FirstLine`，其余默认 `Center`。

行的样式上已经给了竖向内边距（某一边的 `padding_top` / `padding_bottom`，像素或主题档位都算，或者 `padding`、`control_padding_y`）时，那一边保留作者的值，第一行垫高只补没设的边。带子挂在内容边上，所以行首行尾照样对着第一行；内边距比垫高薄时，带子会伸出行的边缘。

视图构造器的 `.leading` / `.trailing` / `.tools` 会把槽放进这条带子，所以先在 `ListItem` 上设好 `auto_height` 或 `trailing_align`，再给槽。经 `AppContext::set_list_item_slots` 挂的槽要自己带带子。内容槽的第一行按行自己的行盒算。

## 三种状态

能点会做事的行用默认；只展示信息、从来不能点的行用 `.read_only(true)`（不响应悬停和点击，文字照常颜色）；本来能做、现在不能的行用 `.disabled(true)`（变灰，并说明原因）。只读不要写成禁用。见 [版式与组合](../guide/composition.md#列表行)。

## 参见

[总览](index.md) · [控件](../reference/components.md)
