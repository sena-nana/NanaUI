# ScrollView

`ScrollView` 是滚动容器。位置的权威是 Runtime 的 `ScrollOffset`，尺寸的权威是 `ScrollMetrics`。任何改了布局盒、子节点或样式的提交结束时，Runtime 会重新测量，并把偏移夹进范围。

## 基本用法

控件表里没有 `<ScrollView>`。节点可点，不可聚焦。

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

## 偏移

起点在右或下边时，对应轴的偏移为负。滚动条不另存一份偏移。

## 轴

`ScrollView::new(axes)` 的 `axes` 是 `ScrollAxes::Horizontal`、`Vertical` 或 `Both`。对应轴的溢出写成 `Scroll`。

## 平滑滚轮

普通鼠标的行滚轮默认以 180 毫秒的 ease-out cubic 过渡移动；连续同向滚动累加目标，反向滚动从当前可见位置立即转向。横向、负偏移和嵌套容器共用这份行为：目标到达边界后，后续滚轮向外层传递。

触控板等像素滚动保持系统提供的位移，不追加过渡。拖动滚动条、`scroll_to` / `scroll_by`、焦点滚入、锚点恢复和无障碍滚动立即完成，并打断该容器正在进行的滚轮过渡。减少动态效果时，正在进行的过渡直接到目标，后续滚轮立即移动。

`ScrollOffset` 始终是当前有效位置；`ScrollChanged` 和 `UserScroll` 随实际移动报告当前偏移，不提前报告目标。内容范围缩小时位置与目标都夹进新范围。过渡结束后不保留滚动 deadline。

## 滚动条跑在边距里

`.gutter_theme(档位)` 让滚动区向左右各伸出这段距离、再把内容缩回同样的距离：内容仍在容器的内容线上，滚动区和它边上的滚动条伸进容器的边距，滚动条不压在内容上，边距也能滚动列表。边距放不下主题的滚动条车道时车道收窄。外边距、内容缩进和车道能占的宽度都按安装的主题解析，换主题时一起跟着变。容器的水平内边距至少要有这么宽；盖在上面的 `scroll_foot_fade` 写同一个值的 `.clear_of_theme(档位)`。

在 Card 或设置组的卡片里，写 `.gutter_theme(ThemeToken::PanelPaddingX)`：这个档位就是卡片自己的水平内边距（`ThemeMetrics::panel_padding_x`，默认 16，`COMPACT_METRICS` 下 10），滚动区和滚动条在任何度量下都正好伸到卡片边上。

`.gutter(px)` 是像素版，只在写它的那套主题下成立：`gutter(16.0)` 在默认主题下和 `gutter_theme(ThemeToken::PanelPaddingX)` 一样，换到 COMPACT 就比卡片内边距多伸出 6。后调用的会替换先调用的，不论哪一种。

## 给标题工具留空

`.heading_room()` 让滚动区向上伸出 `heading_overhang()`、再把内容缩回同样的距离：顶部标题行悬出的工具不被滚动区裁掉。见 [heading_row](heading-row.md)。

## 跟随末尾

`.follow_end(true)` 让布局公布新的滚动几何之后，竖向停在最新内容；你在读旧内容时把它关掉。

## 滚动条

滚动条是 overlay：哪种模式都不占布局，也不改变任何盒子。

### 显隐

`.scrollbars` 不写时跟随主题的策略（`ScrollbarRecipe::visibility`，内置是 `AutoHide`）；写了就是这个滚动区自己的，主题策略不覆盖它。类型是 `nana_ui_core::ScrollbarVisibility`：

- `AutoHide`：指针在容器里（也包括在它里面嵌套的滚动区里）、用户刚滚动过（滚轮、拖动、触控、键盘焦点滚入、无障碍滚动）、车道激活或正在拖动时显示。这些都结束后，再保持主题的 `hide_delay`，然后淡出。程序调用 `scroll_to` 和 `follow_end` 不会让它出现。
- `Always`：这一轴能滚就一直画着，带常驻轨道。
- `Hidden`：不画滚动条。滚轮和 `scroll_to` 照常。

应用要让所有跟随策略的滚动区常驻，用 `ThemeDefinition::with_scrollbar_visibility(ScrollbarVisibility::Always)`。

### 几何

车道是悬停与按下的范围，宽 `thickness`，位于 padding box 边缘（扣掉 border）往内 `track_inset` 处：竖条在右，横条在底，两端各内缩 `track_inset`。两轴都有条时，各自让出拐角，互不重叠。滑块居中于车道，平时宽 `thumb_thickness`，车道激活或拖动时对称地长到 `thumb_active_thickness`；轨道是与激活滑块同宽的胶囊。

容器在末端留出不小于 `track_inset + thickness` 的 padding，滚动条就完全落在留白里、不碰内容。框架不替你留这块空。

### 层级与指针

滚动条画在这个滚动区所有后代之上，不论后代的 z-index；它仍在自己的祖先裁剪与层叠上下文里，后面的兄弟节点和 viewport-fixed 浮层照样能盖住它。能显示滚动条的滚动区因此是一个层叠组。

只有看得见的车道接收指针，淡入中的也算。指针落在车道上时，悬停目标是滚动区本身：下面的行不显示悬停底色或 tooltip，车道激活、滑块变粗。在车道上按下即抓住滑块；按在空轨道上，先把滑块居中到落点。

### 动效与样式

淡入、淡出和变粗按主题的 `ScrollbarRecipe::motion` 播放；减少动态效果时直接到终点。空闲时不留任何 deadline。

尺寸在 `ThemeMetrics::scrollbar`；颜色、动效与显隐策略在 `ScrollbarRecipe`，用 `ThemeDefinition::with_scrollbar` 安装。单个节点可以用 `PaintStyle::scrollbar`（`ScrollbarSkin`）改尺寸或颜色。CSS 的 `::-webkit-scrollbar` 与 `::-webkit-scrollbar-thumb` 写的就是它；CSS 指定的滑块宽度在悬停时不变粗。滚动区伸进容器边距、滚动条落在边距里时，用 `ScrollbarSkin::room` 给出边距宽度：边距放不下主题的车道和两侧内缩时，车道收窄到放得下，滑块不超过车道；绘制时才按已装主题解析，主题本身更窄时以主题为准。

`overflow: auto` 和 `overflow: scroll` 共用同一份 `ScrollOffset` 和裁剪，不再另画一套滑块。自定义滚动条铬只属于 `ScrollView`。

拖拽时只有抓取锚点记在 `ScrollbarDragState` 里，偏移仍在世界上。`scrollbars_revealed` 回答这个滚动区是否请求显示自动隐藏的滚动条；实际画没画出来，看 `AppContext::scrollbars_visible`。

## 借出滚动口

`SidebarFrame` 的正文借用 `project_scrollport`，避免在自己的视觉上再盖一条滚动条。

## 视口

布局结束后，监听了 `ScrollViewportChanged` 的节点会收到新的视口宽高，虚拟列表用它收窄窗口。

## 属性

`.label` 是可访问名。内容超出视口时，该轴公布当前偏移和可达范围；隐藏滚动条不影响这份范围。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `axes` | `ScrollAxes` | `Horizontal`、`Vertical` 或 `Both`。`ScrollView::new(axes)`。对应轴的溢出写成 `Scroll` |
| `.label` | — | 可访问名 |
| `.follow_end` | — | `true` 时，布局公布新的滚动几何之后竖向停在最新内容；读旧内容时关掉 |
| `.scrollbars` | `nana_ui_core::ScrollbarVisibility` | 不写时跟随主题策略（内置 `AutoHide`）；`AutoHide` 悬停或滚动时出现、离开后延迟淡出；`Always` 在能滚时一直画着并带轨道；`Hidden` 不画，滚轮和 `scroll_to` 照常 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `ScrollViewportChanged` | — | 布局结束后，监听了它的节点会收到新的视口宽高，虚拟列表用它收窄窗口 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 子节点 | 滚动的内容。函数侧是 `.children` |

## 参见

[总览](index.md) · [控件](../reference/components.md)
