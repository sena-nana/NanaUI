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

## 边距归属与覆盖

| 容器 | 默认责任 |
| --- | --- |
| Shell / Workspace | 区域、分隔及窗口 chrome，不替业务内容添加页面留白 |
| Stack | 排列和显式 gap，默认 padding 为零 |
| Card / SettingsCard | 内部留白：左右 16、上下 14；不附带外部间距 |
| SettingsPage | 滚动 body：上 20、右 24、下 24、左 24；标题与内容间 gap 为 16 |

页面留白和卡片内部留白是两个不同边界，可以同时存在。框架不会根据嵌套层级自动清零。兄弟间距优先由父级 gap 负责；显式 margin 与 gap 相加，不自动折叠。页标题由 `SettingsPage` 的 tab label 负责；`AppearanceSection` / `AboutSection` 作为 page content 时投影无标题卡片，避免卡片 title 再加 24px 顶边并与页标题重复。需要分区名时才给 `SettingsCard` 设 title。

- `Stack::padding(v)` / `padding_xy(x, y)`、`Card::padding(v)` / `padding_xy(x, y)` 覆盖四边，包括此前的逻辑边声明；后调用者生效，`padding(0.0)` 可明确贴边。
- 原始 `LayoutStyle` 仍是声明式：分边覆盖统一 padding。Card 未声明的边回落默认值，显式零保留；`.style(NodeStyle)` 替换用户声明，但不再隐式取消卡片默认内边距。默认值只存在于投影，删除覆盖可恢复默认。
- `SettingsPage::content_padding(PaddingSpec)` 和 `content_gap(f32)` 只控制内部滚动 body。省略时用标准值；移除覆盖可将对应公开字段恢复为 `None`。full-page Tab 直接承载业务内容，不创建滚动 body，这两个设置也不作用于该模式。

四类组合（挂载仍通过 `ui.nest` / `append_child`）：

```rust
use nana_ui::runtime::{Card, Stack};
use nana_ui_core::PaddingSpec;

// 页面中放卡片：SettingsPage(content) → Stack → Card。
let sections = Stack::column(16.0); // 多张卡片之间的间隔
let card = Card::new();           // 卡片内部使用标准留白

// 卡片内排列控件：Card → Stack → 控件，不再手写第二层 padding。
let fields = Stack::column(8.0);

// 贴边列表：页面或卡片谁负责该边界，就清零谁。
let flush_card = Card::new().padding(0.0);
let flush_page = page.content_padding(PaddingSpec::uniform(0.0));

// 滚动到底：内容放进 SettingsPage.content；底部留白由 body 计算，
// 不再添加末尾 Spacer，也不在页面外壳重复加 padding。
```

### 旧用法迁移

原来依赖 `.padding_xy(...).padding(0)` 未生效的布局，应直接保留需要的最终 padding；原来借 Card `.style(...)` 清零的地方，改为显式 `.padding(0.0)`。SettingsCard 不再默认附带 12px 底部 margin；多卡片页面使用父级 `Stack::column(gap)`，需要保留特定外边距时显式声明。修复后的容器量测计入子项 margin；百分比 padding 四边均相对包含块宽度，Grid 项相对最终单元格。
