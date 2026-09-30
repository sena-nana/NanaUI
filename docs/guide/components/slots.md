# 具名 slot

组合控件按节点收你的内容。slot 是一个方法：先把那段视图建好，再把根节点交给控件。控件表里有标签的，模板用 `<template #名字>`。没有标签的，不要编一个。

空状态有 `<EmptyState>`。`#action` 展开成 `.action`。连字符会变成下划线，`#close-action` 就是 `.close_action`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <EmptyState title="没有任务" message="加一条就开始。">
        <template #action>
            <Button @activate={add}>"添加"</Button>
        </template>
    </EmptyState>
}
```

```rust rust
use nana_ui::runtime::view::{button, empty_state};

empty_state("没有任务")
    .message("加一条就开始。")
    .action(button("添加").on_activate(add))
```

:::

`#default` 就是普通子节点。一个 slot 必须恰好有一个根，否则这次挂载失败，返回 `InvalidInput`，什么也不留下。slot 和控件在同一次提交里建好。里面创建的信号归这个挂载，卸载时一起回收。

slot 的根在视图活着的时候不换。里面的内容要变，就在 slot 里写 `when` / `each` / `dynamic`。

## 对话框没有 `<Dialog>`

`Dialog` 和 `Drawer` 不在控件表里。两种写法都是 `widget(Dialog::new(标题))`，再用 `.body`、`.footer`、`.close_action`。开关换的是这些槽里的界面，不是外层那一行。`Drawer::new(标题)` 的三个方法相同。

:::api

```rust view
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
    .close_action(view! {
        <Button @activate={cancel}>"关闭"</Button>
    })
```

```rust rust
use nana_ui::runtime::view::{button, column, row, text_input, widget};
use nana_ui::runtime::Dialog;

widget(Dialog::new("重命名"))
    .body(column().gap(8).children((
        text_input().placeholder("名称").model(name),
    )))
    .footer(row().gap(8).children((
        button("取消").on_activate(cancel),
        button("保存").on_activate(save),
    )))
    .close_action(button("关闭").on_activate(cancel))
```

:::

槽在面板之前建好，所以 `.initial_focus_on(entity_ref)` 能在打开时聚焦槽里的控件。目标还没建出来时，保持面板自己的焦点。`Drawer` 和 `ConfirmDialog` 同样有这个方法。

`ConfirmDialog::new(标题, 说明)` 的槽是 `.body`、`.close_action`、`.cancel`、`.secondary`、`.confirm`。没给的取消和确认按钮，用 `cancel_label` / `confirm_label` 生成，默认是「取消」和「确认」。

## 表单、播放条和菜单

这些类型也没有自己的模板标签。模板一侧外层是 `<Widget of={...}>`，具名插槽是 `<template #名字>`。不要编 `<FormField>` 这种标签。

| 控件 | 方法 | 放什么 |
| --- | --- | --- |
| `FormField::new(标签)` | `.control` | 标签下面、说明上面的那个控件 |
| `MediaTransportBar::new()` | `.leading` | 播放后面：下一个、跳过 |
| | `.trailing` | 音量前面：速度、清晰度、字幕 |
| | `.secondary` | 进度条下面的第二行，空着会收起 |
| | `.settings` | 设置菜单里的项 |
| `Popover::new()` | `.trigger` | 触发器上的图标、文字、计数 |
| `ActionMenu::new()` | `.trigger` | 同上 |

`Popover` 和 `ActionMenu` 自己仍是那个控件：按下、焦点、Enter / Space 和锚点都属于它。触发器里不要再放一个会按下的控件。`.children(..)` 才是面板上的条目。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::FormField;

view! {
    <Widget of={FormField::new("名称")}>
        <template #control>
            <TextInput label="名称" />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, text_input, widget};
use nana_ui::runtime::FormField;

widget(FormField::new("名称")).control(text_input().label("名称"))
```

:::

壳层的区域（`DesktopShell`、`Workspace` 等）也是具名 slot，方法名见 [声明式视图](../../reference/reactive-view.md) 的 slot 一节。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/async">
    <p class="next-step-link">异步</p>
    <p class="next-step-caption">resource 怎么加载，第一次完成前显示什么。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">分支和列表行怎么进出场。</p>
  </a>
  <a class="next-step" href="/components/dialog">
    <p class="next-step-link">Dialog</p>
    <p class="next-step-caption">标题、尺寸和关闭策略。</p>
  </a>
</div>
