# 原生图表引擎（nana-ui-diagram）

`NativeMarkdown` 里的 ` ```mermaid ` / ` ```mmd ` 围栏由 `nana-ui-diagram` 解析和布局，产出与主题无关的矢量显示列表，再由 Runtime 按当前主题录成自绘（`PaintRecording`），Scene 细分成 GPU 路径网格与普通文字图元。全程不经 SVG 字符串，也不做 CPU 栅格化，任意缩放比都清晰。

## 管线与缓存

| 阶段 | 位置 | 缓存键 | 何时重算 |
| --- | --- | --- | --- |
| 解析 | `nana_ui_diagram::parse` | 源码 | 源码变化 |
| 布局 | `nana_ui_diagram::layout` | 源码 × 测宽身份（引擎、字体代数、字体族与字距）× 尺寸参数 | 源码或字体变化 |
| 录制 | `nana-ui-runtime` `markdown_diagram::record` | 布局结果 × 调色板代数 × 缩放 | 换主题、栏宽比自然宽度更窄 |
| 细分 | `nana-ui-scene` `emit_markdown_diagrams` | 录制 | 录制变化 |

- 布局在 `layout_block` 里每个围栏只算一次，测量、几何与输入三条路径共用同一个 `Arc<DiagramScene>`。
- 滚动、选区拖动和整页重建不重新解析、布局或细分；换主题只重录与重细分。
- 图表比所在栏宽时按比例缩小（从不放大）：几何和字号一起缩，细线保持主题 hairline 宽度。
- 文字按与正文相同的引擎实测字宽（`grapheme_advances`），中日韩与混排都不靠估算。

## 视觉角色

颜色、圆角和线宽在显示列表里都是主题角色，录制时才解析，所以明暗主题与自定义主题自动跟随。作者的 `style` / `classDef` / `linkStyle` 颜色按字面值绘制，填充色上的文字按亮度对比自动取深或浅。

| 元素 | 填充 | 描边 / 文字 |
| --- | --- | --- |
| 节点 | `Surface` | `BorderStrong` hairline；文字 `Text` |
| 连线 | — | `Text` 与 `BorderStrong` 55% 混色；`==>` 加粗，`-.->` 点线 |
| 连线标签 | `Surface` 底片，`Xs` 圆角 | `BorderSoft` hairline；文字 `Muted` |
| 子图 / 分组 | `Subtle` 70% | `Border`；标题 `Muted`、半粗 |
| 便签 | `Warning` 14% | `Warning` 55% |
| 状态色 | `Danger` / `Success` / `Warning` / `Highlight` | — |
| 分类色 | 图表色板 `series_palette`（以强调色为首） | — |

形状圆角：矩形 `Xs`，圆角矩形与分组 `Md`，体育场形 `Full`。

## 布局

分层图（flowchart，以及后续的 class、state、ER、requirement 等）使用：

- 深度优先找回边并反转去环；network simplex 分层（有转轴上限，超限保留当前可行解）。
- 带标签的边把标签当作中间秩上的哑节点参与排版，标签不压节点。
- 重心扫描 + 相邻交换减少交叉，保留交叉最少的顺序。
- Brandes–Köpf 横向定位，长边保持竖直。
- 边端沿节点边展开；出边过多的小节点共用中点再在通道里分叉，箭头端始终各占一个端口。
- 两秩之间的每段水平折线占独立通道，平行边不并线；折角按 `bend_radius` 倒圆。
- 子图递归各自布局（可有自己的 `direction`），以盒子放进父级；穿越子图边界的边经边界端口对接，有外部连线的子图跟随父级方向（与 Mermaid 一致）。

## 类型与头关键字

| 类型 | 头关键字 | 状态 |
| --- | --- | --- |
| flowchart | `flowchart`、`graph`、`flowchart-elk` | 原生 |
| info | `info` | 原生 |
| sequence | `sequenceDiagram` | 原生 |
| zenuml | `zenuml` | 原生 |
| class | `classDiagram`、`classDiagram-v2` | 原生 |
| state | `stateDiagram`、`stateDiagram-v2` | 原生 |
| er | `erDiagram` | 原生 |
| requirement | `requirementDiagram`、`requirement` | 原生 |
| usecase | `usecase-beta` | 原生 |
| agentflow | `agentflow-beta` | 原生 |
| block | `block-beta`、`block` | 原生 |
| architecture | `architecture-beta`、`architecture` | 原生 |
| c4 | `C4Context`、`C4Container`、`C4Component`、`C4Dynamic`、`C4Deployment` | 原生 |
| swimlane | `swimlane-beta` | 原生 |
| eventmodeling | `eventmodeling` | 原生 |
| mindmap | `mindmap` | 原生 |
| treeView | `treeView-beta` | 原生 |
| treemap | `treemap-beta`、`treemap` | 原生 |
| kanban | `kanban` | 原生 |
| ishikawa | `ishikawa-beta`、`ishikawa` | 原生 |
| railroad | `railroad-beta`、`railroad-ebnf-beta`、`railroad-abnf-beta`、`railroad-peg-beta` | 原生 |
| gantt | `gantt` | 原生 |
| timeline | `timeline` | 原生 |
| gitGraph | `gitGraph` | 原生 |
| journey | `journey` | 原生 |
| xychart | `xychart-beta`、`xychart` | 原生 |
| pie | `pie` | 原生 |
| quadrantChart | `quadrantChart` | 原生 |
| radar | `radar-beta`、`radar` | 原生 |
| sankey | `sankey-beta`、`sankey` | 原生 |
| venn | `venn-beta` | 原生 |
| packet | `packet-beta`、`packet` | 原生 |
| wardley | `wardley-beta` | 原生 |
| cynefin | `cynefin-beta` | 原生 |

全部类型都由原生引擎绘制，不再依赖 `mermaid-svg`。

## 配置

只读 frontmatter 的 `title` 与 `config:`、以及 `%%{init: ...}%%` 里会改变布局的键：

- `flowchart.curve`：`rounded`（默认，正交圆角）、`step*`（正交直角）、`linear`（直线）、`basis` 等平滑曲线名（样条）。
- `flowchart.wrappingWidth`：标签换行宽度。

`theme`、`themeVariables` 等主题设置会被读入但不生效：颜色始终来自 NanaUI 主题。`accTitle` / `accDescr` 写入无障碍名称与描述。

## 失败与上限

- 未知头、语法错误、超限：围栏按 ` ```mermaid ` 代码块显示（面板 + 源码），不会出现半张图。
- 上限（`Limits::DEFAULT`）：源码 128,000 字节、10,000 行、标签 2,000 字符、1,500 节点、3,000 条边、子图嵌套 16 层 / 300 个、显示项 65,536（文字 8,000）、画布边长 16,384。超限在解析或布局预算内结束，不靠捕获 panic。
- 网格布局的类型（block、c4）另有 500 个元素 / 500 条连线的上限：绕行路由与标签避让在这个规模内保持在数十毫秒。
- 排序工作量按 `边 × 秩 × 扫描轮数 ≤ 4e6` 收敛；流式输入的每个前缀都只是一次廉价解析加回退。

## 验证

```bash
cargo test -p nana-ui-diagram
NANA_DIAGRAM_BLESS=1 cargo test -p nana-ui-diagram --test diagram golden   # 有意改动后重写 golden
cargo test -p nana-ui-runtime --features rich-text --lib markdown
cargo test -p nana-ui-scene --features rich-text --lib markdown
cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example diagram-probe -- target/diagram-probe
cargo run --release -p nana-ui-diagram --features benchmark --bin nana-diagram-benchmark
```

- `tests/golden/<类型>/*.txt`：用固定测宽 `FixedMeasure` 生成的显示列表文本。
- `tests/corpus.rs`：坐标有限、落在画布内、结果确定、节点不重叠、裁剪成对。
- `tests/robustness.rs`：每个行前缀与随机字节变异不 panic；超限给出 `TooLarge`。
- `diagram-probe` 走真实 Scene 与 GPU，按明暗主题 × 1×/2× 出 PNG。

## 参见

- [Markdown、图片查看器与富内容绘制](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/markdown-rendering.md) · [NativeMarkdown 组件](https://github.com/sena-nana/NanaUI/blob/main/docs/components/native-markdown.md)
