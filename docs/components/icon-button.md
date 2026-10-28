# IconButton

`icon_button(icon, label)` 创建一个图标按钮。`icon` 是 `Icon`，`label` 是可访问名，类型在字段上是 `Arc<str>`。

模板里的标签是 `<IconButton>`。`icon` 必须写成属性。`label` 用属性或这一条子文本。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Icon;

view! {
    <IconButton icon={Icon::Search} disabled={pending} @activate={open_search}>
        "搜索"
    </IconButton>
}
```

```rust rust
use nana_ui::runtime::view::icon_button;
use nana_ui::runtime::Icon;

icon_button(Icon::Search, "搜索")
    .disabled(pending)
    .on_activate(open_search)
```

:::

`@activate={open_search}` 在函数上直接传入。表达式展开成 `.on_activate(move || { … })`。`disabled` 为真时不发 `Activate`。

## 图标

图标用目录常量，例如 `Icon::Search`、`Icon::Add`、`Icon::Close`、`Icon::Settings`。`Icon` 是指向静态几何的 `Copy` 身份。`Icon::parse_name` 只解析壳层名字（`search`、`settings`、`close` 以及它们的别名）。`Icon::Puzzle` 这类目录常量在这里返回 `None`，避免一张名字表把没用到的几何留住。没有从任意字符串构造的 `from_str`。

## 选中

`selected` 由应用写。选中时字形和底色走选中态，控件不会因为点了一下就自己选上。

## 悬停气泡

悬停气泡不是 `label`。可访问名是 `label`。气泡用组件上的 `IconButton::with_tooltip`，配置是默认的 `TooltipConfig`。这不是字段表里的方法。标题行里的工具用 `IconButton::tool(icon, label)`：紧凑尺寸，标签同时作可访问名和气泡。

## 尺寸

默认是方的：边长取主题的 `icon_button_size`（28），`IconButton::size` 换成控件高度档。这是没人给尺寸时的方盒（`NodeStyle::square`）。应用自己写了宽或高（`style.layout.width` / `height`，或模板 CSS 的 `width` / `height`），那一轴就照写的来，不必再把 `square` 置空：22×22 的关闭钮就是 22×22。只写了宽的按钮，高仍是方盒的边长。装别的主题密度时，变的只是方盒，不动应用写的尺寸。

字形大小默认跟控件尺寸档走：取该档的正文字号，`compact` 时取中号字号。`IconButton::icon_size(px)` 直接指定字形边长（逻辑像素），按钮的方盒不变。密集主题里小号按钮的字形太小时用它，让字形更醒目而按钮不变大。非有限值或负值会被忽略，和 `Button::icon_size` 一样。

## 种类

构造时的种类是 `ButtonKind::Ghost`。`selected` 为真，或种类本身是 Selected，绘制都走选中色。要改种类用 `IconButton::kind`。`colors_from_style` 让这一颗按钮改用自己 `style` 里的颜色，不再被种类盖掉；只影响设了它的那一颗。

停用的样子跟着颜色走。按种类配色的按钮停用时整枚降到 50% 不透明度：种类给的图标平时颜色和停用色很接近，淡出才看得出哪枚不能点。`colors_from_style` 的按钮停用时照写的 `style.interaction.disabled` 画，不再淡，节点上写的不透明度也照写的来；要淡就在那里写，例如 `SemanticColorMix::alpha` 或按底色混好的 `SemanticColorMix::new`。用自绘（`Painter`）按 `PaintState::disabled` 画停用态的按钮同样设 `colors_from_style`，否则框架会在画好的停用色上再淡一层。

## 属性

字段是 `icon: Icon`、`label: Arc<str>`、`selected: bool`、`disabled: bool`、`has_popup: bool`。没有 `model`。激活不自己改 `selected`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `icon` | `Icon` | 必须写成属性。指向静态几何的 `Copy` 身份 |
| `label` | `Arc<str>` | 可访问名。用属性或这一条子文本 |
| `selected` | `bool` | 由应用写。激活不自己改 `selected`。选中时字形和底色走选中态 |
| `icon_size` | `Option<f32>` | 字形边长（逻辑像素），盖过尺寸档给的字号；不动方盒。默认 `None` |
| `disabled` | `bool` | 为真时不发 `Activate`。按种类配色的按钮整枚降到 50% 不透明度，`colors_from_style` 的按钮照 `style.interaction.disabled` 画、不淡，见上文「种类」 |
| `has_popup` | `bool` | 弹出菜单的按钮（`aria-haspopup="menu"`）：读屏报成菜单按钮；焦点在它上面时，ArrowUp / ArrowDown 和 `ContextMenu` 键一样发 `keyboard: true` 的 `SecondaryPress`，由你打开菜单。见 [控件](../reference/components.md) 的菜单按钮 |

## 事件

事件是 `on_activate`，类型为 `Activate`。处理器不接收参数。

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_activate`，模板 `@activate` | 不接收参数 | 类型是 `Activate`。函数直接传入。表达式展开成 `.on_activate(move \|\| { … })`。`disabled` 为真时不发 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](index.md) · [控件](../reference/components.md)
