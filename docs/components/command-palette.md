# CommandPalette

`CommandPalette` 只呈现你提供的、当前上下文里可用的动作。真正的 dispatch，以及快捷键的存盘，由你做。无障碍角色是对话框，所以它走 `activate_overlay`。

控件表里没有 `<CommandPalette>`。

## 基本用法

`CommandPalette::new(title, items)` 的条目是 `nana_ui::runtime::CommandPaletteItem`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{CommandPalette, CommandPaletteItem};

view! {
    <Widget
        of={CommandPalette::new(
            "命令",
            [CommandPaletteItem::new("file.save", "保存").shortcut("⌘S")],
        )}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{CommandPalette, CommandPaletteItem};

widget(CommandPalette::new(
    "命令",
    [CommandPaletteItem::new("file.save", "保存").shortcut("⌘S")],
))
```

:::

## 条目

`CommandPaletteItem::new(action, label)` 的 `action` 是稳定的 `ActionId`。

`.shortcut` 是显示用的快捷键文本，`.category` 是分组名。

## 过滤

默认占位是「搜索操作」，空结果是「没有可用操作」，用 `.placeholder` 和 `.empty_label` 替换。

控件按子串过滤，除非 `.filtered_items(true)`：那表示条目已经由你滤好、排好，调色板不再滤第二遍。

改查询仍会发出 `Search`。

## 查询与导航

`.query(...)` 和 `set_query` 同时改查询字符串和输入状态，并把选中下标收成 0；读取当前值使用 `query_text()`。

`set_query` 返回 `CommandPaletteEvent::Search`。

`navigate` 接收 `ActionPickerNavigation`：`Previous`、`Next`、`First`、`Last`、`Confirm`、`Dismiss`。

确认读当前可见项，发出 `Select(action)`。

没有可见项时方向导航不发事件。

## 打开

打开时，调色板是 `OverlayHost` 的直接子节点，调用 `activate_overlay(host, palette)`。

关上用 `dismiss_overlay(host)`，或在 `Dismiss` 里自己收。

`ActionRegistry` 登记动作和快捷键上下文；这份面板只负责把你交来的列表显示出来。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `CommandPalette::new(title, items)` | — | 条目是 `nana_ui::runtime::CommandPaletteItem` |
| `CommandPaletteItem::new(action, label)` | — | `action` 是稳定的 `ActionId` |
| `.shortcut` | — | 显示用的快捷键文本 |
| `.category` | — | 分组名 |
| `.placeholder` | — | 默认占位是「搜索操作」 |
| `.empty_label` | — | 默认空结果是「没有可用操作」 |
| `.filtered_items` | — | `.filtered_items(true)` 表示条目已经由你滤好、排好，调色板不再滤第二遍 |
| `.query(...)` | — | 和 `set_query` 同时改查询字符串和输入状态，并把选中下标收成 0；`query_text()` 读取当前值 |
| `set_query` | — | 返回 `CommandPaletteEvent::Search` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `CommandPaletteEvent::Search` | — | 改查询仍会发出 `Search`。`set_query` 返回这个事件 |
| `Select(action)` | — | 确认读当前可见项，发出 `Select(action)` |
| `Dismiss` | — | `navigate` 接收的 `ActionPickerNavigation` 里有 `Dismiss`。关上用 `dismiss_overlay(host)`，或在 `Dismiss` 里自己收 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
