# heading_row

`heading_row(title, tools)` 是分组或卡片的标题行：左边是标题，右边是一组标题工具。它是 `view::marks` 里的视图，由 `Stack` 拼成，不是注册控件，没有模板标签。

## 基本用法

```rust
use nana_ui::runtime::view::{heading_row, heading_tools, text, widget};
use nana_ui::runtime::{Icon, IconButton};
use nana_ui::theme::TypeRole;

heading_row(
    text("模型").role(TypeRole::Meta),
    heading_tools((
        widget(IconButton::tool(Icon::Plus, "导入")),
        widget(IconButton::tool(Icon::Refresh, "刷新")),
    )),
)
```

- 标题占剩下的宽度，空间不够时先让出并省略；工具保持自己的宽度。
- `.lead(view)` 在标题前放一个前导元素（卡片图标、返回箭头）：它保持宽度，离标题 `SpacingStep::Sm`，在标题行里竖直居中。有没有前导，标题文字之外的布局都一致。

## 工具悬出

`heading_tools(tools)` 是没有外观、没有竖向内边距的工具栏。工具用 `IconButton::tool(icon, label)`（紧凑，目标 `COMPACT_TARGET`，默认 24×24，标签同时是提示）或紧凑菜单。

紧凑目标比标题的行盒高。工具栏默认**悬出**：上下各留出 `heading_overhang()`（`(COMPACT_TARGET − 行盒) / 2`，默认 4px）的负外边距，不撑高标题行。所以：

- 有工具和没工具的标题行同高，同一页的标题和下面的内容对齐；
- 标题在滚动区顶部时，悬出的上半截会被滚动区裁掉——滚动区要写 `ScrollView::heading_room()`，向上留出同样的距离。

标题行上不是工具、但和工具一样高的内容（名称旁的状态胶囊、紧凑按钮）用 `heading_hang(view)`：同样上下悬出，保持宽度，在行内居中。

悬出量按主题的行盒（`LineRole::Line`）算，对应 `Meta` 到 `Body` 字号的标题。标题本身比目标高时（`Heading`、`Title`），写 `heading_tools(..).inline()`：工具留在行内，行高取两者中较高的。

## 样式

`heading_row` 和 `heading_tools` 都接 `.class` / `.css` / `.visible`，写在各自的行或工具栏上。多行标题从顶端对齐：`heading_row(..).css(css! { align-items: flex-start; })`。

## 什么时候不用

- 推进任务的动作（保存、下一步）用文字按钮，放在内容下面，不放进标题工具。
- 对某一行的操作放在那一行的行尾，不放进标题工具。

## 接口

| 接口 | 说明 |
| --- | --- |
| `heading_row(title, tools)` | 标题行 |
| `HeadingRow::lead(view)` | 标题前的前导元素 |
| `heading_tools(tools)` | 标题工具栏，默认悬出 |
| `HeadingTools::inline()` | 不悬出，行高随工具 |
| `heading_hang(view)` | 把非工具内容挂在标题行上，同样悬出 |
| `COMPACT_TARGET` | 紧凑工具的目标档位（`SpacingStep::Page`） |
| `heading_overhang()` | 悬出量（主题表达式） |

## 参见

[版式与组合](../guide/composition.md) · [ScrollView](scroll-view.md) · [IconButton](icon-button.md) · [总览](index.md)
