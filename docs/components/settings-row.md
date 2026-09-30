# SettingsRow

`SettingsRow` 是设置页里的一行：左边标签和提示，右边你的控件。具体的设置值，例如主题、账号和路径，仍由你的状态拥有。

## 基本用法

控件表里没有 `<SettingsRow>`。外层仍是 `settings_row`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::settings_row;

settings_row("静音").control(view! {
    <Switch checked={false}></Switch>
})
```

```rust rust
use nana_ui::runtime::view::{settings_row, switch};

settings_row("静音").control(switch("").checked(false))
```

:::

## 排列

`SettingsRow::new(label)` 默认横排，不按宽度折行。`.hint` 写在标签下面。`.stacked(true)` 始终上下排列。

`.stack_below(width)` 按这一行实际布局宽度折行：窄于阈值时上下排，等于或宽于阈值时同行。无效阈值视为没开。控件节点不重建，宽度变化由布局回流处理。参考里的例子用 `480.0` 作为应用自己的阈值。

## 视图函数的阈值

视图函数 `settings_row(label)` 先 `SettingsRow::new("")`，再 `stack_below(280.0)`，然后把标签写上。280 是装配好的行用的阈值。

## 分组

`.divided(true)` 在行下画一条发丝线，一组的最后一行不要画，组自己的边已经收尾。`.first_in_group` 和 `.last_in_group` 标出组的两端。`.loose` 加大间距。

## 控件名字

`.control(view)` 把控件放在文案后面。控件自己有非空名字时用它自己的，例如 `switch("静音")`；名字是空的，例如 `switch("")`、分段、滑块、下拉，就用行标签。装配通过 `set_labelled_by` 关联两者。行标签改了，控件的无障碍节点跟着重新投影。

## 装配

`assemble_settings_row` 是 `slot_assembler`。视图提交时会跑，视图里不要再调。手工放好控件槽之后要自己调一次。

## 提示

`mount_settings_leaf_row` 保留标签、提示和控件槽。一开始没有提示，也可以随后 `update_component` 把 `hint` 写成 `Some`，写成 `None` 就藏起来。

## 工作区

进入设置若用独立的 `WorkspaceController`，不会覆盖主工作区的尺寸和折叠。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | — | `SettingsRow::new(label)`。默认横排，不按宽度折行。视图函数先 `new("")`，再把标签写上 |
| `.hint` | — | 写在标签下面。可以随后 `update_component` 写成 `Some`，写成 `None` 就藏起来 |
| `.stacked` | — | `true` 始终上下排列 |
| `.stack_below` | — | 窄于阈值时上下排，等于或宽于阈值时同行。无效阈值视为没开。视图函数用 `280.0`。参考里的例子用 `480.0` 作为应用自己的阈值 |
| `.divided` | — | `true` 在行下画一条发丝线。一组的最后一行不要画，组自己的边已经收尾 |
| `.first_in_group` | — | 标出组的一端 |
| `.last_in_group` | — | 标出组的一端 |
| `.loose` | — | 加大间距 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.control` | 把控件放在文案后面。`mount_settings_leaf_row` 保留标签、提示和控件槽 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
