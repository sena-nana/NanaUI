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

`RangeInput` 是每一次可见取值，含拖拽中的预览，适合实时预览。`RangeChanged` 是提交：指针抬起且值变了、键盘步进、无障碍 `SetValue`、`set_range_value`，或读数滑块的键入与恢复默认。取消的拖拽不提交。键盘和无障碍的每一步都是提交。

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

## 渐变轨

`track_gradient(色标)` 把轨道画成一条从最小值端到最大值端穿过这些色标的渐变条，用于取值本身是颜色的刻度，比如色相条、不透明度条。渐变条不画填充，也不加字段内边距；条宽是 12，圆点换成对比色圆环（浅色环夹两道深色细边，中间镂空露出所标的颜色），比条高出一圈，在任何色标上都看得清。圆环用主题的媒体前景色和媒体遮罩色，深浅模式下都是浅环深边。轨道两端只内缩半个圆环，圆环在两端也留在控件里。标签、读数、键盘步进、拖动和读屏 `Slider` 语义都不变；要一条光秃秃的色条，再配 `show_value(false)`。色标少于两个时保持常规外观；画家只读前八个色标，和任何线性渐变一样。同时设了 `rail` 时细轨优先。`hue_spectrum()` 是 0 到 360 度的色相环展开（红、黄、绿、青、蓝、品红、回到红），`ColorField` 的色相条就是它。

```rust
use nana_ui::runtime::{RangeField, hue_spectrum};

RangeField::new(200.0, 0.0, 360.0, 1.0)
    .label("色相")
    .show_label(false)
    .show_value(false)
    .track_gradient(hue_spectrum())
```

## 读数滑块

`value_thumb(true)` 把读数写进滑块：滑块是一个实心 Accent 的宽胶囊（`Full` 圆角），读数（只有数值，不带单位，等宽数字）用 Accent 上的文字色居中写在里面，轨道是一条细线，填充到胶囊。右侧不再单独画读数，也不排读数槽。配上 `show_bounds(true)` 和 `default_value(Some(默认值))`，一行是：

```text
最小值  ━━━━[ 137 ]━━━━  最大值单位  ↺
```

```rust
use nana_ui::runtime::RangeField;

RangeField::new(137.0, 0.0, 200.0, 1.0)
    .label("缩放")
    .show_label(false)
    .unit("%")
    .value_thumb(true)
    .show_bounds(true)
    .default_value(Some(100.0))
```

视图里写成 `slider(0.0, 200.0, 1.0).value_thumb(true).show_bounds(true).default_value(默认值)`，`default_value` 可以绑 `Signal<Option<f64>>`；模板是 `<Slider value_thumb={true} show_bounds={true} default_value={Some(100.0)}>`。

- **胶囊。** 高度是控件的文字行高加 `sm` 一档（常规尺寸 16 + 6 = 22），不超过内容盒；宽度是最小值、最大值和当前值里最宽的那个（等宽数字）加左右各 `md`，至少两个 `page`（48），所以拖动时宽度不变。胶囊中心的行程是轨道道两端各缩进半个胶囊，胶囊两端始终留在轨道道里。轨道在胶囊两侧各断开 `xxs` 一档，透出底色，压在填充上也看得清。禁用时胶囊换成禁用底色、文字换成禁用文字色。键盘焦点可见时，焦点环沿胶囊外沿。
- **组件几何。** `RangeGeometry`（经 `UiWorld::range_geometry` 获取）的 `value_thumb: Option<RangeValueThumb>` 给出胶囊盒（`capsule`）、读数（`text`）、断开的宽度（`gap`）和是否在键入（`editing`）；`track` 仍是滑块中心的行程，指针取值用它。
- **拖动。** 按在胶囊上开始拖动不跳值：值从按下时的值起，按指针位移换算。按在轨道别处先跳到那一点，再从那里相对拖动。
- **键入。** 在胶囊上按下、移动不足 2px 就松开，或焦点在滑条上按 Enter（Space、读屏的按下也算），进入键入：滑条组装的 `NumberInput` 盖在胶囊的位置（比胶囊左右各宽 `xl`），显示当前值、全选并获得焦点，胶囊让位。Enter 或焦点离开时提交：夹在范围内、按 `step` 对齐，值变了就发 `RangeInput` 和 `RangeChanged`。Escape 还原，不发事件。结束后焦点回到滑条。`editing` 表示是否在键入；`begin_range_edit(滑条)` 从代码进入键入。

放进视图时由视图组装恢复默认按钮和键入用的 `NumberInput`，之后每次写滑条都会按属性同步它们。手工用 `create_component` 创建时，调用一次 `assemble_range_field(滑条)`（或在第一次写入后自动组装）。两个子节点都绝对定位在滑条里，不参与行的布局，滑条在有它们时是它们的定位容器。

## 边界值

`show_bounds(true)` 在轨道左端前写最小值、右端后写最大值，和读数同一精度，用 caption 字号、muted 色、等宽数字。单位只跟在最大值后面：单位以文字开头时隔一个空格（`2.0 秒`、`10 px`），以符号开头时紧跟数字（`200%`、`90°`）。两者是布局部件 `LayoutPartKind::Minimum` 与 `LayoutPartKind::Maximum`，轨道道在它们之间；不画标签（`show_label(false)`）时最小值就从内容盒起点开始。不是读数滑块时，右侧读数照常排在最大值之后。

## 恢复默认

`default_value(Some(值))` 在行末放一个 24×24 的紧凑图标按钮（`IconButton::tool`，图标 `Icon::Reset`），读屏名称取框架文案 `range_field.reset`（默认「恢复默认」）。按钮是独立的读屏 `Button` 节点，可以按名字点击。值等于默认值（按 `step` 对齐后）时按钮禁用：淡化、不能点、不能聚焦。点击等同于 `set_range_value(默认值)`：发 `RangeInput` 与 `RangeChanged`。按钮的位置是行末的 `LayoutPartKind::TrailingContent` 部件。`default_value(None)` 去掉按钮。

## 精细拖动

所有滑条在拖动中按住 Alt（macOS 上是 Option）时，指针位移换算的数值变化乘 0.1。拖动途中按下或松开 Alt 不跳值：在切换处按当时的指针位置和值重新锚定，之后按位移相对拖动，不再回到指针所指的位置。不按 Alt 的普通拖动照旧由指针位置取值（含刻度吸附）。代码里是 `update_range_drag_fine(文档, 指针, x, fine)`，`update_range_drag` 等于 `fine = false`。

## 读屏

没人给滑块起名、又放进设置行时，读屏用行标签。自己有 `label` 时用标签。旁边另有可见标题时，`labelled_by={caption}`（`slider(..).labelled_by(caption)`）用那段标题的文字。

## 边界和翻页

`RangeField::new` 会修不一致的边界：非有限或颠倒的收成 `0.0..=1.0`，非法的 `step` 收成跨度的百分之一，值再夹紧并对齐步进。`slider` 走的就是这个构造。翻页步进默认是 `step` 的十倍，组件上用 `page_step` 改。

## 属性

字段是 `value: f64`、`label: Option<Arc<str>>`、`unit: Option<Arc<str>>`、`disabled: bool`。`model` 绑定 `value`，来源是 `RangeInput`，不是提交。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `f64` | `model` 绑定这一字段，来源是 `RangeInput`，不是提交 |
| `label` | `Option<Arc<str>>` | 自己有标签时读屏用它。没人起名、又放进设置行时，读屏用行标签 |
| `disabled` | `bool` | 字段 |
| `unit` | `Option<Arc<str>>` | 读数后面的单位，读屏跟在值后面读。跟着语言变时在视图里绑定（`.unit(..)`），标签不再在括号里写单位 |
| `min` / `max` / `step` | `f64` | `slider` 的三个参数。起点就是 `min`，内部是 `RangeField::new(minimum, minimum, maximum, step)` |
| `show_value` | — | 不在 `<Slider>` 字段表里，`slider` 上也没有同名方法。默认在轨道旁画当前值和单位；`false` 只留轨道，读屏仍能读到数值 |
| `show_label` | `bool` | `false` 不画标签，轨道占满，标签仍是读屏名称 |
| `page_step` | — | 翻页步进，默认是 `step` 的十倍 |
| `rail` | `Option<f32>` | `rail(粗细)` 只画这么粗的细轨，横贯整个控件，没有标签、数值和内边距，圆点只在焦点可见时出现；控件高度就是命中区 |
| `track_gradient` | `Option<Arc<[GradientStop]>>` | `track_gradient(色标)` 把轨道画成渐变条：没有填充和字段内边距，圆点换成对比色圆环，语义不变。少于两个色标时保持常规外观；`rail` 优先 |
| `caution_from` | `Option<f64>` | `caution_from(值)` 把从这个值到最大值的一段标成警示段：这段的轨道与填充换成主题的 Warning 色（填充与未填充处同为实色，位置由滑块标出），取值进入这一段时滑块（胶囊或圆点）也从 Accent 换成 Warning 色，胶囊里的读数取 Accent 文字色与正文色中对比更高的一个，禁用时不换；落在这一段里的 `markers`（含起点上的那个）也画成 Warning 色；读数、步进与读屏语义不变，也不拦住取值；要在边界停一下就在那里配一个 `markers`。值在范围外或等于最大值时不标；`rail`、`track_gradient` 优先。几何在 `RangeGeometry` 的 `caution: Option<RangeCaution>`。只在构造器上，`<Slider>` 字段表里没有 |
| `value_thumb` | `bool` | `value_thumb(true)` 把读数写进宽胶囊滑块，不画右侧读数；在胶囊上拖动不跳值，单击或 Enter 键入。`rail`、`track_gradient` 优先。`<Slider>` 字段表里有，`slider` 上同名 |
| `show_bounds` | `bool` | `show_bounds(true)` 在轨道两端写最小值和带单位的最大值。`<Slider>` 字段表里有 |
| `default_value` | `Option<f64>` | `Some(值)` 在行末放恢复默认按钮，值等于它时禁用；非有限值当作 `None`。`<Slider>` 字段表里有，可绑定 |
| `editing` | `bool` | 读数滑块正在键入。只读状态，由键入的开始与结束维护 |

## 事件

事件有两个，处理器都接收事件引用：`on_input` 是 `RangeInput`，`on_change` 是 `RangeChanged`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_input`，模板 `on:RangeInput` 或 `@input` | `&RangeInput` | 每一次可见取值，含拖拽中的预览。函数原样传入，参数必须是这个引用；其它表达式展开成 `move \|\| { … }`，事件被忽略 |
| `on_change`，模板 `on:RangeChanged` 或 `@change` | `&RangeChanged` | 提交：指针抬起且值变了、键盘步进、无障碍 `SetValue`、`set_range_value`、读数滑块键入后的 Enter 或失焦，或恢复默认按钮。取消的拖拽和 Escape 不提交。键盘和无障碍的每一步都是提交 |
| `.on` 听 `RangeDragging` | `&RangeDragging` | `{ dragging }`。拖动起止。键盘和无障碍步进不算拖动。它不是上面两个事件 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 恢复默认按钮与键入用的 `NumberInput` 由滑条自己组装 |

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
