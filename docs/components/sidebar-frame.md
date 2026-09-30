# SidebarFrame

`SidebarFrame` 是导航侧栏的外框：上面一条固定的顶，中间一块自己滚动的正文，下面一条固定的底。链接和选中项由你放进正文，NanaUI 不内置产品导航。

## 基本用法

`SidebarFrame::new()` 三个槽都是空的。控件表里没有 `<SidebarFrame>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::SidebarFrame;

view! {
    <Widget of={SidebarFrame::new()}>
        <template #top>
            <Text>"工程"</Text>
        </template>
        <template #body>
            <Text>"文件"</Text>
        </template>
        <template #footer>
            <Text>"设置"</Text>
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::SidebarFrame;

widget(SidebarFrame::new())
    .top(text("工程"))
    .body(text("文件"))
    .footer(text("设置"))
```

:::

正文里再放 `SidebarSection` 和 `SidebarRow`。选中哪一行、点了之后去哪，写在你的状态里。

## 滚动

视图上 `.top(view)`、`.body(view)`、`.footer(view)` 按这个顺序成为子节点。`.body` 会先把内容包进 `SidebarFrame::vertical_body_scroll()`，那是一条纵向的 `ScrollView`。

投影时正文借出滚动口，不再在框自己的视觉上盖一条滚动条。框没有单独的 `assemble_*`。头尾和正文就是它的子节点。滚动偏移留在那条 `ScrollView` 上。

`vertical_body_scroll` 是一条纵向 `ScrollView`。视图的 `.body` 用它包住你给的内容，再放进框的正文槽。你自己拿 `SidebarFrame::scroll_body` 时，可以带上已经准备好的 `LayoutStyle`。

长列表只滚中间这一段，顶栏和底栏留在视口里。

## 投影

框的投影会清掉自己的文本和标准视觉，把滚动条留给正文那条 `ScrollView`。`ALWAYS_REPROJECT` 开着，因为正文节点还要被滚动口再投影一次。

## 属性

框的间距字段是 `gap`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `gap` | — | 框的间距 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 选中哪一行、点了之后去哪，写在你的状态里 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.top` | 固定的顶。`new()` 时为空。模板 `#top` |
| `.body` | 自己滚动的正文。先包进 `vertical_body_scroll()`，一条纵向 `ScrollView`。模板 `#body` |
| `.footer` | 固定的底。模板 `#footer` |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
