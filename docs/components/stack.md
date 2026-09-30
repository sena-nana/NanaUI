# Stack

`Stack` 负责把若干子节点排成一行、一列，或铺成一层叠层。

尺寸、对齐和溢出的完整字段在 `LayoutStyle` 上，预设只覆盖常用的 flex。

`Stack::row(gap)` 水平排列，宽度随内容，子项垂直居中，适合工具条和按钮组。

`Stack::fill_row(gap)` 水平占满父级剩余宽度，子项可以收缩。

`Stack::column(gap)` 竖直排列，高度随内容，宽度占满父级。

`Stack::fill_column(gap)` 竖直占满剩余高度，用来做主内容区。

`Stack::bar(gap)` 水平占满整行，自己不伸展，用来做顶栏和底栏。

`Stack::spacer()` 是零宽 flex-grow，把排在它后面的兄弟推到行尾。

基准宽度是 0，否则它会铺满，把兄弟挤出去。

`Stack::overlay_layer()` 绝对定位，铺满已经定位的父级，溢出裁剪，指针事件关掉。

舞台上的 HUD 和弹幕用它当容器，节点池仍由你挂。

Rust 预设默认不参与命中。

空白处的按下也要落在这个容器上时，接 `.hittable()`。

`Stack::from_layout` 承接已经解析好的 `LayoutStyle`，默认可点，和 Vue 布局盒一致。

`.with_layout` 就地改布局字段。

`.gap`、`.align`、`.justify`、`.width`、`.height`、`.grow`、`.shrink`、`.padding` 改预设上的对应项。

`.surface` 写语义背景。

`.outline` 要同时给出边框色和宽度，缺一则不画边框。

控件表里没有 `<Stack>`。

视图函数 `column()` 是 `widget(Stack::column(0.0))`，`row()` 是 `widget(Stack::row(0.0))`。

间距不为 0 时直接构造。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Stack;

view! {
    <Widget of={Stack::row(8.0)}>
        <Text>"标题"</Text>
        <Widget of={Stack::spacer()} />
        <Button>"保存"</Button>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{button, text, widget};
use nana_ui::runtime::Stack;

widget(Stack::row(8.0)).children((
    text("标题"),
    widget(Stack::spacer()),
    button("保存"),
))
```

:::

`.painter` 可以自绘这个容器的外观。

要在内建绘制上加装饰，在 `paint` 里调用 `cx.draw_default()`。

二维码卡片的空白也要接到指针时，用 `Stack::column(...).hittable()`，事件才归这张容器。

[总览](index.md) · [控件合同](../reference/components.md)
