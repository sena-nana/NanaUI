# FindReplaceBar

`FindReplaceBar` 提供可折叠的查找和替换工具条。它只维护输入草稿、展开状态和只读门控；搜索文档、定位结果和修改内容由应用处理。

```rust
let bar = FindReplaceBar::new()
    .query(query)
    .read_only(read_only)
    .expanded(show_replace);
cx.create_component(document, bar)?;
```

调用 `assemble_find_replace_bar` 后，组件会保留查询、替换输入框和导航/替换按钮。用户输入和按钮激活通过 `FindReplaceEvent` 发给宿主。
