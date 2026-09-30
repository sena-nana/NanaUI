# ValidationMessage

`ValidationMessage` 单独显示一条校验说明。它和控件上的 `invalid`、`FormField` 上的 `error` 是三份显示。字段没填对时，控件标 `invalid`，这句话负责把原因念出来。

## 基本用法

`ValidationMessage::new(message, intent)` 的 `intent` 是 `nana_ui::runtime::ValidationIntent::Warning` 或 `Danger`。警告用警告色，危险用危险色。构造时 `compact` 为真，字号用提示档，左侧留出指示条。`.compact(false)` 换成更大的字号和更宽的指示。`.intent` 可以再改。

无障碍角色是文本，名字是这句话，并且标着 `invalid`。控件表里没有 `<ValidationMessage>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ValidationIntent, ValidationMessage};

view! {
    <Widget of={ValidationMessage::new("显示名不能为空", ValidationIntent::Danger)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ValidationIntent, ValidationMessage};

widget(ValidationMessage::new(
    "显示名不能为空",
    ValidationIntent::Danger,
))
```

:::

把它放在对应字段的下面。字段的 `error` 已经写出同一句话时，不必再叠一条，除非你要在字段外面单独强调。`.style` 整份替换样式，前景色仍按 `intent` 在投影时填上。

## 整表

提交前要问整张表，用 `AppContext::validity_of(root)`。它读控件已经发布的无障碍状态，按文档顺序返回子树里 `invalid` 且未禁用的字段。这条消息自己不参加那份名单。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `message` | — | `ValidationMessage::new` 的这句话。无障碍角色是文本，名字是这句话，并且标着 `invalid` |
| `intent` | `nana_ui::runtime::ValidationIntent` | `Warning` 或 `Danger`。警告用警告色，危险用危险色。`.intent` 可以再改 |
| `compact` | — | 构造时为真，字号用提示档，左侧留出指示条。`.compact(false)` 换成更大的字号和更宽的指示 |
| `.style` | — | 整份替换样式，前景色仍按 `intent` 在投影时填上 |

## 事件

没有事件。

## 插槽

没有插槽。

## 参见

[总览](index.md) · [控件](../reference/components.md)
