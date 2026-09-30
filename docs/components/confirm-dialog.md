# ConfirmDialog

`ConfirmDialog` 负责问一次确认。

无障碍角色是警报对话框。

标题和说明由你给，确认和取消按钮由装配建好，并接上 `ConfirmIntent`。

`ConfirmDialog::new(title, message)` 的默认按钮文案是「确认」和「取消」。

`.confirm_label` 和 `.cancel_label` 换成你的话。

字段 `danger` 为真时，确认按钮走危险色。

这是字段，不是 `.danger(...)` 方法。

`.size` 用 `nana_ui::DialogSize`：`Compact`、`Default`、`Medium`、`Wide`、`Workspace`。

`.close_policy` 决定 Escape 和点外面能不能关。

`.initial_focus` 用 `ModalInitialFocus`，默认落在第一个动作上。

`AppContext::assemble_confirm_dialog` 在确认或取消槽还空着的时候建按钮。

你已经用 `set_confirm_slots` 装过槽，它不会覆盖。

视图上的槽是 `.body`、`.close_action`、`.cancel`、`.secondary`、`.confirm`。

这是 `slot_assembler`：视图提交时会跑。

手工 `create_component` 放好槽之后要自己调一次，字段写入不会每次都重装。

打开用 `AppContext::activate_overlay(host, dialog)`。

对话框必须是这个 `OverlayHost` 的直接子节点。

关上用 `dismiss_overlay(host)`。

没有 `<ConfirmDialog>`，也没有 `open()`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ConfirmDialog;

view! {
    <Widget
        of={ConfirmDialog::new("删除项目", "删除后不能恢复。")
            .confirm_label("删除")
            .cancel_label("留下")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::ConfirmDialog;

widget(
    ConfirmDialog::new("删除项目", "删除后不能恢复。")
        .confirm_label("删除")
        .cancel_label("留下"),
)
```

:::

`busy` 为真时，框架按忙碌处理这次确认。

次要动作放进 `.secondary`。

正文不止一句话时，用 `.body` 放你的内容，装配不会拿默认正文换掉它。

[总览](index.md) · [控件合同](../reference/components.md)
