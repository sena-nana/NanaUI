# ActionMenu

`ActionMenu` 是绑在触发器上的动作菜单。它里面是一个 `Popover`，对齐改成起始边，宽度、间距和内边距用菜单自己的默认值。开关跟 popover 一样，写在 `open` 上，由控件自己切换。

控件表里没有 `<ActionMenu>`：模板里的控件是叶子，菜单的条目却是它的子节点。条目 `<ActionMenuItem>` 在控件表里（`label`、`accessible_name`、`disabled`、`danger`、`active`，`@activate`）。Rust 写法有 `action_menu(label)`，触发器文字、可访问名和开关是 `fields::action_menu` 的 `label`、`accessible_name`、`open`；`label` 写成空字符串时菜单关上，触发器也不显示。

## 基本用法

`ActionMenu::new()` 没有标题参数。文本触发器用 `ActionMenu::trigger`，图标用 `trigger_icon`。视图上，条目用 `.children` 放进菜单。

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

## 条目随数据变

条目可以来自带 key 的列表：`.children(..)` 里放 `each(..)`（或 `Store` 的 `keyed(..).each(..)`、`when(..)`），和固定的条目放在一起。条目数没有上限，行增删时已有的条目不重建。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ActionMenu;

view! {
    <Widget of={ActionMenu::new().trigger("预设")}>
        <ActionMenuItem v-for={p in presets} key={p.id} @activate={apply(p.id)}>{p.name.clone()}</ActionMenuItem>
        <ActionMenuItem @activate={manage()}>"管理预设…"</ActionMenuItem>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{action_menu, action_menu_item, each};

action_menu("预设").children((
    each(presets, |p| p.id, |p| action_menu_item(p.name.clone()).on_activate(move || apply(p.id))),
    action_menu_item("管理预设…").on_activate(manage),
))
```

:::

- 列表的那一列算一组条目：组里的条目和菜单自己的条目同样间距，空的列表不占位置，也不多出间距。
- 激活、禁用、可访问名都是条目自己的：行里用绑定（`.disabled(..)`、`.accessible_name(..)`）跟着数据变，不靠重建。
- 打开时，方向键上下、Home、End 在可用的条目之间移动焦点（跳过禁用和隐藏的，两头循环），焦点在触发器上时也一样；Tab 照常。行在打开时插进来，焦点留在原来的条目上；有焦点的条目被删掉，焦点落到接替它位置的条目（没有了就是最后一条，再没有就回到触发器）。`AppContext::action_menu_items(menu)` 按顺序给出菜单的全部条目。

## 触发器

`.trigger(text)` 是文本触发器。

`.trigger_icon(icon, label)` 是 28×28 的图标按钮，可访问名用 `label`。

视图上的 `.trigger(view)` 和 popover 一样，只做显示，里面不要放可按的控件。

## 条目

条目是 `ActionMenuItem::new(label)`。

`.hint` 是一行补充，空字符串会被丢掉。

`.leading` 放一个图标。

`.danger(true)` 走危险色。

`.disabled` 禁掉这一项。

`.active` 标出当前项。

## 位置

`.placement` 和 `.width` 传给内部的 popover。

菜单表面和 popover 一样钉在触发器显示出来的位置上，靠近视口边缘时翻面再收进去。

打开后滚动页面，表面留在原处，直到下一次布局。

## 开关

`.open` 直接写出开关。

点触发器、Escape、点外面，或把 `open` 写成 `false`，都会关上；焦点还在条目上时回到触发器。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `ActionMenu::new()` | — | 没有标题参数 |
| `.trigger(text)` | — | 文本触发器 |
| `.trigger_icon(icon, label)` | — | 28×28 的图标按钮，可访问名用 `label` |
| 视图上的 `.trigger(view)` | — | 和 popover 一样，只做显示，里面不要放可按的控件 |
| `.placement` | — | 传给内部的 popover |
| `.width` | — | 传给内部的 popover |
| `.open` | — | 直接写出开关。开关跟 popover 一样，由控件自己切换 |
| `ActionMenuItem::new(label)` | — | 条目 |
| `.hint` | — | 一行补充，空字符串会被丢掉 |
| `.leading` | — | 放一个图标 |
| `.danger` | — | `.danger(true)` 走危险色 |
| `.disabled` | — | 禁掉这一项 |
| `.active` | — | 标出当前项 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 点触发器、Escape、点外面，或把 `open` 写成 `false`，都会关上；焦点还在条目上时回到触发器 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 视图上，条目用 `.children` 放进菜单 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
