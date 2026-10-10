# XYPad

`XYPad` 是一块两轴垫。指针按下和移动发 `XYPadEvent::Input`，抬起以及焦点上的方向键发 `XYPadEvent::Change`。值是 `XYPadValue { x, y }`，用 `XYPadValue::new(x, y)` 构造。

## 基本用法

`XYPad::new(value)` 默认两轴都是 `0.0..=1.0`，步进 `0` 表示不量化。`.x_range` 和 `.y_range` 改范围，非法范围会被收成有效区间，当前值跟着夹进新范围。`.step` 只接受有限且大于 0 的数，否则当作不量化。`.size` 改高度：小 40、中 48、大 64，比单行控件更高。`.label` 是可访问名。`.disabled`、`.loading`、`.invalid` 跟别的字段一样。

控件表里没有 `<XYPad>`。色块字段装配出来的饱和度/明度垫也是这个类型。

## 画面垫

`.surface(XYPadSurface::Picture)` 把垫子自己的绘制当成取值的画面，比如取色器的饱和度/明度方块。默认的 `XYPadSurface::Plain` 是中性底（样式没指定填充时用 `Subtle`）、中心十字轴和强调色圆点。画面垫不画十字轴，用样式里的背景色和背景图层（`layout.background`、`layout.paint.background_image`、`background_layers`，普通盒子绘制照常画它们）作画面；值用对比色圆环标出：浅色环夹两道深色细边，中间镂空，在亮色和暗色上都看得清，颜色取主题的媒体前景色和媒体遮罩色。后面的背景图层会盖住表面底下描的边框，所以画面垫会在图层之上再描一次边框，悬停、焦点和 `invalid` 在四条边上都看得见。拖动、键盘和读屏语义不变。

`.height(像素)` 换掉 `.size` 给的高度，用于画面垫这种用来取值的大块，而不是行里的控件。不是有限正数时忽略。

```rust
use nana_ui::runtime::{XYPad, XYPadSurface, XYPadValue};

let mut pad = XYPad::new(XYPadValue::new(0.5, 0.5))
    .surface(XYPadSurface::Picture)
    .height(128.0);
std::sync::Arc::make_mut(&mut pad.style.layout).background = Some([0.2, 0.6, 0.9, 1.0]);
```

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

## 拖动和键盘

`inactive` 在禁用或加载时为真。纵轴向上增大：垫子顶部是 `y_max`，底部是 `y_min`。按住 Shift 拖动时，先出现的那一轴锁定（`|dx| >= |dy|` 锁横轴），另一轴保持按下时的值。键盘步进是 `XYPadAdjustment` 的 `Left`、`Right`、`Up`、`Down`，步进之后直接提交 `Change`。

拖动中的预览读 `Input`。只有抬起，或方向键，才是 `Change`。取消的拖动不提交。`value_at` 把垫子局部坐标换成夹紧并量化后的值，供你自己对一下命中。两轴的范围各自夹紧。你把范围收窄之后，当前值会跟着进新区间。`step` 为 `0` 或非有限、非正数时，垫子按连续值处理。Shift 锁定的是拖动里先占优的那一轴，另一轴停在按下时的数。高度由 `.size` 决定：小 40、中 48、大 64；`.height` 可以另给。

`ColorField` 装配出来的饱和度/明度选择也是这块垫子，纵轴同样向上增大。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `XYPadValue { x, y }` | `XYPad::new(value)`。用 `XYPadValue::new(x, y)` 构造 |
| `.x_range` | — | 默认 `0.0..=1.0`。非法范围收成有效区间，当前值跟着夹进新范围 |
| `.y_range` | — | 默认 `0.0..=1.0`。纵轴向上增大：顶部是 `y_max`，底部是 `y_min` |
| `.step` | — | 默认 `0` 表示不量化。只接受有限且大于 0 的数，否则当作不量化 |
| `.size` | — | 高度：小 40、中 48、大 64，比单行控件更高 |
| `.height` | `f32` | 换掉 `.size` 给的高度。不是有限正数时忽略 |
| `.surface` | `XYPadSurface` | 默认 `Plain`：中性底、十字轴、强调色圆点。`Picture`：垫子自己的背景色和背景图层是画面，不画十字轴，值用对比色圆环标出，边框描在画面之上 |
| `.label` | — | 可访问名 |
| `.disabled` | — | 跟别的字段一样。禁用时 `inactive` 为真 |
| `.loading` | — | 跟别的字段一样。加载时 `inactive` 为真 |
| `.invalid` | — | 跟别的字段一样 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `XYPadEvent::Input` | — | 指针按下和移动。拖动中的预览读它 |
| `XYPadEvent::Change` | — | 抬起，以及焦点上的方向键。键盘步进之后直接提交。取消的拖动不提交 |

## 插槽

没有插槽。

## 参见

[总览](index.md) · [控件](../reference/components.md)
