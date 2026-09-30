# Dialog

标题加正文和底栏的浮层。只要确认和取消时，用 [ConfirmDialog](confirm-dialog.md)。

::: info
控件表里没有 `<Dialog>` 标签。两种写法都调用 `widget(Dialog::new(title))`，再用 `.body`、`.footer`、`.close_action` 放进已经建好的视图。
:::

## 基本用法

`Dialog::new` 接收标题。还可以接上 `.description`、`.size` 和 `.close_policy`。`.initial_focus_on(entity_ref)` 会在打开时，把焦点放到某个已经放进插槽的控件上。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Dialog;

widget(Dialog::new("重命名"))
    .body(view! {
        <Column gap=8>
            <TextInput placeholder="名称" v-model={name} />
        </Column>
    })
    .footer(view! {
        <Row gap=8>
            <Button @activate={cancel}>"取消"</Button>
            <Button @activate={save}>"保存"</Button>
        </Row>
    })
```

```rust rust
use nana_ui::runtime::view::{button, column, row, text_input, widget};
use nana_ui::runtime::Dialog;

widget(Dialog::new("重命名"))
    .body(
        column().gap(8).children((
            text_input().placeholder("名称").model(name),
        )),
    )
    .footer(
        row().gap(8).children((
            button("取消").on_activate(cancel),
            button("保存").on_activate(save),
        )),
    )
```

:::

插槽在对话框之前就建好。对话框自己的装配会把它们放进标题下面、底栏和关闭位。打开、关闭、逃出键和焦点归还走浮层的约定，见 [控件](../reference/components.md) 的浮层一节。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| 标题 | `Dialog::new` 的参数 | 构造时给出 |
| `.description` | 文本 | 标题下的说明 |
| `.size` | 尺寸 | 对话框尺寸 |
| `.close_policy` | 关闭策略 | 怎样允许关掉 |
| `.initial_focus_on` | `entity_ref` | 打开时把焦点放到已经放进插槽的控件上 |

## 事件

这一页没有单独的事件名。打开、关闭、逃出键和焦点归还走浮层约定。

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.body` | 标题下面的内容。调用前就要建好 |
| `.footer` | 底栏，通常是按钮行 |
| `.close_action` | 关闭位 |

确认框是 `ConfirmDialog`。它的槽位是 `.body`、`.cancel` 和 `.confirm`。你没有提供的按钮，会用 `cancel_label` 或 `confirm_label` 生成。

## 参见

[总览](index.md) · [ConfirmDialog](confirm-dialog.md) · [控件](../reference/components.md)
