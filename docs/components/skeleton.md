# Skeleton

`Skeleton` 是一块不接交互的占位表面。

内容还没到的时候用它占住尺寸。

它不接收指针，也不能聚焦。

`Skeleton::new(width, height)` 的宽度是 `LengthSpec`，高度是逻辑像素。

`Skeleton::fill_width(height)` 等于宽度 `LengthSpec::Fill`。

`.width` 和 `.height` 可以再改。

非法尺寸会被收成可用值。

背景是 `Subtle`，圆角用主题的 `radius_sm`，没有边框。

挂上之后，Motion IR 用一条无限时间线让不透明度在 1 和 0.48 之间来回。

已经在跑的时间线不会因为再次投影而重头开始。

停放的节点跳过启动，挂上时再开始。

不透明度在合成层上，不写进逻辑布局。

要停掉，卸载或停放这个节点。

控件表里没有 `<Skeleton>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{LengthSpec, Skeleton};

view! {
    <Widget of={Skeleton::new(LengthSpec::Px(160.0), 12.0)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{LengthSpec, Skeleton};

widget(Skeleton::new(LengthSpec::Px(160.0), 12.0))
```

:::

一行标题加两行正文时，用不同高度的几块骨架叠在 `Stack::column` 里。

数据到了就换成真正的文本。

骨架的交互是关掉的，它不进 Tab 顺序。

脉冲从透明度 1 走到 `1 - 0.52`，也就是 0.48，方向来回，次数无限。

时间线 id 由节点算出来；算出来是 0 时这条脉冲不会启动。

再次投影发现时间线还在，就不会把它拨回起点。

[总览](index.md) · [控件合同](../reference/components.md)
