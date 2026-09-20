# 待重录的快照

## 先更正一条：开发机**可以**录像素基线

本文件原先说「这台机器的字体栅格化与录制基线的机器不同，difference 图是整幅画面的
文字重影，不要在本机 `--bless`」。2026-09-20 实测，这条**不成立**：

- 本机（`metal-apple-m4`，与基线同一个 adapter key）**逐字节复现了 200 张**基线，
  其中大量是文字密集的（`button/*/primary`、`text-input/*/value`、`text/dark/normal`、
  `tooltip/*/open`、`command-palette/*/open`）。字体栅格化真不一样的话，这些不可能相同。
- 连跑两遍全套 615 张，**0 张**前后不一致——包括本文件点名抖动的
  `gallery-sidebar-collapsed-dark.png`（自转 `Spinner`）。那条也过期了。

「文字重影」的 difference 图是真的，但它来自**字形位置的亚像素位移**（排版变了），
不是栅格化差异——同一张 side-by-side 肉眼完全看不出区别，而语义基线说这一帧
逐字节相同。两者并存正说明差异在语义层**之下**。

所以剩下的像素债不需要特定机器，需要的是**解释**。

删掉本文件即表示这件事做完了。

## 现状（2026-09-20）

全套 615 个 key：

| | 张数 | 说明 |
| --- | ---: | --- |
| 与基线逐字节相同 | **200** | |
| 基线陈旧，待解释后重录 | **355** | 干净 HEAD 上就有 421 张不同，与后来的改动无关 |
| 本轮已录 | **120** | 见下 |

本轮录的两类，都是**先证明范围再动手**：

1. **本轮代码真正移动的 60 张。** 做法是 A/B：同一台机器上对干净 HEAD 与本分支各跑
   一次全套，比 md5。本分支相对干净 HEAD 只动了 68 个 key，全部落在改过的组件上
   （`segmented-control` 40、`select`/`dropdown`/`search-dropdown` 10、
   `app-title-bar`/`app-shell`/`appearance-section`/`settings` 8、含这些控件的整窗图 10）。
   其中 **8 张是被修复「还原」成与基线逐字节相同的**——Select/Dropdown 触发器内缩那条
   回归修完之后，像素自己回到了基线，和语义层给出的是同一个结论。剩下 60 张重录。

   > 用前缀 bless 要复核。`--bless component-migration/settings/dark/settings-page` 会
   > 连 `settings-page-full` 一起匹配，`component-migration/segmented-control` 会把
   > `all-disabled` / `no-selection` 一起扫进来——这 6 张不在 A/B 集合里，已还原。
   > bless 完一定要拿 `git status` 和 A/B 名单对一遍。

2. **从来没有基线的 60 张。** #101 §3 补的状态矩阵（58）加 `donut-chart` 的 2 张。
   它们每次跑都报 MISSING，没有任何东西会被覆盖，所以零风险。

## 还欠着的 355 张

**没有重录，因为没人能说清它们为什么变。** 这 355 张在**干净 HEAD 上就和基线不同**，
早于本轮所有改动；下面「清单」几节解释了其中大约 57 张（SegmentedControl 自驱 6、
行家族行盒 10、FormField 2、三个离群内边距 21、hover 18），其余约 300 张没有出处。

一个具体的例子说明为什么不能整批 bless：`component-migration/text/dark/{wrap,ellipsis}`
两张不同，而同组的 `normal` / `centered` / `muted` 逐字节相同——三张静态文字对得上、
两张会换行的对不上，指向的是**排版/断行**变过，不是噪声。语义基线看不见它（它记的是
文本图元的盒与属性，不记字形位置）。这类东西 bless 掉就再也没人会去找了。

建议的下一步：按上面的 A/B 方法，对着**引入变化的那个提交**逐类定位，而不是对着今天
的树整批重录。语义基线那 46 张就是这么做的，结果 3 类里查出 2 个真回归。

## 怎么做

```bash
cargo run -p component-gallery --bin ui-snapshots --all-features
```

先**逐张**看 `target/ui-snapshots/*.side-by-side.png`，确认每张的变化与下表的理由
一致；确认无误后按前缀分批重录，不要一次 `--bless` 全部：

```bash
cargo run -p component-gallery --bin ui-snapshots --all-features -- \
  --bless component-migration/segmented-control
```

## 清单

### `SegmentedControl` 改为自驱（6 张）

`component-migration/segmented-control/{dark,light}/{pointer-request,a11y-radio,atomic-reconcile}.png`

激活时控件自己提交选中，不再要求应用回写。基线上「选中」与「焦点」是**两个** pill，
现在只剩一个——被激活的那个。方向键漫游仍然只移焦点、不改选中（手动激活语义），
所以只有 pointer/激活相关的 fixture 变化。

### 行家族行盒统一（10 张）

- `component-migration/list-item/{dark,light}/{three-slots,auto-height}.png`
- `component-migration/settings/{dark,light}/settings-page.png`
- `component-migration/app-shell/{dark,light}/desktop-settings.png`
- `gallery-settings-appearance-{dark,light}.png`

行家族原先把绝对行盒设成等于字号（12/13/14），小于字体自然行高；现在统一到
`ControlSize::line_height()`（16/18），与 `Button` 一致。

**`auto-height` 是最该看的一张**：那里行盒决定行高，基线上两行几乎贴在一起
（13px 字配 12px 行盒），改后行距正常。固定高度的行（`sidebar-row`、`context-menu`）
没有变化，符合预期——行盒在那里不起决定作用。

`settings-page` / `desktop-settings` / `appearance` 的变化来自其中的 `SegmentedControl`
选项标签行盒，胶囊高度由既有测试守着，只是标签基线位移。

### `FormField` 标签预留（2 张）

`component-migration/form-field/{dark,light}/error.png`

标签预留高度从魔数 `字号 × 1.2` 改为 `ControlSize::nearest_text(...).line_height()`，
布局侧与绘制侧现在共用 `form_field_label_line()` 一个来源。标签与输入框之间留白变正常。

### 三个离群内边距归一（21 张）

- `component-migration/list-item/dark/{normal,small,medium,large,hover,pressed,focused,disabled,selected,selected-hover,selected-pressed,keyboard-activation,pointer-activation}.png`
- `component-migration/textarea/dark/{multiline,multiline-selection,selection,placeholder,focused,invalid-focused,disabled,scroll,clipped}.png`
- `component-migration/hosted-textarea/dark/{rust,placeholder,disabled}.png`
- `component-migration/range-field/dark/{minimum,middle,maximum,invalid,disabled,drag,drag-cancel,decimal-step,arrow-increment,arrow-decrement}.png`
- `migration-first-batch-dark.png`、`runtime-scene-{dark,light}.png`

三个值原先不在 `nana-ui-core::space` 尺度上，且与并排控件差 1px：

| token | 旧 | 新 | 理由 |
| --- | ---: | ---: | --- |
| `compact_control_padding_x` | 7 | `space::MD`(8) | 与 `ControlSize::padding_x()` 的 Small 统一。原先并排的 Small Button 内边距 8、Small Chip 7 |
| `field_padding_x` | 9 | `space::LG`(10) | 与并排的 `Select` / `Dropdown`（Medium = 10）对齐，同一行表单文字起始位置一致 |
| `list_item_padding_x` | 9 | `space::MD`(8) | 与 `SidebarRow` 的 `ROW_PADDING_LEFT` 对齐；两者是同一个视觉家族 |

`ControlSize::padding_x()` 里 `Small => 8.0` / `Large => 14.0` 两个裸字面量也改成读
token——否则 8 会有两个来源，等于把刚修掉的问题换个地方重犯。

**变化都是 1–2px 的横向位移**，逐张确认文字没有被裁、没有与相邻元素重叠即可。
全量测试 3047 通过 0 失败：现有测试断言的是行为契约，没有硬编码这三个数值。

### hover 态从来没画出来（18 张）

- `component-migration/{button,checkbox,icon-button,switch,text-input}/{dark,light}/hover.png`
- `component-migration/list-item/{dark,light}/{hover,selected-hover}.png`
- `component-migration/segmented-control/{dark,light}/{hover,selected-hover}.png`

hover 上色是一条 `motion::HOVER_COLOR`（120ms）过渡，派发 `PointerMove` 只是把它**起动**；
`interpolate_color` 在 `progress = 0` 时返回的是**静息**颜色。两处 fixture 路径
（`apply_runtime_state` 和 `exercise_segmented_contract`）派发完直接 flush，动画时钟一步
没走，于是「hover」参考帧画的是没被 hover 的样子——**基线里记的也是这个**。现在两处都在
派发后 `advance_animations(HOVER_COLOR)` 把过渡走完。

量过的证据：同一个 Button，hover 派发后 Quad 的 `background` 仍是 `None`、
`next_animation_deadline = Some(0ns)`；把时钟推到 120ms 之后变成
`Some([0.176, 0.176, 0.176, 1.0])`。

这一改只动了上面 18 张，且每张的变化都圈在被 hover 元素自己的行盒里
（`button/dark/hover` 是 `changed=2496 bbox=(20,20 82x32)`，正好是按钮的 32px 行盒）。
**逐张要确认的是**：hover 底色铺满整个命中区域，没有溢出到相邻控件。

改完之后 `button`/`list-item` 的 `hover` 与 `pointer-activation` 变成字节相同——这是对的，
指针激活结束时指针仍停在控件上。

### Issue #101 状态矩阵补齐（58 张，全部是新键）

- `component-migration/checkbox/{dark,light}/{selected-hover,selected-pressed}.png`
- `component-migration/chip/{dark,light}/{hover,pressed}.png`
- `component-migration/dropdown/{dark,light}/{hover,focused,disabled}.png`
- `component-migration/icon-button/{dark,light}/{selected-hover,selected-pressed}.png`
- `component-migration/interactive-card/{dark,light}/{hover,pressed,disabled,selected-hover,selected-pressed}.png`
- `component-migration/search-dropdown/{dark,light}/{hover,focused,disabled}.png`
- `component-migration/settings-collapsible-card/{dark,light}/{hover,pressed}.png`
- `component-migration/sidebar-row/{dark,light}/{hover,pressed,focused,disabled,selected-hover,selected-pressed}.png`
- `component-migration/sidebar-section/{dark,light}/{focused}.png`
- `component-migration/textarea/{dark,light}/{hover}.png`
- `component-migration/xy-pad/{dark,light}/{hover,focused}.png`

这 29 个 fixture × light/dark 是 #101 §3 查出的缺口：组件自己在
`InteractionStyle` 里声明了这些状态的 paint，但从来没有 fixture 捕获过。它们**没有
旧基线**，每次跑都报 MISSING，不是「像素变了」而是「以前根本没拍过」。

**2026-09-20 已录**（连同 `donut-chart` 的 2 张，共 60 张）：没有任何旧基线会被覆盖，
而且本机已验证与基线兼容、跑两遍零抖动。

语义基线（`snapshots/semantic/`，与 adapter 无关）已经在本轮录好并验过，可以先看它
确认每个状态解析出的颜色/边框是不是预期的，再在录制机上补像素。

三对 fixture 在语义基线里**逐字节相同**，这是组件的真实声明而不是 fixture 没生效：
`dropdown`、`search-dropdown`、`xy-pad` 的 `hovered` 与 `focused` 都指向
`BorderStrong`，悬停与聚焦在视觉上分不开。录完像素后这三对也会是同一张图。

## 语义基线（已重录，146/146 MATCH）

`snapshots/semantic/` 曾有 **46 个 fixture 和干净 HEAD 对不上**，
`cargo test -p component-gallery --bin ui-snapshots --features snapshots` 因此在 main 上
是红的。2026-09-20 重录完毕，现在 146/146 MATCH。

**重录之前先逐条对上「哪个常量动了」，结果 46 张里有 3 类不是常量收敛，是 bug：**

| 症状 | 判定 | 处理 |
| --- | --- | --- |
| 分段控件药丸圆角 7 → 8（160 处） | **回归**。轨道 padding 2 + 描边 1 = 3，药丸要与轨道同心就得是 `radius_md - 3`；收敛把字面量 `3.0` 换成 `space::XXS`(2.0)，丢了描边那一项 | 改 `SEGMENTED_PILL_INSET = padding + HAIRLINE`，圆角回到 7 |
| Select/Dropdown/SearchDropdown 触发器文字内缩 11 → 1 | **回归**。`select_geometry` 从 **authored** box 重新推 padding，而命名了档位的控件 authored box 上本来就没有 padding（那正是命名档位的意义），于是推出 0 | 改成接收调用方已经算好的 `used_layout_padding` |
| `app-title-bar` / `app-shell` 标题左移 86px、少三个窗口按钮 | **平台差异**，不是常量。`AppTitleBar::new` 的 `native_controls` 默认值来自 `WindowChrome::platform_default()`，是 `cfg(target_os)` | fixture 钉死 `native_controls(false)`，基线重新变成三平台通用 |

前两条修完之后，`dropdown` 与 `search-dropdown` **逐字节回到了提交里原本的基线** —— 也就是说
基线一直是对的，是代码漂走了。这是「先解释再重录」而不是直接 bless 的全部理由：
直接 bless 会把两个回归和一份 macOS 专属布局一起焊进共享基线。

剩下真正属于常量收敛、照实重录的（19 个组件 40 个文件）：

| 形态 | 组件 | 来源 |
| --- | --- | --- |
| 行距 1 → 2 | tree-view、reorder-list、action-menu、anchored-action-menu、context-menu、sidebar-frame、sidebar-section、select 菜单、app-shell、settings、appearance-section | `DEFAULT_SPACING` → `space::XXS`，7e81d7fb9 已认成有意的新值 |
| compact badge 盒 −3×−2 | status-badge | 7 → `space::SM`、3 → `space::XXS` |
| toast 指示点 7 → 6 | toast | 7 → `space::SM` |
| 日历标注字号 10 → 11 | calendar-heatmap | 10px → `type_scale::HINT` |
| 端口标签 +1 | graph-canvas | 5 → `PORT_RADIUS + HAIRLINE` |
| 文本量度随可用宽度变化 | empty-state、list-item、form-field、validation-message | 上面几项的下游 |

**一条要记住的结论：语义基线不是平台无关的。** 它对 GPU adapter 无关，但
`cfg(target_os)` 会穿透进来——`app-title-bar` 就是这么混进 46 张里的。fixture 里任何
读 `platform_default()` 的默认值都要钉死。

## 不在此列

~~`gallery-sidebar-collapsed-dark.png` 自身抖动，抖动来自会自转的 `Spinner`。~~
2026-09-20 复核：连跑两遍全套 615 张，**0 张**前后不一致，这张也稳定了。它本轮随
A/B 名单一起重录。

套件还自报「painted nothing but the clear colour… prove nothing」——只画了清屏色的快照，
对任何基线都成立、什么也证明不了（2026-09-18 是 4 张：`segmented-control/{dark,light}/empty`、
`overlay-host/{dark,light}/stacked`）。同样值得单独清理。

~~另有 2 张从来没录过基线：`component-migration/donut-chart/{dark,light}/slices.png`~~
2026-09-20 已随那 60 张一起补录。
