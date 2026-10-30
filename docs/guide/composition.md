# 版式与组合

控件页说明每个控件自己怎么用；这一页说明它们拼在一起时的规则：谁管水平内边距、标题行多高、行尾控件对着哪一行、提示用哪一层。这些规则跨应用通用，NanaUI 的默认值按它们设定，消费方不需要再各写一套补偿。

每条规则后面写了对应的接口。没有接口的地方说明了该怎么写。

## 内容线

一个容器里，文字和控件的左右边缘对齐在同一条竖线上，叫内容线。

- **只有一层负责水平内边距。** 卡片（面板、弹出卡）给了内边距，里面的分组、行、滚动区就不再加；嵌套时外层给了，内层就是 0。两层都加会让内层内容比标题缩进，两层都不加会贴边。
- `settings_group` 的卡片和 `group_row` 都**不**加水平内边距，行间细线缩进 `CONTENT_INSET`（`SpacingStep::Xxxl`）。行要放到内容线上：用 `group_row_on_line(view)`，或 `group_row(content_line(view))`。自己的行构造器已经带了这层内边距时，直接 `group_row(view)`，不要再包一次。
- `settings_row` 自己没有水平内边距，放进卡片或分组时同样要上内容线。
- `Card::new()` 默认带 `SurfacePadding::Panel`。把 `CardKind::Flat` 当分组容器、里面的行自己上内容线时，写 `.padding(0.0)`，否则内容比细线多缩进一层。
- 列表行（`ListItem`）的高亮底板要比文字宽、文字仍在内容线上时，用 `.pill_bleed(true)`：底板外扩一个行内边距，文字不动。

## 分组

| 要放的内容 | 用 | 不要用 |
| --- | --- | --- |
| 设置页的一组同类设置（标题 + 多行 + 行间细线） | `settings_group(title, rows)` | 手搭 `Card` + `Divider` |
| 一块自由内容（说明、图表、按钮排） | `Card`（`Surface` / `Raised`），保留它的内边距 | `settings_group` |
| 已在 Runtime 层组织行和分隔线 | `SettingsGroup`（`settings_group` 的卡片里就是它） | 自己判断哪条分隔线该隐藏 |

- 分组之间的距离由放分组的那一列的 `gap` 给，分组本身不带外边距。
- 隐藏的行用 `group_row(..).visible(cond)`：它会带走自己的分隔线，第一行隐藏时也一样。不要只隐藏行里的内容，那样会留下空行和多余的细线。
- 分组只靠标题和卡片区分，不另加装饰线、色块或图标。

## 标题行

- 标题行用 `heading_row(title, tools)`：标题先让出宽度并省略，工具保持宽度。标题前的图标或返回箭头放 `.lead(view)`，这样有没有前导图标，标题的起点都一致。
- **行高固定为文字的行盒。** 工具用 `heading_tools(..)` 包：紧凑工具（`IconButton::tool`、紧凑菜单，目标 `COMPACT_TARGET`，默认 24px）比行盒高，它们按 `heading_overhang()` 上下各悬出一半，不撑高标题行。所以有工具和没工具的标题同高，同一页的标题对齐。
- 标题行上不是工具但同样高的内容（名称旁的状态胶囊）用 `heading_hang(view)`，同样悬出。
- 悬出量按主题的行盒（`LineRole::Line`）算，适用于 `Meta` 到 `Body` 字号的标题。标题本身已经比目标高时，用 `heading_tools(..).inline()`，工具留在行内。
- **悬出的部分不能被裁掉。** 标题在滚动区顶部时，滚动区要向上留出同样的距离：`ScrollView::heading_room()`。
- 工具是对整组的操作；推进任务的动作用文字按钮，放在内容下面，不放进标题工具。

## 列表行

`ListItem` 有三种状态，按「这一行现在能不能做事」选：

| 状态 | 什么时候 | 接口 |
| --- | --- | --- |
| 可交互 | 点它、按 Enter 会做事（打开、选中、切换） | 默认 |
| 只读 | 只是展示一条信息，从来不能点 | `.read_only(true)`：不响应悬停和点击，文字仍是正常颜色 |
| 禁用 | 本来能做，现在因为某个条件不能做 | `.disabled(true)`：变灰；同时要让用户知道为什么（行内说明或提示） |

- 只读不要写成禁用：禁用的灰色说的是「暂时不行」，用户会去找怎么启用它。
- **多行的行，行首行尾对着第一行。** 名称下面带一行说明时，行尾的开关或按钮要和名称那一行对齐，而不是对着两行整体居中。`ListItem::auto_height(true)` 的行默认就是这样（`TrailingAlign::FirstLine`）；其他行可以写 `.trailing_align(TrailingAlign::FirstLine)`。这时行把第一行垫到单行行的位置，行首行尾放在同样高的带子里居中，单行行和多行行混排时行尾仍在一条线上。先设好 `auto_height` 或 `trailing_align`，再给行首行尾。
- 单行行保持默认的居中。

## 状态层级

从轻到重：

1. **行内说明**：`callout(..).quiet()`，没有底色、不带水平内边距、字小一号，放在已有内容线的块里当一行说明。
2. **提示条**：`callout(icon, tone, text)`，按 `StatusTone` 着色的底色条，需要用户注意但不打断操作的情况（配置不完整、功能受限）。行尾可以放一个去处理的链接或按钮（`.trailing(view)`）。
3. **通知**：`Toast` / `ToastStack`，操作的结果（完成、失败），随时间消失，不占内容位置。
4. **对话框**：`ConfirmDialog`，必须先回答才能继续（删除确认等不可撤销的操作）。

- 同一件事只用一层。不要在提示条之外再弹通知，也不要给每行都加一条提示条。
- **空列表用 `EmptyState`，不用列表行。** 把「没有项目」写成一行列表行，会被画成可点或禁用的样子。`EmptyState` 是居中的图标、标题、说明和可选的一个动作。
- 失败显示在出错的地方：属于某个字段的放在字段下面（`ValidationMessage`），属于整个操作的走通知。

## 窄容器

- `SegmentedControl` 在给它的宽度放不下一行时，默认换行（`wrap` 默认开），选项文字不会被截断或溢出卡片；宽度够时保持一行和自己的宽度。确实需要固定一行的，写 `.wrap(false)`，并保证它的容器足够宽。
- `.fill(true)` 让分段控件占满宽度，换行时每行平分。
- 按钮排用会换行的行（`flex-wrap: wrap`），不要假定一行放得下。
- 标题和行里的文字先让出宽度并省略，控件保持宽度。

## 图标与文字对齐

- 图标和文字并排时，图标居中对齐文字的行盒，不要按像素手动下移。
- 中日韩文字在粗体、较大字号时墨迹偏低，看起来比图标低一点。这是字体的墨迹位置，不是布局错误；目前没有按墨迹居中的对齐模式（见下文「还没有的接口」）。

## 只写档位

- 间距写 `SpacingStep`（`gap_step`、`padding_step`、`space::*`），字号写 `TypeRole`，圆角写 `RadiusTier`，线宽写 `BorderWidth`，控件高度写 `ControlSize`。不要写数字：档位跟着安装的主题和密度变，数字不会。
- 卡片或对话框自己的名字用 `TypeRole::CardTitle`，配 `TextWeight::Bold`。
- 只有接口只收像素时才写数字，并从档位常量取（`space::XXXL`）。这些数字只在默认度量下成立：主题改了间距，档位跟着变，数字不变。能写档位就写档位：细线缩进用 `Divider::inset_step(CONTENT_INSET)`，不用 `inset(content_inset())`——`content_inset()` 只是默认度量下的像素值。
- 仓库自带的 `scripts/audit-theme-hardcoding.py` 也能扫消费方的源码，见下文。

## 滚动区

- **滚动条跑在边距里，不压在内容上。** 卡片有水平内边距时，滚动区用 `ScrollView::gutter(px)` 伸进这段边距（`px` 取卡片的内边距），内容再缩回来：内容仍在卡片的内容线上，滚动条画在空着的边距里；边距放不下主题的滚动条车道时，车道收窄。
- **底部渐隐**：`scroll_foot_fade(height, surface)` 放在装滚动区的定位容器里，盖在滚动区底边上，说明列表还没到底。滚动区用了 `gutter` 时，渐隐写 `.clear_of(px)`（同一个值）：它和滚动区放在同一个定位容器里（卡片内容框中的一列，左右边就是内容线），左边跟着滚动区伸进边距，右边停在滚动条车道之前。列表末尾留出 `height` 的空白，最后一行能滚出渐隐。
- 顶部有标题工具时加 `.heading_room()`（见「标题行」）。

```rust
use nana_ui::runtime::view::{scroll_foot_fade, widget};
use nana_ui::runtime::{ScrollAxes, ScrollView, SemanticColorRole, Stack};
use nana_ui::theme::space;

let gutter = space::XL; // 卡片的水平内边距
widget(Stack::column(0.0).with_layout(|l| {
    l.position = nana_ui::runtime::PositionSpec::Relative;
}))
.children((
    widget(
        ScrollView::new(ScrollAxes::Vertical)
            .gutter(gutter)
            .heading_room(),
    )
    .children(groups),
    scroll_foot_fade(24.0, SemanticColorRole::Surface).clear_of(gutter),
))
```

## 让消费方也跑硬编码扫描

`scripts/audit-theme-hardcoding.py` 默认扫 NanaUI 自己的组件。给它 `--root` 和 `--path` 就能扫应用的源码，`--lint` 只报告新出现的数字字面量：

```bash
python3 ../NanaUI/scripts/audit-theme-hardcoding.py --root . --path crates/app/src/ui --lint
```

- `--lint` 列出每一处 `space::` / `type_scale::` 以外的像素、字号和圆角字面量（文件:行），有就退出 1。
- 想先接受现状、只拦新增时，用 `--output baseline.json` 记下当前计数，之后 `--check baseline.json` 只在某类计数增长时失败（和 NanaUI 自己的门禁一样）。

## 还没有的接口

- **按墨迹居中的对齐。** 图标对准文字墨迹（而不是行盒）需要 nana-text 把字形的墨迹范围交给布局，现在还没有；需要时应用只移动矢量图标，不移动文字（移动文字会改变它在各 DPI 下吸附的基线）。
- **`Card` 的前导图标槽。** 标题行有 `heading_row(..).lead(view)`；`Card` 本身不画标题，卡头由应用用 `heading_row` 拼。
