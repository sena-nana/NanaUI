# SplitPane

`SplitPane` 是两块内容加一条 8px 的拖动手柄。尺寸来自 `nana_ui::SplitPaneModel`。你的内容留在 `first` 和 `second` 槽里。

## 基本用法

手柄不占内容的布局，几何只有一个写入者。控件表里没有 `<SplitPane>`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::SplitPane;
use nana_ui::{SplitAxis, SplitPaneModel};

let model = SplitPaneModel::new(SplitAxis::Horizontal, 240.0, 120.0, 800.0);
view! {
    <Widget of={SplitPane::new(&model).surface(nana_ui::runtime::SemanticColorRole::Surface)}>
        <template #first>
            <Column />
        </template>
        <template #second>
            <Column />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::SplitPane;
use nana_ui::{SplitAxis, SplitPaneModel};

let model = SplitPaneModel::new(SplitAxis::Horizontal, 240.0, 120.0, 800.0);
widget(SplitPane::new(&model).surface(nana_ui::runtime::SemanticColorRole::Surface))
    .first(column())
    .second(column())
```

:::

## 模型

`SplitPaneModel::new(axis, default_size, min_size, max_size)` 的 `axis` 是 `nana_ui::SplitAxis::Horizontal` 或 `Vertical`。

`SplitPane::new(model)` 先没有内容，视图再用 `.first(view)` 和 `.second(view)` 放进去。`SplitPane::from_model(model, first, second)` 直接带上两个节点 id。

`.apply(mutation)` 把 `SplitPaneMutation` 推进模型。拖动手柄的命中和键盘都写进模型。你要记住用户拖到的宽度，就读模型，再在下次 `from_model` 或 `apply` 时交回去。

## 表面和手柄

`.surface(role)` 写下语义背景，重投影时保留。`.handle` 可以换成你自己的手柄节点，它的第一个子节点是 2px 指示条。

## 装配

`assemble_split_pane` 是 `slot_assembler`。它给每个槽包一层壳，这样槽里的 `ScrollView` 继续投影自己的节点，不会和分栏抢尺寸。视图提交时会跑。手工放好两块内容之后要自己调一次。每次改模型不会自动重装。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `axis` | `nana_ui::SplitAxis` | `Horizontal` 或 `Vertical`。`SplitPaneModel::new` 的第一个参数 |
| `default_size` | — | `SplitPaneModel::new` 的参数 |
| `min_size` | — | `SplitPaneModel::new` 的参数 |
| `max_size` | — | `SplitPaneModel::new` 的参数 |
| `.surface` | — | `role`。语义背景，重投影时保留 |
| `.apply` | `SplitPaneMutation` | 把 mutation 推进模型 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| 无 | 无 | 拖动手柄的命中和键盘都写进模型 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `.first` | 第一块内容。`new` 时还没有。模板 `#first`。`from_model` 直接带节点 id |
| `.second` | 第二块内容。模板 `#second` |
| `.handle` | 可以换成你自己的手柄节点，它的第一个子节点是 2px 指示条 |

## 参见

[总览](index.md) · [控件](../reference/components.md) · [工作区](../reference/workspace.md)
