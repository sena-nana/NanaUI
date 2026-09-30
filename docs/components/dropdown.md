# Dropdown

`Dropdown` 是单值或多值选择。

它和 `Select` 共用字段表面和菜单。

选项的身份归你的应用；禁用的选项仍然看得见，只是不能选。

单选用 `Dropdown::single(value)`，多选用 `Dropdown::multiple(values)`。

真正的 `new` 是私有的，参数是 `DropdownSelection`，不要自己编一套 `Dropdown::new`。

选项用 `DropdownOption::new(value, label)`，还可以接 `.hint` 和 `.disabled`。

字段还有 `placeholder`、`size`、`disabled`、`loading`、`invalid`、`opened` 和 `highlighted`。

`inactive` 在禁用或加载时为真。

用户选中之后，控件自己写下选中值并收起菜单。

单值事件是 `DropdownEvent::Select`，多值是 `DropdownEvent::Toggle`，开合是 `Opened` 与 `Closed`。

禁用或加载时 `toggle_open` 不发事件。

控件表里没有 `<Dropdown>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Dropdown, DropdownOption};

view! {
    <Widget
        of={Dropdown::single(Some("rust"))
            .placeholder("语言")
            .options([
                DropdownOption::new("rust", "Rust"),
                DropdownOption::new("vue", "Vue").hint("模板").disabled(true),
            ])}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dropdown, DropdownOption};

widget(
    Dropdown::single(Some("rust"))
        .placeholder("语言")
        .options([
            DropdownOption::new("rust", "Rust"),
            DropdownOption::new("vue", "Vue").hint("模板").disabled(true),
        ]),
)
```

:::

多选把已选值收成一条标签。

没有选中时 `display_label` 返回占位，第二个值表示现在画的是不是占位。

`close` 只在菜单开着的时候发出 `Closed`。

方向键在打开的菜单里移动 `highlighted`，不会越过禁用项。

你要否决这次选择时，在处理函数里把 `selection` 写回你要的值；写成已经存在的值不会多提交一次。

你平时改的是这几项：

- `placeholder` 是没有选中时的提示，也是占位状态下的可访问名。
- `size` 跟别的字段控件同一套高度档。
- `disabled` 和 `loading` 都会让字段停用。这时 `toggle_open` 不发事件。
- `invalid` 写进无障碍状态，提交前的名单仍由 `validity_of` 汇总。
- `opened` 表示菜单是否展开。写成 `false` 会清掉高亮；要发出 `Closed`，调用 `close`。

[总览](index.md) · [控件合同](../reference/components.md)
