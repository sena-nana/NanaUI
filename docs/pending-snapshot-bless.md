# 待重录的快照

这一轮组件改动改变了 40 张快照的像素。它们**没有**在开发机上 `--bless`：那台机器的
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

## 不在此列

`gallery-sidebar-collapsed-dark.png` **在干净工作树上就已经失败**，且自身抖动
（同一棵树连续两次跑给出 3.90% / 4.09%，抖动来自会自转的 `Spinner`）。它与本轮改动
无关，需要单独处理：要么把 Spinner 的相位在快照里固定住，要么把这张排除出套件。

套件还自报「12 snapshot(s) painted nothing but the clear colour… prove nothing」——
12 张只画了清屏色，对任何基线都成立、什么也证明不了。同样值得单独清理。
