# Row

`row()` 是横向排列的 `Stack`。模板里的标签是 `<Row>`。它没有字段表里的属性。间距是 `.gap(…)`，单位是逻辑像素，接受任何 `Px`，整数或 `f32` 都可以。纵向用 `column()`，见 [Column](column.md)。

函数内部是 `Stack::row(0.0)`：宽度随内容收缩，子项在交叉轴上居中，间距从 0 起。再调用 `.gap` 才把间距写上。模板里写 `gap=12`，宏会展开成带类型后缀的 `.gap(12_f32)`。手写保持 `.gap(12)`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Row gap=12>
        <Text>"标签"</Text>
        <Button @activate={save}>"保存"</Button>
    </Row>
}
```

```rust rust
use nana_ui::runtime::view::{button, row, text};

row().gap(12).children((
    text("标签"),
    button("保存").on_activate(save),
))
```

:::

子节点有两种放法。个数写死时用 `.children((a, b))`。建树时要写 `if`、`for` 或 `let` 时，用 `.with(|c| c.add(…))`。`.with` 在建树时跑一次。块里的 `if` 决定的是这一次挂载的结构。之后数据变了，不会重跑这个块。

要跟着数据变的列表用 `each`，见 [视图写法](../guide/essentials/view.md)。

和 `column()` 的差别在预设，不在 `.gap`。列是纵向、交叉轴拉满、宽度占满父级。行是横向、交叉轴居中、宽度随内容收缩。两边的 `.gap` 都接受 `Px`，负数会收成 0。

要把后面的兄弟推到行尾，用 `Stack::spacer()`。它是零宽、可伸长的空隙，不是 `<Row>` 的属性。视图里放成一个子节点：`widget(Stack::spacer())`。基准宽度必须是 0，否则会把兄弟挤出去。

要占满父级剩余宽度、并让子项可以收缩，用 `Stack::fill_row`，不是 `row()`。`row()` 的宽度是收缩的。

[总览](index.md) 和 [控件合同](../reference/components.md)
