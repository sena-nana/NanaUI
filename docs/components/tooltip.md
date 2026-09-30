# Tooltip

`Tooltip` 是一小块跟随指针或锚在触发器旁的说明。无障碍角色是 tooltip，标签就是它显示的文字。

## 基本用法

`Tooltip::new(label)` 使用 `nana_ui::TooltipConfig` 的默认值：位置 `FollowCursor`，延迟 350 毫秒，最大宽度 280。`Tooltip::with_config(label, config)` 换一套配置。

控件表里没有 `<Tooltip>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Tooltip;
use nana_ui::{TooltipConfig, TooltipPlacement};

view! {
    <Widget
        of={Tooltip::with_config(
            "保存当前文件",
            TooltipConfig {
                placement: TooltipPlacement::Top,
                ..TooltipConfig::default()
            },
        )}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Tooltip;
use nana_ui::{TooltipConfig, TooltipPlacement};

widget(Tooltip::with_config(
    "保存当前文件",
    TooltipConfig {
        placement: TooltipPlacement::Top,
        ..TooltipConfig::default()
    },
))
```

:::

`IconButton::with_tooltip` 用这份默认配置给按钮挂提示。你要改延迟或最大宽度时，自己构造 `TooltipConfig`，再交给会显示它的控件。

图表内部用的也是普通 tooltip 节点，离开、卸载或停放之后会关上。

## 位置和表面

`placement` 还可以是 `Top`、`Right`、`Bottom`、`Left`。方向性的位置锚在触发器上，首选的一侧放不下时才翻到对侧。

`gap`、`viewport_padding`、`max_width` 控制间距、视口留白和最大宽度。表面是固定定位，背景 `Surface`，边框 `BorderSoft`，字号和内边距来自 `TooltipConfig` 的常量。

`.style` 整份替换节点样式；依赖别的属性的构造要写在它后面。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `label` | — | `Tooltip::new` 或 `with_config` 的文字，也是无障碍标签 |
| `placement` | — | 默认 `FollowCursor`。还可以是 `Top`、`Right`、`Bottom`、`Left` |
| `gap` | — | 间距 |
| `viewport_padding` | — | 视口留白 |
| `max_width` | — | 最大宽度。`Tooltip::new` 的默认是 280 |
| 延迟 | — | `Tooltip::new` 的默认是 350 毫秒 |
| `.style` | — | 整份替换节点样式。依赖别的属性的构造要写在它后面 |

## 事件

没有事件。

## 插槽

没有插槽。显示的文字就是 `label`。

## 参见

[总览](index.md) · [控件](../reference/components.md)
