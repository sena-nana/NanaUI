# RichTextView

`RichTextView` 画一段带样式范围的文本。值是 `RichText`：一个字符串，加上按字节范围挂上去的样式。值由应用持有，控件只拿一份克隆去投影。

控件表里没有 `<RichTextView>`。标签 `rich-text` 只造出不带样式的纯文本，带样式的值从 Rust 交进来。

## 基本用法

`RichText::builder()` 一段一段地拼。`plain` 用节点自己的样式，`push` 给这一段挂一份样式。没写的字段继承节点的计算样式。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{PaintColor, RichSpanStyle, RichText};

let line = RichText::builder()
    .plain("今天也")
    .push("辛苦了", RichSpanStyle::new().bold().color(PaintColor::srgb([1.0, 0.4, 0.5, 1.0])))
    .build();

view! {
    <Widget of={RichTextView::new(line).font_size(20.0)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{PaintColor, RichSpanStyle, RichText};

let line = RichText::builder()
    .plain("今天也")
    .push("辛苦了", RichSpanStyle::new().bold().color(PaintColor::srgb([1.0, 0.4, 0.5, 1.0])))
    .build();

widget(RichTextView::new(line).font_size(20.0))
```

:::

改值用 `set_component(entity, RichTextView::new(next))`。和节点上已有的值相等时不提交任何东西。不需要组件时，`MutationQueue::set_rich_text(id, rich)` 直接作用在任意文本节点上。

## 样式分三层

`RichSpanStyle` 的字段按「改了要花多少」分成三层。一次变化按它碰到的层定价：

| 层 | 字段 | 改了之后 |
| --- | --- | --- |
| 塑形 `shape` | `family`、`size_px`、`weight`、`italic`、`letter_spacing_px`、`features` | 重新塑形、排版；行盒跟着变高 |
| 绘制 `paint` | `color`、`decoration`、`decoration_color`、`stroke`、`shadows` | 只重绘：不塑形、不排版 |
| 呈现 | `effect` | 不碰文本，交给合成器 |

`size` 比节点字号大的那段，按节点自己的行高比例撑高它所在的行。节点用绝对行高时，比例是行高除以节点字号。

## 描边、阴影与装饰线

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{PaintColor, RichSpanStyle, RichText, RichTextShadow, RichTextStroke};

let black = PaintColor::srgb([0.0, 0.0, 0.0, 1.0]);
let line = RichText::new("注意看").with_span(
    0..9,
    RichSpanStyle::new()
        .stroke(RichTextStroke::new(3.0, black))
        .shadow(RichTextShadow::new([2.0, 2.0], 6.0, black))
        .underline(),
);

view! {
    <Widget of={RichTextView::new(line)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{PaintColor, RichSpanStyle, RichText, RichTextShadow, RichTextStroke};

let black = PaintColor::srgb([0.0, 0.0, 0.0, 1.0]);
let line = RichText::new("注意看").with_span(
    0..9,
    RichSpanStyle::new()
        .stroke(RichTextStroke::new(3.0, black))
        .shadow(RichTextShadow::new([2.0, 2.0], 6.0, black))
        .underline(),
);

widget(RichTextView::new(line))
```

:::

- 描边默认画在填充下面（`TextStrokePlacement::Under`），字形本身保持完整，外面长一圈。`Over` 居中压在轮廓上，和 CSS `-webkit-text-stroke` 的默认一样。
- 阴影最多四层（`MAX_TEXT_SHADOWS`），第一层在最上。`blur_px` 是 CSS 模糊半径，超过 24 逻辑像素按 24 画。`spread_px` 先把覆盖扩开再模糊。
- 下划线和删除线按行、按 run 画，位置和粗细来自字体的 `post` / `OS/2` 表。没写 `decoration_color` 时跟着那几个字的填充色。

画的顺序是：阴影、填充下的描边、下划线、填充、填充上的描边、删除线。都是同一段落的字形实例，一次绘制。

节点的 CSS（`text-decoration`、多层 `text-shadow`、`-webkit-text-stroke`、`paint-order`）是底，span 的绘制层盖在上面。span 没写的字段沿用节点的。

## 内联贴纸

贴纸、表情是文本里的对象，和字一起换行。`RichTextBuilder::object` 放进一个 `RichObject`。它站在基线上，`descent` 让它往下沉一点。改它的尺寸只重新排版，不重新塑形。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{RichObject, RichText};

let line = RichText::builder()
    .plain("早上好")
    .object(RichObject::image(1, "file:///stickers/wave.png", 28.0, 28.0).descent(4.0))
    .plain("今天也加油")
    .build();

view! {
    <Widget of={RichTextView::new(line)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{RichObject, RichText};

let line = RichText::builder()
    .plain("早上好")
    .object(RichObject::image(1, "file:///stickers/wave.png", 28.0, 28.0).descent(4.0))
    .plain("今天也加油")
    .build();

widget(RichTextView::new(line))
```

:::

- `RichObject::image(id, url, w, h)`：图片，和 CSS `url()` 同源规则。
- `RichObject::texture(id, slot, w, h)`：宿主纹理槽。动图由应用解码后写进这个槽，由宿主纹理渲染器画。
- `RichObject::chip(id, label, kind)`：编辑器里的标记。不占宽度，展示框不画它，所以编辑器和展示框断行一致。

## 逐字特效与打字机揭示

`RichSpanStyle::effect(i)` 让一段字播放特效表里的第 `i` 个特效；`cx.set_rich_presentation(view, effects, reveal)` 给出这张表和揭示计划。它们只是呈现：不重新塑形、不重新排版，也不重建字形，文字着色器按运动时钟逐字算位置和透明度。只在还有东西在动时请求帧。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{GlyphEffect, GlyphIntro, RevealSchedule, RichSpanStyle, RichText};

let line = RichText::builder()
    .plain("欢迎")
    .push("来到直播间", RichSpanStyle::new().effect(0))
    .build();
let view = cx.create_component(document, RichTextView::new(line))?;
let reveal = RevealSchedule::uniform(cx.animation_now(), 7, 0.08).intro(GlyphIntro::pop(0.2));
cx.set_rich_presentation(view, vec![GlyphEffect::wave(3.0)], Some(reveal))?;

view! {
    <Widget of={RichTextView::new(RichText::new("欢迎来到直播间"))} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::RichTextView;
use nana_ui::runtime::rich::{GlyphEffect, GlyphIntro, RevealSchedule, RichSpanStyle, RichText};

let line = RichText::builder()
    .plain("欢迎")
    .push("来到直播间", RichSpanStyle::new().effect(0))
    .build();
let view = cx.create_component(document, RichTextView::new(line))?;
let reveal = RevealSchedule::uniform(cx.animation_now(), 7, 0.08).intro(GlyphIntro::pop(0.2));
cx.set_rich_presentation(view, vec![GlyphEffect::wave(3.0)], Some(reveal))?;

widget(RichTextView::new(RichText::new("欢迎来到直播间")))
```

:::

- 特效：`shake`（抖动）、`wave`（波浪）、`jump`（跳动）、`rainbow`（彩虹）、`pulse`（呼吸缩放）、`flicker`（闪烁），`stagger` 是相邻字之间的相位差。
- 揭示：`at_s[i]` 是第 `i` 个字素开始入场的时刻（秒，相对 `start`）；`limit` 让揭示停在某个字素前（暂停标记）。`start` 用 `cx.animation_now()`，和合成器同一个时钟。
- 入场：`GlyphIntro::fade` / `pop` / `rise`。
- 贴纸跟着自己所在的字素揭示、跟着它的特效动。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `RichText` | 文本和样式范围。范围按字节，落在字符中间的端点向前对齐到字符边界 |
| `decorative` | `bool` | 仍绘制并参与布局，但从无障碍树中移除 |
| `font_size` | `f32` | 节点字号，没写字号的 span 用它。不把行盒钉成绝对高度 |
| `font_family` | `String` | 节点字体族，CSS `font-family` 语法 |
| `line_height` | `f32` | 行高，是每个 run 自己字号的倍数 |
| `color` | `SemanticColorRole` | 没上色的文字的语义前景 |
| `width` / `max_width` / `nowrap` | | 同 [Text](text.md) |

## 事件

没有事件。

## 参见

[Text](text.md) · [总览](index.md) · [文本引擎](../reference/text-engine.md#富文本-span)
