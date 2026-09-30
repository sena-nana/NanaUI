# TextArea

`text_area()` 创建多行输入。模板里的标签是 `<TextArea>`。它没有构造参数，初始文本走 `value`。

字段是 `value: String`、`label: Option<Arc<str>>`、`placeholder: Arc<str>`、`disabled: bool`、`read_only: bool`。事件是 `on_input`，类型为 `TextChanged`。处理器接收 `&TextChanged`。没有 `on_submit`。`model` 绑定 `value`，把 `event.value` 转成 `String` 写回。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <TextArea label="备注" placeholder="可选" v-model={notes} />
}
```

```rust rust
use nana_ui::runtime::view::text_area;

text_area().label("备注").placeholder("可选").model(notes)
```

:::

`v-model={notes}` 展开成 `.model(notes)`。`notes` 是 `Signal<String>`。`value` 按文本状态写：写回去的字符串和当前一样时，不替换文本，光标和选区留在原地。

`.on_input(|event: &TextChanged| …)` 自己听输入。模板里是 `on:TextChanged={|event: &TextChanged| …}`。事件里还有选区。

`read_only` 保留焦点、光标、选区、查找和复制，拒绝修改、替换、剪切和粘贴。运行时改成只读会结束还没提交的输入法组合，并拒绝这次之前开始的文本拖放。程序仍可以写入新的权威文本。`disabled` 不参与这些交互。显示只读文档用 `read_only`，不要用禁用样式代替。

行号、诊断、minimap、git gutter 在组件 `TextArea` 上：`line_numbers`、`diagnostics`、`minimap`、`git_gutter`。它们不在 `<TextArea>` 的字段表里。buffer revision、LSP 和 git 状态仍由应用喂。

可选 feature `syntax-highlighting` 在同一块上启用名为 `"highlight"` 的 presenter，方法是 `highlight(language)`，不另造一套编辑器。

编辑器的文本走 `commit_editor_edit`。键入、删除、粘贴都从那里进撤销。不要自己写 `state.value`，那样不会进撤销。文档 revision、冲突和持久化仍归应用。

`@input={save}` 在 `save` 是函数时原样传入，参数必须是 `&TextChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

和 `TextInput` 不同，多行没有 `on_submit`。回车是换行，不是提交。

[总览](index.md) 和 [控件合同](../reference/components.md)
