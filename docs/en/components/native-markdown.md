# NativeMarkdown

`NativeMarkdown` parses CommonMark / GFM source into native blocks and shows them in the tree. While it has focus, copy takes its selection snapshot.

There is no `<NativeMarkdown>` in the control table.

## Basic usage

`NativeMarkdown::new()` is an empty document.

`NativeMarkdown::from_source(source)` and `NativeMarkdown::parse(source)` go through the same parse.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::NativeMarkdown;

view! {
    <Widget of={NativeMarkdown::from_source("# Notes\n\nA paragraph of body text.")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::NativeMarkdown;

widget(NativeMarkdown::from_source("# Notes\n\nA paragraph of body text."))
```

:::

## Feature

This type is behind the Cargo feature `rich-text`.

`components` turns it on.

## Parsing

Parsing enables GFM, tables, strikethrough and task lists, and math delimiters too.

`from_blocks` takes `MarkdownBlock`s that are already split.

`blocks()` reads the parse result.

The source is kept in the control so you can read it again.

## Diagrams and formulas

Mermaid fences and formula fences are drawn by the control itself: formulas with RaTeX, diagrams with the native diagram engine (see [Native diagram engine](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagram-engine.md)), with colours following the theme.

Both kinds of fence also offer a presenter slot carrying the source. `MERMAID_PRESENTER` is `"mermaid"` and `MATH_PRESENTER` is `"math"`. A host that wants its own presentation can take over by this identity; the diagram slot also carries an accessible name and description.

Mermaid source it cannot read is shown as a code block. Tables, strikethrough and task lists go through native blocks.

A diagram wider than the column shrinks to fit and never grows. A click on a diagram (press and release on the same diagram within the click threshold, without dragging out a selection) emits `RichTextEvent::DiagramActivated`, whose `MarkdownDiagram` payload carries the block index and the diagram source; hovering a shrunk diagram shows `ZoomIn`, while a full-scale diagram or link shows `Pointer`. Whether and how to show it larger is the application's call, for example a dialog holding a `NativeMarkdown::from_blocks([MarkdownBlock::Mermaid(source)])` wide enough for the diagram to draw at its natural size. `UiWorld::markdown_diagram_placement(id, block_index)` answers the box the diagram is drawn in, its natural size and its scale; `scale < 1` means it was shrunk.

## Code coloring

A code fence with a language is colored by a registered `TextPresenter`, by default the one named `HIGHLIGHT_PRESENTER` (`"highlight"`). `code_presenter(name)` names another. The presenter gets the fence body and a `HighlightRequest` carrying the language, and returns byte-range `TextSpan`s; each span paints in its theme role and follows theme changes. Out-of-range, overlapping or non-boundary spans are trimmed away. With no such presenter registered the fence stays one color; registering it later colors the fence on the next paint. To replace an already registered presenter (such as taking over `"highlight"` globally), call `AppContext::replace_presenter` (or `UiWorld::replace_presenter`).

The core runtime binds no syntax library. An application wraps its own highlighter as a presenter with `TextPresenterFn::new(name, |text, request| spans)` and registers it with `AppContext::register_presenter`. With the `syntax-highlighting` feature on, `AppContext::new` has already installed the syntect-based `HighlightPresentation`, so fences are colored as they are.

## Alerts

GFM alerts `> [!NOTE]`, `> [!TIP]`, `> [!IMPORTANT]`, `> [!WARNING]` and `> [!CAUTION]` parse into `MarkdownBlockKind::Alert { kind, opens }` paragraphs; the `[!…]` marker itself is not body text. Each paragraph gets a bar down its left edge and a subtle tinted wash in the alert's color, joined across paragraphs, and the first paragraph gets a title above it (if the alert starts with a code block or table, the title is emitted ahead of it). Colors are theme roles (`MarkdownAlertKind::color`: note is accent, tip success, important strong accent, warning warning, caution danger). Titles come from `markdown.alert.*` in `FrameworkStrings` and can be replaced per language. An alert with a marker and no body draws only its title.

## Assembly

`assemble_markdown` is a `slot_assembler`.

On view commit, and after reactive patches, it assembles the blocks into child nodes.

After placing components by hand with `create_component`, call it once yourself.

The write that changes a field does not reassemble automatically.

## Selection

Selection hangs off an internal `TextSelectionGroup` and is compared by identity, not by selection content.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `source` | — | `from_source(source)` and `parse(source)` go through the same parse. The source is kept in the control. `new()` is an empty document |
| `blocks` | `MarkdownBlock` | `from_blocks` takes blocks that are already split. `blocks()` reads the parse result |
| `code_presenter` | `Arc<str>` | Name of the presenter that colors code fences, default `"highlight"` |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `RichTextEvent::SelectionChanged` | `Option<TextSelectionSnapshot>` | A drag changed the selection |
| `RichTextEvent::LinkActivated` | `Arc<str>` | A link was clicked |
| `RichTextEvent::ImageActivated` | `MarkdownImage` | An image was clicked |
| `RichTextEvent::DiagramActivated` | `MarkdownDiagram` | A mermaid diagram was clicked, for the application to open a larger preview |

## Slots

| Slot | Description |
| --- | --- |
| `mermaid` | `MERMAID_PRESENTER` is `"mermaid"`. The control already draws natively; the slot gives the source and the identity |
| `math` | `MATH_PRESENTER` is `"math"`. The control already draws natively; the slot gives the source and the identity |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)
