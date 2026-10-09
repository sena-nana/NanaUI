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

## 由应用决定开合

用户想关掉对话框有三种手势：按 Escape、在对话框外按下再松开、激活关闭位（`.close_action`）。每一种都先在对话框自己身上发一次 `DialogCloseRequested`，`trigger` 说是哪一种。然后才看 `.close_policy`：允许这个手势，框架接着关掉它，宿主随后发 `OverlayClosing`；不允许，它留着。Escape 和点外面只送到挂在 `OverlayHost` 下、用 `activate_overlay` 打开的对话框（浮层约定）；关闭位挂没挂都会发请求。

`DialogClosePolicy::requests_only()` 一种手势都不让框架关。开合全由应用决定：处理函数里要关，就改应用自己的打开状态（对话框随视图卸载），或者调 `dismiss_overlay(host)` 走退出动画；不关就什么都不做，比如先问一句有没有没保存的修改。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dialog, DialogCloseRequested};
use nana_ui::DialogClosePolicy;

widget(Dialog::new("重命名").close_policy(DialogClosePolicy::requests_only()))
    .on(move |_: &DialogCloseRequested| {
        if !dirty.get() {
            open.set(false);
        }
    })
    .body(view! {
        <TextInput placeholder="名称" v-model={name} />
    })
```

```rust rust
use nana_ui::runtime::view::{text_input, widget};
use nana_ui::runtime::{Dialog, DialogCloseRequested};
use nana_ui::DialogClosePolicy;

widget(Dialog::new("重命名").close_policy(DialogClosePolicy::requests_only()))
    .on(move |_: &DialogCloseRequested| {
        if !dirty.get() {
            open.set(false);
        }
    })
    .body(text_input().placeholder("名称").model(name))
```

:::

以前不允许的手势被悄悄吞掉：Escape 不再传给 `on_key`，点外面也没有事件，应用只能事后看对话框还在不在。现在不必轮询，听这个请求就行。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| 标题 | `Dialog::new` 的参数 | 构造时给出 |
| `.description` | 文本 | 标题下的说明 |
| `.size` | 尺寸 | 对话框尺寸 |
| `.close_policy` | 关闭策略 | 哪些手势由框架直接关掉。`DialogClosePolicy::requests_only()` 一种都不关，全交给应用 |
| `.initial_focus_on` | `entity_ref` | 打开时把焦点放到已经放进插槽的控件上 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `.on(\|e: &DialogCloseRequested\| …)`，模板 `on:DialogCloseRequested` | `&DialogCloseRequested` | 用户按 Escape、点外面或激活关闭位，`trigger` 是 `Escape` / `Outside` / `CloseButton`。发在对话框自己身上，先于 `close_policy` |

打开、关闭和焦点归还走浮层约定。框架关掉对话框时，宿主发 `OverlayClosing { root }`。

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.body` | 标题下面的内容。调用前就要建好 |
| `.footer` | 底栏，通常是按钮行 |
| `.close_action` | 关闭位 |

确认框是 `ConfirmDialog`。它的槽位是 `.body`、`.cancel` 和 `.confirm`。你没有提供的按钮，会用 `cancel_label` 或 `confirm_label` 生成。

## 参见

[总览](index.md) · [ConfirmDialog](confirm-dialog.md) · [控件](../reference/components.md)
