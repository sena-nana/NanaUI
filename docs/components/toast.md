# Toast

一条被动的通知：标题、可选的说明，和说它的口气（`ToastTone`：Info / Success / Warning / Danger）。`Toast::new(title, tone)` 创建它。控件表里没有 `<Toast>` 标签，两种写法都调用 `widget(Toast::new(..))`。外观归主题的 `ToastRecipe`，见 [主题](../reference/theme.md)。

一条提示有两种形式。裸提示是一个画好的节点，标题和说明各一行。[`AppContext::assemble_toast`](#复合形式) 把它永久换成由子节点组成的复合形式：文案折行，多出进度条、一个动作按钮、可按的关闭按钮和自己的图标。通知角用 [ToastStack](toast-stack.md)，它装配放进去的每一条。

## 基本用法

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Toast, ToastTone};

view! {
    <Widget of={Toast::new("已保存", ToastTone::Success).description("项目已导出")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Toast, ToastTone};

widget(Toast::new("已保存", ToastTone::Success).description("项目已导出"))
```

:::

`description("")` 等于没有说明。`dismissible(true)` 让裸提示成为可按、可聚焦的关闭目标。裸提示不读 `progress`、`action`、`icon`、`close_label`，这几样只在装配后出现。

## 放置与定时

`Toast::place_in(viewport, align, max_width, insets)` 用与 `Panel::viewport` 相同的预留合同，把提示钉进应用留出的空闲区域，贴住预留的底边，见 [控件](../reference/components.md)。放置过的提示保留 `place_in` 给的宽度。

`Toast::timeout(Duration)` 让提示在屏上满这么久后发一次 `ToastDismissed`，和按下关闭按钮的事件相同。计时器不逐帧推进，只在到期时唤醒宿主。框架不自己移除提示，收到事件后由应用拿掉。

## 复合形式

```rust
use nana_ui::runtime::{Toast, ToastAction, ToastDismissed, ToastProgress, ToastTone};

let toast = cx.create_component(
    document,
    Toast::new("正在优化模型", ToastTone::Info)
        .description("2 / 5")
        .progress(ToastProgress::Determinate(0.4))
        .action("查看")
        .dismissible(true),
)?;
cx.assemble_toast(toast)?;
cx.on(toast, |_, _: &ToastAction, _| { /* 打开详情 */ })?;
cx.on(toast, |_, _: &ToastDismissed, _| { /* 拿掉这条 */ })?;
```

装配是永久的，再调一次什么都不做（返回 `false`）。之后对提示的每次写入都由它的装配器原地对上子节点：进度一变，挪的是同一根进度条，没有重建。整值改写（`*toast = Toast::new(..)`）也不会丢掉装配。

装配后的提示排成一行：

- **标色调的记号**：配方的点或字形，画在左侧留出的方块里，站在标题的第一行，而不是多行提示的正中。配方用字形标色调（`ToastIndicator::Glyph`）时，`Toast::icon` 换掉色调的字形；配方用点或不标时，照配方。
- **文案列**：标题按配方的 `title_max_lines` 折行截断，说明按 `description_max_lines`，`0` 不截断。
- **百分比**：`ToastProgress::Determinate` 且配方的 `progress.show_percent` 为真时，标题旁按框架文案 `toast.percent`（`{percent}%`）写出来，等宽数字，宽度按 `100%` 量好，数字变长时标题不跳。
- **进度条**：在文案列底部，粗细、轨道、填充色和上外边距（`progress.top_gap`）读配方。`ToastProgress::Indeterminate` 时扫动，减弱动态效果时不扫。确定值夹在 `0.0..=1.0`。
- **动作按钮**：`Toast::action(label)` 给一个，种类和档位读配方的 `action_kind`、`control_size`，圆角读 `control_radius`（默认 `RadiusTier::Sm`）。按整条提示的内容高度垂直居中，说明换行和进度条都计入高度；单行提示不因按钮而增高。按下在提示上发 `ToastAction`。
- **关闭按钮**：`dismissible` 时出现，与动作按钮同样垂直居中，`control_size` 档、文本种类，圆角同样读 `control_radius`，按下发 `ToastDismissed`。可访问名是 `close_label`，不给时是框架文案 `toast.close`（「关闭通知」）。

两个按钮按完整内容高度垂直居中，不把一行的提示撑高。主题可将 `control_radius` 设为 `RadiusTier::Xl` 等档位，让文字操作呈胶囊、方形关闭按钮的反馈呈圆形；圆角随当前主题度量解析，切换主题时原地更新。提示的宽度随内容（CSS `width: fit-content`），放不下时文案折行；显示进度时至少为配方的 `progress.min_width`（默认 240），进度条不跟着文案伸缩。放置过的提示保留 `place_in` 给的宽度。

装配后的提示在指针悬停或焦点在里面时暂停计时，离开后从下一帧起按剩余时间继续。放在 [ToastStack](toast-stack.md) 里时，由栈决定什么时候计时。

## 布局

裸提示的记号、标题、说明和关闭目标是布局发布的内容部件（#205），没有子节点：配方的 padding 从边框盒向里留出一条带子，带子里是一行 Flex（记号 `LeadingContent`、文案列、关闭目标 `TrailingContent`），文案列再按 Flex 列放标题（`TextContent`，一行高为 `title.size`）和说明（`Detail`）。有说明时文案列靠带子顶部，没有时与记号、关闭目标一样在带子里垂直居中。提示的高度由测量这份声明给出（带子的块高与一行标题取大），样式不再写 `min_height`；`width: fit-content` 等按内容的宽度把记号、间隔和关闭目标算进去。标题是一行，按它在记号与关闭目标之间的那条宽度整形，放不下时截断加省略号，不再按整个内容宽折行把提示撑高。绘制只读发布的部件。

装配后的提示只发布记号这一个部件：布局把记号和它后面的间隔排在文案子节点之前、配方的 padding 里面，记号站在标题第一行的中线上；padding 只是配方的那份。

## 无障碍

提示是以标题为名、说明为描述的 Status。裸提示和装配后的一样。装配后的提示还是一个 live region：普通口气是 polite，危险是 assertive。原地改标题（比如任务完成）由平台的 live region 播报一次。文案列里的文字不重复朗读；进度条以标题为名、不单独播报变化；两个按钮可以 Tab 到。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `title` | `Arc<str>` | 构造参数 |
| `tone` | `ToastTone` | 构造参数。决定色调记号的颜色，危险时 assertive |
| `description` | `Option<Arc<str>>` | 空串等于没有 |
| `dismissible` | `bool` | 裸提示：可按的关闭目标；装配后：关闭按钮 |
| `timeout` | `Option<Duration>` | 在屏上满这么久后发一次 `ToastDismissed`。不给就一直等 |
| `progress` | `Option<ToastProgress>` | 装配后才有。`Determinate(0.0..=1.0)` 或 `Indeterminate` |
| `action` | `Option<Arc<str>>` | 装配后才有。一个动作按钮的文字，空串等于没有 |
| `icon` | `Option<Icon>` | 装配后才有。配方用字形标色调时换掉色调的字形 |
| `close_label` | `Option<Arc<str>>` | 装配后才有。关闭按钮的可访问名，不给时是 `toast.close` |
| `style` | `NodeStyle` | 外框的样式；`place_in` 只写其中的位置和宽度 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `.on(\|_: &ToastDismissed\| …)` | `&ToastDismissed` | 关闭按钮被按下，或 `timeout` 到了（只发一次）。移除由应用做 |
| `.on(\|_: &ToastAction\| …)` | `&ToastAction` | 装配后的动作按钮被按下 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 部件由 `assemble_toast` 建好，不接受应用的子节点 |

## 参见

[总览](index.md) · [ToastStack](toast-stack.md) · [控件](../reference/components.md) · [框架文案](../reference/framework-strings.md)
