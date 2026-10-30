# scroll_foot_fade

`scroll_foot_fade(height, surface)` 是盖在滚动区底边上的渐隐：从透明过渡到 `surface`，告诉用户列表还没到底，而不是让最后一行被硬切。它是 `view::marks` 里的视图，不是注册控件。

## 基本用法

```rust
use nana_ui::runtime::view::{scroll_foot_fade, widget};
use nana_ui::runtime::{PositionSpec, ScrollAxes, ScrollView, SemanticColorRole, Stack};
use nana_ui::theme::space;

let gutter = space::XL; // 卡片的水平内边距
// 这一列在卡片的内容框里：它的左右边就是内容线。
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

滚动区用 `ScrollView::gutter(px)` 伸进容器的边距、把滚动条画在边距里时，渐隐写 `.clear_of(px)`（同一个值），和滚动区放在同一个定位容器里（上面例子里卡片内容框中的那一列）。滚动区从这个容器左右各伸出 `px`，滚动条车道是右边伸出的那一段；渐隐左边跟着滚动区伸出去，右边停在容器右边、也就是车道开始的地方。所以渐隐盖住整条内容线和左侧边距，滚动条不被盖淡，渐隐的底边也就是滚动区的底边。

## 接口

| 接口 | 说明 |
| --- | --- |
| `scroll_foot_fade(height, surface)` | 底部渐隐 |
| `ScrollFootFade::clear_of(gutter)` | 配合同一容器里 `gutter(gutter)` 的滚动区：盖住滚动区伸到的范围，让出滚动条车道 |

## 参见

[ScrollView](scroll-view.md) · [版式与组合](../guide/composition.md#滚动区)
