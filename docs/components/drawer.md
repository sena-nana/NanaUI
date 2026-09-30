# Drawer

`Drawer` 是从窗口一侧推进来的模态表面。

无障碍角色是对话框。

标题、说明和槽里的内容由你给，框架负责把它放进视口，并处理 Escape 与点外面。

`Drawer::new(title)` 默认贴在右侧。

`.side` 用 `nana_ui::DrawerSide`：`Left`、`Right`、`Bottom`。

`.description` 写在标题下面。

`.close_policy` 接收 `nana_ui::DialogClosePolicy`。

`.initial_focus` 决定打开后焦点落在表面、第一个动作，还是你指定的节点。

视图槽是 `.body`、`.footer`、`.close_action`。

`assemble_modal_slots` 是 `slot_assembler`，视图提交时把这些槽放进表面。

手工建好之后要自己调用 `assemble_modal_slots`。

每次改标题不会自动重装。

打开用 `activate_overlay(host, drawer)`，抽屉必须是该 `OverlayHost` 的直接子节点。

关上用 `dismiss_overlay(host)`。

没有 `<Drawer>`，也没有 `open()`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::Drawer;
use nana_ui::DrawerSide;

view! {
    <Widget
        of={Drawer::new("筛选")
            .side(DrawerSide::Right)
            .description("只影响当前列表")}
    >
        <template #body>
            <Text>"按时间"</Text>
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{text, widget};
use nana_ui::runtime::Drawer;
use nana_ui::DrawerSide;

widget(
    Drawer::new("筛选")
        .side(DrawerSide::Right)
        .description("只影响当前列表"),
)
.body(text("按时间"))
```

:::

模态表面打开时，落在它上面的按下由表面接住。

底下侧栏、分栏和 Dock 手柄的那点容差，只对没被盖住的手柄生效。

关闭先停掉交互并恢复焦点，退出动画期间节点还在。

[总览](index.md) · [控件合同](../reference/components.md)
