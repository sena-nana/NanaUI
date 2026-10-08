# RangeSpanField

两个滑块圈出一段区间，轨道上还能标一个实时值。模板里的标签是 `<RangeSpan>`，函数是 `range_span(min, max, step)`，类型是 `RangeSpanField`。三个参数都是 `f64`。起点是整段：函数内部用 `RangeSpanField::new(minimum, maximum, minimum, maximum, step)`。

## 基本用法

:::api

```rust view
use nana_ui::runtime::{view, RangeSpanOrientation};

view! {
    <RangeSpan min=0 max=1 step=0.01 orientation={RangeSpanOrientation::Vertical}
        low_label="输入下限" high_label="输入上限" indicator={live} v-model={span} />
}
```

```rust rust
use nana_ui::runtime::{view::range_span, RangeSpanOrientation};

range_span(0.0, 1.0, 0.01)
    .orientation(RangeSpanOrientation::Vertical)
    .low_label("输入下限")
    .high_label("输入上限")
    .indicator(live)
    .model(span)
```

:::

`span` 是 `Signal<(f64, f64)>`，依次是下限和上限。`v-model={span}` 展开成 `.model(span)`，听的是 `RangeSpanInput`，拖动过程中的预览也会写回。`live` 是 `Signal<Option<f64>>`，只驱动显示，不写回。

两个滑块也可以分开绑：`low={…}`、`high={…}`。分开写入时一侧越过另一侧会把另一侧一起推过去，而不是停在那里，所以两个绑定谁先落地，结果都一样。`span` 一次写两侧，顺序反了会自己排好。

## 方向

默认横向：最小值在左。`orientation={RangeSpanOrientation::Vertical}`（或 `RangeSpanField::vertical()`）改成竖向：最小值在下，最大值在上，方向键上是增大。

横向时宽度默认铺满，高度至少是 `size` 档位的控件高度。竖向时宽度默认刚好放下滑块和焦点环，高度默认铺满；放进没有确定高度的列里时，自己给一个高度：

```rust
let mut field = RangeSpanField::new(0.2, 0.8, 0.0, 1.0, 0.01).vertical();
Arc::make_mut(&mut field.style.layout).height = Some(LengthSpec::Px(180.0));
```

## 轨道内缩

最小值和最大值不在节点的边上，而是各往里缩 `range_span_track_inset(size)`（实例上是 `track_inset()`）：半个滑块加焦点环，滑块停在两端时焦点环仍在节点里面。控件自己没有内边距和边框，这个距离就从节点边缘量。要在旁边画刻度或实时电平，按它对齐：值 `v` 在主轴上的位置是 `inset + (v - min) / (max - min) * (长度 - 2 * inset)`，竖向从底边量。

## 外观

两个滑块之间是一条实心强调色（`Accent`）细条，粗细约为轨道的三分之二，居中、两端圆角；区间外是中性色轨道（`BorderStrong`）。滑块与 [RangeField](range-field.md) 的滑块同样。禁用时轨道转 `Border`，区间转 `Faint`，滑块转禁用色。

## 实时指示

`indicator` 是轨道上的一个圆点，不吸附步进、不响应输入，画在滑块上面。超出范围的值画在较近的一端，非有限值不画。只改它不会动布局和别的节点：控件的整条轨道是一个自绘节点，改的只是这个节点的绘制键。程序里也可以用 `cx.set_range_span_indicator(field, Some(v))`。

## 可见取值和提交

`RangeSpanInput { low, high }` 是每一次可见取值，含拖拽中的预览。`RangeSpanChanged { low, high }` 是提交：指针抬起且有滑块动了、键盘步进、无障碍的增减或设值，或 `set_range_span` / `set_range_span_thumb`。取消的拖拽不提交，并恢复按下前的一对值。拖动起止另有 `RangeSpanDragging { dragging }`，键盘和无障碍步进不算拖动。三者与 [RangeField](range-field.md) 的 `RangeInput` / `RangeChanged` / `RangeDragging` 一一对应。

## 指针

按在滑块上就抓住这个滑块，不会跳到指针下面。按在轨道上，较近的那个滑块移到指针处（正好在两者中点时取下限）。两个滑块叠在同一个值上时，第一下往哪边动就由哪个滑块跟着走；叠在最小值上只能是上限，叠在最大值上只能是下限。拖动期间控件捕获指针，移动的滑块拿到焦点。滑块不会互相越过：拖过另一个就停在它那里。

## 键盘和读屏

两个滑块各是一个焦点停留点，Tab 先到下限再到上限。方向键按 `step`，PageUp / PageDown 按 `page_step`（默认 `step` 的十倍），Home / End 到它能到的两端——下限的 End 停在上限的值上。焦点可见时滑块外有一圈焦点环。

每个滑块在读屏里是一个 `Slider`，名字是 `low_label` / `high_label`，没有就用 `label`。数值范围按多滑块的约定给：下限的最大值是当前上限，上限的最小值是当前下限。支持 `Increment`、`Decrement` 和 `SetValue`。整个控件是一个带 `label` 的分组。滑块节点用 `thumb_node(RangeSpanThumb::Low)` 拿到。

## 边界

`RangeSpanField::new` 像 `RangeField::new` 一样修不一致的边界和步进。非有限的下限、上限落到最小、最大值；两者夹紧、对齐步进，反了就交换。步进是短小数时，值正好落在格子上（`0.6`，不是 `0.6000000000000001`）。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `low` / `high` | `f64` | 两个滑块。模板写入时一侧会推着另一侧走 |
| `span` | `(f64, f64)` | 模板字段，一次写两侧，顺序反了会排好。`v-model` 绑它，来源是 `RangeSpanInput` |
| `min` / `max` / `step` | `f64` | `range_span` 的三个参数 |
| `indicator` | `Option<f64>` | 实时值。不吸附、不响应输入，只改它是一次绘制更新 |
| `orientation` | `RangeSpanOrientation` | `Horizontal`（默认）或 `Vertical`（最小值在下） |
| `label` | `Option<Arc<str>>` | 分组名，也是没有自己名字的滑块的名字 |
| `low_label` / `high_label` | `Option<Arc<str>>` | 两个滑块的读屏名字 |
| `disabled` | `bool` | 滑块不可聚焦、不收指针，轨道和滑块转为禁用色 |
| `page_step` | — | 不在模板字段表里。翻页步进，默认 `step` 的十倍 |
| `size` | — | 不在模板字段表里。`ControlSize`，决定滑块直径和内缩 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_input`，模板 `on:RangeSpanInput` 或 `@input` | `&RangeSpanInput` | 每一次可见取值，含拖拽中的预览 |
| `on_change`，模板 `on:RangeSpanChanged` 或 `@change` | `&RangeSpanChanged` | 提交：抬起且动了、键盘步进、无障碍增减和设值、程序设置。取消的拖拽不提交 |
| `.on` 听 `RangeSpanDragging` | `&RangeSpanDragging` | `{ dragging }`。拖动起止 |

## 程序接口

| 方法 | 说明 |
| --- | --- |
| `set_range_span(field, low, high)` | 提交一对值，顺序反了会排好 |
| `set_range_span_thumb(field, thumb, value)` | 提交一个滑块，停在另一个滑块那里 |
| `adjust_range_span(field, thumb, RangeAdjustment)` | 像按键一样步进一个滑块 |
| `set_range_span_indicator(field, Option<f64>)` | 只移动实时指示 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 两个滑块节点由控件自己装配 |

控件没有登记到组件注册表，Vue / L3 文档里没有对应标签；只在 Rust 和 `view!` 里用。

## 参见

[总览](index.md) · [RangeField](range-field.md) · [控件](../reference/components.md)
