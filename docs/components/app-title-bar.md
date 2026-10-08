# AppTitleBar

`AppTitleBar` 是窗口顶部那一条标题。高度是 `TITLE_BAR_HEIGHT`。它负责标题文字、左中右三个槽，以及窗口按钮占位。

控件表里没有 `<AppTitleBar>`。

## 基本用法

`AppTitleBar::new(title)` 默认可拖动，不透明，显示窗口按钮。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::AppTitleBar;

view! {
    <Widget of={AppTitleBar::new("Nana").drag_enabled(true)}>
        <template #trailing>
            <Button>"分享"</Button>
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{button, widget};
use nana_ui::runtime::AppTitleBar;

widget(AppTitleBar::new("Nana").drag_enabled(true)).trailing(button("分享"))
```

:::

## 拖动

拖动窗口从条上不接交互的区域开始。

`.drag_enabled(false)` 关掉从这条开始的窗口拖动。

## 窗口按钮

平台默认用系统按钮时，`native_controls` 为真，控件槽是留给系统按钮的空位；否则槽里是自定义的最小化、最大化和关闭。

`.show_window_controls` 和 `.native_controls` 分开：原生占位即使先隐藏也会装上，方便以后再显示时有盒子可跟。

`.maximized` 交给窗口按钮的外观。

窗口按钮若已经由宿主绑定，把壳的 `title_window_controls` 或这条的 `show_window_controls` 按你的按钮方案关掉，避免两套按钮。

## 外观

`.transparent(true)` 只去掉标题栏背景。

`.center_width` 是中间标题列的宽度。

### 叠在媒体上

`.over_media(true)` 给叠在视频上的标题栏，含 `.transparent(true)`。处理和图片查看器看图一样，不随主题：

- 栏后面是一道从上到下的渐变，从主题的 `EffectTokens::media_scrim` 淡到透明。
- 标题和自绘的窗口按钮用主题的 `EffectTokens::media_foreground`（角色 `SemanticColorRole::OnMedia`），两种主题都是白色。
- 按钮悬停与按下是同色的半透明底；关闭钮悬停是实心的 `Danger` 底，图标仍是浅色。

浅色主题下也一样，深色和花哨的画面上都看得清。槽里应用自己的控件不受影响。

## 装配

`assemble_app_title_bar` 是 `slot_assembler`，把三列和窗口按钮装回去。

视图提交时会跑。

手工建好槽之后要自己调一次。

改标题的那一次写入不会自动重装。

## 和壳

`DesktopShell::title` 在你没自带标题栏时，会建一条这样的栏。

壳上的 `workspace_corners` 不写在标题栏上。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `AppTitleBar::new(title)` | — | 默认可拖动，不透明，显示窗口按钮 |
| `.transparent` | — | `.transparent(true)` 只去掉标题栏背景 |
| `.over_media` | — | 叠在媒体上：媒体遮罩渐变垫底，标题和窗口按钮用浅色前景，两种主题一样。含 `.transparent(true)` |
| `.drag_enabled` | — | `.drag_enabled(false)` 关掉从这条开始的窗口拖动 |
| `.show_window_controls` | — | 和 `.native_controls` 分开。宿主已经绑了窗口按钮时，按你的按钮方案关掉，避免两套按钮 |
| `.native_controls` | — | 平台默认用系统按钮时为真，控件槽是留给系统按钮的空位；否则槽里是自定义的最小化、最大化和关闭。原生占位即使先隐藏也会装上，方便以后再显示时有盒子可跟 |
| `.maximized` | — | 交给窗口按钮的外观 |
| `.center_width` | — | 中间标题列的宽度 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 无 |

## 插槽

视图槽是 `.leading`、`.center`、`.trailing`。

| 插槽 | 说明 |
| --- | --- |
| `.leading` | 视图槽 |
| `.center` | 视图槽。`.center_width` 是中间标题列的宽度 |
| `.trailing` | 视图槽 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
