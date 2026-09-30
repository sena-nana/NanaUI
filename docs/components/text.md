# Text

`text(value)` 画一段文本。字段只有 `value: String`。模板里的标签是 `<Text>`。这个参数也是字段：子文本和 `value` 都写它。没有事件，也没有 `v-model`。

带 `{…}` 的子文本展开成 `text!("…")`，里面点到的信号变了会重算。没有插值的字符串展开成 `text("…")`。手写插值用同模块里的 `text!`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Text>"共 {count} 项"</Text>
}
```

```rust rust
use nana_ui::runtime::view::text;

text!("共 {count} 项")
```

:::

`text!` 每次求值走 `format!`。构造时直接 `count.get()` 得到的是当时的常量，之后不再跟着变。要跟着变，用 `text!`，或把 `move || format!(…)` 传给 `text`。

`value` 可以是常量、`Signal<String>` 或闭包。常量在建节点时写进去。信号直接绑定。闭包装箱一次。

这是普通文本，不是另一套 `TextInput`。默认 `user-select: auto`，不可选。写成 `text`、`all` 或 `contain` 时可以复制，选区是文档级的。`text` 拖选，`all` 单击选中该节点全文，`contain` 不延伸到邻居。空选区不清剪贴板，剪切不删这段只读文本。`none` 不进选区。

字号、字重、单行和省略号在组件 `Text` 上：`font_size`、`font_weight`、`nowrap`、`ellipsis`。`font_size` 会同时把行盒设成最接近的控件行高，避免字装进更矮的盒子。单行省略是 `nowrap(true)` 再加 `ellipsis(true)`。这些不是 `text()` 的方法，也不在 `<Text>` 的字段表里。

视图层能跟着主题变的颜色是 `.foreground(…)`。角色在绘制时才解析。样式表里的颜色在编译那份表时就定了。`.foreground` 每个能写样式的控件都有，不只是文本。

没写 `::selection` 时，选中底用主题的 `accent_soft`。模板字符串里连续两个左花括号表示一个花括号，不是插值。

`Text::line_height` 覆盖 `font_size` 推出来的行盒。`width` 和 `max_width` 也在组件上。`color` 写语义前景，不写业务色。

子文本和 `value="…"` 同时出现时，属性优先，子文本不再当作 `value`。

[总览](index.md) 和 [控件合同](../reference/components.md)
