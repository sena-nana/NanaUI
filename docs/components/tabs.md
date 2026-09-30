# Tabs

`Tabs` 是独立的页签条。

选择、重排、关闭请求和跨条拖动都从这里报出来。

页签的值和顺序仍由你保存；重排成功时，控件会把自己留着的选项顺序改到和新顺序一致。

关闭只是请求，控件不删页签。

`Tabs::new(selected)` 接收当前选中的值。

选项是 `TabOption::new(value, label)`，还可以 `.icon`、`.disabled`、`.draggable` 和 `.closable`。

`.closable(true)` 只表示你接受关闭请求，运行时不画关闭钮。

`.draggable(false)` 的页签仍能选中，只是不能当拖动源或「放在它前面」的目标。

条自己还有 `.label`、`.size`、`.fill`、`.strip_id` 和 `.accepts_external_drop`。

默认接受别的条拖进来；`accepts_external_drop(false)` 仍能拖出，只拒绝外部落入。

方向键只移焦点，不改选中。

事件是 `TabsEvent`：`Select`、`Reorder { value, before }`、`Close`、`Transfer { source_strip, value, target_strip, before }`。

`before` 为 `None` 表示放到末尾。

`reorder` 用「被移动的值 + 它后面的值」描述结果。

`transfer_to` 只报告，不改任何一条。

跨窗口拖页签用 `TabDragGroup` /`TabDragSurface`，选择和顺序仍是你的。

控件表里没有 `<Tabs>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{TabOption, Tabs};

view! {
    <Widget
        of={Tabs::new("main")
            .strip_id("editor")
            .label("文件")
            .options([
                TabOption::new("main", "main.rs").closable(true),
                TabOption::new("lib", "lib.rs").closable(true),
            ])}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{TabOption, Tabs};

widget(
    Tabs::new("main")
        .strip_id("editor")
        .label("文件")
        .options([
            TabOption::new("main", "main.rs").closable(true),
            TabOption::new("lib", "lib.rs").closable(true),
        ]),
)
```

:::

`request_close` 在页签禁用或不可关闭时返回 `None`。

你决定关不关，关了再从 `options` 里拿掉。

`Dock` 不会替这条页签做跨窗口语义。

`Reorder` 的 `before` 是移动之后紧挨在后面的那个值。

放到末尾时它是 `None`。

控件随后把留着的 `options` 排成这个顺序。

跨条的 `Transfer` 只报告来源条、值和目标位置，两条上的选项列表都要你自己改。

`strip_id` 用来区分同一窗口里的多条页签。

拖出窗口时，`TabDragGroup` 和 `TabDragSurface` 负责把这次拖动交给另一侧；选中哪一条、最终顺序，仍写在你的状态里。

[总览](index.md) · [控件合同](../reference/components.md)
