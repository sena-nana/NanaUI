# NativeMarkdown

`NativeMarkdown` 把 CommonMark / GFM 源码解析成原生块，并在树上显示。焦点在它上面时，复制取的是它的选区快照。

控件表里没有 `<NativeMarkdown>`。

## 基本用法

`NativeMarkdown::new()` 是空文档。

`NativeMarkdown::from_source(source)` 和 `NativeMarkdown::parse(source)` 是同一条解析。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::NativeMarkdown;

view! {
    <Widget of={NativeMarkdown::from_source("# 说明\n\n一段正文。")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::NativeMarkdown;

widget(NativeMarkdown::from_source("# 说明\n\n一段正文。"))
```

:::

## 特性

这个类型在 Cargo feature `rich-text` 后面。

`components` 会打开它。

## 解析

解析打开 GFM、表格、删除线和任务列表，也打开数学定界符。

`from_blocks` 直接接收已经分好的 `MarkdownBlock`。

`blocks()` 读解析结果。

源码保存在控件里，供你再次读取。

## 图和公式

mermaid 围栏和公式围栏由控件自己画：公式用 RaTeX，图用原生图表引擎（见 [原生图表引擎](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagram-engine.md)），颜色跟随主题。

两种围栏还各给出一个 presenter 槽，带着源码。`MERMAID_PRESENTER` 是 `"mermaid"`，`MATH_PRESENTER` 是 `"math"`。宿主要换成自己的呈现，可以按这个身份接管；图表的槽还带无障碍名称与描述。

读不懂的 mermaid 源码显示为代码块。表格、删除线和任务列表走原生块。

图比正文栏宽时等比缩小，永不放大。点一下图（按下再松开，没有拖出选区）发 `RichTextEvent::DiagramActivated`，载荷 `MarkdownDiagram` 带块序号和图的源码。要不要放大、怎么放大由应用决定，比如开一个对话框，里面放一个够宽的 `NativeMarkdown::from_blocks([MarkdownBlock::Mermaid(source)])`，图就按自然尺寸画。`UiWorld::markdown_diagram_placement(id, block_index)` 回答图画在哪个框里、自然尺寸多大、缩放比多少；`scale < 1` 说明图被缩小过。

## 代码块着色

带语言的代码围栏交给一个已登记的 `TextPresenter` 着色，默认找 `HIGHLIGHT_PRESENTER`（`"highlight"`）。`code_presenter(name)` 换成别的名字。presenter 拿到围栏正文和带语言的 `HighlightRequest`，返回字节区间的 `TextSpan`，每段按主题角色上色，换主题跟着变。越界、重叠或不在字符边界上的区间会被裁掉。没有登记对应 presenter 时，围栏保持单色；之后再登记，下一次绘制就上色。

核心运行时不绑定语法库。应用可以用 `TextPresenterFn::new(name, |text, request| spans)` 把自己的高亮函数包成 presenter，用 `AppContext::register_presenter` 登记。打开 `syntax-highlighting` 特性时，`AppContext::new` 已经装好基于 syntect 的 `HighlightPresentation`，围栏直接上色。

## 提示块

GFM 提示块 `> [!NOTE]`、`> [!TIP]`、`> [!IMPORTANT]`、`> [!WARNING]`、`> [!CAUTION]` 解析成 `MarkdownBlockKind::Alert { kind, opens }` 段落，`[!…]` 标记本身不进正文。每段左侧画一条该类颜色的竖条，段与段之间连起来；第一段上方画标题。颜色取主题角色（`MarkdownAlertKind::color`：说明为强调色，提示为成功色，重要为深强调色，警告为警告色，注意为危险色）。标题来自 `FrameworkStrings` 的 `markdown.alert.*`，可以按语言替换。只有标记没有正文的提示块只画标题。

## 装配

`assemble_markdown` 是 `slot_assembler`。

视图提交时，以及响应式补丁之后，它会把块装配成子节点。

手工 `create_component` 放好之后要自己调一次。

改字段的那一次写入不会自动重装。

## 选区

选区挂在内部的 `TextSelectionGroup` 上，比较时按身份，不按选区内容。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `source` | — | `from_source(source)` 和 `parse(source)` 是同一条解析。源码保存在控件里。`new()` 是空文档 |
| `blocks` | `MarkdownBlock` | `from_blocks` 直接接收已经分好的块。`blocks()` 读解析结果 |
| `code_presenter` | `Arc<str>` | 给代码围栏着色的 presenter 名，默认 `"highlight"` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `RichTextEvent::SelectionChanged` | `Option<TextSelectionSnapshot>` | 拖选改变选区 |
| `RichTextEvent::LinkActivated` | `Arc<str>` | 点了链接 |
| `RichTextEvent::ImageActivated` | `MarkdownImage` | 点了图片 |
| `RichTextEvent::DiagramActivated` | `MarkdownDiagram` | 点了 mermaid 图，供应用打开放大预览 |

## 插槽

| 插槽 | 说明 |
| --- | --- |
| `mermaid` | `MERMAID_PRESENTER` 是 `"mermaid"`。控件已原生绘制，槽给出源码与身份 |
| `math` | `MATH_PRESENTER` 是 `"math"`。控件已原生绘制，槽给出源码与身份 |

## 参见

[总览](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
