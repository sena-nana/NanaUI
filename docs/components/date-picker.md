# DatePicker

`DatePicker` 是月历网格。

它由现有控件拼成：表头两个图标按钮加标题，下面 6×7 个日期按钮。

`assemble_date_picker` 建好这些按钮并复用它们，翻月只换标签，不重建。

你可以把它放进表单，或放进你自己的 `Popover`。

`DatePicker::new(cursor)` 显示 `cursor` 所在的月。

`DatePicker::selected(value)` 同时把这一天标成选中，并打开它所在的月。

日期类型是 `nana_ui_core::CivilDate`，只有年月日，用 `CivilDate::new(year, month, day)` 得到 `Option`。

它不是日期时间库。

`.value` 是选中日，`.week_start` 决定一周从哪天起，`.range(minimum, maximum)` 是闭区间，区间外和非本月的日期不可选。

`.disabled` 让整天都不可选。

`.size` 传给表头图标和日期按钮。

月份标题由你给 `month_label`。

月名跟 locale 有关，框架不带 locale 数据。

选中发 `DateChanged { date }`，翻月发 `DateCursorMoved { year, month }`，选中值不变。

`shift_month` 只移动 `cursor`。

控件表里没有 `<DatePicker>`。

视图提交时由 `slot_assembler` 调用 `assemble_date_picker`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::DatePicker;
use nana_ui_core::CivilDate;

let cursor = CivilDate::new(2026, 9, 1).expect("日期合法");
view! {
    <Widget
        of={DatePicker::new(cursor)
            .month_label("2026 年 9 月")
            .range(Some(cursor), CivilDate::new(2026, 9, 30))}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::DatePicker;
use nana_ui_core::CivilDate;

let cursor = CivilDate::new(2026, 9, 1).expect("日期合法");
widget(
    DatePicker::new(cursor)
        .month_label("2026 年 9 月")
        .range(Some(cursor), CivilDate::new(2026, 9, 30)),
)
```

:::

你在 `DateCursorMoved` 里换掉 `month_label`。

选中日存在 `value` 里；要把它当成业务日期，仍由你决定。

`assemble_date_picker` 是 `slot_assembler`：视图提交时，以及响应式补丁之后，它会跑。

手工 `create_component` 放好之后要自己调一次。

改 `cursor` 或 `month_label` 不会在每次字段写入时自动重装这 6×7 个按钮。

`CivilDate::new` 对不存在的月日返回 `None`。

`range` 的两端都是 `Option`，缺的那一侧不设界。

区间外和非本月的格子不可选，翻月只换标签。

[总览](index.md) · [控件合同](../reference/components.md)
