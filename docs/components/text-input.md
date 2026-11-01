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
| `invalid` | `bool` | 值不对，描 Danger 框；在设置行里，原因写在行的 `error` |
| `character_counter` | `Option<usize>` | 在框内右侧显示已提交字数与上限，默认关闭 |

## 字数计数

Rust 组件用 `TextInput::new("").character_counter(40)`；视图用
`text_input().character_counter(Some(40))` 或 `<TextInput character_counter={Some(40)} />`。
语义属性是 `character-counter="40"`。

计数按 `chars().count()`（Unicode 标量）统计，包括空格，排除输入法尚未提交的预编辑文字。
空值显示 `0/40`，超过上限时计数使用危险色。组合 emoji 的多个标量分别计数。
计数区保留在输入框内部右侧；正文、占位文字、光标、选区及预编辑文字的滚动、裁剪与命中共享剩余编辑区域。

这是显示能力。它允许继续输入、粘贴和提交；应用负责业务发送门控。
`max_length` 是独立的 UTF-16 编辑硬上限；两者可同时使用。

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_input` | `&TextChanged` | 输入时。`model` 也会在这时写回 `value` |
| `on_submit`，模板 `on:TextSubmitted` | `&TextSubmitted` | 回车提交 |

## 插槽

没有插槽。

## 参见

[总览](index.md) · [控件](../reference/components.md)
