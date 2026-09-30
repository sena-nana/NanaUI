# Card

`Card` 是一块不接交互的表面。动作放在它里面的控件上，卡片自己不充当按钮。

控件表里没有 `<Card>`。

## 基本用法

`Card::new()` 的种类是 `nana_ui::CardKind::Surface`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Card;
use nana_ui::CardKind;

view! {
    <Widget of={Card::new().title("工程").kind(CardKind::Outlined)}>
        <Text>"main.rs"</Text>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::Card;
use nana_ui::CardKind;

widget(Card::new().title("工程").kind(CardKind::Outlined)).children(text("main.rs"))
```

:::

真正的操作，例如打开、删除，放进子节点里的按钮。

## 种类

`.kind` 决定表面：

- `Surface` 和 `Raised` 用表面色，没有边框。
- `Outlined` 只画边框。
- `Flat` 没有底色，也没有边框。
- `Selected` 用选中底色，并带一条柔和边框。

你在样式里已经写了背景、边框、圆角或内边距时，种类不会盖掉它们。

## 样式

`.title` 和 `.label` 写同一个标题，标题会成为可访问名，并在内容上方留出标题带。

默认圆角是 `RadiusTier::Md`，内边距是面板档，数值由当前主题决定。

`.padding` 换成四边相同的物理内边距，并清掉面板档。

`.height` 固定高度。

## 加载

`.loading(true)` 把无障碍状态标成忙碌，相位由卡片自己推进。

加载中的卡片仍然是表面。

## 绘制

`.painter` 可以自绘外观。

要保留内建卡片再叠加装饰，在 `paint` 里调用 `cx.draw_default()`。

## 无障碍

无标题时节点文本是空字符串，读屏不拿标题当名字。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `Card::new()` | — | 种类是 `nana_ui::CardKind::Surface` |
| `.title` | — | 和 `.label` 写同一个标题。标题会成为可访问名，并在内容上方留出标题带 |
| `.label` | — | 和 `.title` 写同一个标题 |
| `.kind` | `nana_ui::CardKind` | `Surface` 和 `Raised` 用表面色，没有边框。`Outlined` 只画边框。`Flat` 没有底色，也没有边框。`Selected` 用选中底色，并带一条柔和边框。样式里已经写了背景、边框、圆角或内边距时，种类不会盖掉它们 |
| `.padding` | — | 换成四边相同的物理内边距，并清掉面板档 |
| `.height` | — | 固定高度 |
| `.loading` | — | `.loading(true)` 把无障碍状态标成忙碌，相位由卡片自己推进。加载中的卡片仍然是表面 |
| `.painter` | — | 可以自绘外观。要保留内建卡片再叠加装饰，在 `paint` 里调用 `cx.draw_default()` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 动作放在它里面的控件上，卡片自己不充当按钮 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 真正的操作，例如打开、删除，放进子节点里的按钮 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
