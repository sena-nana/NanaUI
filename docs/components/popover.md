# Popover

`Popover` 是挂在自己触发器上的一块表面。按下、焦点、Enter、空格和面板锚点都是这个触发器。它自己切换 `open`，不用 `activate_overlay`。

控件表里没有 `<Popover>`。

## 基本用法

`Popover::new()` 的触发文字是空的，位置在下方，对齐居中。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ActionMenuItem, Popover};

view! {
    <Widget of={Popover::new().trigger("更多")}>
        <Widget of={ActionMenuItem::new("复制")} />
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ActionMenuItem, Popover};

widget(Popover::new().trigger("更多")).children(widget(ActionMenuItem::new("复制")))
```

:::

## 触发器

`.trigger(text)` 写可访问名，也是纯文本触发器显示的字。

`.trigger_icon(icon, label)` 画成 28×28 的方形按钮，图标在按钮里居中，`label` 是可访问名。图标是布局发布的 `LeadingContent` 部件（#205），改 `trigger_icon_size` 会重新发布它，绘制只读这个盒子。

`.bare_trigger(true)` 去掉静止时的底色和描边。悬停、按下和打开时仍有一层洗色。不传则保持浅底加描边。`ActionMenu` 同样转发。

裸符号不要塞进文本触发器。

要「图标 + 文字 + 计数」，用视图的 `.trigger(view)`，对应 `Popover::trigger_content`。

那段内容是触发器自己的子节点，面板关着也显示，不算面板条目。

里面只做显示，不要放可按的控件。

触发器宽度随内容收缩，交叉轴对齐由父容器决定：在居中的横向工具栏里垂直居中，在默认拉伸的纵向容器里也不会占满整行。

`.trigger_style(style)` 改触发器的样式。`style` 里写了的字段盖在触发器原本的样式上，没写的保持原样：只写一个宽度时，悬停和按下照样变色，打开时的样子、`compact_trigger` 收紧到内容的尺寸、`bare_trigger` 的去底色也都还在。写了圆角（`layout.border_radius` 或逐角的 `layout.paint.border_radii`）时按你写的画，不再用主题的小圆角；写了 `layout.height` 时不再套控件高度档位；写了左右或上下内边距时不再套控件内边距档位，文字触发器的标签也画在你写的内边距里（标签是布局发布的 `TextContent` 部件，横向占触发器减去左右内边距、纵向占整个触发器，测量、整形与绘制读同一个盒子；此前绘制固定按控件档位的内边距，写小的内边距时标签偏右、被挤窄）。只作用于触发器，面板的宽度和内边距仍由 `.width`、`.padding` 决定。`ActionMenu` 同样转发。

```rust
let mut round = NodeStyle::default();
let layout = round.layout_mut();
layout.width = Some(LengthSpec::Px(36.0));
layout.height = Some(LengthSpec::Px(36.0));
layout.border_radius = Some(18.0);
Popover::new().trigger_icon(Icon::Add, "更多").trigger_style(round)
```

## 位置

`.placement` 用 `nana_ui::PopoverPlacement`：`Top`、`Bottom`、`Left`、`Right`，默认 `Bottom`。

`.alignment` 用 `PopoverAlignment`。

放不下首选一侧、对侧放得下时翻到对侧，再收进视口。

打开之后页面滚动，弹出层留在原处，直到下一次布局。

表面是 viewport-fixed，不进父级 isolation group，面板的 overflow 裁不到它。

## 关闭

面板因触发器、Escape、点外面或你把 `open` 写成 `false` 而关上时，焦点若还在面板条目上，回到触发器。

`.close_on_escape` 和 `.close_on_outside` 默认都开着。

## 尺寸

`.width` 有最小宽度，`.gap` 和 `.padding` 小于 0 时收成 0。

## 锚点

没有「任意节点当锚点」这一路，锚点就是触发器自己。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.trigger` | — | 写可访问名，也是纯文本触发器显示的字。`new()` 的触发文字是空的 |
| `.trigger_icon` | — | `(icon, label)` 画成 28×28 的方形按钮，图标居中，`label` 是可访问名 |
| `.bare_trigger` | — | `true` 去掉静止时的底色和描边。悬停、按下和打开时仍有一层洗色。不传则保持浅底加描边 |
| `.trigger_style` | `NodeStyle` | 盖在触发器原有样式上，只替换写了的字段；悬停、按下和收紧尺寸都保留，写了的圆角按你写的画。默认 `None` |
| `.trigger_icon_size` | `f32` | 图标触发器的字形边长（逻辑像素），让它和旁边按 `IconButton::icon_size` 设过的工具一样大；触发器的盒子不变。默认 `None`，用标准字形大小 |
| `.placement` | `nana_ui::PopoverPlacement` | `Top`、`Bottom`、`Left`、`Right`，默认 `Bottom`。`new()` 位置在下方 |
| `.alignment` | `PopoverAlignment` | `new()` 对齐居中 |
| `open` | — | 它自己切换。写成 `false` 会关上 |
| `.close_on_escape` | — | 默认开着 |
| `.close_on_outside` | — | 默认开着 |
| `.width` | — | 有最小宽度 |
| `.gap` | — | 小于 0 时收成 0 |
| `.padding` | — | 小于 0 时收成 0 |

## 事件

事件是 `PopoverToggled` 和 `PopoverClosed`。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `PopoverToggled` | — | 这一页的事件 |
| `PopoverClosed` | — | 这一页的事件 |

## 插槽

`.children` 才是面板里的条目。

| 插槽 | 说明 |
| --- | --- |
| `.children` | 面板里的条目 |
| `.trigger(view)` | 对应 `Popover::trigger_content`。触发器自己的子节点，面板关着也显示，不算面板条目。里面只做显示，不要放可按的控件 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
