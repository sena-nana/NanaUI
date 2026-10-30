# scroll_foot_fade

`scroll_foot_fade(height, surface)` 是盖在滚动区底边上的渐隐：从透明过渡到 `surface`，告诉用户列表还没到底，而不是让最后一行被硬切。它是 `view::marks` 里的视图，不是注册控件。

## 基本用法

```rust
use nana_ui::runtime::view::{scroll_foot_fade, widget};
use nana_ui::runtime::{PositionSpec, ScrollAxes, ScrollView, SemanticColorRole, Stack};
use nana_ui::theme::ThemeToken;

let gutter = ThemeToken::PanelPaddingX; // 卡片自己的水平内边距，随主题变
// 这一列在卡片的内容框里：它的左右边就是内容线。
widget(Stack::column(0.0).with_layout(|l| l.position = PositionSpec::Relative))
    .children((
        widget(ScrollView::new(ScrollAxes::Vertical).gutter_theme(gutter)).children(list),
        scroll_foot_fade(24.0, SemanticColorRole::Surface).clear_of_theme(gutter),
    ))
```

- 渐隐是绝对定位的，放在装滚动区的那个定位容器里，贴着它的底边，左右铺满。
- 它不接收输入：滚轮和指针都落到下面的列表上。
- 列表末尾留出 `height` 的空白，最后一行能滚出渐隐。
- `surface` 用滚动区背后的表面角色，渐隐才看不出边。

## 避开滚动条

滚动区用 `ScrollView::gutter_theme(档位)` 伸进容器的边距、把滚动条画在边距里时，渐隐写 `.clear_of_theme(档位)`（同一个值），和滚动区放在同一个定位容器里（上面例子里卡片内容框中的那一列）。滚动区从这个容器左右各伸出这段距离，滚动条车道是右边伸出的那一段；渐隐左边跟着滚动区伸出去，右边停在容器右边、也就是车道开始的地方。所以渐隐盖住整条内容线和左侧边距，滚动条不被盖淡，渐隐的底边也就是滚动区的底边。

在卡片里，档位写 `ThemeToken::PanelPaddingX`，也就是卡片自己的水平内边距：默认主题下是 16，`COMPACT_METRICS` 下是 10，滚动区和渐隐都跟着走。像素版 `.gutter(px)` / `.clear_of(px)` 只在写它的那套主题下成立，例如 `gutter(16.0)` 换到 COMPACT 会比卡片内边距多伸出 6。

## 接口

| 接口 | 说明 |
| --- | --- |
| `scroll_foot_fade(height, surface)` | 底部渐隐 |
| `ScrollFootFade::clear_of_theme(gutter)` | 配合同一容器里 `gutter_theme(gutter)` 的滚动区：盖住滚动区伸到的范围，让出滚动条车道，随主题变 |
| `ScrollFootFade::clear_of(px)` | 同上，像素版，配合 `gutter(px)`；只在一套主题下成立 |

## 参见

[ScrollView](scroll-view.md) · [版式与组合](../guide/composition.md#滚动区)
