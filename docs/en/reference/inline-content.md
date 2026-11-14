# InlineContent

`InlineContent` is the Runtime's canonical content value. `PlainText` shares the existing string and single-run text path, `StyledText` uses the canonical ranges of `RichText`, and `Mixed` references ordinary `StableNodeId`s with U+FFFC inside one source. Builders, range replacement, Rust views, and templates all project to this value; there are no span wrappers, no per-character UI nodes, and no second tree.

```rust,ignore
let details = cx.create_detached_component(document, Button::new("Details"))?;
let content = InlineContent::builder()
    .text("CPU ")
    .child(details.stable_id())
    .text(" 42%")
    .build();
let paragraph = cx.create_component(document,
    InlineFlow::new(content).width(LengthSpec::Px(240.0)))?;
```

`Text::content`, `Button::content`, `Chip::content`, and `ActionMenuItem::content` accept the same content value; the existing string constructors remain available. Plain-text content stores no extra canonical wrapper, and removing the last span returns to the plain representation automatically. An explicit `accessible_name`, `InlineTextSemantics::accessible_text`, and `copy_text` are separate from the painted string. When not overridden, the atomic child's current text semantics are used. Secure input does not leak its content through copy or through the aggregated a11y text projection.

## Declarative projection

```rust,ignore
view::inline()
    .text("CPU ")
    .child(view::button("Details"))
    .span(" 42%", RichSpanStyle::new().size(18.0))
```

```text
<Inline>
    "CPU " <Button>"Details"</Button>
    <Span style={RichSpanStyle::new().size(18.0)}>" 42%"</Span>
</Inline>
```

Strings, expressions, and Vue interpolations inside `<Inline>` are paragraph source and do not create `Text` child components; elements are ordinary retained atomic children. `<Span>` is lightweight range styling. It accepts one text value and `style={RichSpanStyle}` and never becomes a Runtime component. Dynamic text reuses the existing NodeBindings; children reuse ordinary view slots, keys, and lifecycle.

## Single authority for layout and results

RuntimeLayoutEngine measures atomic nodes through the existing shared intrinsic cache and exports their outer logical size and baseline to `nana-text`. `nana-text` handles shaping, BiDi, CJK, line breaking, clamping, and object placement; the Runtime writes the resulting positions back to the same node. A child's interior is still laid out by its own formatting context.

The exported size includes resolved physical margins; the child's LayoutBox is the border box inside the margin. Text and children share a physical text origin. RTL, vertical-rl/lr, and tight label groups already placed inside controls do not compute a separate set of text positions.

Single-line plain text, and single-line text with paint-only styles, keep the original two box fragments and do not rebuild layout fragments when glyph advances change; glyph/caret positions continue to be read directly from the native retained text layout. Multi-line text, shaping-affecting styles, RTL/vertical, clamping, and mixed content publish fragments per native line/run.

`LayoutResult` publishes TextLine, TextRun, and InlineAtomic fragments. `source` is the logical UTF-8 byte range the fragment covers, and `node` keeps the atomic identity. `LayoutResult.participation` is what the Scene uses to decide atomic content, which keeps the old duplicate-label compatibility logic from swallowing independent Text children. Text in higher-level Button/Chip/menu controls reuses the Runtime's retained text layout directly. A Button's text budget is the available content width minus the fixed icon and gap; a Menu uses a stretchable TextContent slot. The budget does not change with the Plain/Styled/Mixed mode or with the text's own advance. A change in chrome or slot width explicitly invalidates the text constraint; a round trip through a color-only span keeps the original layout result.

An atomic subtree hidden by clamping/ellipsis keeps its identity and state but has no visible border box. The Scene, hit testing, and a11y all receive the same invisible projection, and the subtree reappears once space is restored. An ordinary idle frame does no Inline work; paint-only changes do not touch measure/placement. When an atomic child keeps its fixed outer size and baseline, local relayout stops at the child and the parent paragraph does not reflow. Size changes go through the existing typed invalidation and the bounded shape/layout flush.

## Updates and validation

`MutationQueue::set_inline_content` is the low-level batch interface. Every node it references must be a direct child of the paragraph once the batch ends and must belong to the same document. It does not create or delete children automatically. `InlineFlow`/control projections and `AppContext::set_inline_content` maintain atomic order and delete replaced atomic subtrees. When you use a typed component, persistent application state should still update that component's `content`; direct world mutation is a low-level operation, like the existing `set_text`.

Commit validation rejects duplicate references, wrong source offsets, cross-document references, dangling references, atomic children that are out of flow or floated, and nested interactive roles or focusable nodes inside control content. Later inserting a descendant, changing a role/interaction, reparenting, or changing style also rechecks the affected inline owners. An unreferenced normal-flow child is rejected; a standalone absolute/fixed chrome slot goes through the existing Overlay placement. When an auto block extent and an atomic child that depends on that extent form a constraint cycle, layout fails explicitly instead of iterating indefinitely.

Mixed content requires the host to use the product `NanaTextEngineShaper`. A TextShaper that only estimates sizes cannot provide real line/object placement and returns `InlineTextEngineRequired`.

## Verification entry points and remaining migration

- `cargo test --locked -p nana-ui-scene --test scene_contracts inline_content`: real fonts, wrapping, replacement, source map, RTL/vertical, margins, clamping, identity, nested interaction, and zero parent reflow.
- `cargo test --locked -p nana-ui-runtime --features view-macro --test contracts inline_`: Rust/template, dynamic spans, and wrapper-free projection.
- `NANA_COMPONENT_REQUIRE_GPU=1 cargo test --locked -p nana-ui-devtools --features runtime-agent --test devtools_pixels inline_content_pixels`: real pixels, independent atomic text, opacity, and hover.
- `cargo test --release --locked -p nana-ui-scene --features benchmark --test scene_perf inline_cost -- --nocapture`: 1k/10k plain Text/Button retained allocator bytes, RSS observation, and idle flush p50/p95/p99. Set `NANA_INLINE_COST_OUTPUT` to write JSON. Keep the pre-migration executable, run the before and after executables interleaved, and use `perf/contract.py --evaluate-inline-cost <manifest.json>` to judge p95 ≤5%, retained memory ≤8%, and the entity and allocation gates. The manifest contains a fixed `machine` name and at least three distinct relative paths each for the `before` and `after` reports.

The CPU scope of the cost fixture is idle flush; it cannot be used to claim that first-layout or dirty-frame CPU cost has passed acceptance. Timings taken while other builds/tests are running are observations only. All layout evidence continues to feed into the existing WorkCounters/catalog. `inline_reflows` counts native paragraph placements, and `inline_atomic_measure_requests` counts the requests those placements send to the shared cache; it does not claim each request is a full subtree measure.

This batch's part migrations for ListItem/SidebarRow/FileTab, LabeledValue, and Card are wired into the same LayoutResult. Rich content in multi-line ActionMenuItem options continues to use one retained text primitive, and the Scene merges canonical rich spans into the component text's paint spans; color changes keep the chrome layout generation. Select/Dropdown/SearchDropdown/ContextMenu options can take canonical content with `content(...)`, but an option row is a data row, not a retained paragraph: `label` stores its plain text and carries the layout budget and typeahead/a11y, while `SelectOptionData::content` carries only fill-color spans for the Scene to merge into the row text. Atomic children and spans that change shaping/decoration/annotation are rejected in debug builds; release builds keep only the text. For the related row helpers and index queries, see [Component Layout Audit](component-layout.md). First-layout, dirty-frame, and untested platform boundaries are recorded in the fixed `target/performance/issue196-final/baseline-manifest.json` and cannot be replaced by a single idle cost figure.
