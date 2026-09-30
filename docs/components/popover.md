# Popover

`Popover` 是挂在自己触发器上的一块表面。

按下、焦点、Enter、空格和面板锚点都是这个触发器。

它自己切换 `open`，不用 `activate_overlay`。

`Popover::new()` 的触发文字是空的，位置在下方，对齐居中。

`.trigger(text)` 写可访问名，也是纯文本触发器显示的字。

`.trigger_icon(icon, label)` 画成 28×28 的方形按钮，图标在按钮里居中，`label` 是可访问名。

裸符号不要塞进文本触发器。

要「图标 + 文字 + 计数」，用视图的 `.trigger(view)`，对应 `Popover::trigger_content`。

那段内容是触发器自己的子节点，面板关着也显示，不算面板条目。

里面只做显示，不要放可按的控件。

`.children` 才是面板里的条目。

`.placement` 用 `nana_ui::PopoverPlacement`：`Top`、`Bottom`、`Left`、`Right`，默认 `Bottom`。

`.alignment` 用 `PopoverAlignment`。

放不下首选一侧、对侧放得下时翻到对侧，再收进视口。

打开之后页面滚动，弹出层留在原处，直到下一次布局。

表面是 viewport-fixed，不进父级 isolation group，面板的 overflow 裁不到它。

面板因触发器、Escape、点外面或你把 `open` 写成 `false` 而关上时，焦点若还在面板条目上，回到触发器。

事件是 `PopoverToggled` 和 `PopoverClosed`。

控件表里没有 `<Popover>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ActionMenuItem, Popover};

view! {
    <Widget of={Popover::new().trigger("更多")}>
        <Widget of={ActionMenuItem::new("复制")} />
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{ActionMenuItem, Popover};

widget(Popover::new().trigger("更多")).children(widget(ActionMenuItem::new("复制")))
```

:::

`.close_on_escape` 和 `.close_on_outside` 默认都开着。

`.width` 有最小宽度，`.gap` 和 `.padding` 小于 0 时收成 0。

没有「任意节点当锚点」这一路，锚点就是触发器自己。

[总览](index.md) · [控件合同](../reference/components.md)
