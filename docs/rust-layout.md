# 布局与样式（Rust 路径）

Rust 应用怎么排行与列、怎么写字段样式。Vue 兼容路径的 CSS 子集见[布局](layout.md)；这篇只讲 Rust 路径：不改 CSS，直接构造 `NodeStyle` 或用现成容器控件。

## 先用 `Stack`，不要手写布局字段

[`Stack`](../crates/nana-ui-runtime/src/view_components.rs) 是 L3 布局容器：预设覆盖常用 flex；grid / position / overflow / paint 写同一份 [`LayoutStyle`](../crates/nana-ui-core/src/box_layout.rs)（`with_layout` / `from_layout`）。不要另造布局控件。

常用入口仍是预设：

```rust
use nana_ui::runtime::{JustifySpec, SemanticColorRole, Stack};

// 工具条：水平、随内容收缩、子项垂直居中；默认起点对齐，右对齐写 justify
let toolbar = Stack::row(8.0).justify(JustifySpec::End);

// 主内容区：占满剩余高度
let body = Stack::fill_column(12.0);

// 带边框的卡片容器：一次写全背景、边框、圆角
let card = Stack::column(7.0)
    .padding(16.0)
    .surface(SemanticColorRole::Surface)
    .outline(SemanticColorRole::Border, 1.0)
    .radius(16.0);
```

子节点经 [`AppContext::build`](l3-authoring.md) 挂到容器下（`ui.column` / `ui.child`），容器只负责排列。不要给 `Stack` 加子节点字段。动态区用 `mount`。

| 预设 | 方向 | 尺寸 | 典型用途 |
| --- | --- | --- | --- |
| `Stack::row(gap)` | 水平 | 宽随内容 | 工具条、按钮组；默认起点对齐，`.justify(End)` 右对齐 |
| `Stack::fill_row(gap)` | 水平 | 占满剩余宽 | 整行分段 |
| `Stack::column(gap)` | 竖直 | 高随内容、宽占满 | 页面纵向结构 |
| `Stack::fill_column(gap)` | 竖直 | 占满剩余高 | 主内容区 |
| `Stack::bar(gap)` | 水平 | 占满整行不伸展 | 顶栏、底栏 |

`column` 与 `fill_column` 的区别最常见的出错点：主区没伸展、底部输入区不贴底，几乎都是该用 `fill_column` 的地方写成了 `column`。

需要 CSS 能表达的其余字段时，用 `with_layout` 写 `LayoutStyle`。Vue 解析结果用 `Stack::from_layout` 承接，不套用预设默认。

Rust 预设容器默认不参与命中测试；`from_layout`（Vue 布局盒）默认可点。语义容器（列表、表格）用 [`List`](components.md) / `Table`，`Stack` 只管排版。

## 样式：`NodeStyle` 与 builder

控件接受 `.style(NodeStyle)`。颜色一律用 [`SemanticColorRole`](../crates/nana-ui-core/src/semantics.rs) 语义角色，不要写裸色值（理由见[视觉](look.md)）：

```rust
use nana_ui::runtime::{NodeStyle, SemanticColorRole};

let style = NodeStyle::default()
    .surface(SemanticColorRole::Surface)   // 背景
    .outline(SemanticColorRole::Border, 1.0) // 边框：颜色 + 宽度一次写全
    .radius(8.0);
```

## 边框：颜色和宽度缺一不画

底层把边框拆成两处：颜色在 `NodeStyle.border`（语义角色）或 `LayoutStyle.border_color`，宽度在 `LayoutStyle.border_width`。**任意一边缺省，边框就完全不绘制**，编译期和运行期都不会警告。所以：

- 永远用 `NodeStyle::outline(role, width)` / `Stack::outline(role, width)` 一次写全，不要分别设置 `border` 和 `border_width`。
- 只想关掉边框时写 `border_width: Some(0.0)`，并清掉 `border` 与交互态（hover/focus）的边框角色。
- `Card` 这类自带视觉的控件走另一条路：用 `kind(CardKind::Outlined)` 拿 1px 描边，不要手动叠加。用户 `.style(...)` 显式给出的背景、边框、圆角优先于 `kind` 默认值。

## 与网页 CSS 的默认值差异

字段名对应 CSS，但默认值是 fail-closed 的，按 CSS initial 值推断会错：

- 未写 `flex-shrink` 时按 **0** 处理（CSS 是 1）：定宽行溢出时保留盒子，不会被压扁。需要收缩就显式 `shrink(1.0)`。
- `align_items` 默认 `Start`（CSS flex 是 `stretch`）：子项不会自动横向充满。`Stack` 预设已按用途选好，自写 `LayoutStyle` 时注意。
- `direction` 缺省是 `Column`：不给样式的容器子节点一个接一个竖排。要水平排列就用 `Stack::row`，不要指望默认。

## 浮层

对话框、菜单、抽屉、气泡必须用[控件](components.md)里的浮层（`Dialog`、`ActionMenu`、`Popover`、`Drawer`），锚定到触发控件的槽位；不要用绝对定位或 `fixed` 自己摆。
