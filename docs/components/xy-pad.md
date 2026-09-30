# XYPad

`XYPad` 是一块两轴垫。

指针按下和移动发 `XYPadEvent::Input`，抬起以及焦点上的方向键发 `XYPadEvent::Change`。

值是 `XYPadValue { x, y }`，用 `XYPadValue::new(x, y)` 构造。

`XYPad::new(value)` 默认两轴都是 `0.0..=1.0`，步进 `0` 表示不量化。

`.x_range` 和 `.y_range` 改范围，非法范围会被收成有效区间，当前值跟着夹进新范围。

`.step` 只接受有限且大于 0 的数，否则当作不量化。

`.size` 改高度：小 40、中 48、大 64，比单行控件更高。

`.label` 是可访问名。

`.disabled`、`.loading`、`.invalid` 跟别的字段一样。

`inactive` 在禁用或加载时为真。

纵轴向上增大：垫子顶部是 `y_max`，底部是 `y_min`。

按住 Shift 拖动时，先出现的那一轴锁定（`|dx| >= |dy|` 锁横轴），另一轴保持按下时的值。

键盘步进是 `XYPadAdjustment` 的 `Left`、`Right`、`Up`、`Down`，步进之后直接提交 `Change`。

控件表里没有 `<XYPad>`。

色块字段装配出来的饱和度/明度垫也是这个类型。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{XYPad, XYPadValue};

view! {
    <Widget
        of={XYPad::new(XYPadValue::new(0.2, 0.8))
            .x_range(-1.0, 1.0)
            .y_range(-1.0, 1.0)
            .step(0.05)
            .label("偏移")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{XYPad, XYPadValue};

widget(
    XYPad::new(XYPadValue::new(0.2, 0.8))
        .x_range(-1.0, 1.0)
        .y_range(-1.0, 1.0)
        .step(0.05)
        .label("偏移"),
)
```

:::

拖动中的预览读 `Input`。

只有抬起，或方向键，才是 `Change`。

取消的拖动不提交。

`value_at` 把垫子局部坐标换成夹紧并量化后的值，供你自己对一下命中。

两轴的范围各自夹紧。

你把范围收窄之后，当前值会跟着进新区间。

`step` 为 `0` 或非有限、非正数时，垫子按连续值处理。

Shift 锁定的是拖动里先占优的那一轴，另一轴停在按下时的数。

高度由 `.size` 决定：小 40、中 48、大 64。

`ColorField` 装配出来的饱和度/明度选择也是这块垫子，纵轴同样向上增大。

[总览](index.md) · [控件合同](../reference/components.md)
