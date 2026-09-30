# FormField

`FormField` 负责把标签、控件、提示和错误排成一块字段。

控件是你的子节点。

字段自己不接指针，也不能聚焦。

`FormField::new(label)` 的标签会成为可访问名。

`.hint` 是补充说明，`.error` 是错误文案。

两者都有时，显示和可访问值用错误。

有错误时无障碍状态标 `invalid`。

`.size` 默认中号，决定标签字号和间距。

`.style` 整份替换样式。

显示层是三份分开的东西：控件自己的 `invalid`，字段的 `error`，以及单独的 `ValidationMessage`。

提交前要问整张表还有没有没填对的，用 `AppContext::validity_of(root)`。

它按文档顺序返回子树里已经标了 `invalid`、并且没有禁用的字段。

视图上用 `.control(view)` 把控件放在标签下面、说明上面。

这个槽在 `El<FormField>` 上，提交时写成 `control_child`。

控件表里没有 `<FormField>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::FormField;

view! {
    <Widget of={FormField::new("显示名").hint("将显示在标题栏")}>
        <template #control>
            <TextInput />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text_input, widget};
use nana_ui::runtime::FormField;

widget(FormField::new("显示名").hint("将显示在标题栏")).control(text_input())
```

:::

要标出错误，把 `.error` 写成文案，并让里面的控件自己的 `invalid` 一并为真。

字段的布局是纵向，宽度百分之百。

标签行高按 `ControlSize` 预留，避免换行时盖住控件。

[总览](index.md) · [控件合同](../reference/components.md)
