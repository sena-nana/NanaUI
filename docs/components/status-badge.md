# StatusBadge

`status_badge(label)` 创建一枚状态徽章。`label` 是构造参数，字段类型是 `Arc<str>`。模板里的标签是 `<StatusBadge>`。子文本和 `label` 都写成这一个字段。

字段是 `label: Arc<str>`、`tone: StatusTone`、`compact: bool`。没有事件，也没有 `model`。它只描述状态，不拥有动作。

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

`tone` 和 `compact` 可以是常量、信号或闭包。色调先对应到语义角色，绘制时再对上当前主题，不在这里写具体色值。背景里 Neutral、Success、Danger 用 Subtle，Info 用 AccentSoft，Warning 用 WarningSoft。前景走状态配方里的角色。

紧凑是默认：hint 字号，更小的内边距。`compact(false)` 改用 meta 字号。角是全圆的 pill。

无障碍是文本，名字就是 `label`。没有按下，没有选中，也没有关闭。可选、可关的 token 用 `Chip`。要做一件事，用 `Button`。

`StatusTone` 的默认变体是 `Neutral`。视图函数在你不写 `tone` 时也显式用它。

`compact` 不改变文案，只改变内边距和字号。标签本身仍是那一个 `label` 字段。

`tone` 可以是 `StatusTone`，也可以是 `Signal<StatusTone>`。不要把色值写进 `label`。

[总览](index.md) 和 [控件合同](../reference/components.md)
