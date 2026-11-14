# Layout Result Contract (LayoutResult)

`UiWorld` publishes one immutable `LayoutResult` for each node that has finished layout. It is the authority on box geometry for painting and input; a node's `layout_box` is only a compatibility projection of its `bounds` field.

Nodes that generate no box publish no result: `hidden` or `display: none` nodes, items in a closed menu, inactive overlay branches, parked subtrees, and the nodes inside them. When a node becomes one of these, its old result is withdrawn at the next publish, the same as a cold layout (Issue #386, gate in `world/tests/layout_foundations.rs`); its `layout_box` and those of the nodes inside collapse to an empty box at the origin, and any context it established is withdrawn too (Issue #392, gate in `world/tests/layout_foundations.rs`). After a triggered menu closes, the panel is still present while the exit animation plays; its items keep their open-state layout and still count toward the trigger's `overflow`. The frame after the exit finishes withdraws them, the same as a closed menu in a cold layout (Issue #387, gate in `world/tests/layout_foundations.rs`).

## Structure and generation

Each `LayoutResult` carries:

- the border box, padding box, and content box;
- the first baseline and last baseline;
- the overflow extent and scroll extent;
- the node's own typed layout fragments / parts: text lines and runs, overlay and scroll viewport, content box and component parts;
- the clip chain (`clip`, `clip_parent`), containing-block references, and dependency metadata;
- the monotonically increasing layout generation (`layout_generation`) that produced the result.

The Runtime frame update (`RuntimeFrameUpdate`) exposes this `layout_generation`, so Scene extraction, hit testing, the accessibility projection, and scroll consumers can align their projection versions without relying on the coarser `UiWorld` mutation generation.

`clip` is the nearest overflow container that reaches this node, and the chain continues up through that container's own `clip`. `clip_parent` is the node this node takes clipping and scrolling from: normally the parent; for an `absolute` node whose containing block lies outside a static overflow container, the parent of the outermost container it escapes; and `None` for a `fixed` node (Issue #201; rules under "clip chain" in [Layout (CSS Subset)](layout.md)). Scene extraction, hit testing, and the accessibility projection all attach nodes by `clip_parent`; skipped ancestors pass only transforms, not clipping or scrolling. An accessibility node's `clip` is the viewport region left by this chain. When an ancestor changes positioning or starts or stops clipping, these two fields on the nodes below update at the next publish. Scroll extents follow the same chain: for a node that clips or scrolls, `overflow` merges in only the children whose `clip_parent` is that node; `scroll_extent` and scroll metrics include only the descendants it reaches; `absolute` boxes that escape it count toward their containing blocks; and `fixed` boxes count toward no node.

A result copies none of its children's boxes. Where each child sits in the parent's context, as placements and fragments, is read from the children when asked: `UiWorld::layout_child_placements` / `layout_child_fragments` (page coordinates) and `local_layout_child_placements` / `local_layout_child_fragments` (from the parent's border-box origin, each child measured from its own frame); a call walks the parent's children once, and returns `None` when the node has no published result. So a child that only moves inside its parent's box leaves the parent's result as it was: not rebuilt, not republished, not extracted again; a child reaching past the parent's box (or back) changes the parent's `overflow`, and the result is rebuilt as before. Before 2026-11-11 a result also had a `child_placements` field, began `fragments` with one fragment per child and held one `ChildPlacement` part per child in `parts`: the same child box three times (about 144 B per child), and the whole parent rebuilt whenever a child moved. To migrate, replace `result.child_placements` with `world.layout_child_placements(id)`, read child fragments picked out of `result.fragments` by `node.is_some()` from `world.layout_child_fragments(id)` instead; `LayoutPartKind::ChildPlacement` is gone.

`ExtractedNode::layout_result` shares this immutable snapshot. Scene's Sticky placement reads the containing block, viewport, and scroll extent from it without remeasuring content; the scroll offset is still read from the current `ExtractedNode::scroll_offset`, and the presented position is never written back to the snapshot.

`layout_generation` is independent of the world mutation generation: **it increments only when layout output actually changes**. So hover, opacity, transform, and other paint-only changes trigger no geometry rebuild derived from layout. Layout-affecting writes clear the caches of affected nodes, their descendants, and dependent ancestors before the next result is published, while topology changes keep stable node identity (`StableNodeId`).

## Formatting context and child fragments

`formatting_context` records the formatting context the node actually used to arrange its children (or its own text) (Issue #197), and determines the concrete type of each child fragment `UiWorld::layout_child_fragments` gives:

| `formatting_context` | Child fragment type |
| --- | --- |
| `Flex` | `FlexChildPlacement` |
| `Grid` | `GridChildPlacement` |
| `Inline` | `InlineAtomic` |
| `Flow` / `Overlay` | `ChildPlacement` |

Before layout has recorded a context, only the node's `display` gives a provisional context. If a context switch happens to leave every box in place, a new `LayoutResult` is still published, because the context and fragment types have changed; child identity stays the same.

The child flow, out-of-flow, and float classification in measure and placement consume `ParticipationKind` directly; the inspector reads the same classification. A change in flow participation invalidates the parent's contribution/placement plan even when child identity and the child's own size are unchanged. Children touched only by a parent context/constraint change can still reuse intrinsic facts under compatible constraints; direct content changes and their measure ancestors still invalidate, and cross-context counts are recorded per actual context such as Flex/Grid/Inline.

## Relationship to ComponentGeometry

`ComponentGeometry` remains the compatibility surface for component semantic parts and non-box paint data (for example resize grips, chart paths, editor decoration lines). The text, icon, hint, and spinner boxes of Button, Chip, and ActionMenuItem; Checkbox's indicator box and label; Switch's track, label, and hint; the dot and label of StatusBadge / ValidationMessage; SelectionOption's ring (`Indicator`), icon, and label; a bare Toast's mark, title, description (`Detail`), and close target, and an assembled Toast's mark; and a menu trigger's label and icon are published by Foundation's lightweight parts. The geometry projection reads `LayoutResult.parts` and does not assign positions again. For the classification of other components and the remaining migration boundary, see [Component Layout Audit](component-layout.md).

Component content parts are solved in inline fixed-length slots (`layout_engine::parts::ContentParts`; `CONTENT_PART_SLOTS` is four: Toast's indicator, title, description, and close button are exactly four), not allocated per node. Only parts that exist are published to `LayoutResult.parts`, so the slot count neither changes the shape of published results nor adds to per-node resident memory. Code that reads parts (the geometry projection, paragraph budgets, native text reuse checks) looks them up by part type, not by slot.

## See also

- [Layout Engine (architecture)](/learn/runtime-layout) · [Runtime and Scene](runtime-scene.md) · [Layout (Rust)](rust-layout.md) · [Layout (CSS Subset)](layout.md)

Native Inline TextLine/TextRun/InlineAtomic fragments carry a logical source byte range; `LayoutResult.participation` updates with the parent context for Scene to consume. See [InlineContent](inline-content.md).

Component parts addendum: ListItem's `Note` / `Detail`, LabeledValue's `Label` / `Value`, and Card's `Header` / `Spinner` are all produced in the same publish batch. Card's primary `TextContent` and `Header` point to the same title budget, and ordinary `ComponentContent` points to the body; the renderer must not re-derive the title from the body's position. The title's native baseline is used by both intrinsic measurement and LayoutResult.
