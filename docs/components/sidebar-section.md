# SidebarSection

`SidebarSection` 是侧栏里的一节：标题、可选的计数、悬停时出现的工具，以及一截正文。正文里的行由你给。

## 基本用法

控件表里没有 `<SidebarSection>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Icon, SidebarSection};

view! {
    <Widget
        of={SidebarSection::new("资源")
            .count(2)
            .collapsible(true)
            .empty_text("还没有资源")}
    >
        <template #tools>
            <IconButton icon={Icon::Add}>"新建"</IconButton>
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{icon_button, widget};
use nana_ui::runtime::{Icon, SidebarSection};

widget(
    SidebarSection::new("资源")
        .count(2)
        .collapsible(true)
        .empty_text("还没有资源"),
)
.tools(icon_button(Icon::Add, "新建"))
```

:::

## 开合

`SidebarSection::new(title)` 默认展开，不可折叠，尺寸小号。`.collapsible(true)` 允许收起。`.expanded` 直接写出开合，并把动画进度设成 0 或 1。

点击标题时的展开由 `activate_sidebar_section` 处理。

## 计数和空文案

`.count` 在标题旁显示数量。`.empty_text` 是这一节没有行时的说明。空文案只在没有行时出现。

## 停用

`.disabled` 停掉这一节。

## 工具

`.tools(id)` 是标题栏悬停时替换计数的那块工具。视图上 `.tools(view)` 放进标题，子节点进正文。

## 装配

`assemble_sidebar_section` 是 `slot_assembler`，它建出标题、计数、披露按钮和正文口，并把子节点移进正文。视图提交时会跑。手工放好之后要自己调一次。每次改标题不会自动重装。

## 行的选中

行的选中不在这一节上，写在下面的 `SidebarRow`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `title` | — | `SidebarSection::new(title)`。默认展开，不可折叠，尺寸小号。每次改标题不会自动重装 |
| `.count` | — | 标题旁显示数量。悬停工具会替换计数 |
| `.collapsible` | — | `true` 允许收起。默认不可折叠 |
| `.expanded` | — | 直接写出开合，并把动画进度设成 0 或 1 |
| `.empty_text` | — | 这一节没有行时的说明。空文案只在没有行时出现 |
| `.disabled` | — | 停掉这一节 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 点击标题时的展开由 `activate_sidebar_section` 处理。行的选中写在 `SidebarRow` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.tools` | 标题栏悬停时替换计数。视图上放进标题。模板 `#tools` |
| 正文 | 子节点进正文。装配建出正文口，并把子节点移进去 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
