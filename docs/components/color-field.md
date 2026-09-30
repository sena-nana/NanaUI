# ColorField

`ColorField` 是色块加十六进制。

它是叶子复合件：你写属性时，`assemble_color_field` 自己建出色块、hex 输入和 HSV 选择器，不用再记一次装配。

选择器是现有控件：饱和度/明度用 `XYPad`，色相用 `RangeField`，包在它自己的 `Popover` 里。

宿主拿不到窗口句柄。

`ColorField::new(value)` 的 `value` 是 `0..=1` 的 RGBA。

构造时会收成合法颜色，并算出 `hue`、`sat`、`val`。

默认无障碍名是「颜色」，用 `.label` 换成你的文案。

`.size`、`.disabled`、`.invalid` 跟字段控件一样。

`.opened` 在禁用时强制关上。

开合写在字段自己的 `opened` 上。

不要去改装配出来的那个 `Popover`，下一次装配会把它盖掉。

提交发 `ColorChanged { value }`，拖动中发 `ColorInput { value }`。

hex 框的文本变化会写回 `value`。

同一颜色再次写入时，打开状态和 HSV 光标留在拖动中的位置；颜色变了才按新的 RGB 重算。

子节点身份由运行时持有。

控件表里没有 `<ColorField>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::ColorField;

view! {
    <Widget of={ColorField::new([0.29, 0.57, 0.85, 1.0]).label("强调色")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::ColorField;

widget(ColorField::new([0.29, 0.57, 0.85, 1.0]).label("强调色"))
```

:::

`create_component` 建出来的字段同样会在写入后跑 `assemble_color_field`。

色块点一下翻转 `opened`。

禁用时点色块没有反应，选择器也关着。

叶子复合件的装配挂在写入上：你改 `value`、`label` 或 `opened`，`assemble_color_field` 会重建色块、hex 和选择器。

这和壳层不一样，壳层要等槽位放好再显式 `assemble_*`。

RGBA 四个分量都在 `0..=1`。

提交读 `ColorChanged`，拖动中的预览读 `ColorInput`。

同一颜色再写一次时，HSV 光标留在拖动中的位置；颜色变了才按新的 RGB 重算。

默认无障碍名是「颜色」。

[总览](index.md) · [控件合同](../reference/components.md)
