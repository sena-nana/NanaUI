# TextInput

`text_input()` 创建一行输入。模板里的标签是 `<TextInput>`。它没有构造参数，初始文本走 `value`。

字段是 `value: String`、`label: Option<Arc<str>>`、`placeholder: Arc<str>` 和 `disabled: bool`。`model` 绑定 `value`，输入时发出 `TextChanged`。另外还有 `on_input`（`TextChanged`）和 `on_submit`（`TextSubmitted`）。这两个处理器都接收事件引用。

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

回车提交用 `on:TextSubmitted={|event: &TextSubmitted| …}`。函数写法是 `.on_submit(|event: &TextSubmitted| …)`。
