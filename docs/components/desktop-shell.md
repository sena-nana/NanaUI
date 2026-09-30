# DesktopShell

`DesktopShell` 把标题栏、工作区、导航、检查器和两层浮层宿主装成一页桌面。

格子里的内容仍由你给。

不需要这套壳时，用 `List` 或 `AppShell`。

`DesktopShell::new()` 用一份新的 `WorkspaceModel`。

`DesktopShell::from_model(model)` 沿用你已经有的模型。

`.title(text)` 让装配在没有自带标题栏时建一条 `AppTitleBar`。

`.title_leading`、`.title_center`、`.title_trailing` 是标题栏里的槽。

`.title_center_width` 加宽中间那一列，面包屑需要比短标题更多的宽度。

`.title_window_controls(false)` 避免再挂一套只做展示的窗口按钮。

`.navigation`、`.navigation_footer`、`.primary`、`.inspector`、`.bottom` 放进对应区域。

检查器页签放 `.inspector`，不要放进 `RegionId::PrimaryToolbar`。

`.region(id, content)` 再加一块区域。

`.overlay` 推进对话框用的 `OverlayHost`，`.status` 推进 toast 用的第二层宿主，确认框开着时 toast 仍可显示。

主区域圆不圆角写在 `DesktopShell::workspace_corners`，默认是圆的。

装配时交给壳建出来的 `Workspace`。

不要去改框架建的那个 `Workspace` 节点，下一次装配会用壳上的值盖掉。

`assemble_desktop_shell` 是 `slot_assembler`。

视图提交时会跑。

手工放好槽之后要自己调一次。

每次写入不会自动重装。

控件表里没有 `<DesktopShell>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::DesktopShell;

view! {
    <Widget of={DesktopShell::new().title("Nana").workspace_corners(true)}>
        <template #navigation>
            <Column />
        </template>
        <template #primary>
            <Column />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::DesktopShell;

widget(DesktopShell::new().title("Nana").workspace_corners(true))
    .navigation(column())
    .primary(column())
```

:::

产品状态在 `nana_ui::WorkspaceModel` 和 `WorkspaceMutation`。

折叠、尺寸和可见性走模型。

`WorkspaceController` 只把指针和时钟转成 mutation。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
