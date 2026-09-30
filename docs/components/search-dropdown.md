# SearchDropdown

`SearchDropdown` 是带查询的单值字段。

查询走已提交的 `TextInput` 状态，不是另一套输入框。

选项仍由你提供，控件按查询做子串匹配。

`SearchDropdown::new(value)` 的 `value` 是当前选中值，可以是 `None`。

选项是 `SearchDropdownOption::new(value, label)`，`.hint` 给一行补充。

字段还有 `query`、`placeholder`、`size`、`disabled`、`loading`、`invalid`、`opened` 和 `highlighted`。

事件是 `SearchDropdownEvent`：输入查询发 `Search`，选中发 `Select`，开合发 `Opened` 与 `Closed`。

选中之后控件自己写入 `value` 并收起菜单。

`set_query` 同时改 `query`、输入状态和高亮的第一项。

禁用或加载时 `toggle_open` 不发事件。

控件表里没有 `<SearchDropdown>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{SearchDropdown, SearchDropdownOption};

view! {
    <Widget
        of={SearchDropdown::new(Some("main"))
            .placeholder("搜索文件")
            .options([
                SearchDropdownOption::new("main", "main.rs").hint("src"),
                SearchDropdownOption::new("lib", "lib.rs"),
            ])}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{SearchDropdown, SearchDropdownOption};

widget(
    SearchDropdown::new(Some("main"))
        .placeholder("搜索文件")
        .options([
            SearchDropdownOption::new("main", "main.rs").hint("src"),
            SearchDropdownOption::new("lib", "lib.rs"),
        ]),
)
```

:::

菜单打开且查询为空时，表面显示占位；查询非空时显示查询本身。

关上之后，有匹配的选中值就显示该选项的标签。

`visible_indices` 是当前查询下仍然可见的下标。

`close` 只在开着时发出 `Closed`，并清掉高亮。

过滤规则在控件里。

你要换一套排序或远程结果时，改 `options`。

查询字符串以已提交的输入为准，`set_query` 会同时改 `query`、输入状态，并把高亮移到第一项可见结果。

匹配不区分大小写，看标签、值和 `hint`。

空查询时全部选项可见。

禁用或加载时字段处于 `inactive`，打开菜单的那一下不会发出 `Opened`。

选中事件里的值就是选项的 `value`。

[总览](index.md) · [控件合同](../reference/components.md)
