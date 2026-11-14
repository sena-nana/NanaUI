# nana-ui-diagram

[English](README.md) · 简体中文

NanaUI 的原生 Mermaid 引擎，不依赖浏览器。

它把 Mermaid 源码解析成类型化模型，再布局成 `DiagramScene`：一份由形状和单行文字组成的矢量显示列表。列表里的颜色、圆角和线宽仍是主题角色，换主题只需重新录制，不必重新布局。解析和布局都在 `Limits` 预算内结束，超限时返回 `DiagramError::TooLarge`；解析时跳过的语句记为 `Warning`，不会让整张图失败。

`DiagramKind::ALL` 列出的 34 种图都由原生实现，包括 flowchart、sequence、class、state、er、gantt、pie、mindmap、timeline、gitgraph、sankey、xychart 等。

本 crate 不接触字体、GPU 和界面树，文字宽度由调用方经 `TextMeasure` 提供。Markdown 里 `mermaid` 围栏的识别和按主题录制在 [`nana-ui-runtime`](../nana-ui-runtime) 的 `rich-text` feature 里，应用一般不直接调用这里。

## 示例

```rust
use nana_ui_diagram::{DiagramMetrics, FixedMeasure, Limits, render};

let scene = render(
    "flowchart LR\n  A[Start] --> B[End]\n",
    &FixedMeasure,
    &DiagramMetrics::DEFAULT,
    &Limits::DEFAULT,
)
.unwrap();
```

## 测试

```bash
cargo test -p nana-ui-diagram
NANA_DIAGRAM_BLESS=1 cargo test -p nana-ui-diagram --test diagram golden
```

第二条在有意改动后重写 `tests/golden/`。解析与布局基准需要 `benchmark` feature：

```bash
cargo run --release -p nana-ui-diagram --features benchmark --bin nana-diagram-benchmark
```

更多内容见 [原生图表引擎](../../docs/reference/diagram-engine.md) 和 [NativeMarkdown 组件](../../docs/components/native-markdown.md)。
