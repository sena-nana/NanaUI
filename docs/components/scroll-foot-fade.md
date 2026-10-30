# scroll_foot_fade

`scroll_foot_fade(height, surface)` 是盖在滚动区底边上的渐隐：从透明过渡到 `surface`，告诉用户列表还没到底，而不是让最后一行被硬切。它是 `view::marks` 里的视图，不是注册控件。

## 基本用法

```rust
use nana_ui::runtime::view::{scroll_foot_fade, widget};
use nana_ui::runtime::{PositionSpec, ScrollAxes, ScrollView, SemanticColorRole, Stack};
use nana_ui::theme::space;

let gutter = space::XL; // 卡片的水平内边距
widget(Stack::column(0.0).with_layout(|l| l.position = PositionSpec::Relative))
    .children((
        widget(ScrollView::new(ScrollAxes::Vertical).gutter(gutter)).children(list),
        scroll_foot_fade(24.0, SemanticColorRole::Surface).clear_of(gutter),
    ))
```

- 渐隐是绝对定位的，放在装滚动区的那个定位容器里，贴着它的底边，左右铺满。
- 它不接收输入：滚轮和指针都落到下面的列表上。
- 列表末尾留出 `height` 的空白，最后一行能滚出渐隐。
- `surface` 用滚动区背后的表面角色，渐隐才看不出边。

## 避开滚动条

滚动区用 `ScrollView::gutter(px)` 伸进容器的边距、把滚动条画在边距里时，渐隐写 `.clear_of(px)`（同一个值）：右边停在滚动条车道之前，滚动条不被盖淡。

## 接口

| 接口 | 说明 |
| --- | --- |
| `scroll_foot_fade(height, surface)` | 底部渐隐 |
| `ScrollFootFade::clear_of(gutter)` | 右边让出滚动条车道 |

## 参见

[ScrollView](scroll-view.md) · [版式与组合](../guide/composition.md#滚动区)
