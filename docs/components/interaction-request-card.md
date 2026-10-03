# InteractionRequestCard

`InteractionRequestCard` 是一个中性的请求表面：卡片标题来自 `title`，正文可以通过 `prompt` 和 `body` 槽提供，动作通过 `actions` 提供。它不保存业务状态，也不会替应用发出工作流或权限事件；宿主监听自己挂入的控件和按钮。

字段描述使用稳定的 `key`。每个字段给出宿主创建的控件节点，组件会自动创建并保留对应的 `FormField` 外壳。更新同一个 `key` 时，外壳和控件身份保持不变，因此输入焦点和编辑历史不会因为相邻字段变化而重建。

```rust
let input = cx
    .create_detached_component(document, TextInput::new(""))?;
let prompt = cx
    .create_detached_component(document, Text::new("请补充信息"))?;
let action = cx
    .create_detached_component(document, Button::new("继续"))?;
let card = cx.create_component(
    document,
    InteractionRequestCard::new("需要输入")
        .prompt(prompt.stable_id())
        .actions([action.stable_id()])
        .field(InteractionRequestField::new(
            "message",
            "消息",
            input.stable_id(),
        )),
)?;
cx.assemble_interaction_request_card(card)?;
```

字段的值、校验和动作由应用写回 `TextInput`、`TextArea`、`Select` 或其他控件；卡片只负责组合和保留这些节点。
