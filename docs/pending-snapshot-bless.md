# 待重录的快照

下面这些改动动了快照的像素，但**没有**在开发机上 `--bless`：那台机器的
字体栅格化与录制基线的机器不同，difference 图是整幅画面的文字重影，blessing 会把
开发机的文字渲染写进共享基线。要在**录制基线的那台机器/平台**上重录。

删掉本文件即表示这件事做完了。

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
旧基线**，每次跑都报 MISSING，不是「像素变了」而是「以前根本没拍过」，所以按前缀
`--bless` 即可，不需要逐张比对 side-by-side。

语义基线（`snapshots/semantic/`，与 adapter 无关）已经在本轮录好并验过，可以先看它
确认每个状态解析出的颜色/边框是不是预期的，再在录制机上补像素。

三对 fixture 在语义基线里**逐字节相同**，这是组件的真实声明而不是 fixture 没生效：
`dropdown`、`search-dropdown`、`xy-pad` 的 `hovered` 与 `focused` 都指向
`BorderStrong`，悬停与聚焦在视觉上分不开。录完像素后这三对也会是同一张图。

## 语义基线也欠着 46 个（与 adapter 无关，任何机器都能重录）

`snapshots/semantic/` 有 **46 个 fixture 和干净 HEAD 对不上**，所以
`cargo test -p component-gallery --bin ui-snapshots --features snapshots` 在 main 上
就是红的。这不是像素债——语义基线不分 adapter，随便哪台机器都能 `--semantic --bless`。

差异全是几何，来自 83d1bcefc 的尺寸常量收敛没有把基线一起重录：

| 看得见的形态 | 例子 |
| --- | --- |
| 行距 +1px / 面板高度 +1px | `tree-view`、`sidebar-section`、`action-menu`、`anchored-action-menu` |
| 分段控件圆角 7 → 8 | `segmented-control` |
| badge 盒 44.83×19.20 → 41.83×17.20 | `status-badge` |
| 文本量度位移 | `empty-state`、`graph-canvas`、`dropdown` |

一条都不是颜色。7e81d7fb9 的提交说明已经把其中 `DEFAULT_SPACING` 1.0 → `space::XXS`
2.0 那一条认成**有意的新值**，只是基线没跟上。

重录之前要逐条对上「哪个常量动了」，别当成噪声一把 bless：

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
# 逐个读 target/ui-snapshots/component-migration/<name>/<mode>.txt 与 .baseline.txt 的 diff
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic --bless
```

Issue #102（Phase 1 ThemeDefinition）**没有代为重录**：那是别的提交有意的视觉改动，
混进主题重构的 PR 里就再也没人会审它。#102 自己对这 46 张的贡献是 0——同一台机器上
对干净 HEAD 与 #102 各录一次全部 666 个输出文件，`diff -rq` 无差异。

## 不在此列

`gallery-sidebar-collapsed-dark.png` **在干净工作树上就已经失败**，且自身抖动，抖动来自
会自转的 `Spinner`。2026-09-18 复核：同一份代码连续跑两遍，全套 557 张里**只有它**一张
前后不一致，其余逐字节可复现。它与本轮改动无关，需要单独处理：要么把 Spinner 的相位在
快照里固定住，要么把这张排除出套件。

套件还自报「painted nothing but the clear colour… prove nothing」——只画了清屏色的快照，
对任何基线都成立、什么也证明不了（2026-09-18 是 4 张：`segmented-control/{dark,light}/empty`、
`overlay-host/{dark,light}/stacked`）。同样值得单独清理。

另有 2 张从来没录过基线：`component-migration/donut-chart/{dark,light}/slices.png`，
每次跑都报 MISSING。也要在录制机上补录。
