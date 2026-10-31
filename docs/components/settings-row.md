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

`.divided(true)` 在行下画一条发丝线，一组的最后一行不要画，组自己的边已经收尾。放在 `SettingsGroup` 里、紧跟在别的内容（键值、列表、提示）之后时，组还会让它在行上也画一条，设置行与别的内容的交界两边都有线。`.first_in_group` 和 `.last_in_group` 标出组的两端。`.loose` 加大间距。

哪一行是两端，取决于此刻显示了哪些行。把行放进 `SettingsGroup`（`widget(SettingsGroup::new()).children(..)`）由它来定：成员的显示或隐藏一变，分组就把第一个、最后一个显示的成员找出来，写到直接放在组里的 `SettingsRow` 的 `first_in_group` / `last_in_group` 上，组里的行不要再自己绑这两个字段。其他成员（徽章、数值、一排按钮）也占位置，只是不被写。带 key 的列表和 `when` / `dynamic` 自己的容器会被看穿，里面的每一行都算成员。组里的 `Divider` 是分隔线：只在两个显示的成员之间显示。

## 控件名字

`.control(view)` 把控件放在文案后面。控件自己有非空名字时用它自己的，例如 `switch("静音")`；名字是空的，例如 `switch("")`、分段、滑块、下拉，就用行标签。装配通过 `set_labelled_by` 关联两者。行标签改了，控件的无障碍节点跟着重新投影。

## 标签旁的附件

`.label_accessory(view)` 把一个小东西放在标签后面、和标签同一行，例如状态胶囊：「水印去除 [尚未购买]」。

```rust
settings_row("水印去除")
    .label_accessory(status_badge("尚未购买"))
    .control(button("购买"))
```

附件紧跟在标签后面，隔一个 `space::XS`，和标签上下居中。行变窄时标签先让：它收窄并省略，附件保持自己的宽度。横排和上下排都一样。行的无障碍名字还是标签，附件按它自己的内容读。

没有附件的行和以前一样：标签直接在文案里，不多一层节点。附件只在有的时候才给标签套一条行。附件的根换成 `when(..)` 就能随状态显隐；手工建的行把 `label_accessory` 写成 `None` 再 `assemble_settings_row`，标签回到文案里，那条行一起拿掉。

## 标签列（检查器排法）

`.label_column(Some(width))`（手工建行用 `SettingsRow::label_column(width)`）把行改成检查器那种排法：标签占一列固定宽度，控件从列的右边缘开始长到行尾，行只有控件那么高，上下不留带。几行用同一个宽度，控件就在同一条竖线上对齐，跨卡片也一样。这一列里的标签是行里次要的一半：`Muted` 色、常规字重，数值读起来更醒目。标签比列长时省略。

```rust
settings_row("不透明度")
    .label_column(Some(96.0))
    .control(scrub)
    .trailing(button("恢复默认"))
```

控件要铺满就自己设宽度为 `Fill`；不设就按控件自己的宽度，从列边缘开始放。`.stack_below` 照常生效：行窄于阈值时控件放到标签下面并铺满，两者之间只隔 `space::XXS`。`None` 就是原来的设置页排法。

`.trailing(view)`（手工建行用 `trailing_child`）把行自己的操作放在控件后面、同一行，隔 `space::XXS`。放得下时它保持自己要的宽度，控件让出其余部分；放不下时尾部节点让，控件至少保留三个控件高度（安装主题的 `ControlSize::Medium`，默认主题 96 px，紧凑主题 72 px），所以尾部给的是一个要占满宽度的容器（例如普通的 `Stack::row`）也挤不没控件。有标签列或有尾部节点时，控件和尾部节点放进同一个盒子；有尾部节点时两者又各在一个盒子里（`settings-row-value` / `settings-row-trailing`）。都拿掉以后，控件回到行上，盒子一起拿掉。行标签照旧给控件命名。

## 闪一下

用户从别处跳来看某一行时，应用把页面滚到这一行，再 `AppContext::flash_settings_row(row)` 让它闪一下：行下面出现一块强调色淡底，停一会儿再淡出，结束后什么都不留。

```rust
let row = entity_ref::<SettingsRow>();
// 视图里：settings_row("水印去除").entity_ref(row)
if let Some(row) = row.get() {
    cx.flash_settings_row(row)?;
}
```

淡底用主题的 `Selected`（选中项的不透明浅色底）和 `Sm` 圆角；`AccentSoft` 在半透明的主题里叠在分组浅底上几乎看不出来。行自己没有左右内边距，所以淡底左右各外扩一个列表项的内边距（`ControlPadding::ListItem`），和选中的列表行一样不贴着字；上下盖满这一行。它是行里一个不参与排版、不接指针、读屏看不见的节点，不改行的布局和命中。

动画在表现层上改它的不透明度，从调用之后的下一帧开始（滚过去的那一帧也就是它出现的那一帧）。总时长读安装主题的 `MotionRole::Attention`（默认 1.5 秒）：前 `MotionTokens::attention_hold`（默认 0.2，即五分之一）保持全色，之后按主题的标准缓动淡出。减少动态效果（`reduced_motion`）时不淡出，全色停留同样的时长后直接拿掉。还在闪时再调一次就从头开始。行先被移除时淡底和它的时间线一起走。

## 装配

`assemble_settings_row` 是 `slot_assembler`。视图提交时会跑，视图里不要再调。手工放好控件槽之后要自己调一次。

## 提示

`mount_settings_leaf_row` 保留标签、提示和控件槽。一开始没有提示，也可以随后 `update_component` 把 `hint` 写成 `Some`，写成 `None` 就藏起来。

## 错误

`.error(reason)` 说这一行的值哪里不对：必填项为空、端口被占用、身份码不正确。原因占提示的位置，画成 Danger 的 [ValidationMessage](validation-message.md)，提示先让开；写回 `None` 后提示回来。原因就在用户正看的那一行，有提示的行高度不变。

行本身读作 invalid，读屏的值是原因。控件自己的红框由控件的 `invalid` 管，行只说为什么：两样一起写。

原因的槽在第一次有错误时建好，之后错误清掉也留着（隐藏），下一次出错不再重建。

## 工作区

进入设置若用独立的 `WorkspaceController`，不会覆盖主工作区的尺寸和折叠。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | — | `SettingsRow::new(label)`。默认横排，不按宽度折行。视图函数先 `new("")`，再把标签写上 |
| `.hint` | — | 写在标签下面。可以随后 `update_component` 写成 `Some`，写成 `None` 就藏起来 |
| `.error` | `Option<Arc<str>>` | 值哪里不对。设了就占提示的位置，画成 Danger 的 `ValidationMessage`，行读作 invalid；`None` 让提示回来。视图里可绑定 |
| `.stacked` | — | `true` 始终上下排列 |
| `.stack_below` | — | 窄于阈值时上下排，等于或宽于阈值时同行。无效阈值视为没开。视图函数用 `280.0`。参考里的例子用 `480.0` 作为应用自己的阈值 |
| `.divided` | — | `true` 在行下画一条发丝线。一组的最后一行不要画，组自己的边已经收尾 |
| `.first_in_group` | — | 标出组的一端 |
| `.last_in_group` | — | 标出组的一端 |
| `.loose` | — | 加大间距 |
| `label_accessory` | — | 标签后面同一行的附件。视图里用 `.label_accessory(view)` 插槽 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.control` | 把控件放在文案后面。`mount_settings_leaf_row` 保留标签、提示和控件槽 |
| `.label_accessory` | 放在标签后面、同一行。标签先省略，附件保持宽度 |

## 方法

| 方法 | 说明 |
| --- | --- |
| `AppContext::flash_settings_row(row)` | 让这一行闪一下：强调色淡底停留后淡出，结束后不留痕迹。再调一次从头开始 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
