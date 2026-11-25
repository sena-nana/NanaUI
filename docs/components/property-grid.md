# PropertyGrid

`PropertyGrid` 是专业工具检查器里那种两列密集面板：行分在可折叠的组里，每行左边是标签，右边是应用的控件。整个网格共用一个标签列宽，可以拖动。它没有模板标签，用 `widget(PropertyGrid::new(组))` 放进视图。

这个类型在 Cargo feature `property-grid` 后面（`components` 包含它）。

## 基本用法

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{PropertyGrid, PropertyRow, PropertySection, ScrubField};

widget(PropertyGrid::new([
    PropertySection::new("transform", "变换", [
        PropertyRow::new("x", "X").mixed(true),
        PropertyRow::new("angle", "角度").disabled(Some("由父变形器控制")),
    ]),
    PropertySection::new("draw", "绘制", [PropertyRow::new("opacity", "不透明度")]),
]))
.children((
    widget(ScrubField::new(12.0).mixed(true)).key("x"),
    widget(ScrubField::new(0.0).unit("°")).key("angle"),
    widget(ScrubField::new(1.0).range(0.0, 1.0)).key("opacity"),
))
```

视图里键为行 id 的子节点就是这一行的控件，例如 `ScrubField`、`Select`、`ColorField`。模型里已经没有的行，对应的控件会隐藏起来等待，不会被销毁。

## 组

- 按下组标题会折叠或展开这一组，并发出 `PropertySectionToggled { id, collapsed }`。
- `collapsed_sections()` 返回当前折叠的组。应用把它和标签列宽一起保存，下次用 `PropertySection::collapsed(true)` 和 `.label_width(宽度)` 恢复。
- 视图重新渲染时，用户折叠过的组和拖过的列宽不会被覆盖。

## 标签列

标签列默认宽 96 px，所有行共用这个宽度。标签列右边缘有一条 6 px 的拖动条，横向拖动它会同时改变所有行的标签宽度；标签列和控件列都至少保留 48 px。松手时发出 `PropertyLabelWidthChanged { width }`。

## 混合值与禁用

- `.mixed(true)`：多选时各对象的值不一致。标签改用正文色，读屏会读出「mixed values」。控件自己显示混合状态，例如 `ScrubField::mixed` 显示「—」。
- `.disabled(Some(原因))`：控件变半透明，上面盖一层透明遮罩，指针按不到控件。原因用淡色小字显示在这一行下面，读屏把它当作描述读出。只禁用、不需要说明时写 `.disabled(None::<&str>)`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `PropertyGrid::new(组)` | `PropertySection` | 组和行 |
| `.label_width(px)` | `f32` | 标签列宽，默认 96 |
| `PropertySection::new(id, 标题, 行)` | — | 一组 |
| `.collapsed(bool)` | `bool` | 组是否折叠 |
| `PropertyRow::new(id, 标签)` | — | 一行；控件是键为 `id` 的子节点 |
| `.mixed(bool)` | `bool` | 混合值 |
| `.disabled(原因)` | `Option<文字>` | 禁用，并说明原因 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `PropertySectionToggled` | `id`, `collapsed` | 组被折叠或展开 |
| `PropertyLabelWidthChanged` | `width` | 拖动标签列结束 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 键为行 id 的子节点 | 这一行的控件 |

## 参见

[ScrubField](https://github.com/sena-nana/NanaUI/blob/main/docs/components/scrub-field.md) · [DynamicForm](https://github.com/sena-nana/NanaUI/blob/main/docs/components/dynamic-form.md) · [总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
