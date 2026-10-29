# StatusBadge

`status_badge(label)` 创建一枚状态徽章。`label` 是构造参数，字段类型是 `Arc<str>`；模板里的标签是 `<StatusBadge>`，子文本和 `label` 都写成这一个字段。它只描述状态，不拥有动作。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::StatusTone;

view! {
    <StatusBadge tone={StatusTone::Success} compact={false}>"已保存"</StatusBadge>
}
```

```rust rust
use nana_ui::runtime::view::status_badge;
use nana_ui::runtime::StatusTone;

status_badge("已保存").tone(StatusTone::Success).compact(false)
```

:::

`StatusTone` 的变体是 `Neutral`、`Info`、`Success`、`Warning`、`Danger`。不写 `tone` 时，函数内部用 `StatusTone::Neutral`。`compact` 在组件构造时默认是真；上面的例子把它关掉，内边距和字号走非紧凑的一档。

## 色调

`tone` 和 `compact` 可以是常量、信号或闭包。色调先对应到语义角色，绘制时再对上当前主题，不在这里写具体色值。背景里 Neutral、Success、Danger 用 Subtle，Info 用 AccentSoft，Warning 用 WarningSoft。前景走状态配方里的角色。

`StatusTone` 的默认变体是 `Neutral`。视图函数在你不写 `tone` 时也显式用它。`tone` 可以是 `StatusTone`，也可以是 `Signal<StatusTone>`。不要把色值写进 `label`。

## 紧凑

紧凑是默认：hint 字号，更小的内边距。`compact(false)` 改用 meta 字号。角是全圆的 pill。`compact` 不改变文案，只改变内边距和字号。标签本身仍是那一个 `label` 字段。

## 布局

圆点和标签是布局发布的部件：圆点在它那一格里居中（`LeadingContent`），隔一档间距后是标签（`TextContent`），标签取余下的宽度。紧凑时那一格和间隔是 `sm`、`xs`，否则是 `md`、`sm`，都读当前主题的间距。内边距只是胶囊左右各自的那一档，不再把圆点的位置算进左内边距；测量、换行宽度和绘制都读同一份部件。改 `tone` 只改颜色，不重新布局。从右到左的方向下圆点仍在左侧。

## 无障碍

无障碍是文本，名字就是 `label`。没有按下，没有选中，也没有关闭。可选、可关的 token 用 `Chip`。要做一件事，用 `Button`。

## 属性

字段是 `label: Arc<str>`、`tone: StatusTone`、`compact: bool`。没有事件，也没有 `model`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | `Arc<str>` | 构造参数。子文本写入同一字段。可访问名字。不要把色值写进这里 |
| `tone` | `StatusTone` | 也可以是 `Signal<StatusTone>`，或常量、信号、闭包。变体是 `Neutral`、`Info`、`Success`、`Warning`、`Danger`。不写时函数内部用 `Neutral`。默认变体也是 `Neutral` |
| `compact` | `bool` | 常量、信号或闭包。组件构造时默认是真。`false` 走非紧凑的内边距和字号，不改变文案 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 没有事件，也没有 `model` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 子文本和 `label` 都写成这一个字段 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
