# 双向绑定

`v-model` 做两件事：把信号显示出来，再把控件发出的变更写回去。函数写法是 `.model(信号)`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <TextInput placeholder="新任务" v-model={draft} />
        <Checkbox v-model={agreed}>"同意"</Checkbox>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{checkbox, column, text_input};

column().gap(8).children((
    text_input().placeholder("新任务").model(draft),
    checkbox("同意").model(agreed),
))
```

:::

`draft` 是 `Signal<String>`，`agreed` 是 `Signal<bool>`。`.model` 先绑定字段，再监听那个控件的变更事件，在处理器里 `set`。你不必自己接一对「读」和「写」。

对没有 `v-model` 的控件写 `v-model`，编译模板时会报错。

## 绑到 store 的字段

`v-model` 也收 store 路径（`Store`、`Subfield`、`Item`）。路径既能读也能写，所以直接交给 `.model` 就行，不用再写一个处理器：

:::api

```rust view
use nana_ui::runtime::view;

todos.each(|t| view! {
    <Row gap=4>
        <Checkbox v-model={t.done()}></Checkbox>
        <TextInput v-model={t.title()} />
    </Row>
});
```

```rust rust
use nana_ui::runtime::view::{checkbox, row, text_input};

todos.each(|t| row().gap(4).children((
    checkbox("").model(t.done()),
    text_input().model(t.title()),
)));
```

:::

写回走路径自己的 `set`：只有读了这一行这个字段的绑定会重跑，`store_with_history` 也把它算进当前这一步。行被删掉以后，迟到的写回什么也不做。路径的值类型要和控件表里的字段类型一致；不一致时（例如 store 里是 `u32`，`NumberInput` 要 `f64`），绑字段再接 `.on_change` 自己转换。`.model` 收的是 `Writable<T>`，`Signal<T>` 和这三种路径都实现了它。

## 谁可以绑

控件表里登记了 `model` 的只有这些。信号类型必须对上。

| 控件 | 字段 | 写回时听的事件 |
| --- | --- | --- |
| `TextInput` | `value: String` | `TextChanged` |
| `TextArea` | `value: String` | `TextChanged` |
| `NumberInput` | `value: f64` | `NumberChanged` |
| `Checkbox` | `checked: bool` | `ToggleChanged` |
| `Switch` | `checked: bool` | `ToggleChanged` |
| `Slider` | `value: f64` | `RangeInput` |
| `RangeSpan` | `span: (f64, f64)` | `RangeSpanInput` |
| `Select` | `value: Option<Arc<str>>` | `SelectChanged` |

滑块这一行要看清楚：`.model` 听的是 `RangeInput`，也就是拖动过程中的每一个可见值，不是松手才发出的 `RangeChanged`。拖动时信号会跟着预览走。只要提交后的值，自己接 `.on_change`，不要用 `.model`。见 [事件](events.md)。

文本框写回时走文字状态：新值和当前值相同就不替换，所以把同一次编辑回声到信号上，光标和选区留在原地。`TextChanged` 的 `value` 会转成 `String` 再 `set`。

`Select` 的信号是 `Signal<Option<Arc<str>>>`。选中一项时写成 `Some(那一项的 value)`。选项本身用 `.options(vec)`，每一项是 `SelectOption::new(值, 标签)`。

:::api

```rust view
use std::sync::Arc;

use nana_ui::runtime::view;
use nana_ui::runtime::SelectOption;

let options = vec![
    SelectOption::new("day", "按天"),
    SelectOption::new("week", "按周"),
];
view! {
    <Select options={options} v-model={period} />
}
```

```rust rust
use std::sync::Arc;

use nana_ui::runtime::view::select;
use nana_ui::runtime::SelectOption;

select()
    .options(vec![
        SelectOption::new("day", "按天"),
        SelectOption::new("week", "按周"),
    ])
    .model(period)
```

:::

`period` 的类型是 `Signal<Option<Arc<str>>>`。占位文字用 `.placeholder`，它不是当前值。

开关和复选框的标签是无障碍名字。空标签的开关要放进设置行，让行标签替它起名。见 [无障碍](../scaling/accessibility.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/slots">
    <p class="next-step-link">具名 slot</p>
    <p class="next-step-caption">对话框、表单和菜单怎么放内容。</p>
  </a>
  <a class="next-step" href="/guide/essentials/forms">
    <p class="next-step-link">表单绑定</p>
    <p class="next-step-caption">v-model 和 .model 放在一页表单里。</p>
  </a>
  <a class="next-step" href="/guide/components/events">
    <p class="next-step-link">事件</p>
    <p class="next-step-caption">预览和提交为什么要分开接。</p>
  </a>
</div>
