# List

`List` 是一组条目的容器。它的无障碍角色是列表，读屏按列表来念。里面放什么条目由你决定。

条目控件是 `ListItem`，那一页在总览里单独说明。

控件表里没有 `<List>`。

## 基本用法

`List::new()` 没有子节点。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::List;

view! {
    <Widget of={List::new().label("最近文件")}>
        <ListItem>"main.rs"</ListItem>
        <ListItem>"lib.rs"</ListItem>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{list_item, widget};
use nana_ui::runtime::List;

widget(List::new().label("最近文件")).children((
    list_item("main.rs"),
    list_item("lib.rs"),
))
```

:::

`list_item` 是条目的视图函数，标签既是构造参数，也是显示文字。

选中、禁用和行尾补充写在条目上，也可以由列表统一发出选择请求。

## 点击与选择

列表默认不接指针，也不改变焦点的普通 Tab 顺序；需要由应用管理单选/多选时，打开 `List::multiselectable(true)` 或监听 `ListSelectionRequested`（`each_virtual` 使用 `.multiselectable(...)` 与 `.on_select(...)`）：

- 在可交互的 `ListItem`、`InteractiveCard` 或带 `.aria_selected(...)` 的自定义条目卡片上按下时，发出 `ListSelectionRequested { item, gesture: SelectionGesture::Press(modifiers), step: None }`（虚拟列表上 `on_select` 接收解析出业务键的 `VirtualSelectionRequest<K>`）。
- 焦点在列表条目内时，方向键（一维 `ArrowUp`/`ArrowDown`，`grid_columns > 1` 的网格同时支持 `ArrowLeft`/`ArrowRight` 按行跨列步进）、`Home` / `End`、`PageUp` / `PageDown` 发出 `Move`，带 `Shift` 从锚点扩展，`Ctrl+Shift` 追加一段，`Ctrl+Space` 发 `Toggle`，`Ctrl+A` 发 `SelectAll`。
- 自定义卡片需要接收指针和键盘焦点时，在构建节点上调用 `.hittable().focusable().aria_selected(...)`；CSS 中的 `:selected` 伪类会随选中状态自动同步。
- 虚拟列表把业务选择状态保留在 `SelectionState<K>` 中（滚动移出视口的键同样保留），用 `VirtualListRef::reveal_and_focus(&key)` 滚到并聚焦导航落点。

## 工作区

不需要 IDE 式工作区时，一列 `List` 或一个 `AppShell` 就够。

区域、停靠和拆分留给 `Workspace`。

## 样式与长度

列表不排序，也不虚拟化；长列表外面套 `ScrollView`，或者用虚拟列表自己管理窗口。

默认样式是一份空的 `NodeStyle`。

你要间距、内边距或背景，写在 `.style` 里，或者把列表放进 `Stack::column`。

## 可访问名

列表的可访问名来自 `.label`；不写时，读屏只按角色念「列表」。

## 属性

`.label` 给整张列表一个可访问名。

`.style` 整份替换节点样式。

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `.label` | — | 整张列表的可访问名。不写时，读屏只按角色念「列表」 |
| `.multiselectable` | `bool` | 打开多选列表语义，向辅助技术声明并启用 `ListSelectionRequested` |
| `.grid_columns` | `usize` | 网格列数（大于 1 时开启二维方向键跨列/跨行导航） |
| `.style` | — | 整份替换节点样式。默认样式是一份空的 `NodeStyle` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `ListSelectionRequested` | `{ item, gesture, step }` | 开启选择的列表或 `each_virtual` 在条目按下/选择键时发出 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| 无 | 无 |

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
