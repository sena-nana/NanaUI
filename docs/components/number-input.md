# NumberInput

`number_input()` 创建一个数字输入。它没有构造参数，内部从 `NumberInput::new(0.0)` 起。

模板里的标签是 `<NumberInput>`。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <NumberInput label="数量" placeholder="0" v-model={count} />
}
```

```rust rust
use nana_ui::runtime::view::number_input;

number_input().label("数量").placeholder("0").model(count)
```

:::

`v-model={count}` 展开成 `.model(count)`。`count` 是 `Signal<f64>`。

## 草稿

已提交的数在 `value`。用户键入时只改草稿，不改 `value`。回车或失焦才解析。解析不了就回到上次提交的值，不另编一个数。步进和方向键优先用能解析的草稿，半截草稿解析不了就从已提交值走。到了边界、一步也动不了时，什么都不改。

## 写入

`assign` 把应用给的数夹进范围。离散模式再对齐精度和步进。写进去的结果和当前一样时，不替换文本。

## 范围、步进和精度

范围、步进和精度在组件 `NumberInput` 上：`range`、`minimum`、`maximum`、`step`、`precision`。不在字段表里。`NumberInput::continuous` 只夹紧，不对齐网格；那种模式下 `step` 只决定箭头和步进的增量。

连续模式用 `NumberInput::continuous(value)` 而不是 `number_input()`。那种字段的文本取最短的、能往返的写法，`precision` 不会把小数吃掉。

## 只读

`read_only` 不接受输入。`disabled` 同样不接受。步进按钮是这颗控件自己画的，不是外面再放两个按钮。

## 属性

字段是 `value: f64`、`label: Option<Arc<str>>`、`placeholder: Arc<str>`、`disabled: bool`、`read_only: bool`。`value` 的写入是 `assign`，不是直接改字段。`model` 绑定 `value`，取 `event.value`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `f64` | 已提交的数。写入是 `assign`，不是直接改字段。`model` 绑定这一字段，取 `event.value` |
| `label` | `Option<Arc<str>>` | 字段 |
| `placeholder` | `Arc<str>` | 字段 |
| `disabled` | `bool` | 不接受输入 |
| `read_only` | `bool` | 不接受输入 |

## 事件

事件是 `on_change`，类型为 `NumberChanged`。处理器接收 `&NumberChanged`。

`.on_change(|event: &NumberChanged| …)` 在提交后的值变了才发生。模板里是 `on:NumberChanged={|event: &NumberChanged| …}`。键入过程没有单独的 `on_input`。

`@change={save}` 在 `save` 是函数时原样传入，参数必须是 `&NumberChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_change`，模板 `on:NumberChanged` 或 `@change` | `&NumberChanged` | 提交后的值变了才发生。键入过程没有单独的 `on_input`。函数原样传入，参数必须是 `&NumberChanged`；其它表达式忽略事件 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
