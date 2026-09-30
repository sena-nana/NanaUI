# Column

`column()` 是纵向排列的 `Stack`。模板里的标签是 `<Column>`。横向用 `row()` 或 `<Row>`，属性是一样的，这一页不再另写。

## 基本用法

间距是 `.gap(…)`，单位是逻辑像素，接受任何 `Px`，整数或 `f32` 都可以。模板里写 `gap=8`，宏会展开成带类型后缀的 `.gap(8_f32)`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Text>"标题"</Text>
        <Button @activate={save}>"保存"</Button>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, text};

column().gap(8).children((
    text("标题"),
    button("保存").on_activate(save),
))
```

:::

## 子节点

子节点有两种放法。个数写死时用 `.children((a, b))`。建树时要写 `if`、`for` 或 `let` 时，用 `.with(|c| c.add(…))`。要跟着数据变的列表用 `each`，见 [视图写法](../guide/essentials/view.md)。

`.with` 在建树时跑一次。块里的 `if` 决定的是这一次挂载的结构。之后数据变了，不会重跑这个块。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.gap` | `Px` | 间距，单位是逻辑像素，整数或 `f32` 都可以。模板里写 `gap=8`，宏会展开成带类型后缀的 `.gap(8_f32)` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 个数写死时用 `.children((a, b))`。建树时要写 `if`、`for` 或 `let` 时，用 `.with`。要跟着数据变的列表用 `each` |

## 参见

[总览](index.md) · [视图写法](../guide/essentials/view.md) · [控件](../reference/components.md)
