# Dropdown

单值或多值选择。它和 [Select](select.md) 共用字段表面和菜单。边输入边查，用 [SearchDropdown](search-dropdown.md)。

控件表里没有 `<Dropdown>`。两种写法都调用 `widget`。

选项的身份归你的应用。禁用的选项仍然看得见，只是不能选。

## 基本用法

单选用 `Dropdown::single(value)`，多选用 `Dropdown::multiple(values)`。真正的 `new` 是私有的，参数是 `DropdownSelection`，不要自己编一套 `Dropdown::new`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Dropdown;

widget(Dropdown::single(value))
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Dropdown;

widget(Dropdown::single(value))
```

:::

选项用 `DropdownOption::new(value, label)`，还可以接 `.hint` 和 `.disabled`。`hint` 在菜单里画在这一行的末尾，比标签小一号、用次要色，和 [Select](select.md) 的选项提示一样。

## 选中之后

用户选中之后，控件自己写下选中值并收起菜单。

单值事件是 `DropdownEvent::Select`，多值是 `DropdownEvent::Toggle`，开合是 `Opened` 与 `Closed`。禁用或加载时 `toggle_open` 不发事件。`inactive` 在禁用或加载时为真。

## 读屏

打开的菜单和 [Select 的读屏](select.md#读屏) 一样，是字段下的列表框。多选时列表框可多选（`aria-multiselectable`），勾上的每一项都是「已选」；读屏激活一项就是勾上或取消它，菜单不收起，和按下这一行一样。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| 选中值 | `Dropdown::single` / `Dropdown::multiple` 的参数 | 单值一份，多值一份列表。不要调用私有的 `Dropdown::new` |
| 选项 | `DropdownOption` | `DropdownOption::new(value, label)`，可再接 `.hint` 和 `.disabled` |
| `placeholder` | 字段 | 和 Select 共用的字段表面 |
| `size` | 字段 | 同上 |
| `disabled` | 字段 | 禁用时 `toggle_open` 不发事件 |
| `loading` | 字段 | 加载时 `toggle_open` 不发事件 |
| `invalid` | 字段 | 校验态 |
| `opened` | 字段 | 菜单是否打开 |
| `highlighted` | 字段 | 当前高亮项 |
| `menu_scroll` | 字段 | 菜单的滚动位置。菜单的展开、滚动和滚动条见 [Select 的菜单](select.md#菜单) |
| `bare_trigger` | 字段 | 静止时去掉填充和描边，用作工具条上的选择器；悬停和展开仍有底色，校验态仍描红边 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `DropdownEvent::Select` | 单值提交 | 用户选中一项 |
| `DropdownEvent::Toggle` | 多值切换 | 多选里改一项 |
| `Opened` / `Closed` | 开合 | 菜单打开或关上 |

## 插槽

没有具名插槽。选项是数据，不是子节点。

## 参见

[总览](index.md) · [Select](select.md) · [控件](../reference/components.md)
