# EmptyState

`empty_state(title)` 创建一块空状态。`title` 是构造参数，字段类型是 `Arc<str>`。这是一块说明，不是列表项，也不是徽章。

模板里的标签是 `<EmptyState>`。子文本和 `title` 都写成这一个字段。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Icon;

view! {
    <EmptyState message="把文件拖到这里" icon={Icon::Folder}>
        "还没有文件"
        <template #action>
            <Button @activate={import}>"导入"</Button>
        </template>
    </EmptyState>
}
```

```rust rust
use nana_ui::runtime::view::{button, empty_state};
use nana_ui::runtime::Icon;

empty_state("还没有文件")
    .message("把文件拖到这里")
    .icon(Icon::Folder)
    .action(button("导入").on_activate(import))
```

:::

图标用目录常量，例如 `Icon::Folder`。不需要图标就不要写 `icon`。`message` 可以不给。

## 紧凑

`compact` 默认是假：内容横轴居中。`compact(true)` 改成靠起始边对齐。宽度仍是父级的百分之百。

## 空列表

列表完全为空时，往往和 `v-if` 一起用；跟着数据出现或消失的写法见 [视图写法](../guide/essentials/view.md)。

## 属性

字段是 `title: Arc<str>`、`message: Option<Arc<str>>`、`icon: Option<Icon>`、`compact: bool`。没有事件，也没有 `model`。标题下面可以有一个动作，用 `.action(…)`，不是字段。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `title` | `Arc<str>` | 构造参数。子文本和 `title` 都写成这一个字段 |
| `message` | `Option<Arc<str>>` | 可以不给 |
| `icon` | `Option<Icon>` | 目录常量，例如 `Icon::Folder`。不需要就不要写 |
| `compact` | `bool` | 默认是假：内容横轴居中。`compact(true)` 靠起始边对齐。宽度仍是父级的百分之百 |

## 事件

没有事件。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 动作本身的点击写在那颗按钮上，空状态不另发事件 |

## 插槽

`.action` 放标题和说明下面的那一个动作，例如重试按钮。模板里是 `<template #action>`。这个槽要恰好一个根节点。动作本身的点击写在那颗按钮上，空状态不另发事件。

| 插槽 | 说明 |
| --- | --- |
| `#action` / `.action` | 标题和说明下面的那一个动作，不是字段。要恰好一个根节点 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
