# Divider

`divider()` 创建一条横向分隔线。模板里的标签是 `<Divider>`。它没有构造参数，也没有字段。内部是 `Divider::horizontal()`。没有事件，也没有 `model`。

分隔线不带标签，也不参与操作。分组标题仍是旁边的 `Text`，不要把标题塞进这条线。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Text>"上一节"</Text>
        <Divider />
        <Text>"下一节"</Text>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, divider, text};

column().gap(8).children((
    text("上一节"),
    divider(),
    text("下一节"),
))
```

:::

默认沿交叉轴 `Fill`，并且 `align_self: Stretch`。放进 `align_items: Start` 的列里仍能看见，不会缩成一点。横向这条线拉满父级宽度，高度是线粗。

不接收指针，不可聚焦。无障碍角色是分隔。没另写背景时，颜色是主题的 `BorderSoft`。

竖线不在这个函数里。组件上是 `Divider::vertical()`，高度拉满父级，宽度是线粗。视图标签只有 `<Divider>`，没有第二个标签。

线粗和两端缩进在组件上：`thickness`、`inset`。非有限或非正的值退回默认（粗 1，缩进 0）。它们不在字段表里，`divider()` 上也没有同名方法。

[总览](index.md) 和 [控件合同](../reference/components.md)
