# Button

`button(label)` 创建一个按钮。这个标签既是构造参数，也是可访问名称。模板里的标签是 `<Button>`。子文本和 `label` 都会写成这一个字段。

字段有三个：`label: String`、`disabled: bool`、`loading: bool`。事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。

下面是一个保存按钮。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Button disabled={pending} @activate={save}>"保存"</Button>
}
```

```rust rust
use nana_ui::runtime::view::button;

button("保存").disabled(pending).on_activate(save)
```

:::

`disabled` 和 `loading` 可以是常量、信号或闭包。

`@activate={save}` 在 `save` 已经是函数时，会直接把函数传进去。写成一段表达式时，展开成 `.on_activate(move || { … })`。

需要 `ViewContext` 时，用 `.on_cx(|_button, _event: &Activate, cx| …)`。模板里写成三个参数的 `on:Activate={…}`。
