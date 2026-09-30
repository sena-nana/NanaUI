# TreeView

`TreeView` 把你给出的稳定节点、展开事实和层级画成 disclosure 行。它不读路径，也不监视文件系统。折叠起来的子节点不投影。

## 基本用法

`TreeView::new(nodes)` 接收 `TreeNode<Arc<str>>`。叶子用 `TreeNode::leaf(id, label)`，分支用 `TreeNode::branch(id, label, expanded, children)`。`.icon`、`.selected`、`.disabled` 写在节点上。

树的 `.size` 默认小号。`.style` 整份替换样式，要放在会改布局的构造之前。控件表里没有 `<TreeView>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{TreeNode, TreeView};

view! {
    <Widget
        of={TreeView::new([
            TreeNode::branch(
                "src".into(),
                "src",
                true,
                [TreeNode::leaf("main".into(), "main.rs").selected(true)],
            ),
            TreeNode::leaf("cargo".into(), "Cargo.toml"),
        ])}
    />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{TreeNode, TreeView};

widget(TreeView::new([
    TreeNode::branch(
        "src".into(),
        "src",
        true,
        [TreeNode::leaf("main".into(), "main.rs").selected(true)],
    ),
    TreeNode::leaf("cargo".into(), "Cargo.toml"),
]))
```

:::

默认宽度是填满，高度按可见行数和内边距算出。你自己写了高度时，这个高度会留下，方便放进固定视口。选中项的 id 用 `selected_id` 读。禁用节点不会被选中。

## 展开和选中

`apply_event` 由树自己改展开和选中：`Toggle` 只翻转分支，`Select` 选中那个还没禁用的节点，并清掉其他选中。

`navigate` 按 `TreeNavigation` 在可见行上移动，然后应用得到的事件。可见行从 `visible_rows` 读，深度记在 `TreeRowData` 里。

## 从列表拖入

从 `ReorderList` 拖到树上时，回传的是稳定节点，以及 `before`、`inside` 或 `after`。树不推断这是目录还是文件。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `nodes` | `TreeNode<Arc<str>>` | `TreeView::new(nodes)` 接收的稳定节点 |
| `TreeNode::leaf(id, label)` | — | 叶子 |
| `TreeNode::branch(id, label, expanded, children)` | — | 分支。`expanded` 是展开事实 |
| `.icon` | — | 写在节点上 |
| `.selected` | — | 写在节点上 |
| `.disabled` | — | 写在节点上。禁用节点不会被选中 |
| `.size` | — | 默认小号 |
| `.style` | — | 整份替换样式，要放在会改布局的构造之前 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `TreeViewEvent::Toggle` | `id` | `apply_event` 只翻转分支 |
| `TreeViewEvent::Select` | `id` | 选中那个还没禁用的节点，并清掉其他选中 |

## 插槽

没有具名插槽。节点来自 `TreeNode` 数据。

## 参见

[总览](index.md) · [控件](../reference/components.md)
