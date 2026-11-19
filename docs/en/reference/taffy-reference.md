# Taffy reference benchmark

NanaUI's layout performance reference is [Taffy](https://github.com/DioxusLabs/taffy) 0.11 (the Rust layout engine used by Bevy, Dioxus, Zed and others). Taffy keeps a style, a layout cache and the computed box per node and stops there; it is the baseline for how fast "layout and nothing else" can be. `perf/micro/taffy-reference` has both lay out the same boxes on the same machine and answers three questions:

1. **What the reference is**: Taffy 0.11, on the same pages with the same CSS, every box checked to come out the same.
2. **How large the gap is**: cold layout, incremental edits, resident memory and instructions per box for flex and grid.
3. **What the extra cost buys**: what NanaUI stores and does per node beyond Taffy, which capability each item serves, and what the capabilities Taffy has no counterpart for cost in themselves.

The short answer: flex and grid cold layout is 1.4–1.6× slower while instructions per box are only 1.1–1.2× higher; by the fourth round's breakdown, turning off the three things kept for incremental edits and public geometry (plans, intrinsic facts, the published `LayoutResult`) lands flex at Taffy's per-box time. The gap is a trade-off in features, not a failure to catch up.

This is an observation, not a gate; the numbers move with the machine and its load, so read the ratios and where the cost goes.

## Running

The benchmark is a workspace of its own with its own lockfile, outside the root workspace; Taffy is a dependency of the benchmark only and never of a product crate.

```sh
cd perf/micro/taffy-reference
cargo run --release -- check              # both engines box by box (100 rows by default)
cargo run --release -- check 1000         # 21 001 boxes
cargo run --release -- compare 100,1000   # flex / grid, 2 101 and 21 001 boxes
cargo run --release -- features           # NanaUI-only capabilities, 1000 rows by default
ROUNDS=12 cargo run --release -- compare  # rounds to take the least of (8 by default)
SHOW=1 cargo run --release -- features    # also prints counters and stages of the inline edit and the scroll frames
SHOW=1 cargo run --release -- inline 100  # the inline text scene alone, at 100 paragraphs (1000 by default)
/usr/bin/time -l target/release/nana-taffy-reference reps flex nana 1000 5
                                          # a fixed amount of work, to read instructions retired from outside
```

## Method

- **Pages**: N rows of 10 cells under a 1280 px root, each cell with 0.25 px padding around one 24 × 16 leaf (`1 + N × 21` boxes, the shape of `perf/micro/stack-depth`'s `wide`). Flex page: the root is a column, rows are rows with a 4 px gap. Grid page: every row is a grid of ten `1fr` columns with a 4 px column gap.
- **The same CSS**: on the NanaUI side `Stack::from_layout` writes out the same style as Taffy's field by field, one shared layout per role. Taffy's rounding is off (NanaUI keeps fractional pixels). `check` compares every box's page position and size: 21 001 boxes each for `flex` and `grid` and 2 103 for the flex page with a header, with 0 boxes differing by more than 0.01 px.
- **Cold layout**: for NanaUI, the product frame's first layout (`layout_document_for_viewport`, layout only, no Scene, hit testing or accessibility); for Taffy, `compute_layout`. "Build + layout" starts from an empty tree.
- **Incremental edits**: NanaUI edits in a product frame (`RuntimeDocument::flush`) and reports the layout stage and the whole frame (with the hit index, accessibility and extraction); Taffy does `set_style` (which marks the node and its ancestors dirty as `mark_dirty` does) and `compute_layout`.
- **Timing**: within each round the two engines take turns, swapping order every round, and the least is read; the whole program runs twice (8 and 12 rounds) and the least of both is taken. Release, aarch64 macOS, 2026-11-09, machine load 3–5.
- **Instructions**: read in process from `proc_pid_rusage`'s `ri_instructions`, which is what `/usr/bin/time -l` prints as instructions retired, also the least over the rounds. Cross-checked against `time -l` (`reps flex … 100`, 25 runs less 5, over 20 × 2 101 boxes): NanaUI 39.9k per box (with teardown) against 38.3k for build + layout in process; Taffy 18.0k against 17.8k.
- **Memory**: the heap the laid-out tree holds (allocations less frees), not allocation traffic.
- **Why this differs from Resource budgets**: the 1.24 µs / box, 2 523 B and 23.3k layout instructions at 21 001 boxes in [Resource budgets](/reference/resource-budgets) come from `stack-depth`'s `wide`, whose page uses the `Stack::row` / `column` presets (rows that shrink to content, children centred, and so on), and whose instruction counts subtract whole processes, teardown included. Here the styles are CSS matched one for one with Taffy's, so the absolute values are somewhat lower and the ratios are comparable.

## The gap

21 001 boxes (per box):

| Layout | Engine | Cold layout | Build + layout | Resident | Layout instructions | Build + layout instructions |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| flex | NanaUI | 1.00 µs | 2.04 µs | 2 399 B | 17.8k | 37.8k |
| flex | Taffy | 0.71 µs | 0.82 µs | 1 252 B | 15.7k | 17.4k |
| flex | ratio | 1.4× | 2.5× | 1.9× | 1.1× | 2.2× |
| grid | NanaUI | 1.39 µs | 2.43 µs | 2 638 B | 26.0k | 46.0k |
| grid | Taffy | 0.89 µs | 1.00 µs | 1 278 B | 21.3k | 23.0k |
| grid | ratio | 1.6× | 2.4× | 2.1× | 1.2× | 2.0× |

At 2 101 boxes (everything in cache) cold layout is 0.79 against 0.64 µs for flex (1.24×) and 1.16 against 0.82 µs for grid (1.4×); resident 2 692 / 2 930 B against 1 256 / 1 283 B.

Incremental edits at 21 001 boxes: one leaf in the middle row toggles its `min-height` between 16 and 24 px (that row grows and every row below moves). Per edit:

| Layout | NanaUI layout stage | NanaUI whole frame | NanaUI instructions | Taffy layout | Taffy instructions | Layout ratio | Instruction ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| flex | 206 µs | 513 µs | 11.2 M | 94.2 µs | 2.15 M | 2.2× | 5.2× |
| grid | 459 µs | 758 µs | 16.2 M | 99.2 µs | 2.25 M | 4.6× | 7.2× |

How to read it:

- **Flex and grid**: 10–20% more instructions per box, 40–60% more time. Most of the extra time is the larger number of bytes each node reads and writes (layout is memory-bound on large trees, see the scale curve under "CPU time" in Resource budgets), not more computation.
- **Building**: NanaUI builds at about 1.04 µs per box, Taffy at about 0.1 µs; see the next section.
- **Incremental**: the layout stage is 2–5× slower; the other half of the whole frame is the hit index, accessibility and extraction, which Taffy does not have.

## What the extra cost buys

The flex page at 21 001 boxes, per box. The time shares come from the fourth cold-layout round under "CPU time" in [Resource budgets](/reference/resource-budgets) (turning each item off and measuring again on the `wide` page, for the breakdown only, with wrong results), scaled to the 1.00 µs here; the items do not add up, and all four off is −28%. In the memory column, "tree before layout" and "retained by layout" are measured by this benchmark; the finer items come from the allocation-stack attribution under "Resident memory" in Resource budgets (same page shape, rounds of 2026-11-05 to 11-09) and are approximate.

| Done / stored beyond Taffy | Per box | Capability it buys |
| --- | ---: | --- |
| Measurement plans and container plans (built for every container on every pass; the first layout prepares them for the next frame) | about 17%, ≈ 0.17 µs; ≈ 395 B (measurement plans 244, container plan table 151) | Incremental edits with no measurement: successive size changes within a bucket measure nothing, one row growing among a million measures at most 2, child-list edits replay instead of walking every child (9 `WorkCounters` contracts; not building plans for single-child containers turns them red, see round four in Resource budgets) |
| Intrinsic facts (what each box's used size depended on, recorded while measuring) | about 10%, ≈ 0.10 µs; ≈ 120 B (intrinsic table) | Narrow invalidation: an edit lays out only the boxes it actually affects |
| Publishing `LayoutResult` (a 280 B struct per node, plus slices of fragments, parts, child placements and dependencies) | about 20%, ≈ 0.20 µs; ≈ 614 B | The public geometry contract: fragments, parts, baselines, overflow, scroll extent, containing block; Scene, hit testing, accessibility and IME read it |
| All three above (both plans, intrinsic facts, publishing) off | −28%, ≈ 0.72 µs left | On par with Taffy's 0.71 µs |
| Other retained tables (boxes, placements, padding, contexts, frames, line-box memo, plan entries) | ≈ 230 B | Retained state for relative geometry (`abs = frame + local`) and line-box answers |
| Building: components, node records, interned styles, mount commits | 1.04 µs against Taffy's about 0.1 µs; 1 043 B (Taffy's whole tree is 1 252 B) | The component model and atomic `MutationQueue` commits; the `NodeSpace` classification on `NodeRecord` (2 B) that lets moves, hit testing and accessibility skip subtrees; component views, interaction and accessibility state |
| The product frame's other stages: hit index, accessibility, extraction and Scene | first frame 0.18 + 0.26 + 0.61 µs; +1 897 B (4 296 B resident after the product frame) | Hit testing, the accessibility tree, the GPU scene; this benchmark does not split the 1.9 KB between them |

Together: the 0.29 µs flex cold layout spends beyond Taffy is essentially what is kept for incremental edits and public geometry, and about 1.13 KB of the 1 356 B retained by layout is plans, intrinsic facts and published results. Turning them off gives up the incremental contracts and the `LayoutResult` contract, so they are no longer listed as a gap to close; see "To do" in Resource budgets.

## Capabilities Taffy has no counterpart for

NanaUI only, same machine, also the least over rounds (`features 1000`):

| Capability and scene | Cost | Incremental / query | Where the cost lands |
| --- | --- | --- | --- |
| **Inline text wrapping and baselines**: 1 000 paragraphs, each 400 px wide with 4 text runs (mixed Latin and CJK) and 2 inline-flex boxes, broken into 3 lines (7 001 boxes) | first product frame 3.98 µs / box (layout stage 1.02), 4 789 B / box, 68.4k instructions / box | one run's text in a middle paragraph changed: layout 0.18 ms, frame 0.48 ms, 10.0 M instructions (1.78 ms, 2.10 ms and 44 M before, see "Observations") | nana-text shaping and line breaking (the TextShape stage), inline records, the line-box memo and baselines; the line count changes and the 500 paragraphs below only write their new offset, without packing their line boxes again (1 measured, 505 placed) |
| **Sticky and fixed**: the flex page in an 800 px scroller, every tenth row `sticky; top: 0`, plus 20 `fixed` badges | first frame 2.88 µs / box (2.85 without them) | one scroll: whole frame 84.8 µs, 1.92 M instructions; 9.0 µs and 0.20 M without them; layout stage 0 (before: 6.32 ms / 94.7 M and 5.48 ms / 85.9 M, see "Observations") | Sticky sticks after scroll projection and the Scene places it, with no layout writeback; scrolling does not lay out. A scroll changes only the scroll container's accessibility node; what is left is one hit entry per sticky box and one accessibility node per pin (sticky, fixed) |
| **Moves under retained relative geometry**: a header grows and the column holding every row moves down | 2 100 boxes in the column: layout 5.9 µs, frame 13.6 µs, 0.25 M instructions; 21 000 boxes: 5.8 µs, 17.9 µs, 0.35 M | at both sizes 1 box measured and 3 placed; Taffy 1.0 µs for the same edit | Boxes are stored as offsets from their frame, so moving a parent changes its one offset and the subtree is not laid out again; hit testing, accessibility and the Scene do not walk it either. The cost does not grow with the boxes moved |
| **Hit testing and the accessibility tree**: the flex page's first product frame (21 001 boxes) | per box: hit index 0.18 µs, accessibility 0.26 µs, extraction 0.61 µs; 4 296 B resident (2 399 B after layout) | one hit query 0.16 µs; projecting the whole accessibility tree 0.17 µs / node | Hit entries and accessibility nodes are kept in local coordinates, so a move changes only the root |
| **Deep trees without stack overflow**: a chain of nested flex columns on a 512 KiB thread | 200 levels: NanaUI whole frame 794 µs, Taffy layout 232 µs | 500 levels: NanaUI whole frame 1 969 µs, Taffy overflows the stack | Placement walks at most `SPAN` levels on the thread stack, and measuring and building run iteratively on explicit heap work-stacks; see "Stack" in Resource budgets |

## Observations

These were measured along the way; they are not gates. The ones marked "fixed" were addressed later:

- **An edit in an inline paragraph scales with the paragraphs it moves** (fixed): changing one run takes about 0.17 ms of layout on a 100-paragraph page and about 1.6 ms on a 1 000-paragraph page, with 1 box measured and the rest paragraphs that only move their origin; the flex page's row-height edit, which moves 500 rows as well, lays out in about 0.2 ms. Each moved paragraph costs about 3 µs, an order of magnitude more than a row of plain boxes. Root cause: a paragraph that only moved still packed its line boxes again. An inline formatting context records no container plan, so once its parent's plan moved it, `place_node_body` ran `lay_out_children` on it: each run and atom took its measurement from the cache, `pack_ifc_line_boxes` broke the lines again and aligned their baselines, and each child was then found unchanged (about nine tenths of the layout stage in `sample` sat in these paragraphs' `lay_out_children`). The children's boxes are measured from the paragraph's own origin, so a move cannot change them: now, once `place_node_head` has written the new offset, `keeps_moved_subtree` finds that nothing the body reads has changed and skips the body (conditions in [Relative geometry · Translating a subtree](/reference/relative-geometry#_3-2-translating-a-subtree)); plain-box containers, which used to re-check their affected children against the plan, skip it too. Against 0b60695fd, 8 interleaved rounds swapping the order each round, least of each (load 9–18):

  | Page | Layout | Frame | Instructions per edit |
  | --- | ---: | ---: | ---: |
  | 100 paragraphs (50 moved) | 169 → 34 µs | 212 → 77 µs | 4.48 → 1.46 M |
  | 1 000 paragraphs (500 moved) | 1 779 → 181 µs | 2 101 → 481 µs | 44.0 → 9.98 M |

  Per moved paragraph: layout 3.6 → 0.33 µs, frame 4.2 → 0.90 µs, instructions 88k → 19k, the same order as a row of plain boxes. It is still linear in the paragraphs: they are the root's direct children, so each writes its new offset and publishes its result once, and hit testing and accessibility take about 0.25 µs each per paragraph (the same side as the next item). Cold layout is unchanged (3.94 → 3.96 µs and 69.8k → 69.9k instructions per box).
- **One scroll of a large scroller is linear** (fixed): at 21 001 boxes a scroll frame takes about 5.5 ms, nearly all of it in the accessibility and hit-testing stages (2 100 clip updates); layout and extraction take under 20 µs. Root cause: a scroll was a whole-subtree seed in accessibility, and children's bounds were "offset − parent's scroll", so every scroll projected all 21 001 nodes in the scroller again (about 6 ms); in hit testing each of the 100 sticky rows was patched whole (2 100 entries). Now the scroll is held on the scroll container's own `AccessibilityNode::scroll`, children's bounds do not depend on it, and a scrolled container projects only itself and its pins; sticky hit entries are rebuilt in place first (a2b52d862, see [Relative geometry · Accessibility](/reference/relative-geometry#_7-2-translating-a-subtree)). Alternated with 0b60695fd for 8 rounds, minimum: without sticky / fixed, 2 101 boxes 449 → 4.3 µs and 8.64 → 0.07 M instructions, 21 001 boxes 6 429 → 10.2 µs and 85.8 → 0.20 M; with sticky / fixed 504 → 34.5 µs and 7 375 → 353 µs (9.57 → 0.52 M, 94.5 → 4.05 M). Without pins only the scroll container's own projection is left (O(direct children)). With pins, a stuck sticky branch was then still patched whole in hit testing (the sticky offset was in the entry's transform, which child entries inherit); now the sticky offset is in the entry's origin and each sticky box rebuilds only its own entry (62a76afe6, see [Relative geometry · Hit index](/reference/relative-geometry#_4-hit-index)). Alternated with 77f2b6f65 for 8 rounds, minimum: with sticky / fixed 2 101 boxes 30.5 → 16.1 µs and 0.52 → 0.33 M, 21 001 boxes 312.5 → 84.8 µs and 4.03 → 1.92 M; what is left is one hit entry per sticky box and one accessibility node per pin, neither growing with what the pin holds. The incremental tree equals a full projection and a cold build scrolled straight there to the bit (including a scroll back to the origin, which exposed a missed projection fixed in 36c731159), and the hit grids agree.
