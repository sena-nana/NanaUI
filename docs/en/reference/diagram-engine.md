# Native Diagram Engine (nana-ui-diagram)

` ```mermaid ` / ` ```mmd ` fences in `NativeMarkdown` are parsed and laid out by `nana-ui-diagram`, which produces a theme-neutral vector display list. The Runtime then records it as custom paint (`PaintRecording`) against the current theme, and the Scene tessellates it into GPU path meshes and ordinary text primitives. No SVG strings are involved at any point and there is no CPU rasterization, so diagrams are sharp at any scale factor.

## Pipeline and caching

| Stage | Location | Cache key | Recomputed when |
| --- | --- | --- | --- |
| Parse | `nana_ui_diagram::parse` | Source | The source changes |
| Layout | `nana_ui_diagram::layout` | Source × measurement identity (engine, font generation, font family, and letter spacing) × size parameters | The source or fonts change |
| Record | `nana-ui-runtime` `markdown_diagram::record` | Layout result × palette generation × scale | The theme changes, or the column becomes narrower than the natural width |
| Tessellate | `nana-ui-scene` `emit_markdown_diagrams` | Recording | The recording changes |

- Layout runs only once per fence in `layout_block`; the measurement, geometry, and input paths share the same `Arc<DiagramScene>`.
- Scrolling, selection dragging, and full-page rebuilds do not re-parse, re-lay out, or re-tessellate; a theme change only re-records and re-tessellates.
- A diagram wider than its column is scaled down proportionally (never up): geometry and font size shrink together, and thin lines keep the theme's hairline width.
- Text widths are measured with the same engine as body text (`grapheme_advances`); CJK and mixed scripts do not rely on estimates.

## Visual roles

Colors, corner radii, and line widths are theme roles in the display list and are resolved only at record time, so light, dark, and custom themes follow automatically. Author colors from `style` / `classDef` / `linkStyle` are painted literally, and text on a fill color picks dark or light automatically by luminance contrast.

| Element | Fill | Stroke / text |
| --- | --- | --- |
| Node | `Surface` | `BorderStrong` hairline; text `Text` |
| Edge | — | 55% mix of `Text` and `BorderStrong`; `==>` thick, `-.->` dotted |
| Edge label | `Surface` backing, `Xs` radius | `BorderSoft` hairline; text `Muted` |
| Subgraph / group | `Subtle` 70% | `Border`; title `Muted`, semibold |
| Note | `Warning` 14% | `Warning` 55% |
| Status colors | `Danger` / `Success` / `Warning` / `Highlight` | — |
| Categorical colors | Chart palette `series_palette` (accent color first) | — |

Shape radii: rectangle `Xs`, rounded rectangle and group `Md`, stadium `Full`.

## Layout

Layered graphs (flowchart, and later class, state, ER, requirement, and others) use:

- Depth-first search to find back edges and reverse them to remove cycles; network simplex ranking (with a pivot limit; when exceeded, the current feasible solution is kept).
- Labeled edges take part in layout with the label as a dummy node on an intermediate rank, so labels never overlap nodes.
- Barycenter sweeps + adjacent swaps reduce crossings, keeping the order with the fewest crossings.
- Brandes–Köpf horizontal placement; long edges stay vertical.
- Edge endpoints spread along the node's side. Small nodes with too many outgoing edges share a midpoint and then fork inside the channel; arrowhead ends always get a port each.
- Each horizontal polyline segment between two ranks gets its own channel, so parallel edges do not merge; bends are rounded by `bend_radius`.
- Subgraphs are laid out recursively on their own (they can have their own `direction`) and placed into the parent as boxes. Edges crossing a subgraph boundary connect through boundary ports, and a subgraph with external edges follows the parent's direction (consistent with Mermaid).

## Types and header keywords

| Type | Header keywords | Status |
| --- | --- | --- |
| flowchart | `flowchart`, `graph`, `flowchart-elk` | Native |
| info | `info` | Native |
| sequence | `sequenceDiagram` | Native |
| zenuml | `zenuml` | Native |
| class | `classDiagram`, `classDiagram-v2` | Native |
| state | `stateDiagram`, `stateDiagram-v2` | Native |
| er | `erDiagram` | Native |
| requirement | `requirementDiagram`, `requirement` | Native |
| usecase | `usecase-beta` | Native |
| agentflow | `agentflow-beta` | Native |
| block | `block-beta`, `block` | Native |
| architecture | `architecture-beta`, `architecture` | Native |
| c4 | `C4Context`, `C4Container`, `C4Component`, `C4Dynamic`, `C4Deployment` | Native |
| swimlane | `swimlane-beta` | Native |
| eventmodeling | `eventmodeling` | Native |
| mindmap | `mindmap` | Native |
| treeView | `treeView-beta` | Native |
| treemap | `treemap-beta`, `treemap` | Native |
| kanban | `kanban` | Native |
| ishikawa | `ishikawa-beta`, `ishikawa` | Native |
| railroad | `railroad-beta`, `railroad-ebnf-beta`, `railroad-abnf-beta`, `railroad-peg-beta` | Native |
| gantt | `gantt` | Native |
| timeline | `timeline` | Native |
| gitGraph | `gitGraph` | Native |
| journey | `journey` | Native |
| xychart | `xychart-beta`, `xychart` | Native |
| pie | `pie` | Native |
| quadrantChart | `quadrantChart` | Native |
| radar | `radar-beta`, `radar` | Native |
| sankey | `sankey-beta`, `sankey` | Native |
| venn | `venn-beta` | Native |
| packet | `packet-beta`, `packet` | Native |
| wardley | `wardley-beta` | Native |
| cynefin | `cynefin-beta` | Native |

Every type is drawn by the native engine; `mermaid-svg` is no longer a dependency.

## Configuration

Only the frontmatter `title` and `config:`, plus the layout-affecting keys in `%%{init: ...}%%`, are read:

- `flowchart.curve`: `rounded` (default, orthogonal with rounded corners), `step*` (orthogonal with square corners), `linear` (straight lines), and smooth curve names such as `basis` (splines).
- `flowchart.wrappingWidth`: label wrapping width.

Theme settings such as `theme` and `themeVariables` are read but have no effect: colors always come from the NanaUI theme. `accTitle` / `accDescr` are written to the accessible name and description.

## Failures and limits

- Unknown header, syntax error, or limit exceeded: the fence is shown as a ` ```mermaid ` code block (panel + source). A half-drawn diagram never appears.
- Limits (`Limits::DEFAULT`): 128,000 bytes of source, 10,000 lines, 2,000 characters per label, 1,500 nodes, 3,000 edges, subgraph nesting of 16 levels / 300 subgraphs, 65,536 display items (8,000 text), and a canvas side of 16,384. Exceeding a limit ends within the parse or layout budget, without relying on catching a panic.
- Grid-layout types (block, c4) have an additional limit of 500 elements / 500 edges: detour routing and label avoidance stay within tens of milliseconds at that scale.
- Ordering work converges under `edges × ranks × sweep rounds ≤ 4e6`; each prefix of streaming input is just one cheap parse plus a fallback.

## Verification

```bash
cargo test -p nana-ui-diagram
NANA_DIAGRAM_BLESS=1 cargo test -p nana-ui-diagram --test diagram golden   # rewrite golden files after an intentional change
cargo test -p nana-ui-runtime --features rich-text --lib markdown
cargo test -p nana-ui-scene --features rich-text --lib markdown
cargo run -p nana-ui-devtools --features runtime-agent,nana-ui/components --example diagram-probe -- target/diagram-probe
cargo run --release -p nana-ui-diagram --features benchmark --bin nana-diagram-benchmark
```

- `tests/golden/<kind>/*.txt`: display list text generated with the fixed measurement `FixedMeasure`.
- `tests/corpus.rs`: coordinates are finite, inside the canvas, and deterministic; nodes do not overlap; clips are paired.
- `tests/robustness.rs`: no panic on every line prefix or on random byte mutations; exceeding limits yields `TooLarge`.
- `diagram-probe` goes through the real Scene and GPU and outputs PNGs for light/dark themes × 1×/2×.

## See also

- [Markdown, Image Viewer, and Rich Content Rendering](markdown-rendering.md) · [NativeMarkdown Component](../components/native-markdown.md)
