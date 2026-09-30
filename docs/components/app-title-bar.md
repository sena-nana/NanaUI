# AppTitleBar

`AppTitleBar` 是窗口顶部那一条标题。

高度是 `TITLE_BAR_HEIGHT`。

它负责标题文字、左中右三个槽，以及窗口按钮占位。

拖动窗口从条上不接交互的区域开始。

`AppTitleBar::new(title)` 默认可拖动，不透明，显示窗口按钮。

平台默认用系统按钮时，`native_controls` 为真，控件槽是留给系统按钮的空位；否则槽里是自定义的最小化、最大化和关闭。

`.transparent(true)` 只去掉标题栏背景。

`.drag_enabled(false)` 关掉从这条开始的窗口拖动。

`.show_window_controls` 和 `.native_controls` 分开：原生占位即使先隐藏也会装上，方便以后再显示时有盒子可跟。

`.maximized` 交给窗口按钮的外观。

`.center_width` 是中间标题列的宽度。

视图槽是 `.leading`、`.center`、`.trailing`。

`assemble_app_title_bar` 是 `slot_assembler`，把三列和窗口按钮装回去。

视图提交时会跑。

手工建好槽之后要自己调一次。

改标题的那一次写入不会自动重装。

控件表里没有 `<AppTitleBar>`。

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

`DesktopShell::title` 在你没自带标题栏时，会建一条这样的栏。

壳上的 `workspace_corners` 不写在标题栏上。

窗口按钮若已经由宿主绑定，把壳的 `title_window_controls` 或这条的 `show_window_controls` 按你的按钮方案关掉，避免两套按钮。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
