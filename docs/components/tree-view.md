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

## 多选

`.multiple(true)` 让树维护一个选中集合，以及集合里的一个活动项：

- 按下一行：只选它。
- Ctrl（macOS 上是 Cmd）+ 按下：把这一行加入或移出选中集合。
- Shift + 按下：选中从锚点到这一行之间的所有可见行。
- 焦点在树上时，Shift + 方向键 / Home / End 移动活动项并扩展范围。

每次选中集合变化都发 `TreeSelectionChanged { selected, active }`，`selected` 按树的顺序排列。单选时仍然发 `TreeViewEvent::Select`。`selected_ids` 读选中集合，`active_id` 读活动项。

## 开关列

`.columns([TreeToggleColumn::new(图标, 名称)])` 在每一行末尾画一列图标开关，例如可见、锁定。行的开关状态是 `TreeNode::toggles`：

- 开着的图标用次要文字色，关着的用更淡的颜色。
- 按下开关会翻转它，不改变选中。
- 按住后沿着这一列拖过其他行，会把每一行都设成同一个值。

每个变化发一次 `TreeToggleChanged { id, column, on }`。树自己更新节点上的状态；应用如果不接受，就把节点写回去。

某些行在某一列没有开关时（例如图层树里的分组行没有自己的可见性），用 `.hide_toggle(列)` 去掉那一格，`.hide_toggles()` 去掉这一行的全部开关。被去掉的格子不画、按不到；沿列拖过时跳过这一行，读屏也不报这个开关。列号保持不变，其他行照常。

## 拖放

`.reorderable(true)` 后可以把行拖到另一行上，落点分三种：

- 指针在目标行的上四分之一：`Before`，在行的上缘画一条线。
- 指针在下四分之一：`After`，在行的下缘画一条线。
- 指针在中间：`Inside`，给整行画框。

被拖的是选中集合（从选中的行开始拖时），否则只是这一行。目标是被拖行自己或它的子孙时不显示落点。松手发 `TreeDropRequested { rows, intent }`，树本身不移动任何节点，由应用决定接不接受。拖出树外再松手发 `TreeDragReleased { rows, x, y }`，`x`、`y` 是窗口坐标，应用可以用 `UiWorld::pointer_layout_position` 换算到别的控件里，把行放进另一个列表。不拖动、直接松开一个已选中的行，会只选中这一行。

## 行内重命名

`.renamable(true)` 后，双击一行或在焦点所在的树上按 F2，就会在标签位置打开一个输入框，选中原来的名字：

- Enter 或焦点离开树：发 `TreeRenameRequested { id, text }`，焦点回到树。
- Escape：取消，不发事件。

树不改节点的名字，由应用决定接不接受。`renaming` 读正在重命名的行。手工调用是 `begin_tree_rename` / `end_tree_rename`。

## 从列表拖入

从 `ReorderList` 拖到树上时，回传的是稳定节点，以及 `before`、`inside` 或 `after`。树不推断这是目录还是文件。

## 无障碍

树在读屏里是 Tree，它的名字来自 `.label(名称)`，例如「图层」「大纲」。不写就没有名字；树不会拿某一行的 id 或标签当名字。

每个可见行是树下面的一个 TreeItem。这些行不是保留节点，而是从可见行投影出来的虚拟节点，跟着展开和选中实时变化。每一行带：

- 名字：行的标签。
- 层级：顶层是 1，每深一层加 1。
- 展开：只有分支行有，表示展开或折叠。
- 选中：是否在选中集合里。
- 禁用：禁用节点的行是禁用的。

被折叠的子行不在无障碍树里，展开以后才出现。树有焦点时，读屏的焦点落在活动行上；没有活动行时落在树本身。

读屏可以对行做这些操作，结果和指针、键盘一样，事件也一样：

- 点击：和单击这一行相同，只选中它，并让树获得焦点。
- 聚焦：和方向键移到这一行相同，把活动行移到这里，并让树获得焦点。
- 展开 / 折叠：只对分支行，和点它的三角相同，发 `TreeViewEvent::Toggle`。

禁用的行、已经不存在的行，以及行做不了的操作（例如对叶子展开、对已展开的分支再展开），都会被拒绝，不发事件。

### 开关列

行尾的每个开关是这一行 TreeItem 下面的一个复选框（Checkbox），也是虚拟节点，按列的顺序排列。每个开关带：

- 名字：列名，也就是 `TreeToggleColumn::new(图标, 名称)` 的第二个参数，例如「显示」「锁定」。树不会拿图标当名字，所以列名要由应用给出，界面换语言时跟着换；列名为空时开关没有名字。
- 勾选：这一行这一列的开关状态；节点没写到这一列时是关着的。
- 禁用：禁用节点的行，它的开关也是禁用的。
- 边界：开关图标在行里的矩形。

开关状态变了（按下、拖过，或者应用把节点写回去），读屏马上看到新的状态。

读屏对开关只能点击：和按下这个开关相同，翻转它并发同样的 `TreeToggleChanged { id, column, on }`，树也同样更新节点上的状态。它不改变选中，也不移动焦点。禁用的行、已经不存在的行、树没有的列，以及点击以外的操作，都会被拒绝，不发事件。

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
| `.multiple(bool)` | `bool` | 多选：Ctrl 切换、Shift 范围、活动项 |
| `.columns(列)` | `TreeToggleColumn` | 行尾的开关列；状态写在 `TreeNode::toggles`。列名是读屏里每个开关的名字 |
| `.reorderable(bool)` | `bool` | 行可以拖到别的行之前、之内或之后 |
| `.renamable(bool)` | `bool` | F2 或双击时行内重命名 |
| `.label(名称)` | `Arc<str>` | 树在读屏里的名字；行用各自的标签 |
| `.style` | — | 整份替换样式，要放在会改布局的构造之前 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `TreeViewEvent::Toggle` | `id` | `apply_event` 只翻转分支 |
| `TreeViewEvent::Select` | `id` | 选中那个还没禁用的节点，并清掉其他选中 |
| `TreeSelectionChanged` | `selected`, `active` | 选中集合或活动项变了 |
| `TreeToggleChanged` | `id`, `column`, `on` | 一行的开关被设成 `on`：按下、拖过，或读屏点击开关 |
| `TreeDropRequested` | `rows`, `intent` | 拖放请求；`intent` 是目标与 `Before` / `Inside` / `After` |
| `TreeDragReleased` | `rows`, `x`, `y` | 行被拖出树外松手；`x`、`y` 是窗口坐标 |
| `TreeRenameRequested` | `id`, `text` | 重命名提交 |

## 插槽

没有具名插槽。节点来自 `TreeNode` 数据。

## 参见

[总览](index.md) · [控件](../reference/components.md)
