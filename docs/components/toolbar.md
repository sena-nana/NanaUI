# Toolbar

`Toolbar` 是内容上方的一条横条。

里面的按钮由你放。

相对 `Stack::bar`，它多了壳层表面，以及无障碍角色 `Toolbar`：读屏把里面的控件念成一组。

`Toolbar::new()` 默认画表面，底边一条发丝边框。

`.label` 是可访问名，一个窗口里有多条工具栏时靠它区分。

`.chrome(false)` 关掉表面和边框，用在父级已经有表面的时候。

条本身不接指针，点击落在子节点上。

没有装配函数，子节点就是内容。

控件表里没有 `<Toolbar>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Toolbar;

view! {
    <Widget of={Toolbar::new().label("主工具栏")}>
        <Button>"保存"</Button>
        <Button>"运行"</Button>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{button, widget};
use nana_ui::runtime::Toolbar;

widget(Toolbar::new().label("主工具栏")).children((button("保存"), button("运行")))
```

:::

检查器页签不要塞进主工具栏区域。

`DesktopShell` 里，文档和编辑类动作可以放进 `RegionId::PrimaryToolbar`，检查器走 `.inspector`。

条的高度随内容和内边距，不跟标题栏抢 `TITLE_BAR_HEIGHT`。

条是横向排列，子项居中。

间距用 `space::SM`，左右内边距是 `space::MD`，上下是 `space::XS`。

`chrome` 打开时背景是 `Surface`，只有底边画发丝线，颜色是 `BorderSoft`。

保存、运行这类动作是你放进去的按钮。

条不解释按钮的含义，只把它们放在同一条可访问的工具栏里。

[总览](index.md) · [控件合同](../reference/components.md) · [工作区](../reference/workspace.md)
