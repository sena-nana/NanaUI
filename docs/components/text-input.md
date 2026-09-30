# TextInput

一行文本。初始值走 `value`，不是构造参数。多行用 [TextArea](text-area.md)。

模板里的标签是 `<TextInput>`。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <TextInput placeholder="新任务" v-model={draft} />
}
```

```rust rust
use nana_ui::runtime::view::text_input;

text_input().placeholder("新任务").model(draft)
```

:::

`v-model={draft}` 展开成 `.model(draft)`。`draft` 是 `Signal<String>` 时，输入写回这一个信号，读它的文本会一起更新。

## 提交

回车提交用 `on:TextSubmitted={|event: &TextSubmitted| …}`。函数写法是 `.on_submit(|event: &TextSubmitted| …)`。

`on_input` 的类型是 `TextChanged`，处理器接收事件引用。`model` 绑定 `value`，输入时发出 `TextChanged`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `String` | 当前文本。`model` 绑定这一字段 |
| `label` | `Option<Arc<str>>` | 可访问名称 |
| `placeholder` | `Arc<str>` | 空着时的提示 |
| `disabled` | `bool` | 不能编辑 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_input` | `&TextChanged` | 输入时。`model` 也会在这时写回 `value` |
| `on_submit`，模板 `on:TextSubmitted` | `&TextSubmitted` | 回车提交 |

## 插槽

没有插槽。

## 参见

[总览](index.md) · [控件](../reference/components.md)
