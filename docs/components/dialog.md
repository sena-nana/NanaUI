# Dialog

标题加正文和底栏的浮层。只要确认和取消时，用 [ConfirmDialog](confirm-dialog.md)。

::: info
控件表里没有 `<Dialog>` 标签。两种写法都调用 `widget(Dialog::new(title))`，再用 `.body`、`.footer`、`.close_action` 放进已经建好的视图。
:::

## 基本用法

`Dialog::new` 接收标题。还可以接上 `.description`、`.size` 和 `.close_policy`。`.initial_focus_on(entity_ref)` 会在打开时，把焦点放到某个已经放进插槽的控件上。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Dialog;

widget(Dialog::new("重命名"))
    .body(view! {
        <Column gap=8>
            <TextInput placeholder="名称" v-model={name} />
        </Column>
    })
    .footer(view! {
        <Row gap=8>
            <Button @activate={cancel}>"取消"</Button>
            <Button @activate={save}>"保存"</Button>
        </Row>
    })
```

```rust rust
use nana_ui::runtime::view::{button, column, row, text_input, widget};
use nana_ui::runtime::Dialog;

widget(Dialog::new("重命名"))
    .body(
        column().gap(8).children((
            text_input().placeholder("名称").model(name),
        )),
    )
    .footer(
        row().gap(8).children((
            button("取消").on_activate(cancel),
            button("保存").on_activate(save),
        )),
    )
```

:::

插槽在对话框之前就建好。对话框自己的装配会把它们放进标题下面、底栏和关闭位。打开、关闭、逃出键和焦点归还走浮层的约定，见 [控件](../reference/components.md) 的浮层一节。

## 尺寸

宽度由对话框自己说。`.size` 收 `nana_ui::DialogSize`：五档预设 `Compact` 420、`Default` 520、`Medium` 600、`Wide` 680、`Workspace` 1080，或者 `DialogSize::Width(长度)` 给一个自己的宽度。长度按 CSS 理解：`LengthSpec::Px(560.0)` 就是 560；`DialogSize::capped(560.0, 92.0)` 是 CSS 的 `min(560px, 92vw)`，窗口窄了就让给窗口。`%` 和 `vw` / `vh` 相对遮罩，遮罩铺满窗口。无论哪种，卡片都不会超出遮罩留出的 16px 边距。

卡片站在哪、最高多高是设计系统的事，归主题：`ThemeDefinition::with_dialog(DialogRecipe { top, max_height })`。内置主题是距顶 90px、最高为遮罩高度的 76%。照 `margin-top: 12vh; max-height: 72vh` 写的设计，写成 `LengthSpec::Viewport { axis: ViewportAxis::Height, value: 12.0 }` 和 `value: 72.0`。卡片太高、站不下时往上挪，但不越过边距。装上新的配方，已经打开的对话框连同插槽一起重新摆放。

```rust
use nana_ui::DialogSize;
use nana_ui::runtime::{Dialog, LengthSpec, ViewportAxis};
use nana_ui::theme::{DialogRecipe, ThemeDefinition};

let theme = ThemeDefinition::NANA_DARK.with_dialog(DialogRecipe {
    top: LengthSpec::Viewport { axis: ViewportAxis::Height, value: 12.0 },
    max_height: LengthSpec::Viewport { axis: ViewportAxis::Height, value: 72.0 },
});
let export = Dialog::new("导出资源库").size(DialogSize::capped(560.0, 92.0));
```

## 口气与标题图标

`.danger(true)` 让对话框用危险口气说话：标题取主题 status 配方里危险的那个语义色（内置主题是 `Danger`），压过节点自己的文字色。正文、说明不变。[ConfirmDialog](confirm-dialog.md) 的 `danger` 是同一个口气，除了确认按钮走危险色，标题也一样变色。

`.title_icon(视图)` 在标题前放一个图标。标题行照 CSS 的 `display: flex; align-items: center` 排：`[图标] 标题 [关闭位]`，三者各占一格、在行里竖直居中，标题从图标后面开始，折行宽度让出图标和关闭位。行高取标题块和图标里高的那个；关闭位不撑高这一行，所以确认框忙碌时藏起关闭位，正文也不会跳。

行里的尺寸归主题的 `DialogRecipe`：`icon_size`（图标格，内置 16）、`close_size`（关闭位格，内置 28）、`header_gap`（三者之间的间距，内置 12）和 `header_min_height`（行的最小高度，内置 0）。照 `.dialog-card__header { align-items: center; gap: 8px }` 加一个 24px 关闭钮写的设计，写 `close_size: 24.0, header_gap: 8.0, header_min_height: 24.0`。

:::api

```rust view
use nana_ui::icons_tabler::ALERT_TRIANGLE;
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dialog, IconGlyph};

widget(Dialog::new("删除资源库").danger(true))
    .title_icon(view! { <Widget of={IconGlyph::new(ALERT_TRIANGLE)} /> })
    .body(view! { <Text>"资源库里的文件不会被删除。"</Text> })
```

```rust rust
use nana_ui::icons_tabler::ALERT_TRIANGLE;
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::{Dialog, IconGlyph};

widget(Dialog::new("删除资源库").danger(true))
    .title_icon(widget(IconGlyph::new(ALERT_TRIANGLE)))
    .body(text("资源库里的文件不会被删除。"))
```

:::

## 卡片的分区

卡片分三段：标题行、正文、底栏。每段的内边距、标题行下和底栏上的分隔线、卡片圆角都归主题的 `DialogRecipe`，对话框上不写：

| 配方字段 | 内置值 | 对应的 CSS |
| --- | --- | --- |
| `header: DialogInsets { top, bottom, inline }` | 14 / 8 / 16 | 标题行的 `padding` |
| `body: DialogInsets` | 8 / 10 / 16 | 正文的 `padding` |
| `body_bottom_alone: Option<f32>` | `Some(16.0)` | 没有底栏时正文的下内边距；`None` 照 CSS 沿用 `body.bottom` |
| `footer: DialogInsets` | 0 / 14 / 16 | 底栏的 `padding`；底栏的内容盒是一个按钮的高度 |
| `action_gap` | 8 | 底栏按钮之间的 `gap` |
| `header_divider` / `footer_divider: Option<SemanticColorRole>` | `None` | 标题行的 `border-bottom`、底栏的 `border-top`，粗细是主题的 `border.hairline`，算进所在那一段的高度 |
| `radius: RadiusTier` | `Md` | 卡片的 `border-radius` |

`.footer` 槽和确认框的按钮都放在底栏的内容盒里：分隔线和上下内边距之内，高一个按钮。抽屉不读这份配方，保留自己的排法。

照下面这份 CSS 写的设计：

```css
.dialog-card__header { padding: 12px 14px; border-bottom: 1px solid var(--border-soft); }
.dialog-card__body { padding: 12px 14px; }
.dialog-card__actions { gap: 8px; padding: 10px 14px; border-top: 1px solid var(--border-soft); }
.modal-card { border-radius: var(--radius-xl); }
```

在主题里写成：

```rust
use nana_ui::runtime::SemanticColorRole;
use nana_ui::theme::{DialogInsets, DialogRecipe, RadiusTier, ThemeDefinition};

let theme = ThemeDefinition::NANA_DARK.with_dialog(DialogRecipe {
    radius: RadiusTier::Xl,
    header: DialogInsets::symmetric(12.0, 14.0),
    body: DialogInsets::symmetric(12.0, 14.0),
    body_bottom_alone: None,
    footer: DialogInsets::symmetric(10.0, 14.0),
    action_gap: 8.0,
    header_divider: Some(SemanticColorRole::BorderSoft),
    footer_divider: Some(SemanticColorRole::BorderSoft),
    ..DialogRecipe::DEFAULT
});
```

## 遮罩

对话框和抽屉身后的遮罩归主题的效果 token：`EffectTokens::modal_scrim`（颜色和不透明度，内置是黑色 0.45）和 `modal_scrim_blur`（遮罩把身后的窗口模糊多少，CSS `backdrop-filter: blur(r)`，`r` 是高斯标准差，逻辑 px；内置 0，不模糊）。先模糊身后的窗口，再把遮罩的颜色叠上去，和 CSS 给元素的 `backdrop-filter` 与背景色的先后一样。

画家在线性光里合成：黑色 α 留下身后 `1 - α` 的光。浏览器的 `rgba(0, 0, 0, c)` 是在 sRGB 编码值上混的，留下 `1 - c` 的编码值，约等于 `(1 - c)^2.2` 的光。所以照 CSS 写的遮罩，不透明度要换算：`nana_ui::theme::linear_scrim_alpha(c)` 就是 `1 - (1 - c)^2.2`，CSS 的 0.45 是这里的约 0.73。

```rust
use nana_ui::theme::{EffectTokens, SemanticColor, ThemeDefinition, linear_scrim_alpha};

// .modal-overlay { background: rgba(0, 0, 0, .45); backdrop-filter: blur(2px) }
let mut effects = EffectTokens::DARK;
effects.modal_scrim = SemanticColor::rgba(0.0, 0.0, 0.0, linear_scrim_alpha(0.45));
effects.modal_scrim_blur = 2.0;
let theme = ThemeDefinition::NANA_DARK.with_effects(effects);
```

## 由应用决定开合

用户想关掉对话框有三种手势：按 Escape、在对话框外按下再松开、激活关闭位（`.close_action`）。每一种都先在对话框自己身上发一次 `DialogCloseRequested`，`trigger` 说是哪一种。然后才看 `.close_policy`：允许这个手势，框架接着关掉它，宿主随后发 `OverlayClosing`；不允许，它留着。Escape 和点外面只送到挂在 `OverlayHost` 下、用 `activate_overlay` 打开的对话框（浮层约定）；关闭位挂没挂都会发请求。

`DialogClosePolicy::requests_only()` 一种手势都不让框架关。开合全由应用决定：处理函数里要关，就改应用自己的打开状态（对话框随视图卸载），或者调 `dismiss_overlay(host)` 走退出动画；不关就什么都不做，比如先问一句有没有没保存的修改。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dialog, DialogCloseRequested};
use nana_ui::DialogClosePolicy;

widget(Dialog::new("重命名").close_policy(DialogClosePolicy::requests_only()))
    .on(move |_: &DialogCloseRequested| {
        if !dirty.get() {
            open.set(false);
        }
    })
    .body(view! {
        <TextInput placeholder="名称" v-model={name} />
    })
```

```rust rust
use nana_ui::runtime::view::{text_input, widget};
use nana_ui::runtime::{Dialog, DialogCloseRequested};
use nana_ui::DialogClosePolicy;

widget(Dialog::new("重命名").close_policy(DialogClosePolicy::requests_only()))
    .on(move |_: &DialogCloseRequested| {
        if !dirty.get() {
            open.set(false);
        }
    })
    .body(text_input().placeholder("名称").model(name))
```

:::

以前不允许的手势被悄悄吞掉：Escape 不再传给 `on_key`，点外面也没有事件，应用只能事后看对话框还在不在。现在不必轮询，听这个请求就行。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| 标题 | `Dialog::new` 的参数 | 构造时给出 |
| `.description` | 文本 | 标题下的说明 |
| `.size` | `nana_ui::DialogSize` | 卡片宽度：五档预设，或 `Width(长度)` / `capped(px, vw)`。距顶和最高高度归主题的 `DialogRecipe` |
| `.danger` | `bool` | 危险口气：标题取主题的危险语义色 |
| `.close_policy` | 关闭策略 | 哪些手势由框架直接关掉。`DialogClosePolicy::requests_only()` 一种都不关，全交给应用 |
| `.initial_focus_on` | `entity_ref` | 打开时把焦点放到已经放进插槽的控件上 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `.on(\|e: &DialogCloseRequested\| …)`，模板 `on:DialogCloseRequested` | `&DialogCloseRequested` | 用户按 Escape、点外面或激活关闭位，`trigger` 是 `Escape` / `Outside` / `CloseButton`。发在对话框自己身上，先于 `close_policy` |

打开、关闭和焦点归还走浮层约定。框架关掉对话框时，宿主发 `OverlayClosing { root }`。

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.title_icon` | 标题前的图标，和标题、关闭位在标题行里竖直居中 |
| `.body` | 标题下面的内容。调用前就要建好 |
| `.footer` | 底栏，通常是按钮行 |
| `.close_action` | 关闭位 |

确认框是 `ConfirmDialog`。它的槽位是 `.title_icon`、`.body`、`.cancel` 和 `.confirm`。你没有提供的按钮，会用 `cancel_label` 或 `confirm_label` 生成。

## 参见

[总览](index.md) · [ConfirmDialog](confirm-dialog.md) · [控件](../reference/components.md)
