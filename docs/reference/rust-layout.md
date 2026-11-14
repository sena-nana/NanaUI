# 布局与样式（Rust 路径）

本文说明在 Rust L3 路径中如何排列容器、设置语义样式、控制边框与处理容器内边距。如果你使用 `.vue` 或构建时 `<style>` / `stylesheet!` 编写 CSS，支持的 CSS 属性子集请查阅 [布局（CSS 子集）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/layout.md)。

## 优先使用 `Stack` 预设，避免手写底层布局字段

[`Stack`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-runtime/src/view_components.rs) 是 Rust 路径的标准布局容器。常用排列直接使用预设构造器（声明式视图中对应 `<Column>` / `<Row>` 或 `column()` / `row()`）；需要 Grid、定位、溢出或绘制属性时，通过 `with_layout` 或 `from_layout` 写入同一份 [`LayoutStyle`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/box_layout.rs)，无需另外定义新的布局组件。

Row/Column 预设直接写入 `display: flex` 与方向，进入同一个 Flex formatting context；它们不建立另一种布局身份。`from_layout` 保留传入的 display，Grid 也只通过同一份布局意图表达。Rust、`view!` 和构建期 `.vue` 方言共用这条 Runtime 路径；比较等价 CSS 时，需同时写明预设的尺寸与对齐默认值。

写法只是拼法。预设构造器、`view!` 的 `<Row>` / `<Column>`、带预设意图的 `from_layout`，以及从空布局起写的 CSS，表达的是同一份意图，得到同一个 context、参与方式和布局结果；宽松与窄于内容的两种可用宽度都一样（`tests/view_macro.rs` 的 `issue204_row_column_and_css_share_structured_layout_results`）。

- Row/Column 预设用 `LengthSpec::Shrink` 定尺寸，CSS 写作 `width: shrink` / `height: shrink`（NanaUI 扩展，与 `fill` 对称）。`fit-content` 是另一种长度：可用宽度比内容窄时，它会夹到可用宽度，`shrink` 不会。
- `Stack::min_width(0)` 和 CSS 的 `min-width: 0` 一样，会记下 `allow_shrink`。这样同一个声明不论怎么写，得到的都是同一份意图。

定位和滚动同样只是 Foundation 能力的投影（`issue204_position_and_scroll_css_and_rust_share_structured_results`）。在 ScrollView、Rust 里写了 `overflow-y: scroll` 的普通盒子、CSS 写的同一个盒子这三种滚动容器里，各放一个 sticky 头、一个 absolute 角标和一段正文，三种写法得到的意图和结果都相同。结果比较的是参与方式、包含块、clip、滚动范围和 fragments。ScrollView 只是写入 overflow，不另外带一套布局。

- CSS 写 `top`、`padding-left` 这类物理边时，会额外记一条物理边记录，用来和 `inset-inline-start` 这类逻辑边排先后。
- 同一组里没有逻辑边时，这条记录不起作用。所以 CSS 补丁全部套完后（`view/style.rs` 的 `apply`），会用 `LayoutStyle::drop_inert_physical_edges` 丢掉它，让 CSS 和 Rust 写同一条边时得到同一份意图。
- 同一组里有逻辑边时，记录保留，层叠先后不变。

预设构造器、视图函数 `row()` 和带预设意图的 `from_layout` 挂上同一种组件、同样的节点和意图，首帧、局部修改和空闲帧的布局工作量相同（`world/tests/layout_foundations.rs` 的 `issue204_facades_share_identity_and_layout_work`）。CSS 在构建期降为 `from_layout` 加样式数据，走同一条路径。写法之间唯一有意的差别在命中：Rust 预设默认不参与命中，`from_layout` 和 CSS 布局盒默认可点。

```rust
use nana_ui::runtime::{JustifySpec, SemanticColorRole, Stack};

// 工具条：水平排列、宽度随内容收缩、子项垂直居中；默认起点对齐，右对齐用 JustifySpec::End
let toolbar = Stack::row(8.0).justify(JustifySpec::End);

// 窄宽度下自动换行的操作区
let actions = Stack::bar(8.0).wrap(true);

// 主内容区：占满父容器剩余高度
let body = Stack::fill_column(12.0);

// 带描边的卡片容器：一次性设置背景、边框角色与线宽、圆角
let card = Stack::column(7.0)
    .padding(16.0)
    .surface(SemanticColorRole::Surface)
    .border(SemanticColorRole::Border, 1.0)
    .radius(16.0);
```

`with_layout` 闭包拿到的 `&mut LayoutStyle` 上有几个就地写的简写，写的仍是 CSS 作者会写的那几个字段：`set_absolute_inset(px)`（`position: absolute` 加四边 `inset`）、`set_absolute_fill()`（四边为 0，盖住包含块）、`set_fixed_width(px)` / `set_fixed_height(px)`（尺寸与最小、最大同值，不随内容或 flex 伸缩）。它们返回 `&mut LayoutStyle`，可以连写。手上是 `NodeStyle` 时用 `NodeStyle::layout_mut()` 取可写布局（共享的默认布局会先复制，不会被写穿），不必自己 `Arc::make_mut`。

布局结果 `LayoutBox` 带 `right()`、`bottom()`、`union`、`intersection`（不相交为 `None`）和 `inset(amount)`（四边内收，收过头时在中点缩成 0），算浮层、安全区和命中范围时直接用。

子节点在声明式视图中通过 `.children((...))` 或 `.with(|c| c.add(...))` 挂入容器（见 [L3：用 Rust 构建保留式界面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/l3-authoring.md)）。`Stack` 只负责纯排版；具备列表或表格语义的容器请使用 [`List`](https://github.com/sena-nana/NanaUI/blob/main/docs/components/list.md) 或 [`Table`](https://github.com/sena-nana/NanaUI/blob/main/docs/components/table.md)。

| 预设构造器 | 主轴方向 | 尺寸行为 | 典型用途 |
| --- | --- | --- | --- |
| `Stack::row(gap)` | 水平 | 宽度随内容收缩，交叉轴居中 | 工具条、按钮组（默认起点对齐，`.justify(End)` 右对齐） |
| `Stack::fill_row(gap)` | 水平 | 占满父级剩余宽度 | 整行多栏分段 |
| `Stack::column(gap)` | 垂直 | 高度随内容收缩，宽度占满父级 | 页面纵向堆叠 |
| `Stack::fill_column(gap)` | 垂直 | 占满父级剩余高度与宽度 | 主内容区、带贴底操作栏的面板 |
| `Stack::bar(gap)` | 水平 | 占满整行宽度，子项不自动拉伸 | 顶栏、底栏、状态条 |

::: tip 常见排错：`column` 与 `fill_column`
如果发现主内容区没有撑开、或者底部状态栏/输入框没有贴到窗口底边，几乎都是因为本该使用 `Stack::fill_column` 的容器写成了随内容收缩的 `Stack::column`。
:::

Rust 预设的 `Stack` 容器默认不参与指针命中测试（`pointer_events: false`），不会拦截下层点击；而承接已解析 `LayoutStyle` 的 `Stack::from_layout` 默认可命中。

## 节点样式：`NodeStyle` 与边框规则

控件通过 `.style(NodeStyle)` 接收自定义样式。颜色一律使用 [`SemanticColorRole`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/semantics.rs) 语义角色，切勿硬编码裸 RGBA 值（原因见 [视觉与主题规范](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/look.md)）：

```rust
use nana_ui::runtime::{NodeStyle, SemanticColorRole};

let style = NodeStyle::default()
    .surface(SemanticColorRole::Surface)      // 背景表面色
    .border(SemanticColorRole::Border, 1.0) // 边框：颜色角色 + 线宽一次写全
    .radius(8.0);
```

### 边框必须同时具备颜色与宽度

底层将边框拆分为两处存储：颜色位于 `NodeStyle.border`（语义角色）或 `LayoutStyle.border_color`，线宽位于 `LayoutStyle.border_width`。**只要任意一方缺省，边框就完全不会绘制**（编译期与运行期均不会报警）。因此：

- **绘制边框**：始终调用 `NodeStyle::border(role, width)` 或 `Stack::border(role, width)` 一次性写全颜色与线宽，不要只改其中一个字段。
- **关闭边框**：将 `border_width` 设为 `Some(0.0)`，同时清空 `border` 以及交互态（hover / focus）的边框角色。
- **边框占布局空间**：与 CSS `border` 一样，边框夹在盒子边缘与内边距之间，内容框因它收窄，随内容定高的盒子因它变高。所以选中、聚焦这类状态不要改边框宽度，否则内容会跟着挪动；只换颜色，或者改用外环。
- **自带外观的卡片（`Card`）**：需要 `1 px` 描边时直接使用 `Card::new().kind(CardKind::Outlined)`，不必手动叠加 `NodeStyle`。通过 `.style(...)` 显式指定的背景、边框和圆角优先于 `CardKind` 的默认值。

### 外环：不占布局的状态描边（`ring`）

`NodeStyle::ring(role, width)` / `Stack::ring(role, width)` 画外环，对应 CSS `outline`：描在边框盒外侧并跟随圆角，**不占布局空间**，加上、加粗或去掉都不会挪动任何东西。颜色同样是语义角色（存于 `NodeStyle.ring`，随主题解析），线宽存于 `LayoutStyle.paint.outline`；宽度为 `0` 即去掉外环。

```rust
// 选中时在 1 px 边框外再加一圈强调色外环，布局不变
let card = Stack::column(0.0)
    .border(SemanticColorRole::BorderSoft, 1.0)
    .ring(SemanticColorRole::Accent, if selected { 1.0 } else { 0.0 });
```

外环画在盒子外面，父级裁剪（`overflow: hidden`）或紧贴的兄弟节点会盖住它，留出与线宽相当的间距。

### 随主题解析的语义混色（`SemanticColorMix`）

当需要在两个语义色之间按比例混合、或只降低背景/边框自身的不透明度时，使用 `SemanticColorMix`：

- `SemanticColorMix::new(first_role, second_role, first_weight)`：在预乘 alpha 的 sRGB 空间对当前主题的两种语义色做混合（`first_weight` 取值 `0.0..=1.0`，精度至万分之一；切换主题时自动按新调色板重新解析）。
- `SemanticColorMix::alpha(role, alpha)`：仅降低该表面或边框颜色自身的 alpha，**不会降低后代文本的不透明度**。
- 通过 `NodeStyle::surface_mix(mix)` 与 `border_mix(mix, width)` 设置基态混色（与 `surface` / `border` 后调用者生效；`Stack` 上同名方法直接可用）；在 `bind` 闭包里改容器布局用 `Stack::layout_mut()`，不必整个替换容器；`InteractionStyle` 的各状态（`base` → `selected` → `hovered` → `pressed` → `focused` → `disabled`）可按优先级覆盖基态。

## 与 Web CSS 默认值的关键差异

`LayoutStyle` 的字段命名对齐 CSS，但其缺省值是**显式收敛（fail-closed）**的，直接套用浏览器 CSS 的 initial 值假设会出错：

1. **`flex-shrink` 缺省为 `0`（浏览器 CSS 为 `1`）**：定宽行溢出时子项保持原有尺寸，不会被静默压扁；需要随容器收缩时必须显式调用 `.shrink(1.0)`。
2. **`align-items` 缺省为 `Start`（浏览器 CSS 为 `stretch`）**：子项不会自动沿交叉轴拉伸铺满（`Stack` 预设已按常见用途配好对齐方式，手动构造 `LayoutStyle` 时需留意）。块容器例外：不写 `display`（`LayoutStyle::default()`、`NodeStyle::default()`）、写 `display: Block` 或 `FlowRoot`、不沿行排的容器按 CSS 块布局排，没有 `align-items`，`auto` 宽的块级子项和 CSS 一样撑满包含块（写了 `align-self` 或自己宽度的子项、图片等替换内容、书写模式正交的子项除外）。要子项按内容收缩或按 `align_items` 居中，把容器写成 `display: Flex`（`Stack` 预设都是），或给子项写 `align-self` / `width: Shrink`。
3. **`flex-direction` 缺省为 `Column`（浏览器 CSS 为 `row`）**：未指定方向的容器默认自上而下竖排，需要水平排列请显式使用 `Stack::row`。

与 CSS Flexbox 保持一致的关键行为：一行中的子项会按该行实际分配给它的**主轴使用尺寸（used main size）**去排版内部内容并测量交叉轴高度。例如当 `Fill` 列在定宽侧栏旁被收窄时，其内部的 16:9 缩略图与自动换行文本都会按收窄后的宽度重新算出真实高度，绝不会残留按整窗宽度量出的旧高度（除非子项自身显式锁死了交叉轴 `height` 或自带 `aspect_ratio`）。

## 浮层定位与容器内边距归属

模态对话框、下拉菜单、抽屉与气泡提示必须使用内置浮层控件（`Dialog`、`ActionMenu`、`Popover`、`Drawer`）并锚定到触发槽位，**不要手写 `position: absolute` 或 `fixed` 去拼装浮层**。框架内部的 `PositionSpec::Fixed` 以窗口视口为坐标基准，脱离外层滚动、变换与裁剪链，但仍保留在原 Runtime 树中并遵守结构性透明度与叠放顺序。

各层容器的内边距职责划分如下（框架不会根据嵌套层级自动清零内边距，兄弟间距优先使用父容器的 `gap`）：

| 容器 | 默认间距职责 |
| --- | --- |
| `AppShell` / `Workspace` | 管理区域切分与窗口 chrome，不替业务内容添加页面留白 |
| `Stack`（`Column` / `Row`） | 仅负责子项排列与显式 `gap`，默认 `padding` 为 `0` |
| `Card` / `SettingsCard` | 负责卡片内部留白（左右 `16 px`、上下 `14 px`），不附带外部 `margin` |
| `SettingsPage` | 负责内部可滚动正文的页面留白（上 `20`、右 `24`、下 `24`、左 `24 px`，标题与内容 `gap` 为 `16 px`） |

```rust
use nana_ui::runtime::{Card, Stack};
use nana_ui_core::PaddingSpec;

// 1. 页面中排列多张卡片：用外层 Stack::column(16.0) 控制卡片间距，Card 自带内部留白
let sections = Stack::column(16.0);
let card = Card::new();

// 2. 卡片内排列字段：Card 已提供内边距，内部 Stack 只管 gap，不再叠第二层 padding
let fields = Stack::column(8.0);

// 3. 贴边列表：哪一层容器需要贴边，就显式将该层的 padding 设为 0
let flush_card = Card::new().padding(0.0);
let flush_page = page.content_padding(PaddingSpec::uniform(0.0));
```
