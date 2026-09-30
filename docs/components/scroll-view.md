# ScrollView

`ScrollView` 是滚动容器。

位置的权威是 Runtime 的 `ScrollOffset`，尺寸的权威是 `ScrollMetrics`。

任何改了布局盒、子节点或样式的提交结束时，Runtime 会重新测量，并把偏移夹进范围。

起点在右或下边时，对应轴的偏移为负。

滚动条不另存一份偏移。

`ScrollView::new(axes)` 的 `axes` 是 `ScrollAxes::Horizontal`、`Vertical` 或 `Both`。

对应轴的溢出写成 `Scroll`。

`.label` 是可访问名。

`.follow_end(true)` 让布局公布新的滚动几何之后，竖向停在最新内容；你在读旧内容时把它关掉。

`.scrollbars` 有三种，类型是 `nana_ui_core::ScrollbarVisibility`：

- `AutoHide` 是默认。指针进入容器，或正在拖拽滑块时，滚动条以 overlay 出现，不占布局。
- `Always` 在这一轴能滚时一直画着，并带轨道底。
- `Hidden` 不画滚动条。滚轮和 `scroll_to` 照常。

`overflow: auto` 和 `overflow: scroll` 共用同一份 `ScrollOffset` 和裁剪，不再另画一套滑块。

自定义滚动条铬只属于 `ScrollView`。

控件表里没有 `<ScrollView>`。

节点可点，不可聚焦。

拖拽时只有抓取锚点记在 `ScrollbarDragState` 里，偏移仍在世界上。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ScrollAxes, ScrollView};

view! {
    <Widget
        of={ScrollView::new(ScrollAxes::Vertical)
            .label("日志")
            .follow_end(true)}
    >
        <Column>
            <Text>"最新一行"</Text>
        </Column>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, text, widget};
use nana_ui::runtime::{ScrollAxes, ScrollView};

widget(
    ScrollView::new(ScrollAxes::Vertical)
        .label("日志")
        .follow_end(true),
)
.children(column().children(text("最新一行")))
```

:::

`scrollbars_revealed` 回答当前要不要把滑块画出来。

`SidebarFrame` 的正文借用 `project_scrollport`，避免在自己的视觉上再盖一条滚动条。

布局结束后，监听了 `ScrollViewportChanged` 的节点会收到新的视口宽高，虚拟列表用它收窄窗口。

[总览](index.md) · [控件合同](../reference/components.md)
