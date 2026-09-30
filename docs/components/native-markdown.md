# NativeMarkdown

`NativeMarkdown` 把 CommonMark / GFM 源码解析成原生块，并在树上显示。

焦点在它上面时，复制取的是它的选区快照。

这个类型在 Cargo feature `rich-text` 后面。

`components` 会打开它。

`NativeMarkdown::new()` 是空文档。

`NativeMarkdown::from_source(source)` 和 `NativeMarkdown::parse(source)` 是同一条解析。

解析打开 GFM、表格、删除线和任务列表，也打开数学定界符。

`from_blocks` 直接接收已经分好的 `MarkdownBlock`。

`blocks()` 读解析结果。

mermaid 围栏和公式围栏会被解析，并给出 presenter 槽。

`MERMAID_PRESENTER` 是 `"mermaid"`，`MATH_PRESENTER` 是 `"math"`。

控件不渲染这两样，由宿主自己画进槽里。

`assemble_markdown` 是 `slot_assembler`。

视图提交时，以及响应式补丁之后，它会把块装配成子节点。

手工 `create_component` 放好之后要自己调一次。

改字段的那一次写入不会自动重装。

控件表里没有 `<NativeMarkdown>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::NativeMarkdown;

view! {
    <Widget of={NativeMarkdown::from_source("# 说明\n\n一段正文。")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::NativeMarkdown;

widget(NativeMarkdown::from_source("# 说明\n\n一段正文。"))
```

:::

选区挂在内部的 `TextSelectionGroup` 上，比较时按身份，不按选区内容。

源码保存在控件里，供你再次读取。

表格、删除线和任务列表走原生块；图和公式只留下槽位。

[总览](index.md) · [控件合同](../reference/components.md)
