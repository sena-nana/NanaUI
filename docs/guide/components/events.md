# 事件

控件在交互时发出类型化的事件。你在视图上接住它。没有数据的事件用 `@`，带数据的用 `on:` 或对应的方法。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Button @activate={save}>"保存"</Button>
        <Slider min=0 max=1 step=0.05 on:RangeChanged={|event| applied.set(event.value)} />
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, slider};

column().gap(8).children((
    button("保存").on_activate(save),
    slider(0.0, 1.0, 0.05).on_change(|event| applied.set(event.value)),
))
```

:::

`save` 已经是函数时，`@activate={save}` 直接把函数传进去。写成一段表达式时，展开成 `.on_activate(move || { … })`。`Activate` 没有载荷，处理器不接收参数。按钮、图标按钮、`Chip` 和 `ListItem` 的激活都是这一个事件。

`on:RangeChanged={|event| …}` 展开成 `.on::<RangeChanged>`。滑块上同名的方法是 `.on_change`。`e` 的类型能推断，不必标注。模板里 `@change={|event| …}` 是同一条，编译器按控件表选择事件。

## 预览和提交不是同一个事件

滑块拖动时，每一个可见取值都是 `RangeInput`（`.on_input`），适合实时预览。`RangeChanged`（`.on_change`）是提交：指针抬起并且值变了、键盘步进、无障碍 `SetValue`，或 `set_range_value`。取消的拖拽不提交。

拖动开始和结束另有 `RangeDragging { dragging }`。键盘和无障碍步进不算拖动。

文本框的每次编辑是 `TextChanged`（`.on_input`）。单行框回车是 `TextSubmitted`（`.on_submit`），组字还没上屏时不算提交。复选框和开关是 `ToggleChanged`（`.on_change`），字段是 `checked`。数字框是 `NumberChanged`。下拉选择是 `SelectChanged`，载荷是选中的 `value`。

写错的事件名、在没有该事件的控件上监听，编译模板时会报出行列。

## 要改控件或发给程序

`.on` 只看到事件。要改控件、再发出一个事件，或把消息交给 `RuntimeProgram`，用三个参数：

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Activate;

view! {
    <Button
        on:Activate={|_button, _event: &Activate, cx| {
            cx.dispatch_program(msg);
        }}
    >
        "开始"
    </Button>
}
```

```rust rust
button("开始").on_cx(|_button, _event: &nana_ui::runtime::Activate, cx| {
    cx.dispatch_program(msg);
})
```

:::

模板里把 `on:Activate={…}` 写成 `|组件, 事件, cx|` 就是这一个方法。`cx.emit(事件)` 把你自己的事件放进队列。跨窗口、换 GPU、持久化走 `dispatch_program`，下一帧进 `update`。点击本身不要塞进 `update`。

这些监听写在节点的 `EventListeners` 上，那是 Runtime 里的权威名单。扩展控件不要在类型外面再存一份。见 [扩展控件](registry.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/v-model">
    <p class="next-step-link">双向绑定</p>
    <p class="next-step-caption">让输入直接写回一个信号。</p>
  </a>
  <a class="next-step" href="/guide/components/slots">
    <p class="next-step-link">具名 slot</p>
    <p class="next-step-caption">把一段视图交给对话框和菜单。</p>
  </a>
  <a class="next-step" href="/guide/essentials/events">
    <p class="next-step-link">事件基础</p>
    <p class="next-step-caption">on、on_activate 和 on_cx 放在哪里。</p>
  </a>
</div>
