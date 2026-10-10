# Button

按下后发出 `Activate`。可见标签默认也是可访问名称。同一屏上有多枚可见文字相同的按钮时，用 `accessible_name` 写出各自的动作。要的是一枚图标时，用 [IconButton](icon-button.md)。

模板里的标签是 `<Button>`。子文本和 `label` 都会写成这一个字段。

## 基本用法

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

`@activate={save}` 在 `save` 已经是函数时，会直接把函数传进去。写成一段表达式时，展开成 `.on_activate(move || { … })`。

## 禁用与加载

`disabled` 和 `loading` 可以是常量、信号或闭包。

## 要改控件或发给程序

需要 `ViewContext` 时，用 `.on_cx(|_button, _event: &Activate, cx| …)`。模板里写成三个参数的 `on:Activate={…}`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | `String` | 构造参数，也是可见文字。未写 `accessible_name` 时，它同时是可访问名称。子文本写入同一字段 |
| `accessible_name` | `String` | 可访问名称。空字符串沿用 `label`。可见文字保持 `label`，所以一行里的「设置」可以读成「设置 独立捕获窗口」 |
| `disabled` | `bool` | 常量、信号或闭包 |
| `loading` | `bool` | 常量、信号或闭包 |
| `has_popup` | `bool` | 弹出菜单的按钮（`aria-haspopup="menu"`）：读屏报成菜单按钮；焦点在它上面时，ArrowUp / ArrowDown 和 `ContextMenu` 键一样发 `keyboard: true` 的 `SecondaryPress`，由你打开菜单。见 [控件](../reference/components.md) 的菜单按钮 |
| `Button::content_align` | `TextHorizontalAlignment` | 在组件上。图标和文字这一组放在内容区的起点、中间（默认）或终点；列表行一样的按钮用 `Start` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_activate`，模板 `@activate` | 不接收参数 | 类型是 `Activate`。函数原样传入；表达式包成 `move \|\| { … }` |
| `.on_cx`，模板里三个参数的 `on:Activate` | `&Activate` 和 `ViewContext` | 要改控件，或 `cx.dispatch_program` |

## 插槽

没有具名插槽。子文本写入 `label`。

## 参见

[总览](index.md) · [控件](../reference/components.md)
