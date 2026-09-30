# Chip

`chip(label)` 创建一个紧凑的可选 token，外形是 pill。`label` 是构造参数，字段类型是 `Arc<str>`。模板里的标签是 `<Chip>`。子文本和 `label` 都写成这一个字段。

字段是 `label: Arc<str>`、`selected: bool`、`disabled: bool`。事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。没有 `model`。激活不自己改 `selected`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Chip selected={picked} @activate={toggle}>"Rust"</Chip>
}
```

```rust rust
use nana_ui::runtime::view::chip;

chip("Rust").selected(picked).on_activate(toggle)
```

:::

`@activate={toggle}` 在 `toggle` 已经是函数时直接传入。表达式展开成不接收参数的 `.on_activate(move || { … })`。`selected` 为真时底是 Selected，否则是 Subtle。这个状态由应用写。

`disabled` 为真时不发事件，也不可聚焦。

可关闭在组件 `Chip` 上，不在字段表里：`dismissible(true)`。这是叶子复合件，写入之后自己跑 `assemble_chip`，视图建好时也会跑一次，不必再记一次。视图函数没有 `.dismissible`。常量开关用 `.bind(|chip| chip.dismissible = true)`，这是一次写入，装配会跟着走。

点关闭钮发 `ChipDismissed`，不再给这个 Chip 发 `Activate`。关闭是请求，移除由应用做。听这个事件用 `.on(|_: &ChipDismissed| …)`，模板里是 `on:ChipDismissed={…}`。默认关闭无障碍名是「移除」，组件上用 `close_label` 改。

相对 `Button`：按钮是动作，没有 token 的 `selected`，也没有关闭。相对 `StatusBadge`：徽章只展示，不激活，不关闭。

Chip 不是 list item。无障碍上，本体是一个可激活的 Button，`selected` 映射选中；关闭钮是另一个可聚焦的 Button。

默认尺寸是 `ControlSize::Small`。`Chip::size` 可以改，不在字段表里。

禁用时本体和关闭钮都不发事件。关闭钮被按下时，命中停在关闭钮上，不会再变成一次 `Activate`。

[总览](index.md) 和 [控件合同](../reference/components.md)
