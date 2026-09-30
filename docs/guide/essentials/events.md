# 事件

控件上的监听有两种。一种不接收参数，例如 `Button` 的 `on_activate`。一种接收事件的引用，例如 `TextInput` 的 `on_input`，参数是 `&TextChanged`。这是控件表里写死的，不是你在闭包上标注出来的。

`save` 是一个没有参数的函数。`draft` 是 `Signal<String>`。`Msg` 是你自己的 `RuntimeProgram::Message`，`Start` 只是其中的一个变体。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Activate, TextChanged};

view! {
    <Column gap=8>
        <Button @activate={save}>"保存"</Button>
        <Button on:Activate={|_button, _event: &Activate, cx| cx.dispatch_program_all(Msg::Start)}>"开始"</Button>
        <TextInput @input={|event: &TextChanged| draft.set(event.value.to_string())} />
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, text_input};
use nana_ui::runtime::{Activate, TextChanged};

column().gap(8).children((
    button("保存").on_activate(save),
    button("开始").on_cx(|_button, _event: &Activate, cx| {
        cx.dispatch_program_all(Msg::Start)
    }),
    text_input().on_input(|event: &TextChanged| draft.set(event.value.to_string())),
))
```

:::

## 不接收参数

控件表里标成 `on { … }` 的方法，闭包是 `FnMut() + Send + 'static`。事件值被丢掉。

| 控件 | 方法 | 丢掉的事件 |
| --- | --- | --- |
| `Button` | `on_activate` | `Activate` |
| `ListItem` | `on_activate` | `Activate` |
| `IconButton` | `on_activate` | `Activate` |
| `Chip` | `on_activate` | `Activate` |

`@activate={save}` 在 `save` 已经是函数时，直接把这个函数传进去。`@activate={count.update(|c| *c += 1)}` 这种表达式会包成 `move || { …; }`。

`Activate` 本身没有字段。你不需要它就能知道按钮被激活了。

## 接收 &Event

标成 `with { … }` 的方法，闭包是 `FnMut(&Event) + Send + 'static`。

| 控件 | 方法 | 事件 |
| --- | --- | --- |
| `TextInput` | `on_input` | `&TextChanged` |
| `TextInput` | `on_submit` | `&TextSubmitted` |
| `TextArea` | `on_input` | `&TextChanged` |
| `Checkbox`、`Switch` | `on_change` | `&ToggleChanged` |
| `Slider` | `on_input` | `&RangeInput` |
| `Slider` | `on_change` | `&RangeChanged` |
| `NumberInput` | `on_change` | `&NumberChanged` |
| `Select` | `on_change` | `&SelectChanged` |

`TextChanged` 有 `value` 和 `selection`。`TextSubmitted`、`RangeInput`、`RangeChanged`、`NumberChanged` 都有 `value`。`ToggleChanged` 有 `checked`。`SelectChanged` 有 `value`。

模板里 `@input={|event: &TextChanged| …}` 把闭包原样传给 `on_input`。写成普通表达式时，会包成 `move |_| { …; }`，事件被忽略。要读 `event.value`，就写带参数的闭包。

`Slider` 的 `on_input` 是拖动过程中的每一个值（`RangeInput`）。`on_change` 是提交后的值（`RangeChanged`）：松手、键盘步进，或无障碍设置值。取消的拖动不会发 `RangeChanged`。

控件表里没有的事件，用 `on`。闭包接收 `&E`。例如文字截断听 `TextClamped`，它有一个 `clamped` 字段；容器尺寸听 `SizeChanged`，它有 `width` 和 `height`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::TextClamped;

view! {
    <Text on:TextClamped={move |event: &TextClamped| clamped.set(event.clamped)}>{body}</Text>
}
```

```rust rust
use nana_ui::runtime::view::text;
use nana_ui::runtime::TextClamped;

text(body).on(move |event: &TextClamped| clamped.set(event.clamped))
```

:::

## 要改控件，或给程序发消息

`on` 和 `on_activate` 拿不到组件，也拿不到 `ViewContext`。用 `on_cx`。闭包是 `FnMut(&mut 组件, &事件, &mut ViewContext)`。模板里把 `on:Activate={…}` 写成三个参数，展开的就是它。两个参数及以下展开成 `.on::<Activate>`。

`cx.dispatch_program(msg)` 按消息的 Rust 类型合并，同一帧里同类型只留最后一条。`cx.dispatch_program_all(msg)` 按派发顺序全部保留，下一帧进 `RuntimeProgram::update`。应用的 `Message` 通常是一个枚举，那就是同一种类型。

::: warning
`Message` 是一个枚举时，不要用 `dispatch_program`。同一帧里的两次点击会并成最后一次，前一次丢掉。用 `dispatch_program_all`。
:::

事件处理器不在视图作用域里。这里创建的信号没有归属，`use_context` 也读不到。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/forms">
    <p class="next-step-link">表单绑定</p>
    <p class="next-step-caption">v-model 怎样把输入写回信号。</p>
  </a>
  <a class="next-step" href="/guide/essentials/application">
    <p class="next-step-link">创建应用</p>
    <p class="next-step-caption">update 里处理的是宿主消息，不是每一次点击。</p>
  </a>
</div>
