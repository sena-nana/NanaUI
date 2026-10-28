# AreaLayout

`AreaLayout` 把一块矩形递归切成互不重叠的区域。三维和动画软件常用这种布局。每个区域显示一种编辑器，内容和头部都由应用提供。它没有模板标签，用 `widget(AreaLayout::new(模型))` 放进视图。

## 基本用法

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AreaAxis, AreaLayout, AreaLayoutChanged, AreaLayoutModel};

let mut model = AreaLayoutModel::single("outliner", "outliner");
model.split("outliner", AreaAxis::Horizontal, 0.25, "viewport");
model.set_editor("viewport", "viewport");

widget(AreaLayout::new(model))
    .on(|e: &AreaLayoutChanged| { /* e.change、e.model */ })
    .children((
        outliner_header().key("outliner:header"),
        outliner().key("outliner"),
        viewport_header().key("viewport:header"),
        viewport().key("viewport"),
    ))
```

视图子节点按键放进区域：

- 键是区域 id 的子节点是这个区域的内容。
- 键是 `区域 id:header` 的子节点放进这个区域的头部。头部是一条小号控件高度的横条，放应用自己的头部控件。
- 键是 `editor:类型` 的子节点是这种编辑器的内容，键是 `editor:类型:header` 的子节点是它的头部控件，放进正在显示这种编辑器的区域（见下面「按编辑器类型放置」）。

当前布局里没有的区域、没有区域显示的编辑器，对应的子节点隐藏起来等待，不会被销毁。

## 按编辑器类型放置

按区域 id 给内容，应用就得事先知道每个区域的 id；用户从角上切出来的新区域 id 是 `fresh_id` 生成的，应用没法提前声明它的内容。按编辑器类型给内容就没有这个问题：区域换成哪种编辑器，那种编辑器的内容就移过去。

```rust
widget(AreaLayout::new(model).editor_select(AreaEditorSelect::new(["outliner", "viewport"])))
    .children((
        outliner().key("editor:outliner"),
        viewport_toolbar().key("editor:viewport:header"),
        viewport().key("editor:viewport"),
    ))
```

- 一种编辑器的内容同一时间只在一个区域里：最近一次用 `set_area_editor`（或头部下拉）选它的区域优先，其次是现在装着它的区域，最后是叶子顺序里的第一个；可见的区域优先（有区域最大化时只有它可见）。其他也显示这种编辑器的区域内容为空，应用可以在收到 `AreaChange::Split` / `AreaChange::Editor` 后把它们换成别的编辑器。
- 键为区域 id 的内容优先：有这种内容的区域不接收按类型放置的内容。
- `set_area_layout` 换布局后，按上面的顺序重新决定位置。

## 编辑器类型下拉

`.editor_select(AreaEditorSelect::new([类型…]))` 让每个区域头部最左边出现一个小号下拉，选项是这些类型，名称取 `.labels(...)` 的名称表（没有名称的用 id）；区域当前的编辑器不在列表里时也作为选项列出。选中一项等同对这个区域调用 `set_area_editor`，发 `AreaChange::Editor`。下拉在读屏里的名称取 `AreaEditorSelect::label`，不写时取框架文案 `area_layout.editor_type`（默认「编辑器类型」）。换语言时用 `set_area_labels` 更新选项名称，用 `set_area_editor_select` 更新下拉本身。

## 模型

`AreaLayoutModel` 是纯数据，包含以下几部分：

- `root`：区域树。`AreaNode::Area { id }` 是叶子。`AreaNode::Split { axis, ratio, first, second }` 把矩形切成两半。`AreaAxis::Horizontal` 左右排，`AreaAxis::Vertical` 上下排。`ratio` 是第一半的份额。
- `areas`：每个区域的 `editor`（应用的编辑器类型 id）和 `state`（应用保存区域自身状态时用的键）。NanaUI 只保存这两个值，切换编辑器后显示什么内容由应用决定。
- `maximized`：最大化的区域。

`split`、`join`、`can_join`、`set_editor`、`set_ratio`、`toggle_maximized`、`fresh_id` 直接修改模型。`geometry(范围, 间隙, 最小尺寸)` 不需要文档就能算出每个区域和分隔条的位置。`to_json` / `from_json` 做序列化，`from_json` 会先用 `validate` 检查：树和区域一一对应、id 不重复、最大化的区域存在。

## 指针

- **分隔条**：区域之间的间隙（`gap`，默认 2 px）就是分隔条，可以拖动。间隙画得再细，也至少有 6 px 宽的范围可以抓取：分隔条两边各伸进相邻区域的边缘，并且叠在区域上面。两侧都不会小于 `min_size`（默认 48 px）。松手时发 `AreaChange::Resized`。
- **分割**：从区域头部左上角或右上角的小方块向区域内部拖。横向移动为主时左右切开，纵向移动为主时上下切开，切口落在指针处。拖动中显示一条强调色的切线，松手后才切。新区域显示同一种编辑器，id 由 `fresh_id` 生成。发 `AreaChange::Split { area, new }`。
- **合并**：从角上拖进相邻区域。两者必须是同一次切分的两半。被拖进的区域会盖上强调色的阴影，表示它会被移除；松手后，拖动起点所在的区域占据它的位置。发 `AreaChange::Joined { kept, removed }`。拖进不相邻的区域时不会合并。
- 两半都不能小于 `min_size`。指针离开区域，或者移动不足 8 px，就不会切也不会合并。

## 应用调用

| 调用 | 说明 |
| --- | --- |
| `split_area(布局, 区域, 方向, 比例)` | 切开区域，返回新区域的 id |
| `join_areas(布局, 保留, 移除)` | 合并同一次切分的两半 |
| `toggle_area_maximized(布局, Some(区域))` | 最大化；对已经最大化的区域再调一次就还原，传 `None` 也是还原 |
| `set_area_editor(布局, 区域, 编辑器)` | 换编辑器类型；按类型放置的内容跟着移过来，按区域 id 给的内容由应用跟着换 |
| `set_area_layout(布局, 模型)` | 换一整套布局，例如工作区预设 |
| `set_area_labels(布局, [(编辑器, 名称)])` | 换各种编辑器在读屏里和头部下拉里的名称，例如界面换了语言；布局不变 |
| `set_area_editor_select(布局, Some(下拉) / None)` | 换头部的编辑器类型下拉或去掉它；布局不变 |
| `area_layout_boxes(布局)` | 每个可见区域在布局后的盒子，用来在区域上放浮层 |

除 `set_area_labels`、`set_area_editor_select` 外，每个调用都会发一次 `AreaLayoutChanged`，其中带新的模型，应用可以保存它。

每个区域在无障碍树里是一个 `Region`。它的名称按编辑器类型 id 从 `.labels(...)` / `set_area_labels` 里查，没有给出时就是 id 本身。名称跟着界面语言走，不写进 `AreaLayoutModel`，模型仍然是与语言无关的数据。

视图重新渲染时，`AreaLayout::new` 里的模型不会覆盖用户已经拖过的布局。要整体替换，调用 `set_area_layout`。

## 工作区

工作区就是几套保存好的 `AreaLayoutModel`，切换工作区时调用 `set_area_layout`。只要视图还声明着这些内容子节点，它们就始终是同一批节点：

- 两套布局里都有的区域，原地保留内容。
- 只在旧布局里出现的区域，内容隐藏起来等待。

所以滚动位置、输入中的文字等状态都不会丢。

## 和 Dock 怎么选

[Dock](dock.md) 是 IDE 式停靠：面板可以拖到边上或拖进 Tab 组，也可以浮出成独立窗口，一个位置可以叠放多个面板。`AreaLayout` 是 DCC 软件式的切分：

- 区域永不重叠，一个区域只显示一种编辑器。
- 改布局靠从角上切开或合并，不靠拖放面板。
- 区域头部由应用决定放什么。

面板数量多、需要 Tab 和浮动窗口时用 Dock。编辑器种类固定、要让用户像 Blender 那样随手切分屏幕时用 `AreaLayout`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `AreaLayout::new(model)` | `AreaLayoutModel` | 初始布局 |
| `.gap(px)` | `f32` | 区域间隙，也是分隔条的宽度，默认 2 |
| `.min_size(px)` | `f32` | 分隔条两侧、切开后两半的最小尺寸，默认 48 |
| `.chrome(外观)` | `AreaChrome` | 每个区域的外观：`frame_radius` 是区域框的圆角档（默认 `RadiusTier::Sm`），`header_background` 是头部底色角色（默认 `Subtle`，区域框本身是 `Surface`；要让头部成一条独立色带可以用 `Header`），`header_divider` 在头部和内容之间画一道细线（默认关）。加大间隙、头部改用 `Surface`、打开细线、圆角取 `Lg`，各区域就成了浮在背景上的独立面板 |
| `.labels([(编辑器, 名称)])` | `(String, Arc<str>)` | 各种编辑器的区域在读屏里的名称，也是头部下拉的选项名称，按编辑器类型 id；没给的用 id |
| `.editor_select(下拉)` | `AreaEditorSelect` | 每个区域头部最左边的编辑器类型下拉：`editors` 是选项（类型 id，按顺序），`label` 是读屏名称 |
| `.style(style)` | `NodeStyle` | 根节点样式，默认铺满父级 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `AreaLayoutChanged` | `change: AreaChange`, `model: AreaLayoutModel` | `Resized`、`Split`、`Joined`、`Maximized`、`Editor`、`Replaced` |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 键为区域 id 的子节点 | 该区域的内容 |
| 键为 `区域 id:header` 的子节点 | 该区域头部的内容 |
| 键为 `editor:类型` 的子节点 | 正在显示这种编辑器的一个区域的内容 |
| 键为 `editor:类型:header` 的子节点 | 同一个区域头部的内容，排在编辑器类型下拉后面 |

## 参见

[Dock](dock.md) · [Workspace](workspace.md) · [总览](index.md)
