# ListItem

`list_item(label)` 创建一行列表项。`label` 既是构造参数，也是可读的主文本。模板里的标签是 `<ListItem>`。子文本和 `label` 都写成这一个字段。

字段是 `label: String`、`detail: String`、`selected: bool`、`disabled: bool`。事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。没有 `model`。选中不由控件自己翻。

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

`detail` 非空、又没有 content 槽时，画在行尾：右对齐、小字号、muted，不改变行高。过长的主文本截断省略，不折行撑破行高。

三个槽是这一行自己的子节点，顺序是 leading、content、trailing。`.leading(…)` 放在标签前，`.content(…)` 代替标签，`.trailing(…)` 放在标签后。模板里写成 `<template #leading>`、`#content`、`#trailing`。挂了 content 之后，`detail` 不再占行尾。

`selected` 由应用写。激活只报告点了这一行，不改选中。无障碍角色是 ListItem，`selected` 映射选中，`disabled` 映射禁用。

`@activate={open}` 已经在上面的例子里。函数不接收参数。需要 `ViewContext` 时用 `.on_cx(|_item, _event: &Activate, cx| …)`，模板里写成三个参数的 `on:Activate={…}`。

要跟着数据变的一整列，用 `each`，见 [视图写法](../guide/essentials/view.md)。个数写死的几行放进 `column().children((…))`。行高、间距、`auto_height` 和 `pill_bleed` 在组件 `ListItem` 上，不在字段表里。

槽是这一行自己的子节点。根上写了 `key` 的，可以用装配路径再找到。没写 key 的按位置命名。

`detail` 空字符串表示不画行尾补充。它不增加行高。

[总览](index.md) 和 [控件合同](../reference/components.md)
