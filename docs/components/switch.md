# Switch

`switch(label)` 创建一个开关。`label` 既是构造参数，也是可访问名称。模板里的标签是 `<Switch>`，子文本和 `label` 都写成这一个字段。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Switch loading={pending} v-model={muted}>"静音"</Switch>
}
```

```rust rust
use nana_ui::runtime::view::switch;

switch("静音").loading(pending).model(muted)
```

:::

`v-model={muted}` 展开成 `.model(muted)`。`muted` 是 `Signal<bool>`。和复选框一样，控件自己翻 `checked`，再发 `ToggleChanged`。看见开关拨过去，不需要应用再写一遍。`model` 把 `event.checked` 写回信号。

## 监听

`.on_change(|event: &ToggleChanged| …)` 自己听这次变化。模板里是 `on:ToggleChanged={|event: &ToggleChanged| …}`。`@change={save}` 在 `save` 已经是函数时原样传入，参数必须是 `&ToggleChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。三个参数的闭包是 `.on_cx`。

## 只绑显示

只绑 `.checked(muted)` 时，信号负责显示。用户拨动改的是控件自己的字段；信号下次再写，会盖回去。要让信号跟着走，用 `model`。

## 禁用和加载

`loading` 为真时不接收指针、不可聚焦，无障碍标成忙。`disabled` 同样不参与操作。两者都可以是常量、信号或闭包。

`checked`、`disabled`、`loading` 都可以是常量、信号或闭包。不要在构造时把信号 `.get()` 成一个不再更新的布尔。

## 设置行

放进设置行时，非空标签用控件自己的名字，例如 `switch("静音")`。`switch("")` 没有名字，读屏改用行标签。行标签改了，关联跟着重新投影。这层关联在 `UiWorld` 里，不是控件自己的无障碍状态，控件重新投影不会把它冲掉。

## 字段表以外

组件上还有 `hint`、`size`、`invalid` 和开关画在哪一侧的 `control_position`。它们不在 `<Switch>` 的字段表里。默认画在行尾，`SwitchControlPosition::End`。和复选框共用同一种 `ToggleChanged`。

## 属性

字段是 `label: String`、`checked: bool`、`disabled: bool`、`loading: bool`。`model` 绑定 `checked`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | `String` | 构造参数，也是可访问名称。子文本写入同一字段 |
| `checked` | `bool` | `model` 绑定这一字段，把 `event.checked` 写回信号。常量、信号或闭包 |
| `disabled` | `bool` | 不参与操作。常量、信号或闭包 |
| `loading` | `bool` | 为真时不接收指针、不可聚焦，无障碍标成忙。常量、信号或闭包 |
| `hint` | — | 不在 `<Switch>` 的字段表里 |
| `size` | — | 不在 `<Switch>` 的字段表里 |
| `invalid` | — | 不在 `<Switch>` 的字段表里 |
| `control_position` | — | 不在 `<Switch>` 的字段表里。默认画在行尾，`SwitchControlPosition::End` |

## 事件

事件是 `on_change`，类型为 `ToggleChanged`。处理器接收 `&ToggleChanged`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_change`，模板 `on:ToggleChanged` 或 `@change` | `&ToggleChanged` | 控件自己翻 `checked` 后再发。和复选框共用这一种。函数原样传入，参数必须是这个引用；其它表达式展开成 `move \|\| { … }`，事件被忽略 |
| `.on_cx` | — | 三个参数的闭包 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 子文本和 `label` 都写成这一个字段 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
