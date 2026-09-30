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

悬停气泡不是 `label`。可访问名是 `label`。气泡用组件上的 `IconButton::with_tooltip`，配置是默认的 `TooltipConfig`。这不是字段表里的方法。

## 种类

构造时的种类是 `ButtonKind::Ghost`。`selected` 为真，或种类本身是 Selected，绘制都走选中色。要改种类用 `IconButton::kind`。`colors_from_style` 让这一颗按钮改用自己 `style` 里的颜色，不再被种类盖掉；只影响设了它的那一颗。

## 属性

字段是 `icon: Icon`、`label: Arc<str>`、`selected: bool`、`disabled: bool`。没有 `model`。激活不自己改 `selected`。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `icon` | `Icon` | 必须写成属性。指向静态几何的 `Copy` 身份 |
| `label` | `Arc<str>` | 可访问名。用属性或这一条子文本 |
| `selected` | `bool` | 由应用写。激活不自己改 `selected`。选中时字形和底色走选中态 |
| `disabled` | `bool` | 为真时不发 `Activate` |

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
