# IconGlyph

`IconGlyph` 负责画出一个图标。它是装饰：不接收指针，也不能聚焦。要点击，用 `IconButton`，把图标和可访问名放在按钮上。

控件表里没有 `<IconGlyph>`。

## 基本用法

`IconGlyph::new(icon)` 接收 `nana_ui::runtime::Icon`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Icon, IconGlyph};

view! {
    <Widget of={IconGlyph::new(Icon::Folder).size(16.0).role(nana_ui::runtime::SemanticColorRole::Text)} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Icon, IconGlyph};

widget(IconGlyph::new(Icon::Folder).size(16.0).role(nana_ui::runtime::SemanticColorRole::Text))
```

:::

同一行里既要图标又要可点，把 `IconGlyph` 留在只展示的位置，点击交给旁边的按钮。

## 尺寸

默认边长是小号控件的图标尺寸，前景色是 `Muted`。

`.size` 改边长，小于 0 的值收成 0，宽高、最小宽高都写成这个边长，并且不再伸缩。

`.role` 换成别的语义色。

`.style` 整份替换节点样式；依赖尺寸的构造要写在它后面，否则会被盖掉。

## 投影

投影时节点文本是空字符串。

视觉是 `StandardVisual::Icon`，没有工具提示。

可访问状态保持默认，读屏不会把它当成按钮。

## 图标从哪来

壳层目录里的图标是类型上的常量，例如 `Icon::Close`、`Icon::Add`、`Icon::Folder`。

产品自己的字形走 `icons-tabler` 或 `Icon::from_data`。

## 提示

`IconButton::with_tooltip` 使用默认的 `TooltipConfig`，那是按钮自己的提示，不是这个字形控件的字段。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `icon` | `nana_ui::runtime::Icon` | `IconGlyph::new(icon)` 接收 |
| `.size` | — | 改边长。小于 0 的值收成 0，宽高、最小宽高都写成这个边长，并且不再伸缩。默认边长是小号控件的图标尺寸 |
| `.role` | — | 换成别的语义色。默认前景色是 `Muted` |
| `.style` | — | 整份替换节点样式；依赖尺寸的构造要写在它后面，否则会被盖掉 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 不接收指针，也不能聚焦 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
