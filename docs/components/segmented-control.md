# SegmentedControl

`SegmentedControl` 是横排的分段选择，无障碍角色是单选组。选项是它的子节点，每个选项有自己的标签、禁用和选中。方向键只移动焦点，不改选中；选中跟随点击、Enter 或空格，这是读屏对 radiogroup 的预期。

## 基本用法

公开的视图函数是 `segmented` 和 `segmented_option`。`segmented()` 就是 `widget(SegmentedControl::new())`。控件表里没有 `<SegmentedControl>`。选项仍是 `segmented_option`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::segmented_option;
use nana_ui::runtime::SegmentedControl;

view! {
    <Widget of={SegmentedControl::new().label("输入方式")}>
        {segmented_option("面捕").selected(move || mode_is_face()).on_select(face)}
        {segmented_option("鼠标").selected(move || !mode_is_face()).on_select(mouse)}
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{segmented, segmented_option};

segmented().label("输入方式").children((
    segmented_option("面捕").selected(move || mode_is_face()).on_select(face),
    segmented_option("鼠标").selected(move || !mode_is_face()).on_select(mouse),
))
```

:::

`mode_is_face`、`face`、`mouse` 是你自己的判断和回调。

## 构造

`SegmentedControl::new` 没有参数，默认中号、不拉满。`.label` 给整组一个可访问名，`.size` 改控件档，`.fill(true)` 让选项平分宽度。

横向分段选择默认自动换行（`.wrap(false)` 关闭）：给它的宽度放得下一行时保持一行和自己的宽度，放不下时换行、高度跟随内容，选项文字不会被截断或溢出容器。一行时高度和不换行的控件一样。与 `.fill(true)` 一起使用时，每行按原顺序容纳尽可能多的选项，使用真实文字、图标和主题留白的固有尺寸判断是否放得下，再将该行宽度均分；末行也铺满。`.fill(false).wrap(true)` 保留选项各自的固有宽度，只在放不下时换行。隐藏项不占位，禁用项照常参与布局。单个标签比整行还长时，独占一行并折行增高，不缩小字号。普通选项维持当前主题的标准高度。Radio 与独立 Tabs 不采用此换行策略。

```rust
SegmentedControl::new().size(ControlSize::Small).fill(true)
```

语义组件入口接受 `fill` 和 `wrap` 布尔属性，与 Rust 构造器采用同一套布局。没写 `wrap` 时和 `SegmentedControl::new()` 一样默认换行；写 `wrap="false"`（或 `0`、`no`）才保持一行。

## 竖排

`radio_group()` 用同一套选择，改成竖排的圆环。`radio_group()` 适合选项说明较长、需要竖排的地方，选择合同和横排分段是同一份。

## 选项

选项用 `segmented_option(label)`。`.selected` 告诉控件这一项是不是当前选择，`.disabled` 禁掉它，`.on_select` 在用户选中这一项时运行。

选项被选中后，下一次绑定会按你写的 `.selected` 再说一遍；控件不会替你保存一份产品状态。

## 布局

一个选项是一行：竖排圆环在前（`Indicator`，固定的控件档尺寸，隔 `indicator_gap`），图标其次（`LeadingContent`，分段项取标签字号的 1em，竖排取控件档的图标尺寸，隔主题的 `xs`），标签取余下的宽度（`TextContent`）。这些是布局发布的部件，intrinsic measure、段落预算、摆放和绘制都读这一份；内边距只剩选项自己的内缩，不再替圆环和图标留位置。分段项的标签在它那条宽度里居中，竖排的标签起点对齐。标签不是垂直居中时，图标对齐标签第一行的中线。选中、禁用、悬停只改绘制，不重新布局。从右到左的方向下圆环和图标仍在左侧，不镜像。

## 登记

控件在建好之后，以及绑定改了它或某一项时，自己登记子节点和选中，视图里不要再调 `set_segmented_options`。

## 藏起一项

藏起某一项时要连同禁用一起做，节点还在。藏起一项时，禁用和隐藏一起做，节点还在组里。

## 激活

激活时控件自己改选中，先在控件上发 `SegmentedSelectionRequested { option }`，再在被选中的那一项上发 `SegmentedOptionChosen`。你要否决时，把 `.selected` 写回你要的那一项。

`SegmentedSelectionRequested` 里的 `option` 是那一项的 `StableNodeId`。你按这个身份找到自己的业务值。`SegmentedOptionChosen` 发在被点中的那一项上，方便只听某一段。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.label` | — | 整组的可访问名 |
| `.size` | — | 控件档。`new` 默认中号 |
| `.fill` | — | `true` 让选项平分宽度。默认不拉满 |
| `.wrap` | `bool` | 横向分段选项在宽度不够时换行，默认开启；与 fill 组合时同行均分 |
| 选项 `label` | — | `segmented_option(label)` |
| `.selected` | — | 这一项是不是当前选择。控件不替你保存产品状态，下一次绑定按你写的再说一遍 |
| `.disabled` | — | 禁掉这一项。藏起时要连同禁用，节点还在 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `SegmentedSelectionRequested` | `{ option }`，`option` 是 `StableNodeId` | 激活时先在控件上发。要否决时把 `.selected` 写回你要的那一项 |
| `SegmentedOptionChosen` | — | 再发在被选中的那一项上，方便只听某一段 |
| `.on_select` | — | 用户选中这一项时运行 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 选项 | 子节点，用 `segmented_option`。控件自己登记子节点和选中，视图里不要再调 `set_segmented_options` |

## 参见

[总览](index.md) · [控件](../reference/components.md)
