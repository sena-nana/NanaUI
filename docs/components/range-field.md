# RangeField

模板里的标签是 `<Slider>`，函数是 `slider(min, max, step)`，类型是 `RangeField`。三个参数都是 `f64`。起点就是 `min`：函数内部用 `RangeField::new(minimum, minimum, maximum, step)`。

## 基本用法

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

## 可见取值和提交

`RangeInput` 是每一次可见取值，含拖拽中的预览，适合实时预览。`RangeChanged` 是提交：指针抬起且值变了、键盘步进、无障碍 `SetValue`，或 `set_range_value`。取消的拖拽不提交。键盘和无障碍的每一步都是提交。

## 监听

`.on_input(|event: &RangeInput| …)` 和 `.on_change(|event: &RangeChanged| …)` 分别听这两次。模板里写成 `on:RangeInput={…}`、`on:RangeChanged={…}`。

`@input={save}` 在 `save` 是函数时原样传入，参数必须是 `&RangeInput`。`@change={save}` 则必须是 `&RangeChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

## 拖动

拖动起止另有 `RangeDragging { dragging }`。键盘和无障碍步进不算拖动。它不是上面两个事件，用 `.on(|event: &RangeDragging| …)`。

## 轨道上的值和标签

轨道旁默认画当前值和单位。`show_value(false)` 只留轨道，读屏仍能读到数值。`show_label(false)` 不画标签，轨道占满，标签仍是读屏名称；模板里是 `<Slider label="音量" show_label={false}>`，`slider(..)` 上是 `.show_label(false)`。`show_value` 不在 `<Slider>` 的字段表里，`slider` 上也没有同名方法。要改，在 `RangeField` 上调用 `show_value(false)`，或 `.bind(|field| field.show_value = false)`。

## 细轨

`rail(粗细)` 把滑块画成一条细轨：只画这么粗的轨道和已填充的部分，横贯整个控件，不画标签、数值和字段内边距；圆点只在键盘焦点（焦点可见）落在它上面时出现。控件自己的高度不变，所以 2px 的轨道可以放在 16px 高的命中区里，指针在整个控件宽度上取值。标签仍是读屏名称。粗细不是有限正数时保持常规外观。媒体条的 `Mini` 密度用它画进度和音量。

```rust
let mut seek = RangeField::new(0.0, 0.0, 100.0, 1.0).label("进度").rail(2.0);
Arc::make_mut(&mut seek.style.layout).height = Some(LengthSpec::Px(16.0));
```

## 读屏

没人给滑块起名、又放进设置行时，读屏用行标签。自己有 `label` 时用标签。旁边另有可见标题时，`labelled_by={caption}`（`slider(..).labelled_by(caption)`）用那段标题的文字。

## 边界和翻页

`RangeField::new` 会修不一致的边界：非有限或颠倒的收成 `0.0..=1.0`，非法的 `step` 收成跨度的百分之一，值再夹紧并对齐步进。`slider` 走的就是这个构造。翻页步进默认是 `step` 的十倍，组件上用 `page_step` 改。

## 属性

字段是 `value: f64`、`label: Option<Arc<str>>`、`disabled: bool`。`model` 绑定 `value`，来源是 `RangeInput`，不是提交。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `f64` | `model` 绑定这一字段，来源是 `RangeInput`，不是提交 |
| `label` | `Option<Arc<str>>` | 自己有标签时读屏用它。没人起名、又放进设置行时，读屏用行标签 |
| `disabled` | `bool` | 字段 |
| `min` / `max` / `step` | `f64` | `slider` 的三个参数。起点就是 `min`，内部是 `RangeField::new(minimum, minimum, maximum, step)` |
| `show_value` | — | 不在 `<Slider>` 字段表里，`slider` 上也没有同名方法。默认在轨道旁画当前值和单位；`false` 只留轨道，读屏仍能读到数值 |
| `show_label` | `bool` | `false` 不画标签，轨道占满，标签仍是读屏名称 |
| `page_step` | — | 翻页步进，默认是 `step` 的十倍 |
| `rail` | `Option<f32>` | `rail(粗细)` 只画这么粗的细轨，横贯整个控件，没有标签、数值和内边距，圆点只在焦点可见时出现；控件高度就是命中区 |

## 事件

事件有两个，处理器都接收事件引用：`on_input` 是 `RangeInput`，`on_change` 是 `RangeChanged`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_input`，模板 `on:RangeInput` 或 `@input` | `&RangeInput` | 每一次可见取值，含拖拽中的预览。函数原样传入，参数必须是这个引用；其它表达式展开成 `move \|\| { … }`，事件被忽略 |
| `on_change`，模板 `on:RangeChanged` 或 `@change` | `&RangeChanged` | 提交：指针抬起且值变了、键盘步进、无障碍 `SetValue`，或 `set_range_value`。取消的拖拽不提交。键盘和无障碍的每一步都是提交 |
| `.on` 听 `RangeDragging` | `&RangeDragging` | `{ dragging }`。拖动起止。键盘和无障碍步进不算拖动。它不是上面两个事件 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
