# Layout result contract

`UiWorld` publishes one immutable `LayoutResult` for each laid out node. The
result is the render and input facing authority for box geometry; `layout_box`
is a compatibility projection of its `bounds` field.

Each result carries the border, padding and content boxes, first and last
baselines, the overflow and scroll extents, child placements, typed layout
fragments/parts, clip and containing-block references, dependency metadata and
the monotonic layout generation that produced it. Runtime frame updates expose
that generation so Scene extraction, hit testing, accessibility and scroll
consumers can correlate their projections without using the broader mutation
generation.

The generation is independent from the world mutation generation. It changes
only when layout output changes, so hover, opacity, transform and other
paint-only work cannot rebuild layout-derived geometry. Layout-affecting writes
clear the affected node, descendants and dependent ancestors before the next
published result, while topology changes keep stable node identities.

`ComponentGeometry` remains the compatibility surface for component semantic
parts and non-box drawing data (for example resize grips, chart paths and
editor decorations). Its box placement is anchored by the same `LayoutBox`
projection while consumers migrate to the richer result fields.
