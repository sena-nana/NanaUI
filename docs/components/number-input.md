# NumberInput

`number_input()` 创建一个数字输入。模板里的标签是 `<NumberInput>`。它没有构造参数，内部从 `NumberInput::new(0.0)` 起。

字段是 `value: f64`、`label: Option<Arc<str>>`、`placeholder: Arc<str>`、`disabled: bool`、`read_only: bool`。`value` 的写入是 `assign`，不是直接改字段。事件是 `on_change`，类型为 `NumberChanged`。处理器接收 `&NumberChanged`。`model` 绑定 `value`，取 `event.value`。

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

已提交的数在 `value`。用户键入时只改草稿，不改 `value`。回车或失焦才解析。解析不了就回到上次提交的值，不另编一个数。步进和方向键优先用能解析的草稿，半截草稿解析不了就从已提交值走。到了边界、一步也动不了时，什么都不改。

`.on_change(|event: &NumberChanged| …)` 在提交后的值变了才发生。模板里是 `on:NumberChanged={|event: &NumberChanged| …}`。键入过程没有单独的 `on_input`。

`assign` 把应用给的数夹进范围。离散模式再对齐精度和步进。写进去的结果和当前一样时，不替换文本。

范围、步进和精度在组件 `NumberInput` 上：`range`、`minimum`、`maximum`、`step`、`precision`。不在字段表里。`NumberInput::continuous` 只夹紧，不对齐网格；那种模式下 `step` 只决定箭头和步进的增量。

`read_only` 不接受输入。`disabled` 同样不接受。步进按钮是这颗控件自己画的，不是外面再放两个按钮。

`@change={save}` 在 `save` 是函数时原样传入，参数必须是 `&NumberChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

连续模式用 `NumberInput::continuous(value)` 而不是 `number_input()`。那种字段的文本取最短的、能往返的写法，`precision` 不会把小数吃掉。

[总览](index.md) 和 [控件合同](../reference/components.md)
