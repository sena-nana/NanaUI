# DynamicForm

`DynamicForm` 用稳定的字段描述组装设置、扩展和请求表单。字段的 `id` 是应用拥有的键；同一键更新时保留控件、焦点和编辑历史，字段增删会移除旧节点。

```rust
use nana_ui::runtime::{DynamicForm, DynamicFormField, DynamicFormOption};

let form = DynamicForm::new([
    DynamicFormField::section("general", "常规"),
    DynamicFormField::field("name", "名称", "").binding_identity("project-name"),
    DynamicFormField::toggle("enabled", "启用", true),
    DynamicFormField::choice(
        "mode",
        "模式",
        "safe",
        [DynamicFormOption::new("safe", "安全")],
    ),
]);
```

`DynamicFormEvent` 只报告字段键和值：`Activate`、`Toggle`、`Select` 和 `Input`。应用负责把事件映射到产品状态、校验和持久化；组件不理解业务意图。
