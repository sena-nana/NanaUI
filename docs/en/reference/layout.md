# Layout (CSS subset)

The primary Rust path uses each control's own layout. You do not write CSS. For rows, columns, and borders see [Layout and Style (Rust)](rust-layout.md).

This page describes only the part of web CSS that `nana-ui-css` can map. The `.vue` dialect's `<style>`, `view!`, `stylesheet!`, and `css!` use it at build time. You use it to lay out the skeleton of an interface.

What gets compiled at build time is class selectors (`.card`, `.card.active`), the `@container` rules inside them, and `transition`. Other selectors, interaction states such as `:hover`, `@media`, `@keyframes`, and declarations that have no Style Model field only produce build warnings and do not reach the view. The selector and at-rule capabilities described below belong to the engine itself and are covered by `nana-ui-css` unit tests. `tools/css-parity` checks the box model against a browser using only inline `style` and class hints; it does not run stylesheets.

It is not a browser CSS engine. There is no full selector world. When it does not match, change the layout. Do not fake agreement with tolerances.

For dialogs, drawers, and menus use the overlays in [Controls](components.md).

## Layout intent authority

Component semantics, author CSS/Rust, and runtime constraints all enter one layout intent. Fields are marked component-required or
component-default by `nana_ui_core::LayoutOwnership`; authors may override defaults, and required values are reserved for the component. The final style is still committed through the
Runtime write entry point. `LayoutBox` and the scroll projection are outputs, not writable front-end properties.

Containing block propagation is seeded by the layout mutation footprint. Static frames and
paint-only updates no longer scan the whole projection tree; viewport changes propagate along the constrained axis (see "Constraint changes (#264)" below),
and structural changes still recompute from the root to keep percentages, Fill, and intrinsic sizing correct.

Style, theme, and component state (#261) all enter layout by the footprint of what changed:
- Style writes are classified by the **resolved** layout, that is, the author's layout with design intent applied on top (radius tier, control height,
  padding tier, panel inset). A write that only changes intent also emits the corresponding seeds; a write whose resolved result is unchanged,
  whether or not it is spelled the same, counts as an equivalent write: no seed, no dirty mark.
- A theme that only swaps the palette (Light↔Dark) only repaints, with zero layout. When theme metrics change, only nodes that declared design intent
  are re-resolved (there is an index; the document is not scanned), and seeds are emitted by changed field only for nodes whose resolved result actually changed:
  changing Button's horizontal padding tier relays out only the buttons that use that tier; text inputs, cards, and list rows are not measured;
  for a button placed in a fixed-size cell, the change stops at the cell.
- A Button recipe only changes color roles. A recipe change reprojects the button with zero layout.
- Component states such as hover, press, and focus that only change painting cause zero layout.

All counts live on `WorkCounters`:
- `style_to_layout_seeds`: seeds emitted by style writes;
- `theme_to_layout_seeds` / `theme_metric_dependents_invalidated`: seeds emitted by theme metric changes,
  and nodes whose resolved result changed;
- `theme_palette_layout_invalidations`: seeds emitted by a theme install that did not change metrics; should be 0;
- `component_state_layout_seeds`: seeds emitted by hover, press, focus, interaction, and accessibility state changes;
- `equivalent_style_layout_skips`: equivalent style writes.

The gates are in `world/issue261.rs`: hover causes zero layout; a palette switch does no shaping, measuring, or placement; changing Button's
padding tier seeds only those buttons; the cost of changing one button's padding is the same at 1k and 10k controls;
1000 equivalent writes emit no seeds; a recipe change causes zero layout; metric changes and intent-only writes are compared pass by pass with a full layout.
#259 also added theme workloads: on 1k, 10k, and 100k pages, switching palette and metrics both cause zero layout.

Not done yet: changes to font-related inherited properties (font size, font weight, line height, writing direction) still relay out the whole subtree,
rather than propagating precisely to descendants that depend on em units.

Intrinsic metrics are stored in one place, Runtime's `IntrinsicCache`. It records content and style
facts (min/max inline/block, preferred size, first and last baseline, aspect ratio) and a
generation; the used size resolved against the current containing block is kept separately and must never be written back into this
fact cache. The cache key does not include the name of Flex, Grid, or IFC, so the same constraint class can be reused across
formatting contexts; dependencies such as percentages, Fill, aspect ratio, fonts, writing context, and
viewport go into the constraint or the input identity. Only changes to content, shape styles, borders, min/max, resource
metadata, scale/font, and subtree metrics bump the generation; color, opacity,
transform, hover, and accessibility updates do not.

## LayoutNode, Participation, and FormattingContext (#197)

Layout splits a node into three things, none of which replaces another:

- **Identity**: the `StableNodeId` and its retained record. Layout never creates, wraps, or destroys a node to switch algorithms.
- **Participation** (`ParticipationKind`): how this node is laid out in its parent's formatting context.
  It is derived from the parent's context, the node's resolved layout intent, and the node's own content. It is a view,
  not a component, and it is not stored as another tree.
- **Established context** (`FormattingContextKind`): how this node lays out its children. Layout records the one it
  actually ran: `Flow` (`display: block` or unset, the same block layout), `Flex`, `Grid`, `Inline` (there are inline-level boxes among the in-flow
  children), `Layers` (`display: layers`, stacking), `Overlay` (a modal places its children by slot). A leaf with text establishes
  `Inline` for its own text; the glyphs belong to nana-text and do not become layout nodes. An empty leaf establishes no context.

Measure and placement choose the context with the same function, so the two passes cannot choose different algorithms for the same node.
The algorithms themselves did not change. The context reads only `resolved_layout`, the output of layout intent authority;
it does not parse the style a second time.

| Parent context | Child participation |
|---|---|
| `Flex` | `FlexItem` |
| `Grid` | `GridItem` |
| `Flow` | `FlowItem`; a floated one is `Float`; an inline element split by a block is `InlineSpan` |
| `Inline` | Inline text is `NativeText`, an inline element with children is `InlineSpan`, `inline-block`, controls, and replaced content are `AtomicInline`, a block-level box is `FlowItem` (line break) |
| `Layers` | `LayerItem` |
| `Overlay` | `OverlayItem` |

`Absolute` / `Fixed`, `display: contents` (`Contents`), and nodes that generate no box (`NoBox`)
are the same in every context. Text, images, buttons, containers, and custom render all go through this one table:
they differ only in content (`Children`, `Text`, `Replaced`, `Empty`), not in component type. The same
text node is `NativeText` in `Inline` and, in flex or grid, an item laid out by its intrinsic size.
Nodes may nest arbitrarily: a grid in a flex parent is a `FlexItem` and is still a `Grid` for its own children.

`UiWorld::layout_node(id)` assembles this view from the records on every call: resolved intent, content, established
context, parent context, participation, context generation, and result generation. When the parent has not been laid out yet,
participation is `None`; it does not guess. `LayoutResult::formatting_context` records which context the result was
produced in, and it also decides the kind of child fragment `UiWorld::layout_child_fragments` gives (`Flex` → `FlexChildPlacement`,
`Grid` and `Layers` → `GridChildPlacement`, `Inline` → `InlineAtomic`, everything else →
`ChildPlacement`). A container that generates no box establishes no context: when a container that was already laid out later becomes `NoBox`,
it and the nodes inside it drop the contexts they established, and the boxes collapse to empty boxes at the origin, the same as a cold layout. This does not count as a
context switch and does not count toward `layout_context_transitions`; it only advances the context generation. When shown again,
the whole subtree is placed again and its contexts are recorded again (Issue #392, gate in `world/issue392.rs`).

`Layers` is stacking, synonymous with SwiftUI `ZStack`, Compose `Box`, and Flutter `Stack`: in-flow children share
the same area, aligned by `justify-items` / `align-items` (the child's `justify-self` / `align-self`
takes precedence), start by default. The container follows its largest child; out-of-flow children are positioned as usual and do not grow the container. Hit testing and painting follow document
order, with later children on top. It runs the grid algorithm: one `auto` row track and one column track, all children placed in that one cell; the children's own
grid positions and track declarations on the container have no effect. Measure uses content-sized tracks, so the container follows the largest child;
placement then stretches the cell to fill the container's content box, so alignment is relative to the whole container. The Row/Column presets are still
`Flex`; `Stack::layers()` / `Stack::overlay_layer()` are what produce stacking, and flex and stacking no longer share one kind of
context. Gates are in `world/issue204.rs` (shared area, follows the largest child, nine anchors, RTL, `stretch`,
child overrides, the later layer catches the pointer, same as cold layout) and in `tests/view_macro.rs`
`issue204_layers_css_and_rust_share_one_overlay_context`; the thousand-component
switching sequence in `world/issue199.rs` also goes through `Layers`.

A context switch (the same set of children going Flex → Grid → Inline) only replaces the context record, its generation,
the children's participation, and the layout results; plans compared by style pointer are invalidated naturally. The children's ids, components, state, focus,
listeners, and mounts do not change, and no node is created or destroyed. Each container recorded under a different context counts once toward
`layout_context_transitions`; static frames, paint-only writes, and equivalent writes are all 0.
DevTools `inspect`'s `layout_node` gives the node, an intent summary, content, established context, parent
context, participation, and the three generations: metrics, context, and result.

The gates are in `world/issue197.rs`: the same set of children (a labeled inline-block, inline text, a replaced image,
a nested grid) goes through Flex → Grid → Inline → Flex; each step records exactly one switch, no node is created or destroyed,
participation follows the parent, and every item matches a cold layout; static, paint-only, and equivalent writes record no switch.
The component version in `framework/tests.rs` confirms that TextInput's content and focus and Button's handler are all preserved.

The Flex, Grid, Inline, and Overlay algorithms each live in their own module under `layout_engine/` (`flex.rs`,
`grid.rs`, `inline.rs`, `overlay.rs`); `Layers` reuses `grid.rs`. `context.rs` is the dispatch shared by both passes, and it is also where modals are
recognized: a node with the `ModalFrame` visual establishes `Overlay` regardless of `display`,
and `overlay.rs` then places the children by the slot contract. The gate is `world/issue199.rs`
`issue199_a_modal_frame_places_its_slots_as_overlay_items` (the slots are `OverlayItem`, relayout records no
context switch, incremental matches cold layout; if the dispatch does not recognize the modal, the placement assertion fails). When the same container switches back and forth among Flex → Grid → Layers → Inline for 20 rounds, the retained cache stops changing after the
first round, and on returning to Flex it equals a cold cache that has only laid out Flex (`issue199_repeated_context_switches_keep_the_retained_cache_flat`).

Anchored overlays are now placed by layout; see "Anchor positioning" below. The containing block of `absolute` is resolved per CSS, `fixed` attaches to its own document's viewport, and clipping and scrolling apply along
the clip chain that follows containing blocks. Scene, hit testing, and accessibility read the same chain; see "clip chain" under "Positioning" below (#201).

## Component content parts (#205)

Controls do not lay out their own content. Identity-less content such as icons, indicator boxes, labels, hints, and spinners is declared as lightweight Flex
parts (`layout_engine::parts`). The same retained Flex solver functions distribute the main axis, shrink, and align them,
and the results are published with the node's `LayoutResult.parts`. Parts have no `StableNodeId` and produce no wrapper nodes.
One part declaration is read in four places: intrinsic measure takes the width and spacing other than text (`chrome_extent`),
the paragraph budget takes the available width minus that same amount (`text_budget`), placement solves them, and `ComponentGeometry`
and Scene only read the published boxes and attached paint data. Text fragments (TextLine/TextRun) land in the `TextContent`
part.

Parts can nest directions: a Switch row is a track and a text column, and the text column in turn solves its label and hint as a Flex column;
both solves use the same `flex_parts`. Intrinsic measure subtracts the parts beside the text
(icon, indicator box, dot) from the width the text may take, the same as the paragraph budget, so a box with full-width text does not overflow the available width because of those parts. When a part reads theme metrics that do not appear in the node's own style (Switch's
track and gap, Range's gap and unit font size, Progress's title row and gap, the dot cell width and gap of StatusBadge / ValidationMessage, the icon gap of SelectionOption with an icon), the node is still remeasured when the theme changes those metrics (`visual_parts_read_theme`); a bare Toast's parts read the Toast recipe, and it is remeasured when the recipe changes. A component publishes at most four content parts (the four slots of `ContentParts`; Toast uses all of them).

Migrated: Button, Chip, ActionMenuItem, Checkbox, Switch, Range (label, value slot, unit, #384), Progress (label, cancel, #384), ModalFrame (title, description, body text, #384), StatusBadge, ValidationMessage, SelectionOption (vertical ring, segment item, tab), EmptyState, Toast (a bare toast's mark, title, description, close target; the assembled mark), MenuSurface triggers (labels and icons of Popover / ActionMenu / HoverCard), the text parts of ListItem/SidebarRow/FileTab,
LabeledValue, FormField, Card title, Select trigger. Interaction, data rows, text editing geometry (menu rows,
tree rows, editors), charts, paths, and canvases do not go into parts. For the per-component classification and pending migrations see
[Component content layout and migration audit](component-layout.md).

A component with children declares the parts laid out before its children as leading parts (`parts::LeadingParts`, #385):
`UiWorld::leading_parts` gives a band along the component's own axis (`Column` above the children, `Row` at the inline start:
left in LTR, right in RTL, `LeadingParts::end`, #388), `extent` long and at least `breadth` in the cross direction. The band sits at the start of the content box, inside
the padding: measure adds `extent` in front of the children, treats `breadth` as the lower bound of the content box's cross direction, and subtracts `extent` from the available space handed to the
children; placement starts the children after the band; the component solves its own parts at the start of the content
box. The padding is only what the box declares (written by the author, or the component's default); the published used padding
does not include the parts, and the author's padding and the parts' space neither add up nor replace each other. Leading parts go into the measure
plan's input comparison along with `VisualContent`; grid increments do not re-solve boxes with leading parts. In use: EmptyState's icon, title, and description (before the action
buttons), the title band of a Card with a title (before the body), and an assembled Toast's mark and gap (before the copy). The gate is
`world/issue385.rs`: all three are compared step by step with a cold layout under author padding, theme switches, and part visibility changes, and it checks that the
content box equals the padding box minus the declared padding, the parts start at the content box start, and the first child comes
after the band.

A box whose text is cleared gives up its old metrics: when a box's text becomes the empty string, the next text pass resolves it as
"no text", the metrics go to zero, layout measures it again, and the result matches a cold layout. The gate is `world/issue205.rs`
`issue205_a_cleared_label_lays_out_as_a_cold_unlabelled_control`.

## Incremental reflow frontier

Runtime's incremental layout represents one layout invalidation as a `LayoutInvalidation`: it records the
`InvalidationKind`, the changed fields, and the `LayoutDependencyFootprint` together. Retained layout can merge several seeds of the same frame
into separate measure, placement, writing-context, and scroll/overflow
frontiers, and records work counters such as `layout_frontier_*` and `layout_dependency_edges_visited`.
When a result only changes position, the parent enters only the placement frontier; `LayoutMetricDelta::NONE`
stops upward propagation. A parent constraint sends only the children that consume that axis into measure; descendants that only change position stay in placement.
A box with a fixed border box whose exported size does not change shields its children: ancestors no longer enter measure, and outer siblings no longer enter
placement. A sequential container only remeasures the children that enter the measure frontier, and the suffix placement reuses
the measured sizes. If a recomputation produces a `LayoutResult` bit-for-bit identical to the published result, the layout
generation does not advance and the published result object stays the same.

A style write is classified only by this node's own fields; it walks neither the document nor its descendants. A write with the same effect
(the same value, or synonymous spellings such as `direction: None` and `Some(Column)`) queues nothing and is
recorded in `layout_equivalent_mutations_skipped`; a write that classifies as empty (including writes that only change color,
opacity, or transform) is recorded in `layout_invalidations_zero_delta`; typed causes actually queued are
recorded in `layout_invalidations_created`. A node holds at most one pending
`LayoutInvalidation`; several writes in the same frame merge into that one slot rather than growing with write history. A change to a layout field
only hides the results of this node and of its ancestor chain up to a fixed border box; descendants' results are republished in the pass whose frontier
picks them up.

When text is reshaped and its outward width, height, and baseline are all unchanged, the change stops at the text: no seed, no parent layout,
and the text pass records one `layout_propagations_stopped`. When the text belongs to a fixed-size box itself
(for example the label of a fixed-size button), as long as that box is not inline-level and the parent does not align it by baseline, a metric change
relays out only inside the box: the border box it exports is unchanged, and the parent does not enter measure. When the parent is not a fixed border box, it remeasures from the child
metrics it reads, but like the other ancestors on the export path it does not bring its own siblings into the frontier: if its box really changed,
the following siblings are moved by the container's placement (a sequential container replays the suffix from it); if it did not, nothing moves. The frontier of a label at the start of a long page
does not grow with the siblings after it. The product frame reshapes this pass's range after
layout; text whose reshaping changed its metrics emits its own typed seeds, and the next pass walks the frontier only from those seeds rather than
relaying out the whole range as a fully dependent seed again.

A parent whose child list changed (insert, move, detach, park, despawn) likewise does not bring its own siblings into the frontier: the incoming subtree
is laid out whole with its own topology cause, and if the parent's box really changed, the following siblings are moved by the container's placement. A subtree laid out whole
(incoming, toggled shown or hidden, cause unknown) also does not pull in the following siblings in block flow; only isolated
boxes that export no metrics to the parent still connect to them. When the parent places its children by the new list and only the content box's main-axis length changed, the remaining children
whose style did not change, that do not read the main-axis length, and that have no internal change reuse the sizes recorded by the previous plan instead of being remeasured whole.

A sequential container (where each child's position depends only on the children before it) no longer even walks its children one by one: the edit is what lies between
the common prefix and suffix of the old and new child lists. The measure plan subtracts each departing child's share of size and one gap, and measures each incoming child once and adds it;
the placement plan replays the suffix from the edit, children before the edit do not move, children after the edit move by the difference of departures and arrivals, and the plan is rebuilt around the
edit. Adding or removing a box at the end of a long page does only that edit in a frame; adding or removing at the start moves each following box once, which is real output,
and no subtree is walked whole for measurement. When incoming children change the formatting context (inline-level, float, positioning, `display: contents`,
flex, `order`, auto margin, reading the main-axis length), when all the widest children leave a container whose cross axis follows content, or when the flow becomes empty or starts from empty,
the children are remeasured and placed as before. After the plan patches out a size, the intrinsic facts the container retained measure the content before the patch;
they leave the retained cache with this pass, and a later different constraint remeasures rather than reading the old ones.

Several seeds within the same batch of nodes are first combined into one dependency graph (the union closure of the seeds), and each edge is walked at most twice; the
graph's edge count and adjacency entries are recorded in `LayoutFrontierStats` as `graph_edges` and `scratch_entries`,
and they vary with the closure, not with document size. When a node that no longer generates a box has been written as a zero box by its container, the boxes
retained for its whole subtree also collapse to zero size, consistent with a full layout.

DevTools `inspect` answers why a node was laid out: before layout it is the queued cause; after layout
it is the cause merged when the last pass's frontier picked it up, marked as either the seed itself or brought in by another node's dependency
edge. `reason` and `changed_inputs` say why it was marked, `kind` says which stages run, and
`affected_axes` says along which dependencies the frontier walks outward from it.

The counters named by Issue #255 have no second copy: `layout_dependency_footprints_read` is
`layout_dependency_edges_visited` (one footprint is read per edge visited),
`layout_metric_deltas_none` is `layout_propagations_stopped`, and
`layout_metric_deltas_measure`, `layout_metric_deltas_placement`, and
`layout_metric_deltas_writing` are `layout_frontier_nodes_measure`,
`layout_frontier_nodes_placement`, and `layout_frontier_nodes_writing` respectively.

Execution counters and frontier counters are the same `WorkCounters`. `layout_measure_nodes` counts only nodes that actually computed a used size; hits on the retained used size and the measure plan are recorded in `layout_measure_cache_hits`, and recomputations after a miss in `layout_measure_cache_misses`. Intrinsic metrics hits remain `intrinsic_measure_cache_hits` and are not counted twice. `layout_placement_nodes` is the boxes written by this placement, not the frontier member count. `layout_origin_only_updates` is writes where the size did not change and only the origin did. `layout_result_reused` / `layout_result_changed` come from result publication: identical geometry keeps the original object, and only changed geometry replaces it. `layout_delta_commits` is the publication that actually wrote changed results. `layout_context_local_solves` is not listed separately; it is the `layout_frontier_contexts` already queued in the frontier.

The remaining counters named by Issue #259 also live on this `WorkCounters`, with no other authority.
`layout_measure_requests`, `layout_measure_hits` / `layout_measure_misses`, and
`layout_full_subtree_measures` are #198's `intrinsic_measure_requests`,
`intrinsic_measure_cache_hits` / `intrinsic_measure_cache_misses`, and
`intrinsic_measure_full_subtrees`; the last one is a measurement that actually walked the children.
Plan hits come in two kinds: `layout_placement_plans_reused` is a container placing its children by the retained placement plan
(a container that only moved translates along with its plan and is also recorded here),
and `layout_measure_plans_reused` is a size obtained from the retained measure plan; neither walks the children.
`layout_plan_misses` is a container that had a plan for the corresponding input but still walked its children.
`layout_plan_queries` is not listed separately; it equals the two kinds of hits plus misses.
`layout_plan_rebuilds` is a plan recorded again after a walk over an old plan for the same container and the same constraint;
a container's first plan does not count as a rebuild. `layout_suffixes_replayed` is a sequential container that replayed only the segment after the changed children,
`layout_children_measured` is the children whose size a container read during a walk, `layout_containers_uncacheable`
is a container that placed its children but could not record a plan, and `layout_retain_sweeps` is a sweep of the retained cache that clears destroyed ids.
A positioning context fully relaid out because its plan expired is recorded in `layout_local_subtree_fallbacks`.
A measure plan is reused only if none of the node's own inputs changed: constraint, style, child list, text metrics, and the parts the standard visual paints beside the text
(a button's icon, a checkbox's box). The latter exist only in the visual; when only an icon is added to or removed from a button, style and text are unchanged, yet the plan is still invalidated and remeasured.
`layout_scratch_allocations` is `layout_scratch_entries`: the adjacency entries allocated by each pass's dependency graph in this frame,
freed when the pass ends; `layout_scratch_bytes` is the bytes those entries occupy.
`plan_stats` only does benchmark stage timing and does not count.
The counters output by the four benchmarks (`nana-runtime-benchmark`, `nana-framework-benchmark`, `nana-dirty-frame-benchmark`,
`nana-card-update-benchmark`) share names with `WORK_COUNTER_KEYS` in `perf/contract_support/schema.py`,
and include the plan, scratch, suffix, children, intrinsic measure, constraint propagation, and resize text counters above;
what you read in the report is the same `WorkCounters` the gates read.

Incremental results are checked by two layers of cold comparison. The first layer is `layout_engine::verify`: in tests, and when the
`layout-verify` feature is on, every retained layout pass is followed by a full layout from scratch, comparing box by box.
The second layer is `world::reflow_oracle` for tests, which compares published results: the same inputs are laid out once in a world that has never run
an incremental frame, and the following are compared node by node:
- boxes;
- all geometry covered by `LayoutResult::geometry_eq`: border box, padding box, content box, baseline, fragments,
  overflow, scroll extent, containing block, and so on;
- hit geometry;
- accessibility bounds.

State left between incremental frames (shaping results, plans, retained sizes) exists only on the incremental side; the cold world has none of it,
so errors in that state cannot hide. The gates of #256 and #259 both end with this layer.

`world/issue259.rs` is #259's scale matrix and covers these dimensions:
- document size: 1k, 10k, 100k nodes;
- edits in the same frame: 1, 8, 100;
- edit position: head, middle, tail;
- nesting depth: shallow, or 8 levels of wrapping;
- writing direction: LTR, RTL, vertical-rl.

There are nine scenarios: paint-only, content within a fixed boundary, sequential size change, parent constraint, flex redistribution, grid contribution,
inline reflow, language scope (#260, see "Language scopes and external metrics" in [Text engine](./text-engine.md)),
and font-size scaling scope (#266, see "Font-size scaling scope" in [Text engine](./text-engine.md)).
The scale gate with a scope holding 2000 paragraphs inside a 100k-node document is in `world/issue266.rs`.

The gates read only counters, not time:
- For paint-only changes, all layout counters are zero.
- For a local edit, the frontier, edges walked, boxes placed, children walked, subtrees measured whole, and children walked by result publication
  are exactly the same at 1k, 10k, and 100k.
- For a sequential edit, the boxes placed are no fewer than the boxes that actually move, and no more than the moved boxes plus the path to the edit;
  the number of subtrees measured whole does not grow with the document. At the end where growth pushes nothing after it (the tail of the forward block axis, the
  head of the reverse block axis in vertical-rl), all work in a frame is constant. Only the edited container replays a suffix,
  and the count does not grow with the document; missed and re-recorded plans (`layout_plan_misses`, `layout_plan_rebuilds`)
  also come only from the edited container, are the same at 1k and 10k, and do not exceed the number of boxes that actually move: boxes are placed in page coordinates,
  and a moving group must rewrite its children, but it translates along with its plan (see "Containers that only moved" below); an inline
  formatting context, which cannot record a plan, still walks its own children when it moves. Nothing before the edit is rebuilt.
- A header column 1200 px wide, where a label that wraps only its own text is replaced with longer text that still fits on one line: a frame runs layout only once,
  the cost is the same as a label that cannot wrap, none of the boxes below the label move, and 1k and 10k are the same (for the text side see the "Ellipsis" section of
  [Text engine](./text-engine.md)).
- The same label, with the page's card groups following directly after its column as siblings of the column: longer text that fits on one line (whether or not it can
  wrap) does not change the column height; the graph built, edges walked, and boxes placed in a frame are exactly the same at 1k and 10k, and only the label moves;
  text that wraps in a 300 px column makes the column taller, and each following group moves down exactly once, which is real output and grows with the page,
  while the frontier that finds them, the edges walked, and the number of subtrees measured whole stay the same. A group moving down translates along with its cards (see
  "Containers that only moved" below): children walked, missed plans, and rebuilt plans are also the same at 1k and 10k, and the sizes looked up do not grow with the page.
- The same page, where a fixed box is inserted into that column, or at the start of the page before it, and then removed: the column and the page, whose heights follow content, grow and shrink back,
  each box after the new box moves once without entering the frontier; the frontier that finds them, the edges walked, the graph built, and the number of subtrees measured
  whole, as well as the children walked, missed plans, and rebuilt plans when moving them, are the same at 1k and 10k; when the column has a fixed height, all the
  cost of a frame is the same at 1k and 10k.
- The same page, where the page's own child list changes: a fixed box at the start or end of the page leaves (the one at the start is the first frame after layout from scratch),
  and another one arrives: no subtree is walked whole for measurement, not even the page itself; moving groups translate along with their cards and walk no children for placement. The frontier,
  edges walked, graph built, nodes measured, children walked, and missed and rebuilt plans are the same at 1k and 10k; at the end, no other box
  moves, and all the cost of a frame is the same at 1k and 10k, except for the page's own result (it records one placement per group).
- 100 edits walk the same union closure, each edge at most twice, and no item exceeds the sum of 100 separate edits.
- After 10,000 consecutive edits on 100k nodes, the following hold:
  - the retained cache's box, placement, both kinds of plans, and plan entry counts are the same as at the start;
  - the product frame keeps no full-layout copy;
  - retained used sizes have at most two variants per node, and intrinsic facts at most four;
  - no frame's scratch exceeds the closure of one edit.

Pages at 10k and above skip the per-pass guard (`skip_layout_verify`) and finish with a cold comparison;
1k pages are verified on every pass. The full matrix at 1k and 10k runs 1 and 8 edits; local edits and quiet-end
sequential edits at 10k and 100k run 1 and 100 edits, once each for shallow and 8-level nesting and for the three writing directions
(`issue259_contained_edits_cost_the_same_at_100k_{ltr,rtl,vertical_rl}`). Local edits are placed at the
opposite end from the quiet end: the 10k nested page has only 263 groups, so 100 groups in the middle would overlap the 100 groups at the quiet end,
warming those groups' retained state first so that the 10k page would do a little less than a fresh page; the 100k page does not overlap.

The following techniques keep the work of a local edit from growing with the document.

Reverse main axis (the block axis of vertical-rl, the inline axis of RTL). Origins are measured from the far end: when the container's main-axis size changes,
the children before the change translate by as much as the far end moves. Only boxes that actually moved are moved, and nothing is remeasured;
`ContainerPlan::placed_main` records which main-axis length those origins were placed with.

Containers that only moved. When a preceding sibling changes, a following container is only pushed aside: its placement plan holds except for the origin (placing it back at the original
position could reuse it as is), none of its children are on the frontier or lead to nodes on the frontier, and this pass has not placed it yet. In that case it does not
look up its children's sizes one by one, nor does it rebuild the plan: each child is placed at the new content origin plus the offset within the content box recorded by the plan, inside the original
containing block (for a grid child, its cell); children with their own plans translate down the same way. Absolutely positioned children whose containing block is this container
move with it; when the containing block is a higher ancestor or the initial containing block, they move only with that containing block and decide whether to replay by its current box. Fixed-positioned
children attach to the viewport and do not move. The boxes of the whole subtree must all be rewritten, which is real output; the plan only replaces the origin. The offset is exactly
the value added to the content origin during placement, so the translated positions are bit-for-bit identical to a layout from scratch. Children shifted by a difference at the far end of a reverse main axis, or by a preceding
line of a wrapping flex, have no exact offset, and subgrid children must read the inherited tracks; such containers still place their children again when they move.

Retained intrinsic facts. The budget varies with the live tree (four constraint variants per node), and the facts are stored grouped by content:
when a node's facts change, only its own variants are replaced; deleting a node deletes only its group. Neither scans the whole table.

Retained used size. A node may be measured under several constraints in one pass, but only two variants are kept: they are written back in the order measured
in this pass, keeping the last two measured, so the count is deterministic. The measure plan likewise keeps at most two per constraint; when a container's children changed in some pass
but it was measured under only one of those constraints (for example, the viewport width is scaled back and forth with a child changed in between), the other copy did not see this change and is marked
stale: the next time it is looked up, the children are walked again and the plan rebuilt in place rather than reused directly.

Result publication. A published result, as long as it exists, was built after its child list was last edited: insert,
move, detach, park, and despawn all first drop the parent's result, then drop upward to the first fixed border box (if the parent itself is
a fixed border box, it stops there). So when checking whether an ancestor's result still holds,
only the children covered by this publication need to be checked (located directly through `index_in_parent`); the children of a wide container do not
all need to be compared. A result's source (published first as a compatible write when layout writes back, then published once more by layout) does not count as a geometry change:
a result with unchanged geometry keeps its original object and original source. `layout_result_children_visited` records the children walked
while publication checks or rebuilds; under local edits it does not grow with the document, and only a container whose own children's placement actually changed
pays one rebuild for its own child count.

Timing threshold: `issue259_local_frame_time_scales_flat_to_100k` records the p50, p95, and p99 of one local-edit frame on a fixed machine,
and checks that p95 grows by no more than 1.75x from 1k to 100k:

```bash
cargo test --release -p nana-ui-runtime --lib issue259_local_frame_time -- --ignored --nocapture
```

PR CI uses only counters as the hard gate; timing is not judged in CI. The weekly Runtime performance workflow runs this with
`continue-on-error` and writes p50 / p95 / p99 to the job summary: hosted runners are not fixed machines,
so this step only records and does not block merges.

Constraint changes (#264). Viewport scaling, container resizing, dragging a splitter, and Dock changing panel ratios all go through the same constraint propagation:
- Style writes are classified by axis. Width and its bounds change only the children's inline constraint; height and its bounds change only the block constraint;
  flex grow, shrink, and basis change only the axis the parent's line runs along; box-sizing, aspect-ratio, spacing,
  alignment, and flow shape count for both axes.
- Viewport scaling seeds the document root by the changed axis, and below that only children that consume that axis's constraint are remeasured. A box that reads the viewport
  is seeded by the side it reads (`LayoutStyle::viewport_axes`): `vw` reads width, `vh` reads height, and `vmin` / `vmax`
  and `position: fixed` read both; changing only height does not touch `vw` boxes, and changing only width does not touch `vh` boxes. The engine records
  the viewport each document last laid out with, and whatever entry point it comes from, the engine adds these seeds whenever the viewport changes.
- Measure cache keys and layout plans compare only `viewport_basis`: the side of the viewport the box reads, with the side it does not read recorded as 0.
  A box that does not read the viewport is not invalidated by viewport changes, and a `vw` box is not invalidated by a height-only change.
- For a box whose size on an axis is its own definite length and does not read the containing block, the measure cache key does not include the available size on that axis.
  When the parent changes, it hits the cache directly and no longer walks its children.
- A sibling touched only by a position change no longer pulls its whole subtree into the frontier. When it is placed again at a new origin its subtree is placed along with it;
  when the origin does not change the subtree stays as it was.
- When the main-axis size changes, children that read the container's main-axis size (main-axis percentages, fill, basis) no longer reuse the old plan.
- When a child changes size along its parent's line (the inline axis of a row, the block axis of a column), the size given to flexible siblings on the line (`flex_grow > 0`
  or fill on that axis) changes too: they are also recorded as constraint seeds, and below them only children that read that axis are
  remeasured. When the first panel of a splitter widens, the fill panel beside it narrows, and the children on both sides are asked. A container, regardless of
  `display`, is laid out as one line along `direction` (column by default); grid, inline formatting contexts, and vertical writing do not take this path
  and are still covered by the measure cache keyed by available size.

All counts live on `WorkCounters`:
- `constraint_change_seeds`: seeds that changed a child's constraint;
- `constraint_dependents_considered` / `_remeasured` / `_skipped`: children asked whether they consume this change,
  and of those, the ones remeasured and the ones that kept their measurement;
- `resize_text_relayouts` / `resize_text_reshapes`: text laid out again against the new box in the pass after layout,
  and of those, the ones that needed reshaping (0 when only the wrap width changes);
- `resize_context_solves`: formatting contexts solved from scratch in the pass triggered by a constraint change, that is, containers whose plan could not be reused
  or that had no plan.

The gates are in `world/issue264.rs`. "Measured" reads the nodes layout actually computed (the test-only
`layout_engine::measure_trace`, recorded only after getting past this pass's memo and the retained intrinsics), not the frontier's
MEASURE bit: a changed memo key also remeasures, and the frontier cannot see that work. The layout-verify guard's own full
layout pass is not recorded.
- Drag a real `SplitPane` 240 times in a 10k-node workspace. Fixed boxes and Dock are not measured, text is not reshaped,
  no nodes are added or removed, and the end matches a cold layout; the 1k workspace is compared with a full layout on every pass. Each drag has two constraint seeds:
  the dragged panel and the fill panel beside it.
- When only width changes, boxes that read only height are not measured; when only height changes, rows that read only width are not measured.
- Fixed-size subtrees and their descendants are not measured.
- The cost of one drag is the same at 1k and 100k nodes, and so are the children asked.
- The cost of viewport scaling is the same at 1k and 100k nodes, and Dock does not enter layout; `vw` boxes are measured only when width changes,
  and `vh` boxes only when height changes.
- When changing flex ratios the way Dock does, boxes that read only height are not measured; the same holds when dragging a real `Dock` split ratio,
  and paragraphs that read width are remeasured every time.

For a box whose height follows content, when nothing in its subtree reads the block-direction available size given to it, the measure cache key does not include that axis
(`world/block_reads.rs`). Things that read this axis: percentages on itself or descendants, fill height and its bounds,
flex basis that follows the containing block, column-direction wrap that changes columns by available height, any grid, aspect-ratio, logical box edges,
and a declared vertical writing mode. Each node records whether its own style counts as reading, what role it plays in its parent's measurement (normal, cut off by its own height,
passing through because it has no box of its own, not participating), and how many children pass the read upward; changing a style reclassifies one node, changing the child
list adjusts a count by one, and the walk continues upward only when some ancestor's conclusion changes, independent of document size; parked subtrees keep their conclusions.
The reference layout of the layout-verify guard does not use this rule and measures with the real height, so a read the predicate misses shows up as a box mismatch;
the guard also recomputes the aggregate from scratch and compares.

The same aggregate also fixed an omission: descendants' percentage heights resolve against the available height passed down by the engine, whereas the frontier and container plans
used to treat them as auto per CSS semantics, so such rows stayed at their old height when only height changed. Now children whose subtree passes the read upward get the
block-direction constraint from the frontier, and the column container's plan also records them as depending on the main-axis size.

#259 added resize storms: on 1k, 10k, and 100k pages, the viewport is resized 240 times in a row at the same cost; the vertical-rl
page is compared with a full layout on every pass.

For Gate E (no relayout while the available size still falls within a cached envelope segment) see "Dynamic Layout" below: a toolbar in the first panel whose
solve overflows builds no envelope and does not solve from scratch while dragged within the same cost tier; each frame only redistributes the retained tier.

Replaced content (#263). The three revisions of images, video, HostTexture, and CustomRender are handled separately:
- Content revisions (pixels, video frames, texture generations) and fit and sampling only repaint, with zero layout, and do not advance the intrinsic generation.
- Only intrinsic metadata enters layout: the natural size reported by the resource. The size decoded from an image and the resolution of a video or texture are reported with
  `MutationQueue::set_replaced_metadata`.
- Each resource records the nodes that display it (a resource-to-node index), and a report notifies only those nodes without scanning the document. Of those, only the boxes
  that read the natural size (width or height is auto) relay out; those whose width and height are both decided by style only repaint.
- A box with neither width nor height set uses the natural size; with only one side set, the other side is derived from the natural aspect ratio. The derivation happens on every path that obtains a
  used size, including intrinsic facts reused across contexts (#202).
- The natural aspect ratio behaves like `aspect-ratio` in style: the `stretch` of a flex row or grid does not stretch a replaced box that only sets its width
  to the row height or area height; the height is still derived proportionally from the width.
- min/max do not distort the natural aspect ratio (#202, CSS 2.1 §10.4). With neither width nor height set, the natural size is scaled proportionally first by the width bounds, then by the height
  bounds; when the two sides' bounds conflict, each is clamped and the ratio yields. With only one side set, the other side is derived proportionally and then constrained by its
  own min/max. A 120×80 image with `max-width: 60px` gets 60×40; with `min-width: 240px; max-height: 40px`
  it gets 240×40.
- When a node becomes or stops being replaced content, the baseline it aligns by changes too, and it relays out.
- When an image in Markdown resolves its size, the block list changes, and the document's new height enters layout.
- A container whose width it decides itself no longer falls back to walking all its children because a child widened on the cross axis.

All counts live on `WorkCounters`:
- `replaced_content_updates`: updates that changed only content, not the resource;
- `replaced_intrinsic_metadata_updates`: resources that reported a different natural size;
- `replaced_layout_seeds`: layout seeds emitted by replaced content;
- `replaced_content_only_layout_invalidations`: seeds emitted by content-only updates; should be 0;
- `resource_intrinsic_dependents_notified`: nodes notified through the resource index.

The gates are in `world/issue263.rs`:
- 1200 video frames cause zero layout, and the intrinsic generation does not change;
- when an image's natural size arrives, only the box that reads it relays out; a fixed 300×200 box only repaints; a box 400 wide derives its height proportionally;
- one shared image notifies 100 nodes through the index at the same cost in 10k and 100k documents, and only the 50 that read the size relay out;
- fit and sampling changes cause zero layout;
- 100 consecutive video resolution changes under an explicit size cause zero reflow;
- every pass is compared with a full layout, and the end with a cold layout.

Replaced content is one kind of layout item (#202). Images (`content_image`), `Video`, `GpuTextureView` (HostTexture),
`GpuView` (GPU canvas / render target), and a `CustomRenderNode` attached to an ordinary element (a presentation anchor) all enter the same index by the resource they display
(`ReplacedResource`); `LayoutNode::content` is `Replaced` for all of them; their participation in Flex, Grid, inline, and
absolute is the same as an ordinary box; and the same natural size gives the same box. There is no per-component size path.
The components' default styles differ: `Video`, `GpuTextureView`, and `GpuView` fill (`fill`) when width and height are unset, and read the natural size only when `auto`
is written; an image with no width and height set has no area until its size arrives.

There is only one copy of fit math: `ContentFit::fitted_size` (`nana-ui-core`). Fit is painting; it only reads the box layout gives and does not write back to
layout.
- For `CustomRenderNode.fit` (video, HostTexture, Avatar, Thumbnail), the painter computes the target
  rect with `LogicalRect::fitted`, and `GpuTextureView::texture_uv_at` and `Thumbnail::picture_rect` convert coordinates with the same function.
- An image's `object-fit` is written on `PaintStyle::object_fit` and on `content_image`'s `BackgroundImageFit`. The painter's
  `url()` path converts it to `ContentFit` through `BackgroundImageFit::object_fit` and then computes the size; the keywords of `background-size`
  go through here too; only `background-size` written as lengths (`Length`) follows the background's own rule. Images additionally honor
  `object-position`; HostTexture is centered.
- `ImageViewer`'s initial view is `scale-down`.
- Fit for `<video>` and `<gpu>` in markup: the cascaded `object-fit` takes precedence and the `fit` attribute is a presentational hint;
  both accept all five keywords (previously the attribute accepted only `contain` and treated everything else as `fill`).

Gates in `world/issue202.rs`:
- A: the same pair of images (no size, width only) placed in Flex, a stretched Flex row, Grid, a stretched Grid, inline, and absolute;
  after the natural size arrives, size, participation, and cold layout are compared one by one.
- B: images and CustomRender sit with their bottom edge on the text baseline in a flex row with `align-items: baseline` and in a native paragraph.
- C: each of the five kinds of replaced content above is built the way an application writes it; in six contexts content is always `Replaced` and participation
  is the same; after the same natural size arrives the boxes are the same, with 5 replaced seeds and 0 whole-document fallbacks, compared with a cold layout.
- D: sizes for eight combinations of min/max and natural aspect ratio under Flex, Grid, inline, and absolute, compared with a cold layout.
- E: reporting the same size again produces no work; switching to another resource of the same size gives 0 seeds and 0 measures; switching to a resource of another size
  relays out only the two boxes that read it.
- F: the fit `Video` and `GpuTextureView` get from markup is the same value as an image's `object-fit`.
- G: atoms on the same line in ordinary block flow align by baseline: an inline-block without line boxes, an inline-block whose `overflow` is not visible,
  and an image sit with the bottom margin edge on the baseline; an inline-block with text sits with its last line's baseline on the baseline; and the block's
  height is the span of the line box from the highest top to the lowest bottom. Swapping in a taller atom, text wrapping into several lines in a box of the same size, and text arriving in a previously empty atom
  each place the line again, matching cold layout item by item.
- H: an inline-block whose line boxes are all in descendants sits with the baseline of its last descendant line box on the line's baseline: a fixed box with a label below an empty block,
  a label wrapped into several lines inside a block (takes the second line, not the first), two inline-block labels each on its own line (takes the second line), and a label inside a fixed-
  size block. Descendant text wrapping, the block above getting taller, and text wrapping inside a fixed-size block each place the line again; a text change
  that does not change the line count moves no box. Each step matches cold layout item by item.
- I: the line boxes of ordinary block flow start from the block's strut: when `line-height` equals the font size, an empty inline-block leaves the font's
  descent below it, and the default line height adds half a leading; a very short atom's top is no higher than the strut's ascent; a line occupied by a block-level child alone has no
  strut; lines of a native paragraph likewise leave a gap below atoms.

`nana-ui`'s `scene_paint/replaced_context_tests.rs` has two more: a host texture moving across contexts does not rebind
its texture; under all five fits the `url()` path and the host-texture path give the same target rect.

Baseline: replaced content aligns by its bottom edge (`aligns_by_bottom_edge`), decided by layout, not by the painter correcting y. Inline formatting contexts
(the line boxes of a native paragraph `InlineFlow`, and line boxes in ordinary block flow sharing a line with inline-block atoms) both align by CSS's default
`vertical-align: baseline` (CSS 2.1 §10.8.1), and both read the same function
`LayoutInputMap::atom_line_metrics`:
- An inline-block sits with the baseline of its last in-flow line box on the line's baseline; the baseline value is the last-line baseline from `baseline_metrics`.
  An inline-block whose line boxes are all in descendants (`UiWorld::line_boxes_are_descendants_in`: no text, control, or
  paragraph of its own, only in flow) takes the position of the descendant holding the last line box (CSS 2.1 §10.8.1):
  `inline::descendant_baselines` reads, from the sizes measured for the children in its content box, where placement will put them:
  the segment that arranges lines, distributes main-axis free space, aligns on the cross axis, and converts to the page is `placement::arrange_flow`,
  called by both placement itself and the derivation (#394); the derivation does not arrange separately. The last line box is either the line holding an atom (`ArrangedLine`'s
  `atom_baseline`) or a block occupying a line alone, in which case its own last-line baseline is taken (recursively, if its line boxes are also in descendants).
  The same derivation pass also searches forward for the first line box to get the first baseline (#382). First and last line boxes are searched in flow order. Each pass records the result per node and used
  size in `LayoutInputMap::derived_baselines`; every place that reads an atom's ascent (block measure and placement,
  measure plan comparison and increments, native paragraphs) calls `prime_atom_baselines` first, and `baseline_metrics` then reads it, so
  there is only this one derivation. `justify-content`, reversal, wrapping into columns, floats beside the line (gates E, F, G in `world/issue382.rs`,
  css-parity `T-FL07`), auto margins, equal-width line splitting, and `clear` in block flow all follow placement's result,
  and there are no remaining cases that fall back to an approximation. For the baselines of flex and grid containers see below; subgrid and vertical writing do not derive and are still the approximation of the first line of their own font;
- replaced content, inline-blocks whose `overflow` is not visible, and inline-blocks without in-flow line boxes (`UiWorld::atom_sits_on_line_box`)
  sit with the **bottom margin edge** on the baseline, and `margin-bottom` raises them;
- the line box height is the span, after alignment, from the highest top margin edge to the lowest bottom margin edge; a block-level child occupies a line alone, laid out from the top of the line.
- Line boxes in ordinary block flow with inline-level children start from the block's **strut** (`inline::line_strut`): a zero-width inline box using the block's
  own font face, font size, and `line-height`, whose ascent and descent are the real metrics of the block's own face (#382);
  half the difference between `line-height` and their sum is added above and half below. So an empty inline-block sitting with its bottom edge on the baseline
  leaves the face's descent plus half a leading below it, and a very short atom's top is no higher than the strut's ascent. A line occupied by a block-level child alone has no
  strut. `line-height` reads the block's own `LayoutStyle` (on the CSS path inherited through `inherit_typography_from`),
  not `ComputedStyle`.
  - The metrics are the same as for native paragraphs (`nana-text`'s `LineStrut`): the text stage asks the shaper on the block's behalf
    (`TextShaper::strut_metrics`; `NanaTextEngineShaper` reads `NativeTextEngine::strut_metrics`),
    stores the value per px of the block's font size in `UiWorld::block_strut`, and layout multiplies by the font size to read it. So with the same face and line height, ordinary block flow
    and native paragraphs leave the same gap below an empty atom. The product UI face Noto Sans SC has ascent / descent
    1.16em / 0.288em; at 16px with line height 1.2 the strut's descent is about 2.6px (for Chrome measurements see css-parity
    `T-I04`, `T-I06`, `T-I07`).
  - Only blocks that need one hold a strut (`UiWorld::wants_block_strut`): not a flex / grid container, no line boxes of its
    own, and in-flow inline-level boxes among its children. When the shaper reports no metrics (the default implementation returns `None`), it falls back to the shared approximation
    `TEXT_APPROX_ASCENT_EM` (0.8em), with the remaining 0.2em of the em as descent.
  - Invalidation: changes to the block's font family, weight, italic, or variable axes, changes to a child's display, position, or float, and adding or removing children all mark the
    block (and the parent of the changed child) as a candidate; a change to the font set or language hint (`TextBackendEpoch`) rereads every holder.
    The text stage rereads only candidates and schedules layout through the text-metric-change path only when the value changed; changing only the font size does not reread (the stored value is per px),
    and language does not take part in choosing the preferred face, so it does not reread a new value either. The IFC measure plan records the strut it used and compares it.
    In tests and `layout-verify` builds, after each text stage every retained strut is compared with a fresh reread
    (`UiWorld::audit_block_struts`), and a missed candidate is reported as an error immediately. The gate is B in `world/issue382.rs`.

A flex row with `align-items: baseline` aligns items by their first baseline; replaced content takes the bottom of its border box (CSS Align synthesizes the baseline from the border box in flex / grid).
An item whose line boxes are all in descendants (`UiWorld::aligns_by_first_baseline` and
`line_boxes_are_descendants_in`) takes the position of the first in-flow line box, the same derivation as an inline-block's last-line baseline
(`inline::descendant_baselines`): before measure publishes intrinsic facts, before placement reads the line's baseline, and before the flex plan compares
baselines, `prime_first_baselines` runs first, and `baseline_metrics`' first baseline then reads it; in tests and `layout-verify` builds,
reading without priming is reported as an error immediately. Such items also read content (`atom_baseline_reads_content`), and a fixed size is no longer a metric boundary:
changes to the font size or text of descendants inside, or to the height of a block above, realign the line; changes that leave the first line in place move no item. The gate is
D in `world/issue382.rs`. When grid items align by baseline, those items in the same starting row share one baseline: each item aligns its own
first baseline plus top margin to the deepest one in the row (`grid::arrange_grid_2d`, #394). The line's baseline is published on the TextLine fragment of `LayoutResult`. `LayoutStyle` has no
`vertical-align`; the default `baseline` is the whole behavior. This is not a rule specific to replaced content.

A flex container's baseline (CSS Flexbox §8.5, per Chrome 152 measurements, #394) is read from the item positions given by `arrange_flow`
(`inline::flex_first_baseline`): take the topmost line on screen (placement arranges the last line first for `wrap-reverse`,
so it is the first arranged line); if the line has items aligned with `align-self: baseline`, take their shared baseline, otherwise take the first baseline of the item
closest to the inline start in that line; a column flex takes the topmost item in the column closest to the inline start. So `row-reverse`
takes the leftmost item and `column-reverse` the topmost, not the first in flow. An item without line boxes takes the bottom of its border box.
A flex container's last baseline is its first baseline: an inline-block ending with a flex container also aligns by its first baseline in Chrome.
A box with no text of its own and no line boxes anywhere in its flow has no baseline and takes the bottom of its border box per CSS Align §9.1
(`baseline_metrics`), no longer the font's approximate ascent. Any in-flow item of a flex container may be the one it takes its baseline from,
so these items also read content (`atom_baseline_reads_content`), and changing the font size inside a fixed-size item also realigns the outer line.
A grid container's baseline (CSS Grid 2 §11.8, per Chrome measurements) is likewise read from item positions (`grid::arrange_grid_2d`,
`inline::grid_first_baseline`): take the first row where an item starts; if the row has items aligned by baseline, take their shared baseline, otherwise take
the first item in grid order in that row (the smallest column number, rightmost under RTL, regardless of DOM order; an item spanning rows counts in the row it starts in);
without line boxes take the bottom of its border box; the last baseline is again the first baseline. Grid items likewise read content.
The gates are in `world/issue394.rs`, and the Chrome measurements are css-parity `T-FX01`–`T-FX03`, `T-GB01`, `T-GB02`. Not done yet:
inline-flex and inline-grid are still laid out as block-level on a block's line (not as inline atoms); the cross size of a flex row aligned by baseline and the size of grid
tracks do not account for the space moved out by baseline alignment (CSS's baseline shim); grid's distribution of spanning items to auto row sizes
and the width of a block-level grid inside an inline-block differ from Chrome.

An atom's baseline moves with its content, and invalidation follows it: a fixed-size inline-block whose `overflow` is visible in ordinary block flow
is no longer a metric boundary (`UiWorld::atom_baseline_reads_content`), so changes inside it realign the line;
in-flow blocks in such an atom's flow (and further down) are where the last line box is searched for, so a fixed size is not a boundary there either, and the frontier does not stop
there (`has_definite_size` reads the same function). Inline-blocks whose `overflow` is not visible and replaced content are positioned only by their own box,
so they are still boundaries, and changes inside them do not relay out the line. The IFC measure plan records each atom's ascent on the line; when the box size does not change
but the ascent does, the block is remeasured; when the ascent does not change (descendant text changed but the last line did not move), the plan is reused and no box moves.

#259 added replaced content workloads: at the end of 1k, 10k, and 100k pages, video frames, image natural sizes, and shared images cost the same.

The host reports the sizes decoded from images. How big a `url(...)` image is, only the painter knows:
- An `<img>` with no size set has no area before its size arrives and is not painted. The painter still loads the image for a replaced box with no area
  that falls within the clip region; it just does not paint it.
- When the painter prepares a paint target fresh, it records the natural size (available only after decoding) of every quad it paints with content images.
  It hands it over once when a quad starts showing an image or when the image decodes to another size; when painting into a box with no area it hands it over every time, and this
  box may be waiting for that size. Quads no longer painted are forgotten. The painter writes only its own records and does not touch the document.
- The built-in host takes this target's records after the window presents (`SceneWgpuPainter::take_image_natural_sizes`),
  commits them through `commit_image_natural_sizes` as `set_replaced_metadata(ReplacedResource::Url(..), ..)` to
  this window's document, and requests the next frame. Boxes that read the natural size relay out on the next frame.
- Only images that some node in the document displays (`UiWorld::shows_replaced`) and whose size the document does not have yet are committed. Images that Markdown and
  Painter draw themselves do not enter the document.
- Remote images are loaded in buckets by the document's fetch host, and a size is handed only to the document of the target that painted it; an image one document's policy allows
  does not leak its size to another document. When the window is hidden and only an output is painting, the output target's records are taken.

The host-side gates are in `nana-ui`'s `scene_paint/natural_size_tests.rs`: one case each for a data URL and a local HTTP image.
A box with no size set first has no area; once the image is ready and the host commits, it is laid out and painted at the decoded size; after that nothing more is reported.

`GpuTextureView`'s HostTexture does not report. Such a texture is prepared by `painted_extent` by default, which is the paint
size layout decided; treating it as the natural size would make auto-sized nodes and the texture enlarge each other. A host producing content for its own texture reports it with
`set_replaced_metadata` itself when needed.

Virtual lists (#262). A row height change moves the list by the difference without scanning the logical collection:
- The row height index `VirtualListLayout` is chunked by 512 rows. A run of rows with the same height is stored as one count: one million rows at the estimated height
  take only about two thousand chunks. Two Fenwick trees (chunk total height, chunk row count) answer prefix sum and offset queries: O(log C), plus
  one segment within a chunk.
- Measuring a row's new height changes only its chunk and O(log C) index entries. The first time a chunk stops being uniform, its 512 rows
  are expanded once. Inserting rows touches only the chunk they land in, and the chunk index is rebuilt only when a chunk splits; deleting rows rebuilds the chunk index every time. A rebuild is O(C).
- The tree `VirtualTreeLayout` also stores each row's descendant count and depth in chunks. Expanding and collapsing look for ancestors upward by depth, skipping whole chunks that are all
  deeper than the target, without scanning the rows before the parent row by row.
- A list measured by content (`VirtualListItems::measured`) reads only the heights of mounted rows. When the data has not changed (fingerprint
  and row count both unchanged), the mounted rows are still at the indices they were last placed at and are not looked up by key again. When the window is still the same indices (measuring
  only moved rows), the window's keys are not looked up again either.
- When a row grows taller, the mounted rows below it change only `top`, as an origin translation; the list's total height changes once. The measure plan of a positioned container ignores
  insets when comparing styles, so rows where only `top` changed reuse their measurement instead of being remeasured whole.
- `each_virtual(..).measured()` rereads row heights after every layout. When row content later grows taller (expanding, wrapping), the rows below
  move along.

All counts live on `WorkCounters`:
- `virtual_row_metric_updates`: rows that changed in the row height index;
- `virtual_prefix_index_updates`: index entries written since the list last placed rows, including the consumer's own inserts and deletes;
- `virtual_rows_remeasured`: rows for which the list got a new height, that is, rows measured at a new height or given a new height;
- `virtual_rows_repositioned`: rows whose placement changed;
- `virtual_logical_rows_scanned`: the number of key-by-index and index-by-key lookups;
- `virtual_scroll_extent_updates`: the number of times the list's total height changed;
- `virtual_rows_materialized_from_layout`: rows newly created for the window.

The gates are in `world/issue262.rs`. One million rows, a 40-row viewport, 8 rows of overscan above and below:
- A: one visible row grows by 8 px. No logical row is looked up (cap 64), only this one row is measured; index writes stay within one chunk plus
  O(log C) entries; the rows below only translate their origin.
- B: a cell changes from "99" to "98" and the row height stays the same: the list's total height and the other rows' placement are unchanged, and the list does not relay out.
- C: row 21 of the window grows taller: the placement of the first 20 rows is unchanged, and the rows after it only translate; the measurement amount is the same as when a row near the top of the window grows.
- D: data of a row that is not mounted changes: zero layout. If the consumer changes the fingerprint because of it, only the window's keys are looked up again. Scrolling there
  shows the new data.
- E: in a tree of one million rows, expanding a visible node with 10 children creates only those 10 rows; the rest are not rebuilt, and only the window's
  keys are looked up.
- A table cell's text changes with its size unchanged: zero layout, and the window is not placed again.

#259 added virtual list workloads: one row growing taller in lists of 100,000 and 1,000,000 rows costs the same. Only the index depth
O(log C) varies with the collection.

Not done yet:
- When the data of `each_virtual` (`Keyed`) changes, all items are regrouped, O(N): it receives the whole list. With the retained
  API (`sync_virtual_list_*`) the data stays with the consumer, and there is no such step.
- Tree row heights are fixed; measuring by content is only for lists. Virtual table column widths are fixed; there are no content-sized columns.
- When text is wider than its box and cannot fit a single word (for example a list width of 0), its width changes before and after relayout, and incremental
  layout and full layout disagree. The gate gives the list a width; this problem is tracked separately.

Responsive rules (#265). A node can switch styles by the size of some container: `MutationQueue::set_responsive(node,
ResponsiveRule)`.
- A rule reads one axis of the container's content box: inline or block (by the container's writing direction), or width or height (independent of writing
  direction).
- There are three kinds of container:
  - the node's parent (`ResponsiveContainer::Parent`);
  - a specified node (`ResponsiveContainer::Node`);
  - the nearest query container upward (`ResponsiveContainer::Nearest { name }`), which is CSS `@container` semantics:
    an ancestor whose `container-type` is not `normal` and that answers this axis. `inline-size` answers only its inline axis; `size` answers both.
    When a name is written, the name must also be in `container-name`. When no such ancestor is found, the node keeps the style the author wrote.
    When an ancestor changes `container-type` or `container-name`, or the node moves, the rules below look for a container again.
- Query containers do not apply size containment: their size still follows content. When a variant in turn grows or shrinks the container, the convergence rule below stops it,
  which differs from browsers.
- Breakpoints split sizes into a finite number of buckets, with at most 16 breakpoints. Each bucket can carry one variant:
  - `ResponsiveRule::below` / `at_least` are written with closures and compared by whether they are the same closure;
  - `ResponsiveRule::from_buckets` gives the breakpoints and each bucket's `StyleVariant` directly.
  `StyleVariant::between(base, target)` is data: it records the fields that differ between two layouts and the target's values, and compares by value,
  so sending the same rule again is a no-op. A bucket without a variant uses the style the author wrote.
- A variant is written onto the author's style, as if the author had written it directly: design intent is resolved after the variant, the same way it treats the author's style.
  Every field in `LayoutStyle` can change, and beyond layout, color, background, font, and opacity change too.
  - A node in a bucket with a variant has two styles: the one the author wrote (`node_style()` returns this one, the projection compares against it, and later
    `SetStyle` starts from it), and the effective one (author style plus variant, which the pipeline reads).
  - Switching buckets is one style change, classified, marked dirty, and seeded by the same rules as `SetStyle`, entering the same incremental layout. A variant that only changes
    painting only repaints and emits no seeds.
- Rules are indexed by container when registered. For the axes of a container's content box that a layout writeback changed, only rules reading that axis are evaluated at the end of that commit,
  without scanning the document. Rules still in their original bucket change nothing.
- Convergence: bucket switches in one frame take at most 4 rounds. When a rule would return to a bucket it has already left in this frame, or the rounds run out, it stays in
  the current bucket (counted as one fallback) and the frame converges; the next size change reevaluates. Rules whose variants change their container's own size
  thus stop deterministically and do not oscillate back and forth under 240Hz resizing.
- When a node moves under another parent, a `Parent` rule reads the new parent instead; when it is taken off (Detach, Park) it reads no container and returns to its own layout; it also returns to its own layout after the rule is cleared.

All counts live on `WorkCounters`:
- `container_query_size_changes`: changed container axes read by rules;
- `container_query_rules_evaluated`: rules evaluated; only those whose axis changed are evaluated;
- `container_query_results_changed` / `container_query_results_unchanged`: rules that switched / did not switch buckets;
- `container_query_downstream_invalidations`: layout seeds emitted by variants of switched buckets;
- `container_query_convergence_rounds`: evaluation rounds in which some rule switched buckets;
- `container_query_cycle_fallbacks`: bucket switches stopped for convergence.

The gates are in `world/issue265.rs`:
- A: 100 rules under one panel; a panel width change evaluates only those 100, at the same cost in 10,000- and 100,000-node documents;
- B: 240 consecutive resizes within the same bucket: no rule switches buckets, variants emit no seeds, and rows are not measured;
- C: crossing one breakpoint: only the 10 rules that break there switch buckets, each emitting one seed, and the other 90 fixed rows are not measured;
- D: 8 levels of nested containers with 50 fixed leaves each: resizing evaluates only 7 rules and does not touch the leaves, converges in at most 4 rounds with no
  fallback, and the result matches a cold layout of the same input;
- E: the index stores only registered rules; one frame's bucket-switch history is cleared by the next frame.

#259 added responsive workloads: at the end of 1k, 10k, and 100k pages, resizing within a bucket, crossing a breakpoint, crossing back, and nested resizing cost the same.

Every layer projects to the same kind of rule:
- Rust: `MutationQueue::set_responsive`;
- `.vue`'s `<style>`, `view!`, `stylesheet!`: at build time `@container` is compiled into the stylesheet, and the rule is installed when the element mounts
  (see [Reactive views](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/reactive-view.md); `css!` writes only declarations, and `@container` inside it produces a warning).

The CSS path only produces data and does not measure containers (see "`@container`" below).

A formatting context with a dependency index can use `LayoutDependencyGraph` to express parent constraints, containing blocks,
writing direction, and the local coupling of flex/grid/inline. Runtime mutation authority and the product frame publish
typed seeds merged per node. Layout scheduling consumes typed seeds directly.
Uncertain structural changes keep the root-level recompute boundary.

## Layout recursion and the stack

For the overall stack contract (runtime environment, tree depth limit, which traversals may recurse, gates and comparisons with peers) see [Resource budgets](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md). This section covers only layout's two recursions.

Measure (`measure_node`) recurses by tree depth: sizes are computed upward from children that received constraints passed down. Every level first goes through `layout_engine::recursion::level`:

- It reads the remaining stack of the current thread (`crate::stack::remaining`: read by `stacker` on desktop platforms and OpenHarmony; iOS and Android read the thread's own stack bounds); when less than 256 KiB remains, this level continues on a freshly allocated 4 MiB stack segment (`stacker::grow`). So on every platform the framework supports, debug or release, whatever the thread stack size, no tree depth can make measure run out of stack.
- The same reading also prices this level: how much less stack remains at this level's entry than at the parent level's entry is the stack this level used. Each pass takes the maximum and publishes it as `WorkCounters::layout_measure_stack_per_level` and `layout_placement_stack_per_level` (bytes, maximum within a frame; 0 when the platform cannot report it).
- The tests in `layout_engine::stack_budget` set a per-level budget (measured separately for the block, flex column, and grid routes), and lay out a 400-level tree on a 1 MiB thread. The fallback prevents crashes; the budget keeps each level's frame from growing back.

Placement (`place_node_scoped`) needs no fallback. After measure, every box's size is settled, so any node can serve as a boundary: one call descends at most `recursion::SPAN` levels (24 in release; in debug each level's frame is more than three times release's, so it cuts at 8 levels; both builds use about 90 KiB). A node at that level writes its own box and then queues the rest of its subtree as a `Boundary` in this pass's queue; before returning, the outermost call places the queue in enqueue order, and more can be enqueued while placing. So the recursion depth of placement never exceeds `SPAN`, independent of tree depth, and when the call returns the caller still sees the whole subtree placed. `WorkCounters::layout_placement_boundaries` records how many subtrees this pass placed from the queue. In test builds `SPAN` is 2, so every runtime test goes through boundaries.

The precondition for cutting this way is that placing a subtree reads only its own inputs (where it stands, how big it is, containing block, fonts, viewport) and never what sibling subtrees wrote while being placed, so placing it later gives the same result.

Placing one level takes two steps. `lay_out_children` walks the container's children once: measures, packs floats, breaks lines, aligns, solves the 2D grid, settles a box for each in-flow child, and returns the `Descend` list of pending placements and a draft plan; it places no subtree itself. `place_node_body` enters those subtrees in turn only after that frame exits, then records the plan and places floats and positioned children, in the same order as before. The three fast paths that reuse plans (`place_moved_container`, `place_edited_child_list`, `place_from_plan`) are each a function of their own and only take stack when reached.

Per-level stack on x86_64 Windows (MSVC): placement went from 25.8 KiB to 9.0 KiB in debug and from 4.8 KiB to 3.2 KiB in release; measure is 12.2 KiB in debug and 2.5 KiB in release. On aarch64 macOS, release placement was 2.7 KiB per level, and after splitting into a head (`place_node_head`, which writes its own box) and a body (`place_node_body`, where boundaries resume) it is 3.5 KiB; debug placement is 10.9 KiB and measure 11.9 KiB. Placement no longer goes deeper than `SPAN` levels, so the per-level price times `SPAN` is its whole stack (about 86 KiB in release, about 88 KiB in debug). The head returns before the body runs, and the three fast paths and `lay_out_children` are marked `#[inline(never)]`, so a level occupies only one of their frames at a time.

Building nodes from views (`mount_view*` through `ViewBuilder::nest_dyn`, one level per element) also recurses by depth, and every level also goes through the same fallback (`crate::stack::level`, sharing the 256 KiB red zone and the 4 MiB stack segment with measure). `view::tests::a_view_deeper_than_the_stack_still_mounts` mounts, lays out, and drops a 400-level view on a 1 MiB thread.

Several tree-walking queries that layout reads no longer recurse by depth either: the line box query (`UiWorld::holds_line_box_in`) uses an explicit stack on the heap and is memoized per node within a layout pass (`LayoutInputMap::line_boxes`), so a chain of boxes asking level by level is each settled only once instead of each level walking its own subtree again; the upward walk along ancestors for an atom's baseline (`atom_baseline_reads_content`) is a loop; and when publishing layout results, each box's clip and containing block (`layout_anchors`) are memoized per node within one publication and derived from the ancestors' results, instead of each box walking to the root.

### Roadmap: subtree-level scheduling, recursion within subtrees

The goal is a hybrid architecture. Measure is bottom-up and keeps using recursion plus caching; placement is managed by a scheduler in units of subtrees, recursing within each subtree. In order:

1. Done: the recursion fallback and per-level stack budgets; placement split into two steps, "settle the boxes" and "enter the subtrees".
2. Done: placement boundaries. A node reaching `SPAN` levels becomes a scheduler task unit (`Boundary`), and recursion depth within a subtree does not exceed `SPAN`; a boundary's result depends only on its inputs.
3. Measure levels: move the non-recursive parts of `measure_node_level` out of that frame.
4. Scheduling: slice by frame budget, with unfinished subtrees reusing the previous frame's results as a whole (double-buffered results); order by distance from the viewport; cancel an unfinished subtree when a new change invalidates it. Queue length and slice count go into `WorkCounters`. Boundaries currently cut only at a fixed depth; scroll containers and `layout_isolation` nodes are the next batch of cut points.
5. Place mutually independent boundary subtrees in parallel: each subtree writes its own output, merged at the end.

The slicing granularity of the measure stage is limited by content dependencies: a container whose size is decided by content passes the dependency all the way up to its ancestors, and such a chain can only be one unit. Very large flat lists rely on virtualization, not scheduling.

## Dynamic Layout

When one line does not fit, the flexibility a child declares is compressed by cost (#207). The contract and the solver live in `nana_ui_core::dynamic_layout`; text (#211), components, and containers use the same set of types, with no second cost model.

The contract (#208):
- A child exports a bounded envelope `EnvelopeShape` to its parent: at most 8 segments, each carrying a capacity, a visual cost per unit `LayoutCost` (a finite value or an explicit `Forbidden`), an adjustment kind, and an execution class `ExecutionClass` (PlacementOnly < LocalBox < LocalReflow < Structural). Past 8 segments, the two most expensive segments in the same class are merged, and the cost only rounds up. The envelope carries no preferred size; that is still the authority of intrinsic measurement.
- Cost says how much worse the result looks, not how slow the computation is; the class says how much work it takes.
- A node declares through `LayoutStyle.adaptation: Option<AdaptationProfile>`: as a child, `inline` / `block` say how far padding, the gap between children, and the gap inside a box can each shrink, and at what cost per pixel, and `aggregate_children` says to hand the children's flexibility to the parent as one; as a container, `solve_overflow` says whether its own line is solved when it overflows.

Solving (#210): `solve_deficit` first uses each class's capacity sum to decide whether the gap can be covered at all (without reading segments), finds the cheapest class that covers it, takes the cheaper classes whole, and does not look at the more expensive ones; within that class it consumes level by level from lowest cost to highest, the level that covers the gap is shared by capacity, and the remainder goes one unit at a time in participant order. Everything is integer arithmetic in 1/64 px, the same input gives the same result, independent of the order of building and scaling. When the segments taking part in one class exceed the budget, they are shared in proportion to class capacity, the same every time (`budget_fallbacks`). Discrete structural adaptation must be allowed explicitly, and a candidate's fixed cost prunes as a lower bound.

Hierarchy (#209): a sequential container merges and normalizes its own gap with the segments of its children's envelopes; on the cross axis it scans the breakpoints, and the cost is the sum of each child's next unit cost for every child over the target. A parent reads only the summary and never trial-lays out its children.

Wiring (#212):
- Nothing is solved by default. An ordinary Row / Column / Stack / Grid still overflows or falls back to the old shrink when it overflows, the solver runs zero times, and envelopes are queried zero times; a child's declaration is just a value in the style, and costs nothing until a container reads it.
- For a single-line container that does not wrap and has `solve_overflow` true, when it overflows in `distribute_flex_main`: children whose main axis follows content hand over their envelopes (those whose author wrote a px / fill / percentage main axis are not compressed), and they are solved together with the container's own gap; what is left then goes to the shrink factors.
- At placement a child uses the same solver to split its share between its own padding (half on each side) and its own line; the part handed to that line shows up as that line's overflow, which the line solves in the same way. `aggregate_children` implies that this line is solved too.
- A child that only compresses chrome keeps its content box unchanged and skips the forced re-measure against the used main-axis size; a container plan is not reused when the content box origin moves; a container with solving enabled is recorded in the plan as depending on the main-axis size.
- Not done: grid track floors, the gap inside a box between a button's icon and its text, and vertical writing.

Retention (#213):
- An envelope is retained by the box's style, padding, gap, and the generation of its children's envelopes; it does not depend on the size given to it, so scaling still finds it. When the shape has not changed the generation does not move, so the parent's summary is not rebuilt either.
- An overflowing line retains the level at which solving stopped (`SolveFrontier`) and the generation of the participants. While the gap still falls in that level, only that level is re-shared — the last step of a cold solve, with the same result; if it crosses levels or the participants change, it solves from the start. In tests and under `layout-verify`, every re-share is asserted equal to a cold solve run alongside it.
- In the pass where a parent reuses its plan and does not solve again, children keep the allocation retained from last time.
- There is no hysteresis by default; a retained solve is only a hint and does not change the answer: scaling down and scaling up to the same width give the same result.
- The devtools inspect gained `dynamic`: the envelope's segments (lengths in 1/64 px), the generation, this pass's allocation, and the part taken out of padding.

Counts live in `WorkCounters.dynamic` (`DynamicLayoutCounters`): lines considered, solves, lines needing no adjustment, solves that touch only one level, envelope queries / hits / misses / rebuilds, segments read, candidates created and pruned, refinement, beam width, child settlements, child rearrangements, structural adaptation, budget fallbacks, scratch allocations and peak, reuse, incremental re-shares, cold solves, and participants skipped by class. The four benchmarks and the perf schema emit them as `dynamic_<field>`. Diagnostics are layout domain ids 6–8.

Gates:
- `world/issue212.rs`: a line that fits is box-for-box identical to one with no declaration; on overflow it takes gap before padding; shrink only consumes what is left; a container that does not solve does not read declarations; during a drag every pass is compared with a full layout; inspect.
- `world/issue213.rs`: within one level, scaling only re-shares and builds no envelope; crossing levels cold-solves once; coming from above and from below to the same width gives the same result.
- `world/issue214.rs`: A, a static frame and a paint-only change do not solve; B, a cheap gap over 1k items reads one segment per item, 1k→2k is linear, and scaling within a level allocates nothing; C, a nested tree with 4 branches settles each node at most once per pass and the work grows with the node count; E, 240 scalings cross three levels and cold-solve only where they cross; F, a line over budget falls back exactly once and deterministically; G, across 500 frames of scaling, adding and removing items, and changing declarations, the retained envelopes and solves follow the tree.
- Gate E in `world/issue264.rs`; the `dynamic_layout` unit tests in `nana-ui-core` (including the exhaustive check that an incremental re-share of every gap within a level equals a cold solve).

## What works

**Flex.** You can use `flex-direction`, `flex-wrap`, `gap`, `align-items`, `align-self`, and `justify-content`. With wrapping there is also `align-content`. `stretch` and `normal` share the remaining cross space evenly among the lines. There is also `order`, `flex-grow`, `flex-shrink`, and `flex-basis`. A sidebar plus a main region needs nothing beyond this set.

Items are broken into lines by their size **before** they grow. That is CSS's hypothetical main size. It takes `flex-basis` first. If that is absent, the main-axis size. If that is absent too, the content. It is also constrained by `min-*` and `max-*`. Only after the lines are formed is the space left in a line distributed by `flex-grow`.

So two columns with `flex: 1 1 320px` sit side by side when they fit and each take a line when they do not. An item with `flex-grow` also does not take a whole line to itself and push out siblings that would have fit.

When you do not write the `flex-shrink` longhand, it counts as **0**. Not web CSS's initial **1**. An overflowing fixed-width row (a list, a toolbar) keeps its boxes. It is not quietly squashed.

When the `flex` **shorthand** omits shrink, it still writes 1, as CSS does. For example `flex: initial`, `flex: 1`, `flex: 1 100px`. When you need web-like shrinking, write `flex-shrink` explicitly, or use the shorthand. `flex: none`, `auto`, and the numeric shorthands still write shrink with the CSS meaning.

**Sizes.** You can use pixels, percentages, `em`, `rem`, `vw`, `vh`, `min`, `max`, and `clamp()`.

The lightweight `calc` supports `+`, `-`, `*`, `/`, parentheses, and nested `calc`. It is evaluated after `var()` expansion. The result folds into the existing `%±px`, `vh±px`, `em±px`, or plain px spec. Nesting is capped at 16 levels of CSS parentheses or functions. Unary `+/-` counts separately, up to 16 signs. A unitless result, `0px+number`, division by zero, and a non-finite f32 all fail closed.

Nested `min`, `max`, and `clamp` that share a unit and can be folded may be used. Mixed units that merge into the existing `Min2`, `Max2`, `Clamp3` may too. For example `min(10px, max(1px, 50%))` and `min(1px, 2%, 3px)`. They are resolved against the containing block at layout time. When the three branches are incomparable, it still fails closed.

There is also `min-content`, `max-content`, `fit-content`, and two NanaUI extension keywords: `fill` (`LengthSpec::Fill`, which takes the available space and is synonymous with `100%`) and `shrink` (`LengthSpec::Shrink`, the size content measures to under the current available space, which is what the Row/Column presets use). `shrink` differs from `fit-content`: when the available width is narrower than the content, `fit-content` clamps to the available width and compresses children, while `shrink` takes the sum of the sizes its children measure to.

A leftover `calc` that does not fold into a length atom can go through the `LengthSpec::Calc` AST. Do not pretend it has been flattened.

On a flex line that does not wrap, `min-content` and `max-content` are both the sum of the children. On the block and column axes and with wrapping, `min-content` takes the widest child.

`fit-content` has the widest child as its lower bound and the available width as its upper bound. Text that has wrapped measures its content width at the width where it would not wrap, capped at the available width. So a box that narrowed to its content grows wider when the upper bound relaxes. It does not stay at the width it last wrapped at.

For text that wraps, only lines broken at a width no wider than this box count at their own width: a long word that does not fit makes the line, and the box, wider. Lines broken when there was no box yet (the first shaping) or in a wider box do not count, and the text is re-broken for this box. So the result for the same tree does not depend on how wide the box was when the text was last shaped, and incremental layout matches a layout from scratch.

When a flex child's `min-width` or `min-height` is written as `min-content`, `max-content`, or `fit-content`, the floor is the content size it measures. When the line runs short of space, the other children are compressed first. It does not go below its own content. When the main-axis size is `auto`, `100%` or `Fill` measures to the available width.

**Boxes.** You can use padding and margin. `margin: 0 auto` centers horizontally in a block or column formatting context.

That includes CSS logical properties under `direction: ltr | rtl` and `writing-mode: horizontal-tb | vertical-rl | vertical-lr`. For example `padding-block`, `margin-inline`, `inset-inline`.

Which physical side a logical edge lands on is decided by the final writing-mode plus direction. The type is `nana_ui_core::WritingContext`.

Under `horizontal-tb`, `direction: rtl` maps inline start and end to right and left. `padding-inline-start` lands on the right side.

In vertical writing, inline-start is at the top. With rtl it is at the bottom. block-start is on the right in `vertical-rl` and on the left in `vertical-lr`.

A logical edge and a declaration for the same physical side resolve by which comes last. That includes the cascade across layers. If a stylesheet writes a logical edge and `rtl` only appears in a later layer or through inheritance, it is remapped too.

HTML's `dir="rtl"` or `dir="ltr"` is a presentational hint. It writes the same used `direction`. An author's CSS `direction` overrides it. `dir="auto"` has no first-strong bidi. It fails closed and does not pretend to be ltr.

In vertical writing the block axis is horizontal and the inline axis is vertical. block-start is on the right in `vertical-rl`. There is no `sideways-*`.

`text-orientation: mixed | upright | sideways` inherits. It decides whether glyphs in vertical writing stand upright or lie on their side. `upright` turns the used `direction` into `ltr`, per CSS. So the line head returns to the top for `vertical-rl; direction: rtl; text-orientation: upright`. Logical edges land accordingly too.

Text nodes lay out in columns. CJK stands upright. That uses the font's vertical metrics and the `vert` punctuation glyphs. Latin letters and digits lie on their side. Columns break at the box height. See [Text engine](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/text-engine.md#writing-mode-与-59).

TextInput and TextArea also edit inside columns. The caret crosses a column. Up and down arrows move along the column. Left and right arrows cross columns. A click hits the glyph in the column.

Under `horizontal-tb`, `direction: rtl` puts the start of the inline axis on the right. All of these flip with it: the main-axis start and item order of a flex **row**, `justify-content`'s start and end, and grid column order. Column 1 is the rightmost track. When the total track width is less than the container, the whole thing sits to the right. Also `justify-items` and `justify-self` start and end, and the cross-axis start of a **column** flex. A row flex's cross axis is the block axis and is unaffected.

All of this happens in the layout layer. It does not change the values of the `flex-direction`, `flex-reverse`, and `justify-content` style fields.

flex and IFC placement is computed in flow-relative coordinates. It measures from the main-axis start and the cross-axis start. Lines break in the original order. Alignment keywords are read as written. Only when the children's coordinates are written out are they converted to the page through `WritingContext`. The reversed axis is only this one conversion. It does not reverse the child list. It does not flip alignment keywords. So for a wrapping RTL row, the first line holds the first items.

With vertical writing plus `direction: rtl`, the inline axis starts at the bottom. That is CSS Writing Modes §2.1. The first item of an IFC and of `flex-direction: row` is at the bottom of the column. `text-align: start` sits at the bottom. The cross-axis start of a column flex is at the bottom. CJK in the text still reads top to bottom. The line-relative order of the bidi algorithm does not move with `direction`. Neutral punctuation at the end of a paragraph lands at the other end of the line, as in horizontal RTL.

Grid columns are tracks on the inline axis. Rows are tracks on the block axis. That is CSS Grid §3. In vertical writing, columns run vertically. `vertical-rl` rows stack from right to left. Column 1 in vertical rtl is at the bottom. Track sizes are computed from lengths along or across rows. Placement likewise converts flow-relative coordinates to the page.

`writing-mode` and `direction` inherit in layout too. A container that writes neither lays out along its ancestors' direction. The same holds for nodes built directly through the Rust API or the style tree. When an ancestor changes direction, descendant containers relay out with it. Logical edges on a box that does not write direction (for example `padding-inline-start`) also land on a physical side according to the inherited direction.

A percentage margin or padding resolves on all four sides against the containing block's **inline size**. That is CSS Box Model §5. This inline size comes from the **containing block**'s writing direction, not the box's own. For a horizontal child box inside a vertical parent, `margin-top: 10%` is computed against the parent's height. The same holds for percentages in IFC line breaking and in a grid one-dimensional track.

Still missing are `unicode-bidi` isolation and full bidirectional text.

The default is border-box. content-box is recognized too.

**Grid.** You can use `grid-template-columns` and `rows`, `fr`, `minmax`, and `repeat(N, …)` with a fixed count. At layout time `repeat(auto-fit | auto-fill)` is also expanded. It can be mixed with fixed tracks before and after it. For example `80px repeat(auto-fit, 1fr)`. auto-fit expands to as many tracks as the container fits. Empty tracks collapse.

There is also `grid-column`, `grid-row`, `span`, automatic placement through `grid-auto-flow`, `grid-auto-columns` and `rows` for implicit tracks, and `justify-items` and `justify-self` on grid items.

After track resolution, a `width` or `height` percentage on an item, and `Fill`, resolve against the **final cell**. The measuring phase still treats `100%` and `Fill` as indefinite, so auto tracks are not blown open.

Columns are sized first, then rows (CSS Grid §11.5). An auto row's height is the height of its items **laid out at the column widths they span**, not the measurement taken before columns were resolved: there, items with `100%` / `Fill` measured at zero width, a chip that can wrap took a line each, and an item with `auto` width measured at the whole container width, so the chips crowded onto one line. So five 60 px chips in two 197 px columns break into three plus two, with a row height of 46, matching a browser (T-G36).

`grid-template-areas` plus `grid-area: header` gives named areas. There are also `[name]` named lines on tracks. For lines with the same name, `foo 2` or `2 foo` picks the Nth. In `foo / foo`, the end takes the next line with the same name after the start. It is not CSS's swap.

The leading line name in `[start] 80px repeat(auto-fit, …)` is kept. Line names inside repeat are kept too. Names inside `repeat(N, …)`, `auto-fit`, and `auto-fill` are copied once per expansion. Adjacent names at a seam merge. So `mid 2` is resolved against the expanded geometry.

In a fixed-height grid, an empty auto row (measuring about 0) eats the remaining height. An auto row with content (T-G26) keeps its content height and is not stretched. This is not CSS `align-content: stretch` acting on tracks.

When automatic placement has scanned 4096 cells and still does not fit, the item lands outside the scanned area. This is not a host error.

`repeat()` cannot nest. A track list also has at most one auto-repeat. That is CSS Grid's grammar. The body of `<track-repeat>` is `<track-size>`. The body of `<auto-repeat>` is `<fixed-size>`. Neither is a `<track-list>`. This is not a NanaUI gap.

For a form such as `repeat(3, repeat(2, 1fr))`, the whole axis fails closed and records `NestedRepeat`. That matches a browser discarding the whole declaration.

A whole-value `subgrid` inherits the track sizes and gap the parent grid has already resolved across the span the item covers. When the parent has no tracks it computes to `none`. No auto tracks are invented.

Writing `subgrid` as a single token in a track list, such as `subgrid 80px`, still records [`GridTrackListUnsupported::Subgrid`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/box_layout.rs).

When you build `LayoutStyle { display: Grid }` directly, without going through css_map, `align-items` defaults to `start`. Hand-written tests rely on this. `Default` cannot tell "not written" from an explicit `start` by display. Parsed through `display:grid`, it writes `stretch`.

Block layout has no `align-items`. A container with no `display`, `display: block` or `flow-root` that stacks its children down the block axis (`LayoutStyle::stacks_as_block`) lays them out as CSS block layout does: a block-level child with no `align-self` and an `auto` width fills the containing block less its left and right margins (CSS 2.1 §10.3.3), whatever `align_items` says (`LayoutStyle::child_align_items`). So `LayoutStyle::default()`, `LayoutStyle { display: Some(Block) }` built directly (through `Stack::from_layout`, say) and `display: block` parsed by css_map give the same box. Some children keep their own width: one that names `align-self` (it still applies in the flow), one whose width is not `auto` (`Px`, a percentage, `Fill`, `Shrink`, `fit-content` and so on), replaced content with a natural size (an image, say; CSS 2.1 §10.3.4), and a child whose writing mode is orthogonal to the container's (what it has along that axis is its block size, which its content gives; CSS Writing Modes 3 §7.3). A flow laid along a row (`direction: Row`) and every flex, grid and layers container still align by `align_items`. To keep children at their content width, centred or at the end, make the container `display: Flex` (every `Stack` preset is), or give the child `align-self` or `width: Shrink`.

`display: layers` is a NanaUI extension value; see `Layers` above. It does not write `stretch`: children of a stack align to the start by default, as with ZStack / Box; when `justify-items` is not written they align to the start too, not to grid's `stretch`.

**Inline subset.** `display: inline` and `inline-block` inside a block container go through an inline formatting context, that is, an IFC.

Inline children are arranged along the **inline axis**. Rightward under `horizontal-tb`. Downward under `vertical-rl` and `vertical-lr`. A block-level sibling closes the current line and takes a line of its own.

In vertical writing a box's physical width and height are still width and height. But the main axis of an IFC and of `flex-direction: row` follows the writing-mode's inline axis. `vertical-rl` lines start at the right.

You can use `text-align`. `start` and `end` follow `direction`, horizontally only. `left` and `right` stay physical. There is also `center`. For placing inline boxes, `justify` is treated as `start`, while inheriting to the text below as in CSS so paragraphs are justified (see [Text engine](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/text-engine.md#cjk-行决策211)).

`white-space: pre` preserves newlines and spaces during measurement.

`align-items: baseline` reads the same intrinsic metrics. Text nodes prefer the first- and last-line baseline that `nana-text` retains; the fallback is used only when the host supplies an ascent alone. Replaced content and Custom nodes use the block-axis end as an explicit fallback. Flex, Grid, and IFC do not each estimate their own set of baselines.

Within a line of an IFC, atoms align on the baseline (`vertical-align: baseline`; see the baseline paragraph under "Replaced content" above): an inline-block takes its last-line baseline (when the line box is in a descendant, the actual position of the last line box in the descendant), replaced content, atoms whose `overflow` is not visible, and atoms with no line box take their bottom margin edge, the line starts from the block's strut, and the line height is the aligned span. Lines of native paragraphs and of ordinary block-level flow share this rule.

A flex line's retained plan records the baseline each baseline-aligned child used at the time. In the next pass, if a child's size is unchanged but its baseline changed (the font size changed inside a fixed-size button), the line realigns anyway and does not keep the old position.

**Float subset.** `float: left | right` takes a box out of block flow. Several floats on the same side sit side by side by geometry. When they do not fit they fold to the next line.

`clear` in flow, and `clear` on the float itself, both use the occupied bottom edge after wrapping. Not the pre-arranged height of one box.

An IFC line box narrows by the left and right insets where the current line intersects the margin boxes of **sibling** floats. That is shrink-to-avoid-float. An atomic inline item that does not fit in a line folds to the next line. If it still does not fit after narrowing, the line box moves down below the bottom edge of the nearest occupying float.

An in-flow **block-level** border box does not narrow to avoid floats. That matches a non-BFC block in CSS.

Floats do not take part in the height of normal flow (CSS 2.1 §10.6.3): a block container's `auto` height counts only its in-flow content, plus however far `clear` and line boxes shortened or pushed down by a float push that content. An ordinary block holding nothing but floats is 0 tall, and the floats hang out of its content box. When the container establishes a block formatting context (BFC) of its own, its height contains the packed floats (§10.6.7): one line of floats is one line tall, not the floats stacked and summed. A BFC is established by the document root, a float, `absolute` / `fixed`, `inline-block`, `overflow` other than `visible`, `display: flow-root` (`DisplaySpec::FlowRoot`), and flex, grid, layer and overlay items. Floats sit side by side inline-wise: they give the container a max-content width of their margin boxes' sum and a min-content width of the widest one. Gate: `world/float_containment.rs`, css-parity `T-FL08`.

This is not full exclusion. An ancestor float does not intrude into a child block's IFC. There is no `shape-outside`.

A float on a flex item or grid item is blockified per CSS, then ignored.

**Positioning.** You can use `relative`, out-of-flow `absolute`, window-relative `fixed`, and in-flow `sticky`. `sticky` sticks only after the scroll projection. It does not write back into Runtime's `LayoutBox`.

An `absolute` box's containing block is the same as in CSS (CSS 2.1 §10.1):

- the **padding box** of the nearest positioned ancestor (`relative`, `absolute`, `fixed`, `sticky`). Static boxes in between are skipped; their padding does not count.
- when there is no positioned ancestor, the initial containing block: a viewport-sized rectangle at the document origin.

So to have an absolute child hug a box, that box must write `position: relative`. `inset: 0` covers its padding box, not its content box.

**Clip chain (#201).** An overflow container (a box whose `overflow` is not `visible`) clips and scrolls only those descendants for which the container is the containing block or is above the containing block (CSS Overflow 3, CSS 2.1 §11.1.1). So:

- when an `absolute` box's containing block is outside a static scroll container, it neither scrolls with that container nor is clipped by it; when the containing block is inside the container, as usual.
- a `fixed` box and everything under it are not clipped or scrolled by any ancestor.

Each box's published `LayoutResult` records this chain: `clip` is the nearest overflow container that reaches it, and the chain continues up through that container's own `clip`; `clip_parent` is the node from which the box takes clipping and scrolling — usually the parent; for an `absolute` box that escaped an overflow container it is the parent of the outermost container it escaped; for a `fixed` box it is `None`. `UiWorld::layout_clip_chain` lists the whole chain from the published results. An ancestor that was skipped still passes its transform to it, but no clipping or scrolling. There is one difference from CSS here: `clip-path` on a skipped ancestor and component chrome clipping (EmptyState, dialogs) do not apply to it either, whereas CSS's `clip-path` would.

Scene extraction, the hit index, accessibility, and Select menu room all hang boxes from `clip_parent` and do not derive the chain along ancestors themselves. The walks that clip an area by the boxes above it (menu room, and the chrome clipping of EmptyState and dialogs in accessibility) go through `UiWorld::clip_ancestors`, that is, up along the published `clip_parent`: a `fixed` box (the field itself or one above it) ends it, and a menu opened by a `fixed` field takes the whole window (#205 gate `issue205_menu_room_and_chrome_clips_follow_the_clip_chain`). In the hit index an escaped box hangs under its `clip_parent`'s entry, the translation of the container it escaped does not carry it while scrolling, and the points it covers are ordered by paint order. An accessibility node's `bounds` is still the whole box, and `clip` is the piece of viewport the overflow containers on the chain leave for it at this scroll position, or `None` when there is no container on the chain; what is painted and what the pointer can hit is the part of `bounds` inside `clip`. Scrolling still does not run layout. When an ancestor changes positioning or starts or stops clipping, the chain recorded below it is republished with it.

The gates are in the `clip_chain` module of `world/issue201.rs`: a positioned page holds a static scroll container, inside it a positioned card with a static scroll container holding a line of in-flow boxes, an `absolute` box whose containing block is the card, and a `fixed` box; after the card a static clipping box holds an `absolute` box whose containing block is the page. Assert each one's `clip` / `clip_parent`; scroll the two containers to several offsets, and for each, without running layout, accessibility position and clipping and pointer hits (both the hit index patched in place and the rebuilt one) follow the chain; Select menu room is likewise clipped only by the containers on the chain; when the card and the inner container change positioning or overflow, the result matches a cold layout item by item (including accessibility `clip`), and the hit index patched in place (letting the scroll translation live first, then changing the chain) gives the same hits and candidates on a page-sized grid as a rebuilt one; a skipped ancestor's transform (on the style and presented by a compositor animation) still applies to the escaped box; a sibling later in tree order that covers the escaped box catches the pointer, however the index orders them. `runtime_document_clip_chain_tests.rs` in `nana-ui-scene` runs real `RuntimeDocument` frames and compares Scene's painted positions and clips, accessibility `bounds` / `clip`, hits, and culling one by one, the same after scrolling and after the chain changes.

Scroll range is computed along this chain too (CSS Overflow 3: a box counts into the scrollable overflow of the boxes on its containing block chain). An `absolute` box counts into its containing block's range and not into the static scroll container it escaped; a `fixed` box and everything under it count into no container. See "Overflow" below. The gates are in the `scroll_range` module of `world/issue201.rs`: a static scroll container holds an `absolute` box whose containing block is the page and which sits far below, and a `fixed` box further down (with an `absolute` box inside it); the container's scrollable distance, `LayoutResult::overflow`, `scroll_extent`, and offset clamping all exclude them, and the `absolute` box counts into the page that reaches it; when the container becomes `relative` and the box switches between `absolute` and `fixed`, the range moves with the chain and matches a cold layout item by item (including metrics and the clamped offset); scrolling the two containers runs no layout, and the range does not change. `out_of_flow_boxes_count_where_their_clip_chain_reaches_them` in `world/scroll_bounds.rs` compares, at the index level, a full walk along the clip chain with a fresh index: positioning flips, a subtree moving between containing blocks, parking and re-attaching, the top layer starting to clip while parked, and a trigger menu opening (menu items become `fixed`) and closing again.

Incremental layout tracks by containing block: when a containing block moves or changes size, the boxes hanging from it replay even if none of the static boxes in between were laid out this pass; when only a static box in between moves, the box does not move. When an ancestor starts or stops being a containing block, the boxes below re-hang from the new one, and the published `LayoutResult::containing_block` updates with them. The gates are in `world/issue200.rs`: every step compares boxes, results, hits, and accessibility against a cold layout.

`sticky` resolves its inset against the nearest scroll viewport and is constrained by its normal-flow containing block: it is released at the end of the parent container. An axis with no inset scrolls as usual. A nested Sticky continues from the parent's presented position; nested scroll containers each own their viewport. Runtime's hit testing, accessibility, and Scene painting share one placement projection, and scrolling triggers no content measure, layout, or text shaping. Changing the containing block's size or remounting invalidates only its registered Sticky branch.

A `fixed` box's containing block is the viewport of the document it belongs to: the origin is the window's top-left corner, and the size is the viewport that document last laid out with. Whether an ancestor is a positioned box or has padding does not count; an ancestor's scrolling and clipping do not apply to it either, and hit testing and accessibility likewise read its box at the window position. The initial containing block used by an `absolute` box with no positioned ancestor is this document's viewport too. When one `AppContext` holds several documents, each document records its own viewport (`UiWorld::document_viewport`): when one document changes its viewport, only its own `fixed`, `vw` / `vh` boxes and the boxes on the initial containing block replay; the other document runs no layout, and its boxes, results, and recorded viewport are unchanged. Gate `issue200_fixed_boxes_hang_from_their_own_windows_viewport`: two documents with different viewports, each with a 25%-wide `fixed` box hidden inside a `relative` card and a static box with padding, an `absolute` box on the initial containing block, and a `fixed` box inside a scrolling list; the list scrolls and the two documents take turns resizing, and at every step boxes, hits, and accessibility ranges sit on their own viewport, the other document does not move, and in the end it matches a cold layout.

When one `AppContext` carries several documents, each document's frame takes work with `AppContext::take_system_work_for(document)` (that is how `RuntimeDocument` takes it): global work such as style and text is taken as usual, layout seeds are taken only for this document, and the other document's edits stay in the pending table for its own frame. Taking everything at once with `take_system_work` and then handing it to one document's layout does not lose them either: that pass puts the other document's seeds back into the pending table. Gate `issue200_an_edit_pending_in_one_window_waits_for_that_windows_frame`: after the second document's box becomes wider, the first document paints first and runs no layout pass at all; only the second document's own frame applies the edit; when everything is taken at once and handed to the first document, the edit likewise waits for the second document's frame.

`fixed` is only for pinning an ordinary node to the viewport. Product overlays still go through controls. Do not build a dialog of your own with `fixed`.

**Anchored positioning (#200).** A `fixed` box can carry `LayoutStyle::anchor` (`AnchorSpec`). It states the anchor rectangle, the preferred side, the alignment, the gap, and optionally a room region. Layout places it against the real viewport at the size the box measured, using `resolve_anchored_surface`:

- if it fits on the preferred side, put it on the preferred side; if not, and it fits on the opposite side, flip to the opposite side;
- then pull it in along both axes into the room region. The room region is the intersection of the viewport and `room`.

This is the "flip, then pull in" part of CSS anchor positioning. Every anchored overlay shares this one resolver: ContextMenu, AnchoredActionMenu, trigger menus, HoverCard, Popover, and the Select drop-down. When Select fits on neither side, it takes the wider side and truncates.

When the window resizes, layout replays these surfaces directly without reprojecting the component. Hit testing and accessibility read the box layout gives. This field has no CSS spelling; controls write it. The gates are in `world/issue200.rs`.

The menus of Select, Dropdown, and SearchDropdown are not layout boxes; they are the field's component geometry: they are placed with the same resolver from the field's box and "the space the menu can be seen in" (`SelectMenuRoom`: the window, minus the clips on the field's clip chain, at the current scroll position). What Scene paints, what `select_option_at` selects, and the menu surface carried by the field's entry in the hit index all come from this one geometry. The space is recorded when the field projects; after each layout pass (after scroll retention), the engine computes the space again for the menus open in the document against the layout just published, and writes it back only when it changed, so when the window resizes or the field or an outer container is moved, the menu flips and truncates again without waiting for the field to reproject, and the hit index rebuilds the field's entry with it. Only scrolling runs no layout: while the page scrolls the menu follows the field and is placed again at the next change, and painting and hits still use the same one. Gate `issue200_a_select_menu_follows_the_viewport_and_hits_where_it_paints`: the window gets shorter and then taller, the menu flips up and down, and at every step the row painted at a point is the row the pointer selects, the hit on the field is that point, the old position no longer hits, and it matches the menu a cold layout produces. The field's (`ComboBox`) range is its layout box; while the menu is open there is a list box under the field, one option node for each row that shows, and its range likewise comes from this geometry (the row intersected with the menu viewport, with "the space that can be seen" at this moment, then through the field's transform), so what a screen reader reads, what is painted, and what the pointer selects are the same row; see [Select's screen reader](https://github.com/sena-nana/NanaUI/blob/main/docs/components/select.md#读屏). The gates are in the same file: `issue200_a_select_listbox_*`, `issue200_an_open_select_menu_reads_as_a_listbox_of_its_rows`, and others.

When an out-of-flow box has no height written and does not write both `top` and `bottom`, its height comes from content. A child inside it that fills by height (`Fill`, percentage height) has no definite height to resolve against and is treated as content. It no longer takes the containing block's height. When the containing block's height is grown by these boxes (a virtual list row measured by content), that would grow a bit on every layout pass and the frame would never settle.

This applies only to the children inside it. The box's own percentage `min-height` / `max-height` still resolves against the containing block's height: for `fixed` that is the viewport, for `absolute` its containing block. An out-of-flow box's containing block always has a definite height (CSS 2.1 §10.7). So a card with `position: fixed; max-height: 80%` grows with its content up to 80% of the viewport. It does not have to be rewritten as `80vh`.

For an out-of-flow box with `aspect-ratio`, the height is derived from the width left by `left` / `right` and the width's bounds, as in flex, grid, and inline: `width: 120px; max-width: 60px; aspect-ratio: 1.5` gives 60×40, and `left: 0; right: 480px` in a 600-wide containing block gives 120×80 (#382, gate A in `world/issue382.rs`, css-parity `T-P21`). A box that writes both `top` and `bottom` still takes its height between the two insets.

These rules are the same in every context (CSS Sizing 4 §5.1). The ratio converts between the box `box-sizing` names: `border-box; padding: 6px; width: 120px; max-width: 60px; aspect-ratio: 1.5` gives a 60×40 border box, and `content-box; padding: 6px; width: 120px` gives a 120×80 content box and a 132×92 border box. The derived height is then constrained by the box's own `min-height` / `max-height`: the example plus `min-height: 50px` gives 60×50. The auto height of a flow container also reads this height (#393, gate A in `world/issue382.rs`, css-parity `T-S23`).

**Stacking context subset.** Scene sorts by `(z_index, document_order)`. It treats an isolated group as one layer. These groups are: a group whose `opacity` is between 0 and 1, `isolation: isolate`, and a `position` other than static with a written `z-index`.

A high-z child does not paint above a sibling outside the group that comes later.

**Exception:** a `position: fixed` surface paints in the root stacking context. The popup layers of Popover, ActionMenu, and HoverCard are in this class. They are not clipped into the card by a parent's isolated group.

The popup layer's content is made viewport-fixed by Runtime. Extraction to Scene follows that style too. So an ancestor's overflow clipping, scrolling, and transform do not apply to it.

The card face is the backing the trigger node paints. It likewise carries none of the trigger's transform or clipping. It sorts into the same layer of the root stacking context as the popup content (`MENU_OVERLAY_Z_INDEX`). It sits above the trigger and above the rest of the page, including siblings that come later with a higher `z-index`. It is below the popup content.

Hit ordering at overlaps cuts at the same prefix. Otherwise a later sibling card would steal menu clicks. The card face itself takes the pointer in this layer too. A press that lands between items or on padding stays in the menu. It does not fall through to the page underneath. Nor is it treated as a press on the trigger that opens or closes the menu.

The menus opened by Select, Dropdown, and SearchDropdown also sort into the root stacking context. The layer is `MENU_OVERLAY_Z_INDEX`, or the field's own `z-index` when that is higher, and within the layer they order by the field's document order. So it paints above a later sibling group, and hits at overlaps go to it. Unlike a popup layer it is not viewport-fixed: it carries the field's transform and is still clipped by the field's outer layers. Menu placement uses only the part that can be seen; see [Select's menu](https://github.com/sena-nana/NanaUI/blob/main/docs/components/select.md#菜单).

Do not rely on your app raising the whole card's `z_index`.

This is not the full CSS Appendix E. Negative z layers, the float and inline layers, transform creating its own layer, and so on are not all done.

Hit testing still walks the tree. z inside a group compares only among siblings.

**`display: contents`.** Children are hoisted into the parent's formatting context. The node itself has no box.

**Text.** You can use font size, font weight, font family, line height, letter spacing, and color.

`line-clamp` and `-webkit-line-clamp` limit the line count and add an ellipsis.

`text-decoration: underline | line-through` is drawn as a line on the text box by Scene.

`text-emphasis`, `text-emphasis-style`, `text-emphasis-position`, and `text-emphasis-color` (and the `-webkit-` prefixes) put an emphasis mark on each glyph: `dot`, `circle`, `double-circle`, `triangle`, `sesame` with `filled` / `open`, or a string (only its first glyph is taken). The position is `over` or `under` (Chinese usually uses `under`), default `over right`. All three inherit; the shorthand sets style and color and leaves position alone. A mark is an ordinary glyph at half the font size, and punctuation and whitespace get none; the line box grows by the mark's actual height. Changing style or position relays out only, and changing color only repaints. See "Emphasis marks" in [text-engine.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/text-engine.md).

`font-feature-settings` and `font-kerning` write into `nana-text`'s OpenType features.

`font-variation-settings` hands the axes already declared to the shaper. `wght` still merges into `font-weight`. If the font does not have the axis, **that axis** fails closed. The tag is not rewritten as `wght`. Nor is it stuffed into `FontFeatures`.

`user-select: text | none | auto | all | contain` writes into `LayoutStyle`. It inherits, in the same class as `cursor`. Under `auto`, ordinary `TextContent` is not selectable. `text` allows document-level drag selection and copying. `all` selects the whole text node on a single click, with no drag. `contain` is selectable, but the selection stays inside that subtree and does not extend to neighbors. `none` never enters a selection. A child that explicitly writes `text` turns it back on.

This is not a second TextInput.

An author's `::selection` and `::-moz-selection` resolve only `background`, `background-color`, and `color`, painted onto the document selection highlight. Other properties fail closed. This is not the full CSS Highlight API. When nothing is written, the selection backing still uses the theme's `accent_soft`.

`word-break: keep-all` is skipped. `line-break: strict|loose`, `text-spacing-trim`, `text-autospace`, `text-justify`, and `text-wrap-style` go to the text engine's CJK line decisions (#211), and all inherit. Letter spacing is approximate.

**Hiding.** `display: none` takes no space and takes no clicks.

`visibility: hidden` **still takes space**. It takes part in flex and grid measurement. But it does not paint and does not hit.

`pointer-events: none` still paints and still takes space; it just does not hit itself. A child that writes `auto` can be clicked through the hole.

This is Runtime `UiWorld` hit testing. Not window alpha, and not a shaped window.

The internal `layout.hidden` (sidebar, menus, and so on) skips layout like `display: none`.

**`transform`.** It affects painting only and does not enter layout. This is CSS's correct behavior, not a gap.

The 2D subset is affine: `matrix()`, `rotate()`, `rotateZ()`, `translate()`, `scale()`, `skew()`.

The z of `translate3d` and `translateZ` does not change the projection at z=0 without perspective.

`perspective()` together with `rotateY`, `rotateX`, `rotate3d`, or a `matrix3d` with a perspective remainder goes through the same Scene planar homography. That is a 4-vertex perspective divide. **Quad, Text, Icon, and HostTexture share this `(g,h)`.** A node never gets a trapezoid underside with affine glyphs.

Spinner, stroke mesh, and Custom nodes other than HostTexture paint **identity** under perspective. There is no fake 3D such as `scaleX(cos)`.

A parent's `perspective` property and `transform-style: preserve-3d` still fail closed.

`transform-origin` is a separate 2D field. The default is the box center. It applies to both 2×3 and 4×4. `transition` and `@keyframes` interpolate its `%` and `px`. It still does not count as layout.

Animating `transform` itself goes through the compositor overlay. It does not change logical style every frame. See [Architecture](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/architecture.md).

In `transform-box`, `border-box` and `view-box` are relative to the border box. Under HTML, `view-box` is the border box. There is no SVG viewport. `content-box` and `fill-box` are relative to the content box. The content box is the border box minus border and padding. A padding `%` resolves to 0 when there is no containing block. That matches the other paint-time padding resolution. `stroke-box` does not resolve.

**Painting.** You can use `background`, `background-color`, and `background-image: linear-gradient(...) | radial-gradient(...)`.

The initial value of `background-size` is CSS `auto`. There are also `cover`, `contain`, `stretch`, `scale-down`, and lengths.

The initial value of `background-repeat` is `repeat`. `round` scales the tile by an integer grid. `space` and mixed values fail closed and are not painted as repeat.

`mask-image` and `-webkit-mask-image` can be the alpha of a linear or radial gradient, or a `url()` texture. The GPU allows at most 8 mask stops. Both take only alpha; the stops' and texture's colors do not take part, that is, CSS's default `mask-mode: match-source`: `linear-gradient(90deg, transparent, black)` goes from hidden on the left to shown on the right. `mask-mode: luminance` is not supported.

A mask is evaluated on the node's layout box and applies only to what the node itself paints, not to its subtree:

- the node's own quad (the background fill; borders and `box-shadow` are unaffected) and the content of a HostTexture.
- a gradient mask also applies to every command recorded by `.painter`: fills, strokes, shadows, text, icons, and images, again in node-local coordinates. So a hand-drawn shape and the node's background fade along the same gradient. The part outside the layout box (hand-drawn shadows, outer glow) takes the value at the box edge and is not clipped away. This differs from CSS's `mask-clip: border-box` on purpose: a directional glow has to reach outside the box.
- with a mask, rectangles in hand-drawn content no longer take the quad fast path; they are always triangulated and multiplied by the mask in the path shader. Text, icons, and images each paint into a layer, and the mask multiplies before the layer is composited.
- a `url()` mask does not apply to hand-drawn content; that part paints without a mask as usual and counts into the metric `runtime.paint.mask_unsupported`.

Neither child nodes nor the node's own text are affected by the mask. CSS's masking of a whole subtree would require isolating the subtree into a layer first, and NanaUI does not do that. To mask children, put the mask on the children themselves.

`clip-path` supports `inset(...)`, `polygon(...)`, `circle(...)`, `ellipse(...)`. An inset's `round` writes into the rounded-box SDF. The radii survive a non-translational transform. A polygon is point-in-polygon tested against itself and its children. Children composite through a dest group. It is not a bounding-box clip.

`filter` supports `brightness()`, `saturate()`, `contrast()`, `hue-rotate()`, `invert()`, `opacity()`, `blur()`, `drop-shadow()`. `blur` and `drop-shadow` are the element's own filters. blur is capped at 16 px. They are separate from `backdrop-filter`.

`drop-shadow` samples the already painted alpha outline in the dest compositing group, then offsets it, then blurs it with the same kernel. It is not `box-shadow`'s box geometry.

A dest compositing group is used when there are children, text, custom painting, or `blur`, `drop-shadow`. hue and brightness on a leaf go through the quad shader. An unknown function still fails the whole table closed. Multi-layer `drop-shadow` and spread are not implemented.

`box-shadow` supports `inset` and comma-separated multiple layers. The GPU limit is 4.

`outline` paints only a solid extra stroke. It does not enter layout.

The GPU subset of `mix-blend-mode` is `normal`, `multiply`, `screen`. They go through the dest group's BlendState. The other keywords fail closed.

`line-clamp` and `-webkit-line-clamp` use the existing text ellipsis and set overflow hidden.

`border-image` is a minimal subset. `url()` or `linear-gradient` with `slice` (optionally `fill`) goes through the existing quad URL texture 9-slice. A `linear-gradient` is rasterized into a texture first, then sliced. `border-image-width` defaults to `1` times the slice. `repeat` is stretch only. `radial-gradient`, outset, round, repeat, space, and a non-default width still fail closed.

The four sides' `border-*-width` and `border-*-color` take part in layout and in the GPU stroke.

`border-style: dashed | dotted` applies a per-side periodic mask on the same rounded-box SDF ring. It does not open another pipeline. `double`, groove, ridge, inset, and outset still take their used width but are not stroked.

`background-image: url(...)` shares the same decode plus URL texture cache as `<img src>` and inline `<svg>`. It supports `data:image/png|jpeg|svg+xml`, `http://`, `https://`, `file://`, and relative paths. `.svg` goes through the existing `resvg`. The longest side is 2048, preserving the ratio. It is not a second vector engine.

A relative URL and a `file:` URL go through the same [`canonicalize_within_jail`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-core/src/url_jail.rs) as stylesheets. A non-local file host is rejected. `Url::path()` is not fallen back to an arbitrary filesystem path.

`http(s)` is not judged by this jail. It goes through the host-injected `fetch_host` (`MountOptions.fetch_host`). The origin allowlist, per-hop redirect re-checks, stripping authorization on cross-origin, and timeouts all follow `FetchPolicy`. With nothing injected, it is rejected.

Cancellation fires when the painter is dropped and when an image goes 120 consecutive frames with no reference. What ends is the network wait, that is, a socket shutdown. Not a decode already under way.

The body cap is the smaller of 8 MB and `FetchPolicy.max_response_bytes`.

A relative path prefers the document or a host-set base. See [`set_background_image_url_base`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui/src/scene_paint/image_url.rs). When unset, the installed and portable builds are relative to `ApplicationPaths::runtime_resources()`. A development layout, and an embedding host with no application paths, is relative to the process cwd.

The jail for an absolute path and for `file:` is unchanged. A host base is used when there is one, otherwise the cwd.

`nana://res/<logical path>` reads from packaged resources. See [Packaging and distribution](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/packaging.md). A relative `url()` in a packaged stylesheet is rewritten at load time into an absolute `nana://res/` address inside the same package.

An SVG's `<image href>` does not read local files.

A texture for the same URL is cached by URL and rebound in batches by URL. On the GPU, each quad has at most 8 gradient stops.

A frame with a CSS `url()` still goes through 4× MSAA. It is separate from the interleaved path of HostTexture and backdrop.

`backdrop-filter: blur()` is a per-node dest sampled blur. In `blur(r)`, `r` is the Gaussian standard deviation, as in CSS. It does rotation mapping, and also the ancestor's inset and polygon clip. It is not a whole-window Mica or Acrylic.

`<img>`'s `object-fit: cover | contain | fill | none | scale-down` writes through the cascade into `PaintStyle::object_fit` and the fit of `content_image`; `<video>` and `<gpu>` lower it into `CustomRenderNode.fit`. All three are sized by the painter with the same `ContentFit::fitted_size` (see "Replaced content" above). The HTML attr is a presentational hint. A stylesheet overrides the attr. Inline overrides the stylesheet again.

**Hit testing.** `pointer-events` is `auto` or `none` and **inherits** per CSS.

When the parent is `none`, a descendant that does not write the property has a used value of `none` too and cannot be hit. Only a child that explicitly writes `auto` becomes a target again. A point on the parent box that does not land on that `auto` child passes through.

Leaving hit testing takes effect at commit. After setting `none`, turning off `InteractionState::pointer_events`, moving under a `none` parent, or into a parked branch, hits made after the commit and before the next frame rebuilds the hit index pass through to what is below. Becoming hittable again waits for the next frame to resolve styles.

An unknown keyword fails closed. It does not change an already specified value. `inherit` and `unset` clear the specified value.

Planar 3D hit testing shares the same inverse homography as painting Quad, Text, Icon, and HostTexture. It does not hit an affine box. Nor does it hit the empty corners of a trapezoid's AABB.

**Overflow.** `overflow`, `overflow-x`, and `overflow-y` can be `visible`, `hidden`, `clip`, `auto`, `scroll`.

`hidden`, `clip`, `auto`, and `scroll` all clip painting and clipping hits. Clipping and scrolling apply to descendants along the clip chain; see "Positioning".

When both axes clip, descendant painting is rounded by the box's own `border-radius`. What is clipped is the border box. When the corners differ, the smallest is taken, capped at half the short side. That matches CSS. The box's own background and border are still clipped only by the square border box. They are not rounded again. Hit testing still uses the square box.

For `overflow: auto|scroll` the scroll authority is Runtime's `ScrollOffset`. The wheel goes through this path. Scrolling does not write back into `LayoutBox`.

The offset keeps its fraction. The picture snaps to device pixels. See [Runtime and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/runtime-scene.md).

The scroll origin is the start edge. That is CSSOM View's scrolling area. Rows with `direction: rtl`, the block axis of `vertical-rl`, and `row-reverse` and `column-reverse` start at the right or bottom. The initial view is the right or bottom edge. Content that overflows to the left or up is scrolled to with a negative offset. As in CSSOM, the offset goes from 0 toward negative. Content past the other side of the start edge cannot be scrolled to.

The scroll range counts only the boxes the scroll container reaches, that is, the boxes whose clip chain passes through it (see "Clip chain" under "Positioning"): an `absolute` box whose containing block is outside the container counts into its containing block, and a `fixed` box counts nowhere. The content index carries an `absolute` box's range through the boxes between it and its containing block, landing it in the containing block; the carried range depends only on the subtree it came from, so moving the subtree does not rebuild the index. A box that clips or scrolls has only the children whose `clip_parent` is itself merged into its `LayoutResult::overflow`; other boxes still merge in all their children. The scroll range counts only up to the nearest clipping box, as with CSS's scrollable overflow. An inner scroll container counts only its own box into the outer range, not the overflow of its content: with a scroll area inside a scroll area, the outer one does not become scrollable because the inner one's content grew. A box with `overflow: hidden` / `clip` likewise counts only its own box on the axis it clips, and on the axis it does not clip counts its content as usual. A scroll container clips on both axes. To query which axes clip, use `UiWorld::overflow_contained_axes`.

Every scroll container has metrics. At the end of a commit that writes layout boxes, adds or removes children, or changes styles, Runtime re-measures the affected scroll containers and clamps their offsets into range. Whether the box was written by the engine or by the host. A multi-line text editor's metrics follow its shaping result. A `vertical-rl` editor likewise scrolls to the columns on the left with a negative offset.

The counts on the scroll path are on `WorkCounters` (#381), always on, add-only:
- `scroll_viewport_updates`: how many times a scroll container's viewport size changed. Measuring the viewport the first time, and dropping it when it stops scrolling, each count once. Storing identical metrics does not, and neither does only the content extent changing.
- `scroll_content_extent_recomputes`: the number of entries whose range is recomputed in the content index. Pure scrolling is 0. When range recomputation creeps into every scroll, this column moves first.
- `scroll_clip_updates`: the number of nodes whose clip the scroll path rewrote, that is, the entries rebuilt in the hit index, not the number of scrolled containers. An ordinary descendant shares one translation of the container and is not rewritten; a sticky branch replays with scrolling and counts its entry; a container with perspective falls back to a full rebuild and counts every entry. When clip propagation degrades to O(subtree), this column grows with it.
- There is no `scroll_layout_reflows`. The "pure scrolling does not lay out" assertion rests on existing counts: `layout_measure_nodes`, `layout_placement_nodes`, and `text_shaped` are all 0.
- A virtual list gets no name of its own. Rows created for a new window are `virtual_rows_materialized_from_layout`; the entities of those rows are `entities_spawned`, and the entities of rows leaving the window are `entities_despawned`.

The gates are in `world/issue381.rs`. The page is as tall as the viewport and holds two lists, each with a sticky header carrying two children and 50 rows:
- a scroll-only frame: no layout, and viewport and extent are both 0. Scrolling one list counts 3 clips, which is its sticky branch, excluding its 50 rows and the other list's header; scrolling both counts 6; scrolling the page counts 0.
- the viewport goes from 600 to 500: only the page, which follows the viewport, counts one viewport change, with 1 extent recompute (only the page's own entry) and 0 clips. Storing identical metrics does not count. A row in a list gets taller, so the extent changes, and the viewport does not count.
- a virtual list scrolls past its window: all three scroll-frame counts are 0. After syncing, the number of rows created equals the row count of the new window; the entities created next frame are the new window's, and those removed are the old window's. Extent recomputes do not exceed the number of published batches times the entity count, and the set is the same for ten thousand rows and a million rows.

A custom scrollbar is chrome, [`ScrollView`](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md). `overflow` does not draw a second thumb.

**Selectors.** In a view only class and compound class selectors take effect (`.card`, `.card.active`), and any other selector gives a build warning. What follows is the part the engine itself can parse and match. The tree information matching needs (sibling index, whether it is empty, whether it is selected, focus, the descendant bitset of `:has()`) is supplied by whoever calls the engine; the engine does not walk the tree itself.

The engine knows type, class, id, and attribute selectors, and the combinators space, `>`, `+`, `~`.

There are also `:root`, `:first-child`, `:last-child`, `:only-child`, `:nth-child()`, `:nth-of-type()`, `:nth-last-child()`, `:first-of-type`, `:last-of-type`. They match on the sibling index and count the caller supplies. Simple `:not()`, `:is()`, and `:where()` work too.

`:disabled` and `:not(:disabled)` are just the presence match of `[disabled]`.

`:checked` reads the `checked` flag the caller supplies. Which nodes count as selected is up to the caller. `input:checked + label` goes through the existing sibling combinators.

`:empty` and `:not(:empty)` read the `is_empty` the caller supplies. The engine does not count child nodes and text itself.

A cheap subject `:has(.class|#id|type)`, with comma OR, reads the descendant bitsets the caller computed (`has_bits`, `has_args`). After de-duplication the arguments number at most 64. The engine does not scan the subtree itself.

A cheap subject `:focus-within` reads the `focus_within` flag the caller supplies.

`:focus-visible` maps to `:focus`. There is no keyboard-versus-pointer distinction. This fails closed: it does not match when unfocused.

`::before`, `::after`, `::placeholder`, `::selection`, and `::-moz-selection` are only parsed and matched to a declaration block; nothing generates boxes from them or paints with them.

Placeholder text color and document selection colors are Style Model fields: `LayoutStyle::placeholder_color`, `placeholder_opacity`, and `selection_background`, `selection_color`. Runtime's TextInput and document selection paint from them.

`:has` with a combinator, `:focus-within` written on an ancestor, and those written on ancestors are not parsed by the engine and count into `skipped_selectors`.

**Cascade.** Inside a stylesheet, normal comes first, then `!important`. Then specificity and source order decide. Compiling class rules in a view uses the same order.

A view has no prop or inline layers: class rules apply in the order above, written onto the layout the element builds. The engine's own `rebuild_layout_style` still has those two layers; the order is in the paragraphs after "`@layer`" below.

`:hover`, `:focus`, and `:active` go into the interactive bucket, and `matched_interactive_rules` picks rules from the state the caller supplies. They are not treated as layout conditions. In a view they are a build warning.

`transition` in a view compiles into the element's implicit animation. Only `opacity`, `transform`, `width`, `height`, and `background` (including `background-color`) are recognized; other properties give a build warning.

`@keyframes` and `animation` are only parsed, not played. In a view they are a build warning.

A `font-variation-settings` transition gives a build warning in a view. To animate by axis, use Rust's `TransitionBuilder::font_axis`: one Layout-class track per axis.

Which animation class each property takes (Compositor, Paint, Layout), the fallbacks, and the performance meaning are in [Architecture](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/architecture.md).

**What is recomputed when a node is added or removed.** Nothing. The view runtime does not match selectors, so adding or removing a child does not re-cascade siblings, parents, or ancestors.

An element in a view takes styles only from the class combination it writes (including those `class_when` currently turns on). Tree-structure selectors such as `:first-child`, `:nth-child()`, `+`, `~`, `:empty`, and `:has()` only give a build warning in a view.

**What mounting a styled node costs.** `stylesheet!`, `view!`, and a `.vue` `<style>` are compiled into a `StylePatch` at build time. At runtime no CSS is parsed and no element is matched against selectors. When an element's class combination first appears, the rules it can hit are picked out of the stylesheet, giving a `StyleSite`. After that, elements with the same combination and the same initial layout reuse this already composed style.

So the mount cost does not vary with how many rules the stylesheet has, nor with how many rules one element hits, nor with how many nodes are already mounted.

Measured (#348): each row is a row container, a text, and a button, each with one class, 3 nodes in total. release, Apple M4, 2026-10-30, load about 3. The timing covers building the view, composing styles, and committing to `UiWorld`; it excludes layout and painting afterwards.

| Stylesheet | Hit per element | Cold mount of 4,000 rows (12,001 nodes) | Per row | Append another 500 rows, per row |
| --- | ---: | ---: | ---: | ---: |
| 10 rules | 1 / 4 rules | about 37 ms | 8.9–9.0 µs | 9.2–9.3 µs |
| 100 rules | 1 / 34 rules | about 37 ms | 9.0–9.1 µs | 9.4–9.5 µs |
| 1,000 rules | 1 / 334 rules | about 37 ms | 9.0–9.2 µs | 9.0–9.4 µs |

The same row without a class costs about 8.5 µs. Appending 500 rows to a list that already has 0, 1,000, 2,000, or 4,000 rows takes the same time.

**Conditional class.** Add one `class_when` per element, sharing one signal across all rows; half the rules need this class to be on to match.

- Cold mount is 9.9–10.1 µs per row, and appending 10.1–10.4 µs per row. That is about 10% more than fixed classes alone, the extra being one binding per element.
- After turning the signal on, 13,500 elements across 4,500 rows change style together. One `flush_reactive` takes about 9.4 ms, about 2 µs per row.

Under 10, 100, and 1,000 rules these numbers are the same: each "class combination × conditional class on" is composed once.

**`@container` rules.** Half the rules sit in an `@container` block with three width ranges, so every element carries one responsive rule. Cold mount and append are both 10.0–10.6 µs per row.

An inserted row only walks its own subtree to find the nearest query container; it no longer scans every responsive rule on the page. Before that, every inserted row scanned them all, and a 4,000-row cold mount took 240 ms, with appends at 120 µs per row. The gate is `world::responsive::tests::inserted_rows_find_their_container_without_visiting_every_rule`.

**Unmount.** Unmounting the whole list (4,500 rows) at once costs about 2 µs per row, and about 2.5 µs with the conditional class.

When rows that read the same signal are unmounted one by one, the subscriber list compacts only once dead subscribers reach half, so the per-row cost does not grow with the row count. The gate is `view::reactive::tests::disposing_readers_one_by_one_compacts_in_amortized_constant_time`.

**Budget:** mounting 4,000 styled nodes within 100 ms. Re-measure with `cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --profile-styled-mount`.

The gate `view::style::site_tests::equal_elements_compose_once_however_many_mount` guards "equal elements compose once": mounting 300 more identical rows leaves the compose count unchanged.

**`@import`.** The view's build-time compile is given no loader, so an `@import` in `<style>` is never loaded, `nana://res/` included. What follows is the engine's behavior when the caller supplies a loader.

It parses `url(...)` and quoted paths. A relative path is relative to the importer, or to `FsStylesheetLoader`'s `base`. After canonicalization it must fall under `base`. `stylesheet_base_from_href` derives `base` from the stylesheet's address. Without a loader, `@import` is skipped. The process cwd is not scanned.

With a `PackagedStylesheetLoader`, a `nana://res/` packaged stylesheet loads as usual. Its internal relative `@import` resolves inside the package.

`http(s)`, `data:`, a jail escape, more than 1 MB, and a prelude with `layer` or `supports()` all fail closed. They are recorded as skipped and not loaded.

An `@import` written after ordinary rules is ignored per CSS. An `@import` inside an `@media` block is ignored likewise.

`file://` is percent-decoded. A non-local host is skipped.

Cycles and depth are capped at 16. Imported rules merge into the same cascade. An import already resolved is cached by canonical href. A viewport change does not re-parse it.

**`@media`.** The subset is `min/max/width`, `min/max/height` (px), `orientation`, `prefers-color-scheme`, and the `screen`, `all`, and `print` types.

When a condition matches, `ParsedStylesheet::flatten` merges the rules into the cascade. After a viewport or theme change, flatten again with a new `MediaEnvironment`; the CSS text is not rescanned. In a view `@media` is a build warning.

**`@font-face`.** The engine only parses `font-family`, `src`, and `font-weight` (including a range such as `200 700`); it does not register fonts. In a view it is a build warning.

To use your own font, call [`register_host_font_face`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui/src/nana_text.rs) in the host: one family plus one weight range is registered as one face and written into the shared `FontSystem`. It uses the same library as `bundled-fonts`. To replace a whole group, use `HostFontScope`. This is not CSSOM's `FontFace`.

When the host already has resolved faces, it can hand them to [`ingest_host_font_faces`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui/src/font_face_ingest.rs): `HostFontFaceSpec`'s `weight` and `weight_end` are the `font-weight` range, registered as one piece. It reads local files inside the jail, `nana://res/`, and `data:`. The CSS engine's `css_at_rule::load_font_face_bytes` reads only files and resource packs and does not recognize `data:`, because `data:` is not a file path. Inline fonts are decoded by ingest itself.

The read cap for a single font file is 8 MB.

**`@supports`.** It is evaluated at parse time. When it matches, the inner rules merge into the same cascade, so class rules inside the block still compile in a view.

The predicate subset is the engine's existing one. `display: flex|grid|block`, and the other `display` keywords already parsed. `color` is a value `parse_css_color` can parse. `width` is a value `LengthSpec::parse` can parse. `not`, `and`, and `or` can be added.

An unknown predicate fails the whole block closed and counts into `skipped_at_rules`. For example `selector()`, `lab()`, `display-p3`, and properties not on the list.

**`@container`.** The engine only compiles it into data; it does not measure containers. Container sizes are read by Runtime's responsive rules (see "Responsive rules (#265)" above).

The prelude is `[name] condition`:
- A condition can be combined with `and`, `or`, and `not`, without mixing `and` and `or` at one level.
- The features are `width`, `height`, `inline-size`, and `block-size`, optionally with `min-` / `max-`, or written as a range `(width < 480px)`, `(400px <= width < 800px)`.
- Lengths accept only px and unitless 0.

Each query compiles into a half-open interval on one axis. `max-width: 480px` includes 480, with an upper bound of `480f32.next_up()`; `<` excludes the bound.

The `@container` rules an element hits cut the container sizes into buckets, at most 16 breakpoints. The rules true in each bucket merge in cascade order (`!important`, specificity, source order), giving that bucket's whole style. The fields where it differs from the base style are that bucket's variant and are handed to Runtime's rules: `ResponsiveContainer::Nearest { name }`, which finds the nearest container with the same name that answers this axis.

`container-type` (`normal`, `inline-size`, `size`), `container-name`, and the `container` shorthand write into `LayoutStyle`. A query container does no size containment; its size still follows its content.

All of the following fail closed and count into `skipped_at_rules`:
- other units, `aspect-ratio`, `orientation`, `style()`, `scroll-state()`;
- two axes in one query;
- a comma list, a name alone, a nested `@container`.

When the rules an element hits ask about two containers or two axes, or there are more than 16 breakpoints, all of its `@container` rules stop taking effect. The engine counts them into `container_queries`, and in a view it gives a build warning.

Only ordinary style rules take effect inside the block. `:hover`, pseudo-elements, `transition`, `@keyframes`, and `@font-face` written inside it do not.

**`@layer`.** `@layer name { }` and the anonymous `@layer { }` merge their inner rules into the cascade in author source order and record the layer name (`ParsedStylesheet.layer_names`).

There is **no** cascade-layer precedence. Unlayered does not beat layered. `!important` is not reversed by layer either.

`@import … layer()` and `supports()` still do not load.

The engine's `rebuild_layout_style` first writes the stylesheet's normal declarations, the props' and inline normal declarations, and `nana-*` class hints; then the stylesheet's important declarations; finally the props' and inline important declarations. So a stylesheet's `width:80px !important` is not overridden by an ordinary `width:200px` prop.

A view does not go down this path and does not recognize `nana-*` class hints: a class takes styles only from the rules written in `<style>`.

A custom property drops `!important`. It uses the same `split_important_flag` as ordinary declarations. So `--gap: 8px !important` yields `8px` through `var(--gap)`. Not a value with the flag, which could not be parsed.

## What not to expect

Do not expect these:

- `sideways-rl` and `sideways-lr`.
- Full Tate-chu-yoko.
- Vertical ruby.
- Vertical emphasis marks (`text-emphasis-position`'s `right` / `left` are parsed, but no marks are painted in vertical writing).
- Vertical tables.
- Vertical writing with float wrapping.
- Using a whole rotated box to stand in for glyph orientation.
- Editor decorations in a vertical editor are not painted at all, rather than painted sideways across the columns. That includes the line-number gutter, the minimap, indent and column guides, end-of-line diagnostics, completion, hover, and signature popups. This is #59.
- `unicode-bidi` isolation, and full bidirectional text in an IFC.
- The remaining CSS math functions such as `tan()`, `atan2()`, `sin()`.
- Mixed-unit nested `min`, `max`, `clamp` that do not fold into a length atom.
- Multiplying and dividing lengths. The full `calc()` AST still only resolves the subset where leftover flattening failed.
- Treating `:hover` or `:focus` as a layout condition.
- `:has()` with a combinator.
- An unknown `@supports` predicate.
- `@import … layer()` and `supports()`.
- Full cascade-layer precedence.
- Full float exclusion. An ancestor float intruding into a child block, `shape-outside`, and narrowing of in-flow block-level border boxes are all absent.
- A full IFC. Inline atoms align on the baseline and line boxes have a strut, but there are no other `vertical-align` values and no inline unboxing. When the line box is in a descendant's box, flex / grid containers, equal-width columns, or a block-level flow with auto margins or floats, the first and last line positions are not derived and are still approximated from the font itself.
- `subgrid` as a track-list token.
- Using `position: fixed` as a Dialog or Drawer. Use Nana's overlays.
- A custom `url()` cursor image.
- An unknown `user-select` keyword.
- The full CSS Highlight API. A selection has only the two fields `selection_background` and `selection_color`, and CSS's `::selection` is a build warning in a view.
- `::before` and `::after` generating boxes, `content`, `attr()`, counters, and `quotes`.
- `-webkit-app-region` and `app-region`. An arbitrary box cannot be the title bar. Window dragging only goes through `AppTitleBar` and then `nana-window`. Writing it does not make a drag.
- Custom scrollbar chrome for `overflow:auto`. Use `ScrollView`.
- `<iframe>` loading. See [In-app browser](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#应用内浏览器). This one is not implemented.
- Browser-style `<video>` decoding and playback. For host-pushed frames and poster rules see [Live frames](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md).
- Treating `<canvas>` as a 2D bitmap or as a browser's 2D / WebGPU context. GPU content goes through `GpuTextureView` (`"nana.host-texture"`) in [Live frames](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md). That is not Chromium 2D.
- A parent's CSS `perspective` property and `preserve-3d`. The element's own `perspective()` with `rotateY` already paints.

A malformed `font-variation-settings` still fails closed. An axis the font does not have is skipped at shaping time and not rewritten as `wght`.

Android and iOS are **not** product layout targets. The two experimental hosts ([Mobile host](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/mobile-host.md)) still use the same `LayoutStyle`, Runtime, and UiScene, with no second mobile layout engine. See [Android](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/android.md) and [iOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ios.md). HarmonyOS / OpenHarmony share this same layout too and are only another compile target; see [OHOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ohos.md).

`:has`, `::before`, `::after`, `::placeholder`, and `::selection` are only parsed and matched by the engine; in a view they are all build warnings. See "Selectors" above.

`writing-mode: horizontal-tb | vertical-rl | vertical-lr` is wired into logical edges and into the inline axis of IFC and flex lines. See "What works" above.

Theme colors go through controls and appearance settings. Do not change framework tokens with arbitrary business colors.

An unknown declaration does not enter the Style Model. In a view it gives a build warning. Do not assume "writing it makes it take effect somewhere".

## Relation to Rust layout

What CSS parses is the same `LayoutStyle`.

L3 expresses this contract with [`Stack`](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md). Use `from_layout`, or the field builders.

What actually computes boxes is Runtime's `RuntimeLayoutEngine`. Product frames go through `RuntimeDocument::flush`.

## Page and card whitespace

Where NanaUI assigns container margins is in the [Rust layout contract](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md#浮层定位与容器内边距归属).

CSS covers only the edges you declare. `padding: 0` explicitly zeroes them.

A sibling's margin and the parent's gap add up. There is no automatic de-duplication and no automatic collapsing.
