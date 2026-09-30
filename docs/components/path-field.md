# PathField

`PathField` 是路径文本加一个浏览按钮。

它是叶子复合件：写入属性时 `assemble_path_field` 自己建出 `TextInput` 和 `IconButton`，不用再记一次装配。

控件够不到父窗口，所以浏览不会打开系统对话框。

按钮激活时，只要字段没禁用，就发 `BrowseRequested`。

你再去开系统对话框，拿到路径后写回 `value`。

`PathField::new(value)` 接收当前路径。

默认无障碍名是「路径」，用 `.label` 替换。

`.placeholder`、`.size`、`.disabled`、`.invalid` 写在字段上，装配时同步到里面的输入框；禁用同时禁用浏览按钮。

输入框发出的 `TextChanged` 会写进 `value`，并原样再发出去。

子节点 `input` 和 `browse` 由装配填写。

你不要自己往这两个槽里塞控件。

控件表里没有 `<PathField>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::PathField;

view! {
    <Widget
        of={PathField::new("/tmp/project")
            .placeholder("选择目录")
            .label("工程路径")}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::PathField;

widget(
    PathField::new("/tmp/project")
        .placeholder("选择目录")
        .label("工程路径"),
)
```

:::

收到 `BrowseRequested` 之后，打开对话框、把选中的路径写回这个字段，都是你的事。

字段只负责把路径显示出来，并在文本改动时更新 `value`。

无效时边框用危险色，无障碍状态标 `invalid`。

浏览按钮是装配出来的 `IconButton`，图标是文件夹，可访问名是「浏览」。

禁用字段时，输入框和这个按钮一起禁用，按钮不再发 `BrowseRequested`。

`input` 和 `browse` 两个子节点由 `assemble_path_field` 填写。

你改 `value`、`placeholder` 或 `label` 时，这次写入会自己重建它们。

默认无障碍名是「路径」。

[总览](index.md) · [控件合同](../reference/components.md)
