# StatusBar

`StatusBar` 是内容下方的一条横条。它把状态念成 live status：辅助技术在焦点还没移过来时也能听到变化。普通布局盒表达不了这个角色。

## 基本用法

`StatusBar::new()` 默认画表面，顶边一条发丝边框，朝向内容。控件表里没有 `<StatusBar>`。没有装配函数，分支、光标位置、编码这一类读数由你放进子节点。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::StatusBar;

view! {
    <Widget of={StatusBar::new().label("状态")}>
        <Text>"第 1 行"</Text>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::StatusBar;

widget(StatusBar::new().label("状态")).children(text("第 1 行"))
```

:::

读数变了就改子节点的文本。不要为了刷新状态去重装壳。

## 表面

`.label` 是可访问名。`.chrome(false)` 用于父级已经有表面的时候。条本身不接指针。

条是横向排列，子项居中。`chrome` 打开时背景是 `Surface`，发丝线画在顶边，朝向上面的内容，颜色是 `BorderSoft`。左右内边距同样是 `space::MD`。分支名、光标和编码分成几个文本节点放进这条里。

## 和 toast

`DesktopShell` 的 `.status` 槽是 toast 那一层浮层宿主，和这条状态栏不是同一个东西。toast 走 `status` 宿主，这条栏放在工作区底部或你自己的正文下面。

## 角色

角色是 `Status`，文本一变，辅助技术可以在焦点还停在编辑器里时听到。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.label` | — | 可访问名 |
| `.chrome` | — | `new()` 默认画表面，顶边一条发丝边框，朝向内容。`false` 用于父级已经有表面的时候。打开时背景是 `Surface`，发丝线在顶边，颜色是 `BorderSoft`，左右内边距是 `space::MD` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 子节点 | 分支、光标位置、编码这一类读数。没有装配函数。条是横向排列，子项居中 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
