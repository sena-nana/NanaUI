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

## 竖排

`radio_group()` 用同一套选择，改成竖排的圆环。`radio_group()` 适合选项说明较长、需要竖排的地方，选择合同和横排分段是同一份。

## 选项

选项用 `segmented_option(label)`。`.selected` 告诉控件这一项是不是当前选择，`.disabled` 禁掉它，`.on_select` 在用户选中这一项时运行。

选项被选中后，下一次绑定会按你写的 `.selected` 再说一遍；控件不会替你保存一份产品状态。

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
