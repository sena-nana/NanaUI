# settings_group

`settings_group(title, rows)` 是一组设置：标题在上，凸起的卡片在下，行之间是缩进的细线。它是 `view::marks` 里的视图；卡片里管理行和分隔线的是 Runtime 的 `SettingsGroup`。

## 基本用法

```rust
use nana_ui::runtime::view::{group_row_on_line, settings_group, settings_row};

settings_group(Some("输入".into()), vec![
    group_row_on_line(settings_row("发送").control(send_chord)),
    group_row_on_line(settings_row("镜像").control(mirror_switch)).visible(advanced),
])
```

- `title` 为 `None` 时不画标题（页面标题已经说明了这组是什么）。
- 卡片由 `RaisedSurfacePainter` 画：细边框，亮色下一道短阴影。

## 内容线

卡片和 `group_row` **都不加左右内边距**；行间细线缩进 `CONTENT_INSET`（`SpacingStep::Xxxl`）。行的文字和控件要放在同样的内容线上：

- `group_row_on_line(view)`：等于 `group_row(content_line(view))`，常用写法；
- `group_row(view)`：`view` 已经带了这层内边距（应用自己的行构造器）时用，不要再包一次。

`settings_row` 自己没有左右内边距，所以同样要上内容线。没上的行（设置行、按钮、说明文字都一样）会贴着卡片边缘，和缩进的细线对不齐。

## 隐藏行

`group_row(view).visible(cond)`：隐藏的行带走它的分隔线，第一行也一样——分隔线只在两个显示的行之间显示。不要只隐藏行里的内容，那会留下空行。

## 什么时候用别的

| 内容 | 用 |
| --- | --- |
| 一组同类设置 | `settings_group` |
| 一块自由内容（说明、图表、按钮排） | `Card`，保留它的内边距 |
| 用 `Card` 当分组容器、行自己上内容线 | `Card::new().kind(CardKind::Flat).padding(0.0)` |

分组之间的距离由放分组的那一列的 `gap` 给；分组只靠标题和卡片区分，不加装饰。

## 接口

| 接口 | 说明 |
| --- | --- |
| `settings_group(title, rows)` | 分组；接 `.class` / `.css` / `.visible`，作用在放标题和卡片的那一列上 |
| `group_row(view)` | 一行，原样放入 |
| `group_row_on_line(view)` | 一行，放到内容线上 |
| `GroupRow::visible(cond)` | 按条件显示，隐藏时带走分隔线 |
| `content_line(view)` | 把任意视图放到内容线上（只加水平内边距） |
| `CONTENT_INSET` / `content_inset()` | 内容线的档位 / 默认度量下的像素值 |

## 参见

[版式与组合](../guide/composition.md) · [SettingsRow](settings-row.md) · [Card](card.md) · [Divider](divider.md)
