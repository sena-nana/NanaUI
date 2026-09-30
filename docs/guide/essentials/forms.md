# 表单绑定

输入要同时显示一个信号、并把用户的修改写回去，用 `v-model`。它展开成 `.model(信号)`。信号的类型由控件决定，不能换成别的。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <TextInput placeholder="标题" v-model={title} />
        <Checkbox label="完成" v-model={done} />
        <Slider min=0 max=1 step=0.05 v-model={volume} />
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{checkbox, column, slider, text_input};

column().gap(8).children((
    text_input().placeholder("标题").model(title),
    checkbox("完成").model(done),
    slider(0.0, 1.0, 0.05).model(volume),
))
```

:::

`title` 是 `Signal<String>`，`done` 是 `Signal<bool>`，`volume` 是 `Signal<f64>`。`v-model={title}` 把信号原样传进去，不会包一层闭包。

`.model` 做两件事：用信号驱动控件上的那个字段，并听一次变更事件，把事件里的值 `set` 回信号。

| 控件 | 信号 | 写回时听的事件 |
| --- | --- | --- |
| `TextInput`、`TextArea` | `Signal<String>` | `TextChanged`，取 `event.value.to_string()` |
| `Checkbox`、`Switch` | `Signal<bool>` | `ToggleChanged`，取 `event.checked` |
| `Slider` | `Signal<f64>` | `RangeInput`，取 `event.value` |
| `NumberInput` | `Signal<f64>` | `NumberChanged`，取 `event.value` |
| `Select` | `Signal<Option<Arc<str>>>` | `SelectChanged`，取 `Some(event.value.clone())` |

`Button` 没有 `v-model`。模板里写上会报「这个标签没有 `v-model`」。

滑块的 `.model` 听的是 `RangeInput`，拖动过程中的每个值都会写回信号。松手才提交的值是 `RangeChanged`，那是 `on_change`，不是 `v-model`。

文本写回走字段的 `text_state`：新字符串和控件里现有的一样时，不替换文本，光标和选区留在原地。所以把同一次输入再写进信号，不会把光标打回开头。

`Select` 还要有选项。`options` 是 `Vec<SelectOption>`，每一项用 `SelectOption::new(value, label)`，都可以再 `.disabled(true)`。禁用的选项仍显示。

:::api

```rust view
use std::sync::Arc;

use nana_ui::runtime::view;
use nana_ui::runtime::view::signal;
use nana_ui::runtime::SelectOption;

let picked = signal(None::<Arc<str>>);
let options = vec![
    SelectOption::new("day", "日间"),
    SelectOption::new("night", "夜间"),
];
view! {
    <Select placeholder="选一个" options={options} v-model={picked} />
}
```

```rust rust
use std::sync::Arc;

use nana_ui::runtime::view::{select, signal};
use nana_ui::runtime::SelectOption;

let picked = signal(None::<Arc<str>>);
select()
    .placeholder("选一个")
    .options(vec![
        SelectOption::new("day", "日间"),
        SelectOption::new("night", "夜间"),
    ])
    .model(picked)
```

:::

模板里同样是 `options={options}` 加 `v-model={picked}`。`placeholder` 可以是字符串，它会变成 `Option<Arc<str>>`。

`.model` 是在原有监听之外再加一个。你再写 `on_input`，两个都会跑，后者不会换掉前者。只想读变更、不写回信号时，不要用 `v-model`，用 [事件](events.md) 里的 `on_input` 或 `on_change`。

::: warning
构造时写 `.value(title.get())` 只放进当时的字符串。之后信号变了，输入框不跟着变；用户输入了，信号也不更新。双向绑定用 `.model(title)`。
:::

数字框的 `on_change` 在提交后的值变化时给出 `NumberChanged`。只读用 `.read_only(true)`，`TextArea` 和 `NumberInput` 都有这个字段。禁用用 `.disabled(…)`，上面这几个控件都有。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/refs">
    <p class="next-step-link">句柄</p>
    <p class="next-step-caption">挂载之后按类型拿到建成的节点。</p>
  </a>
  <a class="next-step" href="/guide/essentials/events">
    <p class="next-step-link">事件</p>
    <p class="next-step-caption">不写回信号时，on_input 和 on_change 怎么接。</p>
  </a>
</div>
