# Checkbox

`checkbox(label)` 创建一个复选框。`label` 既是构造参数，也是可访问名称。模板里的标签是 `<Checkbox>`。子文本和 `label` 都写成这一个字段。

字段是 `label: String`、`checked: bool`、`disabled: bool`。事件是 `on_change`，类型为 `ToggleChanged`。处理器接收 `&ToggleChanged`。`model` 绑定 `checked`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Checkbox disabled={locked} v-model={done}>"完成"</Checkbox>
}
```

```rust rust
use nana_ui::runtime::view::checkbox;

checkbox("完成").disabled(locked).model(done)
```

:::

`v-model={done}` 展开成 `.model(done)`。`done` 是 `Signal<bool>`。用户点下去，控件自己翻 `checked`，再发 `ToggleChanged`。应用不用把勾回写一遍才能看见。`model` 用 `event.checked` 写回这一个信号。

要自己听这次变化，用 `.on_change(|event: &ToggleChanged| …)`。模板里写成 `on:ToggleChanged={|event: &ToggleChanged| …}`。`@change={save}` 在 `save` 已经是函数时原样传入，参数必须是 `&ToggleChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。三个参数的 `on:ToggleChanged={|checkbox, event, cx| …}` 展开成 `.on_cx`。

要否决这次勾选，在处理函数里把你要的状态写回去。重复写入当前已有的值不会多一次提交。

只绑 `.checked(done)` 时，信号负责显示。用户拨动改的是控件自己的 `checked`，信号不变；信号下次再写，会把控件盖回信号里的值。要让信号跟着走，用 `model`。

`disabled` 为真时不接收指针，也不可聚焦。无障碍角色是 Checkbox，名字是 `label`。混合态在组件 `Checkbox` 的 `indeterminate` 上，绘制和无障碍都优先于 `checked`。`invalid` 把边画成危险色，`size` 改尺寸。这两项和混合态都不在字段表里。

`checked` 和 `disabled` 可以是常量、信号或闭包。闭包在信号变化时重算。构造视图时调用 `.get()`，得到的是当时的常量。

可见文字和可访问名是同一个 `label`。`ToggleChanged` 只有 `checked`，就是拨动之后的值，不是拨动前的值。

父级下面只选中了一部分时，用组件上的 `indeterminate`，不要用 `checked` 假装半选。

[总览](index.md) 和 [控件合同](../reference/components.md)
