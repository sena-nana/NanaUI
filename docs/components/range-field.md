# RangeField

模板里的标签是 `<Slider>`，函数是 `slider(min, max, step)`，类型是 `RangeField`。三个参数都是 `f64`。起点就是 `min`：函数内部用 `RangeField::new(minimum, minimum, maximum, step)`。

字段是 `value: f64`、`label: Option<Arc<str>>`、`disabled: bool`。事件有两个，处理器都接收事件引用：`on_input` 是 `RangeInput`，`on_change` 是 `RangeChanged`。`model` 绑定 `value`，来源是 `RangeInput`，不是提交。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Slider min=0 max=100 step=1 label="音量" v-model={volume} />
}
```

```rust rust
use nana_ui::runtime::view::slider;

slider(0.0, 100.0, 1.0).label("音量").model(volume)
```

:::

模板里的 `min=0` 展开成带后缀的 `0_f64`。手写时参数已经是 `f64`，写成 `0.0` 即可。

`v-model={volume}` 展开成 `.model(volume)`。`volume` 是 `Signal<f64>`。拖动过程中的预览也会写回它，因为模型听的是 `RangeInput`。

`RangeInput` 是每一次可见取值，含拖拽中的预览，适合实时预览。`RangeChanged` 是提交：指针抬起且值变了、键盘步进、无障碍 `SetValue`，或 `set_range_value`。取消的拖拽不提交。键盘和无障碍的每一步都是提交。

`.on_input(|event: &RangeInput| …)` 和 `.on_change(|event: &RangeChanged| …)` 分别听这两次。模板里写成 `on:RangeInput={…}`、`on:RangeChanged={…}`。

拖动起止另有 `RangeDragging { dragging }`。键盘和无障碍步进不算拖动。它不是上面两个事件，用 `.on(|event: &RangeDragging| …)`。

轨道旁默认画当前值和单位。`show_value(false)` 只留轨道，读屏仍能读到数值。`show_label(false)` 不画标签，轨道占满，标签仍是读屏名称。这两项不在 `<Slider>` 的字段表里，`slider` 上也没有同名方法。要改，在 `RangeField` 上调用这两个方法，或 `.bind(|field| field.show_value = false)`。

没人给滑块起名、又放进设置行时，读屏用行标签。自己有 `label` 时用标签。

`RangeField::new` 会修不一致的边界：非有限或颠倒的收成 `0.0..=1.0`，非法的 `step` 收成跨度的百分之一，值再夹紧并对齐步进。`slider` 走的就是这个构造。翻页步进默认是 `step` 的十倍，组件上用 `page_step` 改。

`@input={save}` 在 `save` 是函数时原样传入，参数必须是 `&RangeInput`。`@change={save}` 则必须是 `&RangeChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

[总览](index.md) 和 [控件合同](../reference/components.md)
