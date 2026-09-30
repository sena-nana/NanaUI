# ActionMenu

`ActionMenu` 是绑在触发器上的动作菜单。

它里面是一个 `Popover`，对齐改成起始边，宽度、间距和内边距用菜单自己的默认值。

开关跟 popover 一样，写在 `open` 上，由控件自己切换。

`ActionMenu::new()` 没有标题参数。

`.trigger(text)` 是文本触发器。

`.trigger_icon(icon, label)` 是 28×28 的图标按钮，可访问名用 `label`。

`.placement` 和 `.width` 传给内部的 popover。

`.open` 直接写出开关。

条目是 `ActionMenuItem::new(label)`。

`.hint` 是一行补充，空字符串会被丢掉。

`.leading` 放一个图标。

`.danger(true)` 走危险色。

`.disabled` 禁掉这一项。

`.active` 标出当前项。

视图上，条目用 `.children` 放进菜单。

文本触发器用 `ActionMenu::trigger`，图标用 `trigger_icon`。

视图上的 `.trigger(view)` 和 popover 一样，只做显示，里面不要放可按的控件。

控件表里没有 `<ActionMenu>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ActionMenu, ActionMenuItem};

view! {
    <Widget of={ActionMenu::new().trigger("文件")}>
        <Widget of={ActionMenuItem::new("保存")} />
        <Widget of={ActionMenuItem::new("删除").danger(true)} />
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ActionMenu, ActionMenuItem};

widget(ActionMenu::new().trigger("文件")).children((
    widget(ActionMenuItem::new("保存")),
    widget(ActionMenuItem::new("删除").danger(true)),
))
```

:::

菜单表面和 popover 一样钉在触发器显示出来的位置上，靠近视口边缘时翻面再收进去。

打开后滚动页面，表面留在原处，直到下一次布局。

点触发器、Escape、点外面，或把 `open` 写成 `false`，都会关上；焦点还在条目上时回到触发器。

[总览](index.md) · [控件合同](../reference/components.md)
