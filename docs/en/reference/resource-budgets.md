# Resource budgets: stack, memory, and performance

This page is the framework's contract for stack, resident memory, and CPU time. Each item gives a budget, the gate that holds it, the command that measures it, and a comparison against a peer implementation that can be run on the same machine. A number without a gate is an observation, not a budget; a comparison that has not been run gets no number.

## Stack

### Runtime environment

The framework's frame path (mount, style, layout, publish, hit index, accessibility, scene extraction, drop) runs on threads the host provides, and the thread stack size is decided by the platform:

| Thread | Stack |
| --- | ---: |
| Windows main thread (MSVC default link) | 1 MiB |
| macOS / iOS non-main thread | 512 KiB |
| Rust `std::thread::spawn` default | 2 MiB |
| Linux, Android main thread | 8 MiB (`ulimit -s`) |

So the budget is set by the tightest tier: a full frame, in both release and debug, must complete within **256 KiB** (half of a 512 KiB thread kept as margin).

### Tree depth limit

A node may sit at most at depth `MAX_TREE_DEPTH = 512` (`nana_ui_runtime::MAX_TREE_DEPTH`). Beyond that, mount returns `UiWorldError::TreeTooDeep` and never reaches any recursion. Blink's HTML parser uses the same limit for DOM nesting.

### Rule: which traversals may recurse

1. **Measurement** and **view tree building** are the only two places that recurse by tree depth. In measurement, sizes must be computed upward from children that received constraints passed down; in tree building, an element must build its children inside its own scope. Neither has an iterative form that preserves semantics. Every level must pass through a fallback: measurement goes through `layout_engine::recursion::level`, tree building through `crate::stack::level`. When less than 256 KiB of stack remains, that level continues on a newly allocated 4 MiB stack segment (`stacker::grow`). On desktop platforms and OpenHarmony (a Linux target) the remaining stack is read by `stacker`; on iOS and Android it is read from the thread's own stack bounds (`pthread_get_stackaddr_np` / `pthread_getattr_np`). `stacker` cannot read these two platforms, so the fallback previously had no effect there. The fallback therefore works on every platform the framework supports.
2. **Placement** is bounded recursion: one call descends at most `recursion::SPAN` levels (24 in release, 8 in debug, about 90 KiB in both builds). Nodes below that only write their own box; their subtrees are queued as boundaries for the current pass, and the outermost call places them in order before returning. The recursion depth is independent of tree depth, and placement needs no fallback. See [Layout · Layout recursion and the stack](layout.md#layout-recursion-and-the-stack).
3. **All other tree traversals** must use a loop or an explicit stack on the heap, and must not recurse by depth. Converted in this change: hit index construction (`HitIndex::insert_tree`), hit queries (`HitIndex::visit_hits`) and hit tree counting (`count_hit_entries`), style resolution along the ancestor chain (`resolve_style`), line box queries (`holds_line_box_in`), and atom baseline lookup up the ancestors (`atom_baseline_reads_content`). When adding a traversal, verify it with `scripts/audit-stack-frames.py` and the gates below.
4. **Application-owned nested models** fall into two classes; see "Application model nesting" below. Models with no depth limit (tree view data) never recurse by depth. Pane split trees (Dock, PaneTree, region layout) have the structural limit `MAX_SPLIT_DEPTH` and recurse within it.
5. **Parsing runtime content** (Markdown, Mermaid diagrams, CSS, ICU messages, paint scripts) recurses with the nesting depth of the input, so every site has a depth limit or was turned into a loop; see "Runtime content nesting" below. Past the limit it returns a parse error, skips that section, or degrades to plain text. It does not panic.

### Runtime content nesting

| Parser | Limit | Past the limit |
| --- | --- | --- |
| Mermaid: zenuml braces | `MAX_BRACES` 64 | `TooLarge(ClusterDepth)` |
| Mermaid: railroad groups and calls | `MAX_DEPTH` 48 | `TooLarge(ClusterDepth)` |
| Mermaid: containers in block, c4, state, class, sequence, architecture, flowchart | `Limits::cluster_depth` 16, total containers `Limits::clusters` 300 | `TooLarge(ClusterDepth)` / `TooLarge(Clusters)` |
| Mermaid: usecase JSON tables | 32 | Warning, no table |
| Mermaid: `%%{init}%%` config | 8 levels | Deeper objects are skipped whole; later keys are still read |
| Mermaid: indented trees (mindmap, treeView, treemap, ishikawa, kanban) | Source budget (about 490 levels) | Layout does not recurse per level; no limit needed |
| Markdown (`rich_text`) | None | Blocks are flat, `pulldown-cmark` does not recurse, `layout_block` adds only one level |
| ICU messages (`i18n::message`) | 16 levels | Parse error, shown as source text |
| Paint script path objects (`paint_script::parse_path`) | None | Unrolled into a loop; the JSON itself is capped at 128 levels by `serde_json` |
| CSS: `@media` / `@container` / `@supports` / `@layer` blocks | `MAX_NESTING` 32 | That block is skipped and counted in `skipped_at_rules` |
| CSS: parentheses in container queries and `@supports` conditions, `color-mix()` | `MAX_NESTING` 32 | Query `Invalid` / condition unknown / color invalid |
| CSS: parentheses and math functions in `calc()` | `MAX_CALC_DEPTH` 16 | Value invalid |
| CSS: `var()` reference chains | `MAX_VAR_DEPTH` 32 | Treated as unresolved: the fallback value is used; a self-reference `--a: var(--a)` also stops here |

Every parser has a test that parses ten thousand levels on a 512 KiB thread (for indented trees, the deepest the source budget can hold): `nana-ui-diagram/tests/deep_nesting.rs`, `nana-ui-css/tests/deep_nesting.rs`, and unit tests in `rich_text`, `i18n::message`, and `paint_script`. Depths within the limit are also bound by the stack budget: each level of nested CSS blocks takes about 6.5 KiB of frame in debug (style rule processing was split out into `parse_style_rule`, outside the recursive frame), so 32 levels take about 210 KiB.

The railroad nesting limit counts "parentheses and calls" (`MAX_DEPTH` 48 primaries), but within one level of parentheses an expression can nest several more levels (`!( … )***` is a predicate plus three repetition marks, seven levels), and after parsing `measure` and `draw` recurse by expression level. So both parsing and layout keep only the recursion itself in the frame: `Fail` is boxed (`Result<Expr, Fail>` drops from 48 B to 40 B); branches that read a single token live in `primary_token`; repetition marks and the `-` exception live in `repetition_marks` / `except` / `predicated`; call assembly lives in `build_call`; and the arithmetic, text, and paths of each node kind live in functions outside `*_node` / `draw_*`. Per-level frames under aarch64 macOS debug:

| Path | Before | After |
| --- | ---: | ---: |
| Parse, one level of parentheses (`alt → seq → term → primary → primary_inner`) | 7.1 KiB | 3.0 KiB |
| Parse, one level of call (plus `call`) | 10.1 KiB | 3.7 KiB |
| Layout, one level of expression (`measure` + branch / `draw` + branch) | 3.0 / 3.6 KiB | 0.4–0.7 / 0.3–0.5 KiB |

Stack needed for a full render at the limit depth (bisected to the page): before, the deepest in debug was a predicate plus repetition marks, needing about 1.2 MiB, which overflows a 512 KiB thread; call nesting needed about 480 KiB. After, every kind of nesting fits in 180 KiB in debug, and release fits in 180 KiB (210 KiB before). Gate `deep_nesting::railroad_at_its_depth_limit_fits_half_the_thread`: each of the seven kinds of nesting at 47 levels of parentheses renders successfully on a 256 KiB thread (half of 512 KiB) with no fewer primitives than the unnested diagram, and 48 levels return `TooLarge(ClusterDepth)`. It runs in both debug and `--release`. The code before the change overflows the stack on it in debug.

### Application model nesting

| Model | Depth | How it avoids overflow |
| --- | --- | --- |
| `TreeNode` (data for `TreeView`: file trees, JSON documents) | Unbounded | Traversal goes through `nana_ui_core::tree_preorder` (a preorder iterator with its stack on the heap, same order as recursion) or an explicit stack on the heap. `Clone`, `PartialEq`, and `Drop` for `TreeNode` are hand-written to not recurse (descendants are detached before drop). Only the derived `Debug` still nests per level |
| `DockNode`, `PaneTreeNode`, `AreaNode` (pane split trees) | `MAX_SPLIT_DEPTH` 64 | The framework's own splits (a Dock drop onto an edge, `AreaLayoutModel::split`) are refused at the limit; a layout at the limit depth saves and loads back through `serde_json`. Deeper trees the application builds itself (including hand-edited layout files, up to about 125 levels) are folded to the limit as they enter the framework: through constructors (`Dock::new`, `DockWorkspace::new`, `PaneTree::new`, `AreaLayout::new`, persistence restore), and on every create and update of these three components (`ComponentView::fit_framework_limits`, so writing the public fields directly does not escape it). Dock gathers every item in the out-of-bounds subtree into one tab group and loses none; PaneTree and region layout keep the first pane or region of the out-of-bounds split and drop the rest (region layout also removes them from `areas` and `maximized`). The folding itself does not recurse. Traversals inside the framework therefore recurse only on trees within the limit; over-deep values the application holds and traverses itself are outside the framework's jurisdiction |
| JSON specs of built-in components (`tree_node_from_json`, `dock_node_from_json`) | `serde_json`'s 128 levels | Traversal walks the parsed `serde_json::Value`, whose depth the parser has already capped |

Tests: `tree_view::tests::data_deeper_than_the_stack_walks_clones_compares_and_drops` (twenty thousand levels of data on a 512 KiB thread: list rows, select, navigate, expand and collapse, clone, compare, drop; without the iterative drop, the stack overflows); `nana-ui-runtime/tests/deep_models.rs` (Dock, region layout, and PaneTree at the limit depth on a 512 KiB thread: clone, compare, place, save, load back, drop, plus the deepest hand-written Dock JSON); `dock::tests::a_drop_past_the_split_depth_is_refused`, `area_layout::model::tests::a_split_past_the_split_depth_is_refused`; and the five-thousand-level application-built trees in `deep_models.rs`: through constructors, created and laid out after writing the public fields directly, and written during an update. All three paths pass on a 512 KiB thread; removing the component hook overflows the stack (mutation experiment done).

### Gates

| Gate | What it guards | Command |
| --- | --- | --- |
| `nana-ui-scene/tests/deep_tree_stack.rs` | A 508-level tree (limit minus 4) runs a full frame on a 256 KiB thread (both release and debug): mount, layout, extraction, hit testing, accessibility, change the deepest text and run another frame, drop | `cargo test -p nana-ui-scene --test deep_tree_stack`, then again with `--release` |
| `layout_engine::stack_budget` | Per-level stack of measurement and placement stays within budget (debug 16 / 12 KiB, release 4 / 4 KiB); a 400-level tree lays out on a 1 MiB thread | `cargo test -p nana-ui-runtime --lib stack_budget` |
| `view::tests::a_view_deeper_than_the_stack_still_mounts` | A 400-level view mounts, lays out, and drops on a 1 MiB thread | `cargo test -p nana-ui-runtime --lib a_view_deeper` |
| `scripts/audit-stack-frames.py` | Stack frame of a single function | See below |
| `framework::tests::context_and_world_stay_small_enough_to_move` | `AppContext` ≤ 6 KiB and `UiWorld` ≤ 10 KiB, both moved by value during construction | `cargo test -p nana-ui-runtime --lib context_and_world` |
| `deep_nesting::railroad_at_its_depth_limit_fits_half_the_thread` | The seven kinds of railroad nesting render at the limit depth on a 256 KiB thread | `cargo test -p nana-ui-diagram --test diagram deep_nesting::railroad`, then again with `--release` |

`WorkCounters` publishes `layout_measure_stack_per_level`, `layout_placement_stack_per_level` (stack per level, in bytes, maximum over a frame), and `layout_placement_boundaries` (how many subtrees placement drained from the queue).

### Stack frames of single functions

A function's frame is paid on every call, recursive or not. `scripts/audit-stack-frames.py` reads how much each function's prologue subtracts from the stack pointer in the binary and lists the functions over budget:

```bash
cargo test --release -p nana-ui-scene --test deep_tree_stack --no-run
python3 scripts/audit-stack-frames.py target/release/deps/deep_tree_stack-<hash> --budget 16384
```

Budget: functions on the frame path may not exceed 16 KiB in release, and the same applies to the construction path (`AppContext::new` / `from_world` / `typed`, `RuntimeDocument::new`). `AppContext` and `UiWorld` are moved by value during construction: the frame of `AppContext::new` holds the newly built world, `from_world` takes it over, and `RuntimeDocument` then takes over the context, so each of those frames holds them whole. Since 2026-11-05 the large fields of both are boxed: `world: Box<UiWorld>` in the context (the world is four fifths of the context), and the four `WorkCounters` in the world (1.7 KiB each) merged into `counters: Box<WorldCounters>`. Each world access from the context and each counter increment in the world costs one more pointer load; the interleaved `perf/micro/stack-depth` benchmark is within noise (see below). aarch64 macOS, `--all-features`:

| Type | Before | After |
| --- | ---: | ---: |
| `AppContext` | 18 848 B | 3 888 B |
| `UiWorld` | 14 968 B | 8 128 B |

| Function (release) | Before | After |
| --- | ---: | ---: |
| `AppContext::from_world` | 35.2 KiB | 6.3 KiB |
| `RuntimeDocument::with_context` / `new` | 21.6 / 18.2 KiB | 7.2 / 3.7 KiB |
| `AppContext::default` | 14.5 KiB | 7.8 KiB |
| `UiWorld::new` | — | 6.9 KiB |

In debug, the largest on the construction path is `AppContext::with_builtins` at 15.3 KiB (temporaries from field initialization); `from_world` and `new` are 7.8 KiB, and `RuntimeDocument::with_context` / `new` are 5.5 / 3.7 KiB (19 / 18 KiB before). Gate: `framework::tests::context_and_world_stay_small_enough_to_move` requires `AppContext` to stay within 6 KiB and `UiWorld` within 10 KiB (`cargo test -p nana-ui-runtime --lib context_and_world`). The frames themselves are measured with the script above; in release no `nana` function currently exceeds 16 KiB (script exit code 0).

In debug, several functions on the frame path used to be far over budget: `UiWorld::derive_component_geometry` 111 KiB, `UiScene::build_node_primitives` 74 KiB, `controls_base::build` 72 KiB, `overlays::build` 28 KiB, `text::build` 19 KiB (in release, under 9, 5, and 6 KiB respectively). They are all large `match`es on `StandardVisual`, and unoptimized code reserves a slot for the locals of every branch, so whichever branch runs occupies all of them. Now every branch of 12 lines or more runs inside `own_frame(|| ...)` (the runtime's `crate::stack::own_frame` and the scene's `primitives::own_frame`, an `#[inline(always)]` single call): in debug the closure is its own frame, so a branch's locals occupy stack only when it runs; in release the closure is inlined and the code is unchanged. In debug these functions are now under 16, 24, and 16 KiB, and the largest branch closures are 20 and 29 KiB. Treat new large `match` branches the same way.

### Comparison with peer implementations

`perf/micro/stack-depth` measures, on the same machine, "the smallest thread stack that completes one layout". Each probe runs in a child process and is bisected to 4 KiB. NanaUI and [Taffy](https://github.com/DioxusLabs/taffy) 0.11 (the Rust layout engine used by Bevy, Dioxus, Zed, and others) lay out the same chain of nested flex columns:

```bash
cd perf/micro/stack-depth && cargo run --release -- report
```

| Depth | NanaUI layout | NanaUI full frame | Taffy layout |
| ---: | ---: | ---: | ---: |
| 64 | 116 KiB | 116 KiB | 132 KiB |
| 256 | 116 KiB | 116 KiB | 484 KiB |
| 500 | 116 KiB | 116 KiB | 916 KiB |

(release, aarch64 macOS, 2026-11-02.) The stack NanaUI needs is independent of depth: placement walks at most `SPAN` levels on the thread stack (about 3.5 KiB per level), and measurement and tree building switch to a new stack segment when less than 256 KiB remains, so this number is essentially `SPAN` levels of placement plus the fixed cost of the frame path. Taffy takes about 1.8 KiB per level, and 500 levels overflow a 512 KiB thread.

## Resident memory

`nana-ui-runtime/tests/node_memory.rs` (the `component-memory` gate in `perf/scenarios/catalog.json`) measures the heap each node keeps resident after layout (allocations minus frees, counted per thread): Text no more than 4.2 KB, Button 5.8 KB, ListItem 6.4 KB, and the per-node figure does not grow with scale between 1k and 10k nodes.

Comparison (`cargo run --release -- wide`, a page of 1 column × N rows × 10 cells, each cell a column wrapping a row; "layout allocations" is the number of allocations in one `layout_document` call):

| Boxes | NanaUI per box | NanaUI layout allocations / box | Taffy per box | Taffy layout allocations / box |
| ---: | ---: | ---: | ---: | ---: |
| 2 101 | 3.5 KB (was 3.7) | 25.5 (was 32.7) | 0.9 KB | 6.9 |
| 10 501 | 3.0 KB (was 3.2) | 25.4 (was 32.6) | 1.5 KB | 6.9 |
| 21 001 | 3.0 KB (was 3.2) | 25.3 (was 32.6) | 1.5 KB | 6.9 |

(release, aarch64 macOS, 2026-11-02, interleaved 8 rounds with the pre-change binary.) The two store different things: Taffy stores only layout style and layout cache; a NanaUI node also carries the component record, resolved style, retained layout results, invalidation and dependency indexes, and accessibility state. Where NanaUI's roughly 3.0 KB per box goes at 10 501 boxes (difference measured by temporarily turning each part off):

| Part | Per box |
| --- | ---: |
| Node body present before layout (record, component, style reference, indexes) | ≈ 1.5 KB |
| Published `LayoutResult` (280 B struct, plus one placement, fragment, part, and dependency per child in the parent, about 160 B) | ≈ 0.55 KB |
| Retained intrinsic and used-size caches (`intrinsics`, two slots of 112 B each; `intrinsic_metrics`) | ≈ 0.4 KB |
| Container plans and measurement plans | ≈ 0.2 KB |
| Box, padding, and context tables | ≈ 0.2 KB |
| Full-page snapshot (`full_snapshots`, a copy of the boxes and three tables) | ≈ 0.1 KB |

Even with the different scope, about 2× per box is a real gap; see "To do".

Since 2026-11-05, attribution uses allocation stacks (`HOLD=1` keeps the page alive, `MallocStackLogging=1` plus `malloc_history <pid> -allBySize`; see the cold layout section under "CPU time"). This is more accurate than turning parts off one by one, and it also shows the allocator's size classes. Product first frame (`FRAME_PATH=1`) at 21 001 boxes: after the change, 3.10 KB resident per box (3.59 KB before), going to:

| Part | Per box |
| --- | ---: |
| Node table (inline 432 B records; 21 001 nodes sit in 32 768 slots after doubling; boxing would save the empty slots but slows incremental edits, see step 7 under "CPU time") | 675 B |
| Component views (one `Box<Stack>` per node) and component index | 264 B |
| `text_nodes` (one entry per node, `TextNodeState` 144 B; building on demand is under "To do") | 190 B |
| Remaining tree-building tables (child lists, styles, interaction, indexes) | 285 B |
| Published `Arc<LayoutResult>` (280 B struct) | 320 B |
| Result slices (dependencies, parts, fragments, child placements; equal ones already shared) | 294 B |
| Measurement plans (two per cell) and slot table | 319 B |
| Retained intrinsic table (`intrinsics`, two 112 B slots per entry) | 240 B |
| Container plan hash table | 151 B |
| Remaining retained layout tables (boxes, placements, padding, contexts, reference frames, line box memo, plan entries) | 588 B |

(The total includes `AppContext` itself and the built-in registries amortized per box, about 0.25 KB more than the resident figure `wide` reports.)

After the second round on 2026-11-06 (see the second round of cold layout under "CPU time"), the same measure gives 2.73 KB resident per box (the figure `wide` reports; the allocation-stack total is 2.93 KB, including `AppContext` and registries). The changes:

| Part | Per box |
| --- | ---: |
| Node table (records share `NodeStyle`, 248 B, cache-line aligned to 256 B, 32 768 slots) | 675 → 400 B |
| Measurement plans and entries (containers with no content of their own do not allocate `PlanOwnContent`; size class per entry 320 → 160 B) | 319 → 244 B |
| Per-node `Arc<NodeKind>` (shared by tag) | about 64 B → 0 |
| Component views (one `Box<Stack>` per node, `NodeStyle` stored by value inside, 224 B class) | 213 B |
| `text_nodes`, published `Arc<LayoutResult>`, intrinsic table, container plan hash table | 190 / 320 / 240 / 151 B, unchanged |

Plain box page at 2 101 / 21 001 / 67 201 boxes: 3 472 / 3 102 / 3 246 → 3 025 / 2 727 / 2 798 B; control page at 3 101 / 31 001 boxes: 3 456 / 3 459 → 3 161 / 3 211 B.

After the third round on 2026-11-06 (see the third round of cold layout under "CPU time"), 2.54 KB resident per box (the figure `wide` reports; the allocation-stack total is 2.74 KB). The change is in the dense tables indexed by id: they used to reserve by node count, but ids run from 1 to the highest, with more slots than nodes, so the last insert doubled the table. After reserving by the highest id:

| Part | Per box |
| --- | ---: |
| Retained intrinsic table (`RetainedIntrinsic`, 120 B per slot) | 240 → 120 B |
| Retained box table and padding table (32 B per slot) | 65 + 65 → 33 + 33 B |
| Measurement plan slot table, context and remote origin tables | about −20 B |

Plain box page at 2 101 / 21 001 / 67 201 boxes: 3 025 / 2 727 / 2 798 → 2 840 / 2 541 / 2 791 B; control page at 3 101 / 31 001 boxes: 3 161 / 3 211 → 2 963 / 3 014 B. At 67 201 boxes the ids exceed `DENSE_LIMIT`, and the tables were already capped at the limit, so it barely changes.

After the fourth round on 2026-11-09 (see the fourth round of cold layout under "CPU time"), the plain box page at 21 001 boxes holds 2 541 → 2 523 B per box (allocation-stack total 2 748 → 2 730 B): the first layout hands the pass's measurement plan table to the retained cache whole instead of inserting entry by entry into a table that doubles up from empty; the control page at 31 001 boxes 3 014 → 3 012 B. Tree building is unchanged (12.2 allocations, 1 031 B per box). Recording no plans for single-child containers would save another 360 B per box but breaks the incremental contracts and was withdrawn; see the same section.

**The node id gives up its zero (2026-11-11).** `StableNodeId` has always reserved zero, `new(0)` being `None`, but held a plain `u64`, which the compiler could not know: every optional node id (`Option<StableNodeId>`) took 16 bytes. Holding a `NonZeroU64` it takes 8, and the types that carry one shrink with it: `LayoutResult` 280 → 256 B, `LayoutPart` 40 → 32 B, `LayoutFragment` 88 → 80 B, `Hierarchy` 40 → 32 B, `AccessibilityNode` 416 → 400 B. The plain box page (`wide 2000`, 42 001 boxes, the product's first frame) holds 2 522 → 2 452 B per box and the controls page (62 001 boxes) 3 011 → 2 916 B; cold layout per box, alternating with the build before over four rounds and reading the minimum, 1.40 µs against 1.40 µs. The public interface is as it was (`new` returns an `Option`, `get` a `u64`).

**System font files are mapped, not read onto the heap (2026-11-11).** A platform's font files are large and an application uses a small part of them: PingFang, Hiragino and Apple Color Emoji are 60–190 MB each, and a line of text reads a few tables and a handful of glyphs. The font set used to `fs::read` a whole file the first time one of its faces needed bytes and keep it, and every face of one `.ttc` read its own copy. Files are now mapped read-only through `font/mapped.rs`, and the faces of one file share one mapping; the bytes stay in the page cache the system already shares between processes, and only the pages that are read are resident.

Five lines of mixed Chinese, English, Japanese, Korean and emoji text laid out with system fonts (macOS, `FontSystem::with_system_fonts`, release):

| | Before | After |
| --- | ---: | ---: |
| Live heap afterwards | 1 282 MiB | 0.7 MiB |
| Peak memory footprint of the process | 1 349 MB | 3.7 MB |
| Time to lay the five lines out | 116 ms | 1–2 ms |

`nana-text/tests/system_font_memory.rs` (it reads the machine's fonts, so it is `#[ignore]`d and run by hand on the target platform) asks for the bytes of every installed face: 929 faces in 442 files, 2 194 MiB of font files, 0.04 MiB of heap. A file an application registers with `register_file` is still read at registration: a mapping needs the file not to be shortened in place while it is mapped, which the system font directories can be relied on for and an application's own file cannot. When mapping fails (a file system without it), the file is read instead.

**What a running window holds (2026-11-11).** macOS arm64, `release`, 8 seconds after the window appears, `footprint` and `heap` / `malloc_history` (`MallocStackLogging=lite`): the minimal hosted window `examples/reactive-sfc` has a process footprint of 30 MB, 21 MB of it heap; the first page of `component-gallery` is 36 MB with 26 MB of heap. The minimal window's heap by the nearest frame of each allocation's call stack:

| Source | Heap |
| --- | ---: |
| AppKit's own objects (the window, menus, allocations inside the event loop) | about 11 MiB |
| ColorSync colour conversion lookup tables (27 of them, 272 KiB each) | 7.3 MiB |
| naga's shader IR (the Metal backend writes MSL per pipeline, so modules are kept) | about 4.2 MiB |
| Metal shader and pipeline objects | about 1.9 MiB |
| The default application icon (the decoded 256 px image and the 4 lookup tables it brings in) | about 1.7 MiB |
| System font metadata (names and weights of 929 faces) | 0.6 MiB |
| Reserved queues (input, diagnostics, window requests) | 0.3 MiB |

The first two rows are the system frameworks', and any AppKit window has them; the rest holds no waste of the kind the system fonts had. One thing was changed in this round: the window service's request queue reserves 1 024 places of the largest request, and the two requests that create and control a window were over three hundred bytes each, 400 KiB in all; their contents are now boxed and the queue is about 50 KiB. The shader IR follows the total WGSL joined into each module (`color.wgsl` goes into nine modules). Since then each entry point is a module of its own holding only the functions it reaches (see [Startup · Shader IR cut per entry point](./startup.md#shader-ir-cut-per-entry-point)); this row has not been measured again on macOS.

## CPU time

The cost of incremental frames is governed by the performance contract in `perf/` (`perf/README.md`): an idle frame does no work, paint-only changes do not relayout, and one edit does not scan siblings. The criterion is `WorkCounters`, not wall-clock time.

Cold-start full-page layout has no gate, only observations (the same `wide` as above, release, interleaved 8 rounds with the pre-change binary, reading the minimum):

| Boxes | NanaUI | NanaUI before the change | Taffy |
| ---: | ---: | ---: | ---: |
| 2 101 | 1.78 µs / box | 2.01 µs / box | 0.67 µs / box |
| 10 501 | 2.22 µs / box | 2.56 µs / box | 0.71 µs / box |
| 21 001 | 2.64 µs / box | 2.93 µs / box | 0.73 µs / box |

NanaUI's timing includes the layout writeback commit and result publishing (Taffy has no equivalent step). At 10 501 boxes, one layout spends about 39% in placement, 26% in measurement, 18% in the two publishes, and the rest in writeback and wrap-up. Deep chain (`report`, also interleaved, reading the minimum): layout at 64 / 256 / 500 levels 0.16 / 0.90 / 2.49 ms, full frame 0.84 / 7.4 / 24.7 ms; after fixing the quadratic terms in extraction and the anchor memo (see "To do"), layout 0.14 / 0.52 / 1.06 ms, full frame 0.66 / 4.1 / 11.6 ms. `cargo run --release -- stages 125,250,500` lists the minimum time of each stage of a frame by `FrameStage`.

Mounting a deep chain (`Mount` in `stages`, one `mount_view_root`) was also quadratic: 0.74 / 2.4 / 8.7 ms at 125 / 250 / 500 levels, now 0.28 / 0.55 / 1.11 ms, ×2 per doubling. The cause was that every insert walked the parent chain three times: cycle check (`has_ancestor`, which also inserted into a SipHash set), depth computation (`ancestor_depth`), and subtree height (`subtree_height`), and each step cloned a node snapshot out of the world through `ValidationPlan::node` and staged it. After commit, it walked the parent chain twice more to clear layout results. Now:

- A node stores its own depth in `Hierarchy::depth`. Insert and detach update the moved subtree breadth-first, and validation reads it directly while the batch has not yet changed structure. Within a batch, settled depths are also memoized per node (`ValidationPlan::depths`): placing a node with no children only changes its own entry, and moving a node with a subtree clears it. In test builds, every read is checked against an actual walk.
- Read-only parent chain and subtree traversals use reads that neither stage nor clone (`parent_of`). A child with no children cannot form a cycle, so the cycle check is skipped for it; the cycle check's bound uses a step count instead of a hash set.
- When the world has no layout results yet, both clearing chains return immediately (`layout_results_suppressed` also moved from SipHash to `NodeSet`, hashed by id).

Mounting into a UI that has already been laid out (2026-11-06): when the tables are not empty, both clearing chains still run, and every parent that gains a child walks up to the nearest fixed-size boundary. Building a deep chain top-down, or inserting many siblings at once deep in the tree, costs "inserted nodes × depth". The `insert` subcommand (first lays out a 200-row wide page, then calls `mount_view` under a target leaf; `deep` is an N-level chain, `wide` is N siblings as roots, `under-deep` first mounts a 400-level chain and then mounts N siblings beneath it), interleaved 8 rounds with the pre-change build, reading the minimum, mount (the two commits for tree building and insertion):

| Shape | 100 | 200 | 400 |
| --- | ---: | ---: | ---: |
| `deep` | 343 → 276 µs | 748 → 502 µs | 2 034 → 969 µs |
| `wide` | 238 → 234 µs | 434 → 432 µs | 805 → 814 µs |
| `under-deep` | 773 → 228 µs | 1 507 → 426 µs | 3 012 → 806 µs |

`deep` used to grow ×2.2–2.7 per doubling, now ×1.8–1.9; `under-deep` used to cost about 7.5 µs per node (for the 400-level part), now about 2 µs, the same as `wide`. The approach: within one commit's apply loop, record the ancestors each chain has already walked (`ResultChainMemo`, `world.rs`), and a later chain stops when it reaches one of them. Correctness rests on three points:

- In the apply loop, results only leave and never enter (publishing happens after the last mutation), so what a chain did above a given ancestor (removing or hiding results) stays valid. A clearing chain may only stop at an ancestor recorded by a clearing chain; a hiding chain may stop at either kind (removed results have nothing to hide).
- Walking up from that ancestor follows the same series of parents. Mutations that change a parent (`Detach`, `ParkSubtree`, `DespawnSubtree`, moving a node that already has a parent, and creating or inserting a node that has been recorded) and a whole-list reorder (`ReorderChildren`) clear the records first. Meanwhile, a node on the chain becoming a fixed boundary can only make the chain stop earlier.
- For a record that stopped at a fixed boundary, before stopping the walk checks again whether it is still a boundary; if not, the record is not used and the walk continues upward as usual (style loosening between two inserts is such a case). The chain's starting node is not recorded: every newly created node starts a chain from itself, and recording it would clear all records when it is inserted.

In test builds and under `layout-verify`, every early stop walks the skipped segment for real and asserts that it would change nothing. `world/result_chain_memo.rs` compares the result tables with the records on and with full walks, within a batch (boundary loosening and tightening), between batches, on moves, on `Detach` / `ParkSubtree` / `DespawnSubtree`, on remount, and on inherited style changes, and then compares the layout of the next frame. The incremental edit and cold layout benchmarks and the `report` deep chain are all within noise.

The frame after mounting (2026-11-06): once mounting was linear, the frame after an `insert` mount was still superlinear. Split apart with `PRINT_STAGES=1` / `PRINT_COUNTERS=1` (`insert` now also prints the minimum time and counters per stage for this frame; `CHAIN=<depth>` changes the chain depth of `under-deep`, and `under-deep-at-once` mounts the same tree in one go for comparison), the counters (nodes remeasured, placed, published, extracted, accessibility, hit rebuilt) were all linear in "new nodes + depth", and the superlinear part was each node walking up its own ancestor chain:

- **Page origin**: `record_layout_origin` is an O(depth) sum (by design of relative geometry), and extraction (`extract_nodes_reusing`) and accessibility projection (`project_accessibility_delta`, `project_nodes`) read it several times for every node they touch, so a new N-level chain costs N² additions. In the `under-deep` 400 frame it alone took 38% of all samples.
- **Hit testing**: when rebuilding each entry in place, `live_child_seed` accumulates scroll translation along the ancestors (`inherited_shift`), and so does `replace`'s `child_shift` when patching; `refresh_bounds_of` measures the depth of every entry from scratch for sorting; building an entry, `used_pointer_events` searches upward for the nearest `pointer-events`.
- **Layout**: `AffectedIndex::new` walks the parent chain to the root for every affected node to see whether it is in some island, even when there are no islands.
- **Accessibility**: boxes resized in place (every level of the chain) ask "do I and every record above me intersect the offset?", and the translation root asks "are they all upright?", each walking the whole chain afresh.

Every fix makes the chain walked only once per traversal, with results bit-identical to walking the whole chain afresh:

1. While extraction and accessibility projection hold a `PageOriginScope` (`world/page_origins.rs`, which borrows the world, so boxes cannot be written meanwhile), a box deeper than `DIRECT_DEPTH` (32) walks up to the first reference frame whose sum is already known, adds its own segment on top, and records the sum for each reference frame along the way. The origin is added level by level from the top down, and a reference frame's sum is exactly the accumulated value on reaching it, so continuing from it gives the same bits (the `Document` node and the topmost local value are handled as before, not added starting from +0). In test builds and under `layout-verify`, every result is compared bitwise against a fresh walk.
2. Hit testing: `HitIndex::shifted` records whether any entry has been scroll-translated since the index was built. If not, every translation is +0 and so is the sum, and `inherited_shift` / `child_shift` do not walk. If so, in-place rebuilds of deep parents use `ShiftSkips` to skip entries that carry no translation and do not hand out local coordinates (adding ±0 does not change a sum that starts from +0), and record for each entry the first entry above it to stop at. `refresh_bounds_of`, after 32 steps, looks up known depths and records those along the way. During a range rebuild (no style changes), `pointer_events_memo` is on, and `used_pointer_events` for deep nodes records the answers along the way.
3. Layout: `AffectedIndex::new` does not walk when there are no islands; when there are, it stops at a node in `reach` (it and everything above are outside islands) or at a node known to be in an island.
4. Accessibility: the two chain questions are memoized per node in `ProjectionMemo` (`records_all`), and deep nodes walk up to the first known answer.

Each step interleaved 8 rounds with the previous one, reading the minimum (ROWS=200, release, the frame after mounting, µs):

| Shape | Before | 1 origin | 2 hit | 3 layout | 4 accessibility |
| --- | ---: | ---: | ---: | ---: | ---: |
| `deep` 100 | 1 058 | 958 | 965 | 955 | 933 |
| `deep` 200 | 1 940 | 1 536 | 1 528 | 1 492 | 1 466 |
| `deep` 400 | 4 388 | 2 759 | 2 739 | 2 646 | 2 574 |
| `wide` 400 | 1 582 | 1 629 | 1 584 | 1 584 | 1 607 |
| `under-deep` 100 | 8 066 | 5 532 | 4 043 | 3 808 | 3 533 |
| `under-deep` 200 | 9 526 | 6 131 | 4 583 | 4 321 | 4 003 |
| `under-deep` 400 | 12 468 | 7 426 | 5 972 | 5 450 | 5 133 |

`deep` per doubling goes from ×1.8–2.3 to ×1.6–1.8 (the intercept of about 0.4 ms is the 200-row page), and `under-deep` per extra sibling goes from about 15 µs to about 5 µs; mounting itself and `wide` are unchanged. Per stage (minimum of `RUNS=20`, before → after):

| Shape | Layout | Hit | Accessibility | Extraction |
| --- | ---: | ---: | ---: | ---: |
| `deep` 400 | 1 650 → 1 444 | 178 → 176 | 1 154 → 111 | 985 → 428 |
| `under-deep` 100 | 2 740 → 2 386 | 1 860 → 258 | 1 853 → 126 | 1 264 → 433 |
| `under-deep` 400 | 4 169 → 3 483 | 1 911 → 341 | 3 495 → 175 | 2 284 → 640 |

The cold deep-chain full frame (`stages`) also becomes linear: at 400 levels, accessibility 1 188 → 85 µs, extraction 760 → 209 µs. The amount of work is guarded by `world/deep_walk_tests.rs`: 64 leaves are mounted under a 400-level chain, and test builds count the upward steps of each kind via `page_origins::steps`, requiring no more than "touched nodes × 48" (each walk first goes at most 32 steps directly). Removing any one memo (origin, translation skips, `shifted`, depth, `pointer-events`, islands, chain answers, and the islands' "no islands, no walk" counts too) turns the corresponding test red. For temporarily exported mount scenarios (`deep` / `wide` / `under-deep`, with and without fixed boundaries, with and without scrolling, with or without opacity / clipping / `z-index` / transforms mixed into the chain), the per-frame layout boxes, results, Scene primitives and ops, hit grid, and accessibility tree and deltas are byte-identical to the pre-change export (also under `layout-verify`, where the second mount was removed). The frames that mount twice were nondeterministic at the time; see the "After mounting twice" paragraph below. Incremental edits (`edits 2000 height|width`, `CONTROLS=1 … text`, `SCROLLED=1`, `ROTATED=1`, `transform`, `OPEN_SELECT=1`) and cold layout (`wide`, `FRAME_PATH=1`), interleaved 8–12 rounds, are within noise.

What remains is linear constants, changed once more later (see the next paragraph): when mounting into a laid-out deep chain and mounting siblings beneath it, the boxes on the chain pick up their retained measurement plans and are not remeasured (in the `under-deep` 100 frame, measurement misses 1 502 → 203); when the newly mounted chain contains a word wider than its box, there is one extra layout pass (the same as the nana-text engine; see the next paragraph). The frame for `under-deep` 100 / 200 / 400 is 2.9 / 3.3 / 4.1 ms, and `deep` 1.24 / 2.03 / 3.67 ms. Taffy cold-lays out chains of 100 / 200 / 400 levels in 70 / 130 / 252 µs; NanaUI's layout in the frame that mounts `deep` is 785 / 1 376 / 2 530 µs (two passes; before the change, one pass with a wrong result, 491 / 828 / 1 568 µs). The gap is the same order of magnitude as cold layout (see above), and also linear.

After mounting twice (2026-11-06): mounting a column under a row 24 px wide with `min-height` 16 px, then mounting siblings in the column, made `layout-verify` report that retained layout differed from full layout (column retained 24, full 32.5; likewise for the 108 nodes of `under-deep`). When the chain's padding was a non-round value like 0.37, the same `under-deep` binary also produced different widths on each run (296.74 or 329.20). The root cause was in host shaper text, not in the retained tables:

- The width text asks for (`text_inline_size`) depends on how wide it last wrapped its lines (`text_wrap_limit`): if it wrapped at no wider than this box, it computes the line width; otherwise it asks only for `min(natural width, limit)`. The nana-text engine's layout reports line overflow (`CLIPPED_WIDTH`) and records the widest unbreakable line as negative infinity (65ba2f497); the host shaper does not report overflow, so this path still recorded the box width. `FixedShaper` reports 32 wide in a 23.5-wide box: in the mounting frame, the text is first shaped with no box (limit +∞), and layout gives the column 24; after layout it is reshaped at 23.5, metrics unchanged and no seed, so the limit becomes 23.5. From then on it asks for 32, but this frame has settled at 24. On the next sibling mount, the pass that remeasures this text computes 32.5, while the pass that picks up the retained measurement still gives 24. `layout-verify`'s full layout always remeasures, so it reports a difference.
- With chain padding of 0.37, the padding summed over 400 levels exceeds 24, and the deepest content box is left with only 0.002 px from rounding: the text reshapes at 0.002 wide, limit 0.002, asking for 32; once the chain grows to 328, it reshapes again at 32 wide, limit 32, and asks for only 0.002 again. Which side each frame settles on, and which boxes the next frame remeasures versus pick up retained measurements, follows the iteration order of hash sets in that frame's passes (`affected` is a `HashSet` hashed with the per-process random seed), so the same binary gives different results on each run. At 0.25 the content box is exactly 0 and always settles on the same side (equally wrong, differing from full layout).

The fix (`world/text.rs`): if the widest line the host shaper measures is wider than the box it wrapped in (by more than 0.5 px), the text is judged unbreakable, recorded as negative infinity, and seeded as the engine does. Text already judged unbreakable last time is measured by the host once more at 0.5 px narrower than its widest line, and stays unbreakable only if it is still as wide (matching the engine's second layout; only unbreakable text gets this second measurement). The width text asks for then changes only with the seed, picking up and remeasuring give the same answer, and iteration order can no longer decide the result. Introduced in 7e0651a72 (#254, where `text_inline_size` started reading `wrap_limit`; the same reproduction does not fail on its parent a293fef57); 65ba2f497 only fixed the engine path. Manual bisection used trees exported with `git archive`, each with its own `CARGO_TARGET_DIR`.

Once fixed, these frames picked up their retained measurement plans, which exposed another quadratic term: a container's plan recheck, sequential patch, flex line patch, and participant check (`measure_plan_children_unchanged`, `sequential_measure_delta`, `flex_line_measure_delta`, `flow_membership_holds`) each walked the whole pass's `affected` to pick out its own children, so with every level of a chain picking up its plan the cost was "levels × affected nodes". Now each pass builds a table of affected children per parent once (`AffectedChildren`, each list sorted by id), and a container walks only its own. Test builds count `Walk::PlanChildren`, and `deep_walk_tests::retained_plans_down_a_deep_chain_recheck_their_own_children` requires no more than "touched nodes × 48" (80 618 steps under the mutation where every container walks all affected nodes, against a limit of 22 272).

Tests are in `world/mount_under_narrow_row.rs`: the column is 32.5 in the mounting frame; when siblings are mounted later, the `layout-verify` guard agrees with full layout; host text that can wrap is no longer judged unbreakable after the row widens (column 72.5 / 50 / 32.5 for row 200 / 50 / 24); under a 40-level chain with 0.37 padding, 8 siblings mounted in different batches give identical boxes across nine runs, and the chain width is padding plus word width. Reverting the fix (the judgment, the overflow judgment, the second measurement), or making the second measurement always keep the judgment, turns the corresponding test red. Temporarily exported `under-deep` (0.25 / 0.37 padding, with transforms, clipping, `z-index`, fixed boundaries, and scrolling mixed into the chain, 20 groups, each 6 processes × 3 runs): before the change, each of the 12 groups at 0.37 produced two different layout results; after it, each group produces one, and the Scene `Debug` output and every node's `LayoutResult` are byte-identical across the 6 processes.

Interleaved 8 rounds with the pre-change build, reading the minimum (release): incremental edits and cold layout are within noise (−6% to +3%, mostly slightly faster). The frame after an `insert` mount: `under-deep` 100 / 200 / 400 3 541 / 4 109 / 5 167 → 2 884 / 3 311 / 4 149 µs, `wide` unchanged, `deep` 954 / 1 468 / 2 576 → 1 240 / 2 028 / 3 667 µs: words wider than their box are now accounted for, so the mounting frame lays out one extra pass (the pre-change result for that frame was wrong; the product path uses the nana-text engine, which already lays out the same tree twice).

Ratios are more trustworthy than absolute values, and comparisons should be interleaved over several rounds, reading the minimum. For profiling, use `cargo run --release -- wide-loop` with `sample` (the wide tree has shallow stacks, so attribution is trustworthy).

Further work (2026-11-02): the input table and style memo table of `LayoutInputMap` moved from hash tables to `DenseNodeMap` indexed by node (the same layout as `NodeTable`: ids within 65k index directly). All three wide-tree sizes are 7–8% faster, interleaved three rounds with the pre-change build (2 101 boxes 1.79 → 1.66 µs, 21 001 boxes 2.74 → 2.55 µs). The `holds_line_box_in` memo table no longer allocates first on a hit. The table a container plan uses to find child measured sizes by id is built only when there are more than 16 children. Layout allocations per box 25.3 → 24.3.

After that (2026-11-02): the four empty slices in `LayoutResult::from_box` use `Arc::default()` (previously `Arc::from([])` allocated four headers per result). The eight temporary column arrays in `distribute_flex_main` go into a thread-local reusable `FlexColumns`, and a nested solve that finds the slot empty builds its own. When the frame driver commits the layout writeback it sets `pass_publishes_written_boxes`, so the commit no longer publishes `CompatibilityWrite` results for the written boxes; the batch `RuntimeLayout` publish that follows builds them once (previously the second publish only judged each result from the first as unchanged). Wide tree, interleaved three rounds with the pre-change build, reading the minimum: 2 101 / 10 501 / 21 001 boxes 1.91 / 2.51 / 2.69 → 1.59 / 2.19 / 2.45 µs, 9–17% faster; layout allocations per box 24.4 → 16.2. Allocation count dropped by a third, while time dropped by just over a tenth: small allocations on macOS are already cheap, and what remains is computation. `wide` also lists "tree building + layout": NanaUI about 4.1–4.9 µs / box, Taffy 0.7–0.85 µs. Taffy spends almost nothing on tree building; NanaUI's tree building (creating components, queuing mounts, committing) is about 2.4 µs / box.

Scale curve (`wide 10,25,…,3200`, 10 cells per row): NanaUI is about 1.5 µs / box from 211 to 1 051 boxes, then rises steadily to 3.16 µs at 67 201 boxes; Taffy 0.69 → 0.83 µs. At equal box counts the shape does not matter (3200×10, 320×100, and 32×1000 are all about 2.7 µs), and every stage (measurement, placement, wrap-up, publish) slows by the same ratio: the rise comes from the per-node memory touched by each pass exceeding the cache, not from some superlinear algorithm. Per-node data read by each pass: `NodeRecord` 520 B, the `LayoutInput` rebuilt every pass 264 B, `LayoutResult` 280 B plus slices, plus each pass's caches, about 1.2–1.5 KB. Taffy keeps a node's style, cache, and output in one `SlotMap` slot, with the cache as 9 fixed slots inside the node. So `DenseNodeMap` was given an interface matching `HashMap<StableNodeId, V>` (stores keys, iterates in id order), and the eight tables in the layout pass and retained caches that have one entry per node (boxes, placements, padding, remote origins, contexts, intrinsics, measurement plans, latest intrinsic keys) switched to it. `ContainerPlan` is 264 B per entry and exists only for containers, so it stays a hash table. This step was only 0–5% faster over three interleaved rounds: hashing itself is not the main cost; what must shrink is the bytes read per pass.

Instructions and cycles (`/usr/bin/time -l … reps <rows> <rounds>`, then `SKIP_LAYOUT=1` to build the tree only; the difference is layout itself; per box):

| | Tree building instructions | Layout instructions | Layout cycles | Layout IPC |
| --- | ---: | ---: | ---: | ---: |
| 1 051 boxes | 48 249 | 29 143 | 6 558 | 4.44 |
| 67 201 boxes | 48 109 | 30 420 | 11 629 | 2.62 |

Taffy's whole process (tree building, layout, and freeing) takes about 17k instructions per box; NanaUI about 77k. Layout's instruction count does not change with scale, but cycles rise 77% and IPC falls from 4.4 to 2.6: on large trees layout is memory-bound. When everything fits in cache, NanaUI still runs more than twice Taffy's instructions per box. Tree building takes about 48k per box, more than layout itself.

First step toward a layout arena (2026-11-02): the `LayoutInput` read per node in each pass (264 B, with a modal's three text metrics inlined in every box) is replaced inside the pass by a private `PassInput` (modal part boxed, about one cache line); the public `LayoutInput` is unchanged. Large trees are 3–5% faster over three interleaved rounds. Also tried by turning off one at a time, each worth only 3–10%: the in-pass intrinsic LRU (`IntrinsicCache`; turning it off is 6–12% faster but misses reads and computes extra, so it can only be removed by replacing it with fixed slots in the node) and the full-page snapshot (takes no time, 88 B per box). Second step: in-pass intrinsic facts no longer go into the public LRU `IntrinsicCache` (hashed by key; every insert and hit updates the whole table's recency chain), but into `PassIntrinsicFacts`: facts are kept in one array in measurement order, and each node records its latest entry in an id-indexed table, chained back to older ones. The counters are the same as the cache's (requests, hits, misses, cross-context, baseline queries, entries and bytes), and nothing is evicted within a pass (the old budget of four variants per node was never reached). The public `IntrinsicCache` and its contract tests are unchanged. 3–6% faster over three interleaved rounds. Third step: `seeded_intrinsic` (a hash set) and `new_metrics` (a hash table) merge into this fact table. Each fact also records "visible to lookup" and "the latest value handed to the retained cache"; plain leaves record only the latter, as before when they bypassed the cache. The order handed to the retained cache changes from hash order to measurement order. Large trees are 8% faster over three interleaved rounds (21 001 boxes 1.91 → 1.75 µs, 67 201 boxes 2.26 → 2.08 µs).

**All the numbers above are for the full path (`layout_document`), which product frames do not take.** The frame driver's first frame is `layout_document_for_viewport`: when the document has no viewport yet, it seeds the root and runs one scoped pass covering the whole tree. `wide` with `FRAME_PATH=1` measures this path: 2 101 / 21 001 / 67 201 boxes 2.72 / 3.68 / 4.75 µs, 2.2× the full path, with 9.5 more allocations and about 700 B more per box. The first frame has no retained cache to reuse, so frontier expansion and materializing inputs one by one are pure overhead.

Step 0 (2026-11-02): when the document has no retained cache at all (`viewport` is empty and `boxes` is empty), the engine computes internally as a full pass, and the framework's call counters are unchanged; the full-page snapshot is recorded and replayed only when the caller asks for a full pass. The full pass previously did not build measurement plans (they would afterwards be either thrown away or cleared); here `records_measure_plans` makes the first frame build them as usual, since otherwise later incremental edits would grow more expensive with document size (issue259's cost contract would fail). An empty box with no children also records a plan in the scoped pass, so the first frame skips the leaf shortcut to match. Product first frame, three interleaved rounds: 2.72 / 3.68 / 4.75 → 1.52 / 2.06 / 2.45 µs, 1.8–1.9× faster; without recording plans it could reach 1.33 / 1.76 / 2.14, and that 15% difference comes from the plan recorded for empty leaves. Later evaluated (2026-11-03): a plan with no children to replay answers the same question as the memo table, so plain leaves (`plain_leaf`) no longer record one in either the first frame or scoped passes, and a container that becomes a leaf clears its old plan. On the control page, layout instructions drop 7.5% and time is 5–6% faster; the plain box page is unchanged.

Step 1 (stop storing copies of child geometry) was measured before deciding: not building each child's placement and fragment in the parent result at publish time leaves product first-frame time unchanged (0–2%, within noise), saving 177 B of memory and 1 allocation per box. Both are public fields of `LayoutResult`, so no breaking change is made for this gain; it is left until `LayoutResult` changes for another reason.

Step 2: `wide` with `CONTROLS=1` (each cell has a label and a button; Taffy skips this row). Profiling the first frame on the control page, SipHash takes 5.2% of layout: in the first frame, `scoped_scroll_retention_targets` inserts every node of the whole tree into a std `HashSet`, and `consulted` at the end of the engine pass is also a std `HashSet`. Switching to `NodeSet` hashed by id and hashbrown makes both pages 4–6% faster (control page 3 101 / 31 001 boxes 1.67 / 2.41 → 1.58 / 2.30 µs). A component presence mask (one bit for appearance on `NodeRecord`, checked before the table lookup) gave no measured gain: the control page was unchanged and the plain box page was 3% slower (when the appearance table is empty it already returned on an emptiness check of the whole table). Looking up a missing key in hashbrown usually reads only one group of control bytes, cheaper than expected, so it was reverted.

Step 3 (write output only once) first re-measured the product first frame by stage (67 201 boxes, ns / box): placement 840, measurement 525, publish 381 (build 228, store 128), end-of-pass merge 350, writeback comparison 61, commit 74, prefetch 60. The writeback commit is only 6%: giving the frame driver a separate commit with only `WriteLayout`, skipping the general commit's dozen or so scans of the whole batch, measured no change (the scans are only per-item enum matching), so it was reverted. Done in the end-of-pass merge: `remote_overlays` moved from std `HashSet` to `NodeSet`; when the retained tables are empty (first frame), they take over the pass's padding, remote origin, context, and placement tables whole (`DenseNodeMap::absorb`) instead of inserting entry by entry; `DenseNodeMap::reserve` actually reserves, and the in-pass fact table and latest intrinsic key table reserve by node count. Both pages are 1–3% faster.

Next came measurement and placement themselves (together 57% of cold layout). The counters (`last_work_counters`) showed nodes measured more than once: the plain box page measures 3 101 times for 2 101 boxes, the control page 6 101 times for 3 101 boxes. Temporarily printing every node and constraint actually computed showed that cells were remeasured: the measurement pass measures them with the 1280×800 the row received, the placement pass measures again with the row's actual content box 281×16.5, and both results are the same. Taffy remeasures too in the same scenario (when a container is laid out at a known size, it rebases its children on the new available width); the difference is the cost of a single measurement. So the in-pass used-size memo moved from a table hashed by the whole `MeasurementKey` to `used_order` plus a latest index and older chain per node: a lookup compares only this node's few keys. Both pages are 4–7% faster (plain box page 67 201 boxes 2.35 → 2.21 µs, control page 3 101 boxes 1.61 → 1.53 µs). Also: the one-shot `presence_live` no longer creates, fills, and discards an ancestor memo table; latest intrinsic keys store fact indexes instead (56 → 16 B per slot). Neither showed a measurable time difference; they are kept as simplifications. Note that release builds merge functions with identical machine code (`deduplicated_symbol`), so function names in samples can be misattributed; judgments must rely on counters and turn-off-and-remeasure.

`LayoutInputMap::style` is called about 20 times per box (42 501 calls / 2 101 boxes on the plain box page), each cloning and then releasing an `Arc`; on this machine an uncontended clone plus release costs about 5 ns more than a borrow. A borrowing `style_ref` was added (returning `StyleRef`: borrowed when the pass has the input, an owned handle only for styles memoized on demand), and 44 of the 70 call sites that compiled with it were switched (tried one by one; the remaining callers need to keep the handle or later take a mutable borrow of the map). Both pages are 2–6% faster (plain box page 67 201 boxes 2.40 → 2.26 µs). Later steps: placement writes the retained tables directly, saving the end-of-pass merge; publishing builds results only for nodes whose geometry changed; the tree-building path gets its own profile.

Cold layout throughput and per-box memory (2026-11-05): measure first, then change. `wide` (release, `FRAME_PATH=1`, 21 001 boxes) instructions per box: tree building 48.5k, layout 31.0k, total 79.5k; Taffy's whole process (tree building, layout, and freeing) 17.4k. Time is summed from `sample`'s call tree, trusting only ratios and turn-off-and-remeasure: release builds merge functions with identical machine code, so `mach_absolute_time` in the profile is actually in `free`, and `Arc<LayoutStyle>::grow_one` actually belongs to `Vec<StableNodeId>`.

- **Tree building** (creating components, queuing mounts, two commits) is more than half fixed per-commit overhead. `malloc` / `free` take about 30% of tree building (48.7 allocations per box). The largest sites: `subtree_ids` / `mark_subtree` / `unlink_from_parent` call `UiWorld::node` to get children, each time cloning `NodeKind`'s `String` and the child `Vec`; when a new node is parked, `retire_subtree_from_document` builds a SipHash set for an empty pointer table; `ValidationPlan` stages tables for batches like "create + write style + park"; and the `MutationQueue` for creating components grows. Next are `Arc::make_mut` in the Stack constructor copying the 2.6 KB `LayoutStyle`, which `LayoutInterner` then compares and discards (about 14%), and `restyle` comparing the whole style for new nodes (about 15%).
- In **layout**, allocation takes about 18%; the rest is spread across measurement (about 37%), placement (about 35%, including remeasuring cells against the new content box), and publish (about 16%), with no single hotspot.
- **Resident memory** is attributed block by block by allocation stack: `HOLD=1` keeps the page alive, with `MallocStackLogging=1` plus `malloc_history <pid> -allBySize` (this measures residency, not allocation traffic). At 21 001 boxes, per box: the node table `Vec<Option<NodeRecord>>` after doubling growth 887 B (the record itself 568 B), published `Arc<LayoutResult>` 320 B, measurement plans 457 B (two per cell, one per empty leaf, each `Box<MeasurePlan>` 320 B), the retained intrinsic table 240 B, the container plan hash table 207 B (`ContainerPlan` 256 B inline in the buckets), `text_nodes` 190 B, component views 224 B (one `Box<Stack>` per node).

The plan, ordered by gain and risk: first cut tree building's fixed overhead (purely internal, bit-identical), then shrink the node table and measurement plans (storage changes only), then the lookups and allocations in layout that can be removed in place. The shape of `LayoutResult` (each child's fragments and parts in the parent result) and dense hot arrays for the layout pass are structural changes, so they come last, after measuring the earlier steps. Each step is interleaved 8 rounds with the previous step's binary, swapping order every round and reading the minimum (`FRAME_PATH=1`, plain box page 2 101 / 21 001 / 67 201 boxes, control page 3 101 / 31 001 boxes); instructions are `instructions retired` from `reps 1000 5` divided by 105 005 boxes:

1. **Tree building path**: the sites listed above switch to borrowing, reserving, and building tables on demand; the set in `commit_with_mount_lifecycle` and the inner set of `nodes_by_component` become `NodeSet`; `StickyDependencies` does not hash when there are no sticky boxes. Tree building allocations per box 48.7 → 29.7, tree building instructions 48.5k → 36.8k; tree building + layout 3.62 / 4.11 / 4.56 → 3.05 / 3.53 / 3.96 µs / box; layout and residency unchanged.
2. **Boxing node records (reverted, see step 7)**: `NodeTable` slots store `Box<NodeRecord>`, so growth moves only pointers and each extra slot is 8 B. Plain box page residency 4 025 / 3 588 / 3 835 → 3 501 / 3 282 / 3 296 B / box, and instructions per edit drop slightly too (`edits 2000 height` 22.3M → 21.7M).
3. **Per-box layout memory**: boxes with no children, text, visual, or natural size no longer record a measurement plan (what they measure is only their own style, and the memo table answers by the same key; previously only `plain_leaf` skipped it); `GridTrackPlan` is boxed (`ContainerPlan` 256 → 184 B); within one publish, a result's `parts` / `child_placements` / `fragments` share one `Arc` with any of the last 4 equal slices (results are in their own coordinates, so leaves of the same size and label parts with the same text are identical). Plain box page residency 3 501 / 3 282 / 3 296 → 3 214 / 3 009 / 2 977 B, layout allocations 19.1 → 17.8, layout 1.40 / 1.93 / 2.20 → 1.34 / 1.82 / 2.10 µs; control page 3 624 / 3 729 → 3 361 / 3 475 B; layout instructions 30.8k → 29.5k.
4. **Lookups and allocations in layout**: `holds_line_box_in` checks the children first, and when the children are all known or all leaves it settles without building a work stack (measurement is bottom-up, so almost every box settles at this step); when measurement records an intrinsic fact, `insert`, `record`, and `note_latest` merge into one lookup (`PassIntrinsicFacts::insert_measured`). Layout allocations 17.8 → 16.8, layout 1.35 / 1.80 / 2.09 → 1.30 / 1.76 / 2.06 µs, layout instructions 29.5k → 28.6k.
5. **Validation fast path**: two kinds of batches no longer build a `ValidationPlan`: "create one node, write only its style / interaction / accessibility / component type, optionally park it" and "only `WriteLayout` / `PlaceLayout`". The checks are the same as the original path (exists, not retired, value ranges of style and box; it falls back to the original path when the world has inline content or this id has overlay references). Tree building instructions 36.6k → 30.2k; tree building + layout 3.10 / 3.65 / 3.97 → 2.81 / 3.30 / 3.63 µs.
6. **Shared accessibility state**: `NodeRecord::accessibility` stores `Arc<AccessibilityState>`, with one default value for the whole world; the record shrinks 568 → 432 B (records were still boxed then, so the malloc block went 640 → 512 B). Plain box page residency 3 214 / 3 009 / 2 977 → 3 078 / 2 873 / 2 840 B; labels and buttons each have their own state, so the control page saves only 33 B.
7. **Reverting the boxing**: after step 6, four binaries were run in rotation on a quiet machine: before the change, boxed, stored inline in blocks of 64 (block table resident in cache, record loaded once), and unboxed (12 rounds for transforms, 8 for row height). Full frame for changing the root column's transform: 16.8 / 18.1 / 17.0 / 16.1 µs; changing row height: 1 105 / 1 086 / 1 150 / 1 084 µs. Boxing adds one dependent load to every record lookup, and the records touched in incremental edits are mostly cold, so transform edits are 8% slower; block lookup adds a dozen or so instructions each time, adding 1.3M instructions per row-height change (+6%). Incremental edits may not regress, so records went back to an inline `Vec<Option<NodeRecord>>`. After step 6 shrank the record to 432 B, the inline table itself is also smaller (the plain box page at 21 001 boxes is still about 0.5 KB / box below the pre-change figure), but the empty slots from doubling growth are back: 230–400 B / box more than boxed.

Combined (interleaved 8 rounds with 9059c271c, reading the minimum):

| Page | Boxes | Layout µs / box | Tree building + layout µs / box | Resident / box | Layout allocations / box | Taffy layout / tree building + layout / resident |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain boxes, product first frame | 2 101 | 1.34 → 1.26 | 3.70 → 2.76 | 4 025 → 3 472 B | 19.2 → 16.9 | 0.68 / 0.71 µs / 944 B |
| Plain boxes, product first frame | 21 001 | 1.84 → 1.66 | 4.29 → 3.24 | 3 588 → 3 102 B | 19.1 → 16.8 | 0.74 / 0.80 µs / 1 506 B |
| Plain boxes, product first frame | 67 201 | 2.12 → 1.95 | 4.61 → 3.60 | 3 835 → 3 246 B | 19.1 → 16.8 | 0.75 / 0.81 µs / 944 B |
| Plain boxes, full `layout_document` | 21 001 | 1.62 → 1.50 | 4.06 → 3.07 | 3 154 → 2 866 B | 15.6 → 14.3 | — |
| Control page, product first frame | 3 101 | 1.35 → 1.31 | 4.05 → 3.39 | 3 796 → 3 456 B | 15.3 → 14.1 | — |
| Control page, product first frame | 31 001 | 2.10 → 1.99 | 4.99 → 4.22 | 3 753 → 3 459 B | 15.2 → 14.0 | — |

Instructions per box (`reps 1000 5`, product first frame): tree building 48.5k → 30.4k, layout 31.0k → 28.5k, total 79.5k → 58.9k (Taffy 17.4k); full path total 75.8k → 56.4k; control page total 83.4k → 69.6k. Gap to Taffy at 21 001 boxes: layout 2.5 → 2.2×, tree building plus layout 5.4 → 4.1×, residency 2.4 → 2.1×. Incremental edits (`edits 2000`, interleaved 8 rounds with 9059c271c, reading the full-frame minimum, µs): height 1 109 → 1 079, `SCROLLED=1` height 1 243 → 1 223, `ROTATED=1` height / transform 1 209 → 1 216 / 17.6 → 16.8, transform 15.8 → 15.4, `OPEN_SELECT=1` height 3 733 → 3 517, control page text 21.3 → 21.2 (10 rounds rotated separately); width 119 → 122, with instructions per edit 2.49M → 2.52M (+0.9%; several intermediate commits moved back and forth between 2.40M and 2.48M), and across several rotations width fluctuated ±5% in both directions. Instructions per row-height change 21.8M → 21.9M.

Tried and not kept: writeback looks up the retained cache four times per box by document (padding, remote origin, context, reference frame); merging these into one lookup left instructions unchanged (the document tables are hashed by id, so lookups were already cheap), so it was reverted. Evaluated but not done: a Stack preset cache (sharing one base for `row` / `column`) only saves the case where the constructor is not chained afterward; the first `width` and similar on a chain still copy, so on the plain box page only rows would benefit. Building `text_nodes` on demand: `invalidate_text` does nothing when there is no entry, so building on demand would lose invalidations and revision numbers from before the build; all write sites must first become "get or build". Removing each child's fragments and `ChildPlacement` parts from the parent result: fragments are #197's public contract (Scene and `view!` tests read them by kind), and `child_placements` is how publishing decides whether a parent result still holds; that decision must be replaced first (replaced and removed on 2026-11-11, see "Children's boxes are no longer copied into the parent's result").

Second round of cold layout (2026-11-06): re-measuring b8378f5d2, the 21 001-box wide tree (product first frame) takes 30.3k instructions per box for tree building and 29.0k for layout; Taffy's total is 17.5k. In tree building's `sample` call tree (trusting only ratios and turn-off-and-remeasure), `restyle` takes 17% (of which the whole comparison of two layouts in `changed_fields` is 4%, `node_presentation_eq` computed three times per restyle 2%, and the rest a dozen or so field checks and four `NodeStyle` clones); `Arc::make_mut` copying in the Stack constructor and the interner's whole comparison together take 13% (on the same page both repeat on the same pair of layouts); `NodeRecord::new` and `NodeStore::insert` take about 5% each (every node builds an `AccessibilityState::default` and then compares it with the default, and allocates a `String` and an `Arc<NodeKind>` for the "stack" tag); parking and mounting a leaf allocates a subtree `Vec` and a work stack for one node. Layout has no single hotspot (publish 6%, measurement 5%, every other item under 3%). Each step is interleaved 8 rounds with the previous one, swapping order every round and reading the minimum (`FRAME_PATH=1`, plain box page 2 101 / 21 001 boxes, control page 3 101 / 31 001 boxes):

1. **Memoize restyle comparisons by Arc pair** (4d5f6ff1e): every new node restyles from the shared default layout to an already-interned layout. The dozen or so checks merge into `LayoutPairFacts`, computed once, and `UiWorld::layout_pairs` remembers the last 8 pairs by pointer pair (holding both `Arc`s, so equal pointers mean equal values); the equality check in `write_node_style` consults it too; `node_presentation_eq` is computed once. Tree building instructions 30.4k → 28.2k; tree building + layout 2.65 / 2.98 → 2.50 / 2.89 µs / box, control page 3.21 / 3.96 → 3.17 / 3.85.
2. **Fixed per-node cost of tree building** (3f869824b): `NodeKind` is shared by tag (`NodeStore::intern_kind`); records take the shared default accessibility state directly and build the default style only once; parking, mounting, marking dirty, changing depth, and leaving the document no longer allocate for a single leaf node. Tree building allocations 20.7 → 12.7 per box, instructions 28.2k → 24.9k, residency −45 B; tree building + layout 2.45 / 2.81 → 2.31 / 2.68, control page 3.06 / 3.80 → 2.90 / 3.67.
3. **Stack constructor chains stop copying the layout** (050ce4935): `layout_edits::edit_layout` memoizes results by "the starting `Arc`'s pointer + the bit-exact edit"; the second time the same edit is seen on the same start it is recorded, and afterwards that `Arc` is reused directly. Each of the five presets is one edit. When the interner replaces a result, the interned one is recorded. Chains that appear only once still copy once, and `with_layout` / `layout_mut` and `calc()` still copy (so a cell's `with_layout(padded)` still copies). Tree building instructions 24.8k → 23.1k (control page −3.8%); tree building + layout 2.41 / 2.79 → 2.35 / 2.70, control page 3.03 / 3.75 → 2.96 / 3.63.
4. **Slimmer measurement plans** (d0036cf30): a container's own text, struts, and visual go into `Option<Box<PlanOwnContent>>`, which is not allocated when bitwise empty; checks against empty still compare item by item with `==`. Residency −56 B (control page −30 B, since plans with text take one more allocation); time within noise (12 rounds: cold layout −2% to +1.7%, incremental edits −1.5% to +2.1%).
5. **Records share `NodeStyle` and are cache-line aligned** (c297d6c69): `NodeRecord::style` stores `Arc<NodeStyle>` (`NodeStyleInterner` searches the last 8 for one with the same layout pointer and equal as a whole), shrinking the record 432 → 248 B, padded to 256 B with `#[repr(align(64))]`. Shrinking without aligning made layout for a row-height change on the page with an open Select 5.6% slower (12 rounds; where records straddle cache lines changed); after aligning, everything got faster: residency −274 / −343 B (21 001 / 2 101 boxes), control page −186 / −232 B; layout 1.51 → 1.41 µs / box (21 001), control page 1.82 → 1.70; incremental edit full frame height 1 082 → 1 039, `SCROLLED=1` 1 215 → 1 131, `ROTATED=1` 1 194 → 1 127, `OPEN_SELECT=1` 3 067 → 2 841, transform 15.3 → 13.8 µs, the rest within noise.

Tried and reverted: **growing the node table by a quarter** (instead of doubling, at least 64 slots): residency at 21 001 / 67 201 boxes drops a further 130 / 242 B, but 12 rounds of re-measurement showed row-height changes with `OPEN_SELECT=1` 3–4% slower and `insert` mounts 5–15% slower: after aligning to 64 B, the system `realloc` does not guarantee alignment, so Rust takes `realloc_fallback` (a new allocation plus copying the whole block), and more growth steps mean more copying. **Memoizing Stack chains step by step instead of by preset**: a `column(0.0)` chain does six table lookups, and cells saved no copies but spent more; it was only kept after becoming one edit per preset.

Combined (interleaved 8 rounds with b8378f5d2, reading the minimum):

| Page | Boxes | Layout µs / box | Tree building + layout µs / box | Resident / box | Taffy layout / tree building + layout / resident |
| --- | ---: | ---: | ---: | ---: | ---: |
| Plain boxes, product first frame | 2 101 | 1.21 → 1.19 | 2.62 → 2.33 | 3 472 → 3 025 B | 0.70 / 0.71 µs / 944 B |
| Plain boxes, product first frame | 21 001 | 1.52 → 1.41 | 3.02 → 2.60 | 3 102 → 2 727 B | 0.72 / 0.78 µs / 1 506 B |
| Plain boxes, product first frame | 67 201 | 1.85 → 1.71 | 3.41 → 2.95 | 3 246 → 2 798 B | 0.75 / 0.81 µs / 944 B |
| Plain boxes, full `layout_document` | 21 001 | 1.37 → 1.29 | 2.88 → 2.48 | 2 866 → 2 546 B | — |
| Control page, product first frame | 3 101 | 1.25 → 1.26 | 3.25 → 2.98 | 3 456 → 3 161 B | — |
| Control page, product first frame | 31 001 | 1.81 → 1.70 | 3.95 → 3.50 | 3 459 → 3 211 B | — |

Instructions per box (`reps 1000 5`, product first frame): tree building 30.3k → 22.9k, layout 29.0k → 28.7k, total 59.3k → 51.6k (Taffy 17.5k); control page total 69.5k → 63.6k. Gap to Taffy (21 001 boxes): layout 2.1 → 2.0×, tree building plus layout 3.9 → 3.3×, residency 2.1 → 1.8×, instructions 3.4 → 2.9×. Incremental edits (`edits 2000`, full-frame minimum, µs): height 1 093 → 1 063, width 117.3 → 116.4, transform 15.5 → 13.7, scale 16.0 → 14.0, `CONTROLS=1` text 21.3 → 21.2, `SCROLLED=1` 1 209 → 1 180, `ROTATED=1` height / transform 1 197 / 15.4 → 1 103 / 14.1, `OPEN_SELECT=1` 3 230 → 2 906; `insert` mounting 100 / 200 / 400 nodes: `deep` 281 / 504 / 939 → 258 / 453 / 867 µs, `wide` 220 / 418 / 804 → 212 / 378 / 702, `under-deep` 219 / 428 / 778 → 206 / 372 / 706; the frame after mounting −2.4% to +4% (only `wide` 100 is positive, within noise). For 13 temporarily exported page-and-edit groups (plain boxes height / width / transform / scale, control page text, MIXED, OPEN_SELECT, SCROLLED, ROTATED height / transform, `insert` deep / wide / under-deep / under-deep-at-once), the per-frame layout boxes, `LayoutResult`, Scene, hit grid, and accessibility tree on all five commits are byte-identical to b8378f5d2.

The remaining gap and its causes: of tree building's 22.9k instructions per box, the fixed commit overhead (focus, lifecycle, and mount sets in the two `commit_mutations` calls, `MutationQueue`, the `ValidationPlan` fast path) and `restyle` are still the bulk; a cell's `with_layout(|l| *l = padded())` closure cannot be recognized by value, so it still copies once and the interner then compares; component views store `NodeStyle` by value (224 B per `Box<Stack>`). Layout's 28.7k instructions per box are spread across measurement, placement, and publish (no single item above 6%); shrinking it substantially further requires packing the per-node hot data read by the layout pass into dense arrays indexed by node; see "To do".

Third round of cold layout: dense layout hot data (2026-11-06). Measure first: at 21 001 boxes (product first frame) layout takes 28.6k instructions and 6.94k cycles per box, with IPC 5.2 at 2 101 boxes and 4.1 at 21 001 boxes. The large tree spends about 20% more cycles than the small one; that is the ceiling memory work can recover, and the rest is instruction count. In layout's `sample` call tree, malloc / free take about 20% (a dozen or so temporary `Vec`s per container), measurement's style checks (`sizes_own_axes`, `viewport_axes`, normalizing constraint categories) about 6%, inserts into id-hashed pass outputs and the line box memo about 4%, and building `PassInput` (cloning and releasing two `Arc`s per node) about 3%.

Fields each pass reads and writes per node (by code reading, plain box page and control page):

| Level | Fields | Where they live now |
| --- | --- | --- |
| Hot (every node, every pass) | Child list, layout style, writing direction and the containing block's writing direction, text metrics | Node input view `LayoutInputView` (48 B); children and plain boxes' styles borrow the `Arc` in the record |
| Hot | Style check bits (own size, whether a leaf reads the block axis, plain leaf with no content, viewport axes, aspect ratio, full percentage) | `StyleFacts` (u16) in the same view, memoized by style pointer |
| Hot | Boxes written by placement | Dense table in the full-page pass's `PassOutput`; on the first layout it becomes the retained box table directly |
| Hot | Padding, contexts, remote origins, measurement plan slots, used-size index | Each its own `DenseNodeMap`, reserved once by the document's highest id |
| Warm (containers) | In-flow children, child sizes, children to descend into, sorted children and lines | Free stacks in the in-pass `PassScratch`, returned after use |
| Cold | Modal slots and text, visual, replaced content, inline records | Asked of the world when used |

The dense index is simply `StableNodeId` (`NodeTable` and `DenseNodeMap` already index by it; ids increase in creation order and are not reused). Adding, removing, reordering, and detaching nodes do not change indexes, so no free list or generations are needed. The full-page pass's tables are allocated once by the highest id; a scoped pass touches only a few nodes, and its output stays hashed. Per node: view 48 B (56 B per table slot; the old `PassInput` was also 56 B), check bits 2 B, output 32 B.

Each step is interleaved 8 rounds with the previous one, rotating order every round and reading the minimum (`FRAME_PATH=1`, plain box page 2 101 / 21 001 boxes, control page 31 001 boxes; instructions are `reps 1000 5` minus `SKIP_LAYOUT=1`, divided by 105 005 boxes):

| Step | Layout µs / box | Control page | Resident B / box | Layout instructions / box | IPC |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before (3637f6c8c) | 1.20 / 1.41 | 1.74 | 3 025 / 2 727 | 28.6k | 4.12 |
| 1 Dense output, input as view | 1.16 / 1.38 | 1.66 | Unchanged | 28.3k | 4.26 |
| 2 Reserve by highest id | 1.15 / 1.36 | 1.64 | 2 840 / 2 541 | 28.1k | 4.25 |
| 3 Style check bits | 1.11 / 1.33 | 1.58 | Unchanged | 27.2k | 4.23 |
| 4 Work list reuse | 1.04 / 1.24 | 1.53 | Unchanged | 24.5k | 3.97 |
| 5 56 B per slot, own style on demand | 1.03 / 1.24 | 1.52 | Unchanged | 24.8k | 3.99 |

1. **Output and input**: boxes written by placement used to go into a table hashed by id and were collected and sorted at the end of the pass; the full-page pass writes a dense table and hands it out in id order. `PassInput` becomes the view `UiWorld::layout_input_view` hands out, so two `Arc`s are no longer cloned per node per pass; placement's `NodeFacts`, `ArrangedChild`, and in-flow child collection all borrow this view.
2. **Reserve by the highest id**: `DenseNodeMap` has more slots than nodes (ids start at 1), so a table reserved by node count always doubles on its last insert; `reserve_through` reserves by the highest id, and the retained intrinsic table (120 B per slot) and the box and padding tables each lose half their slots. On the first layout the box table takes over the pass's output table.
3. **Style check bits**: the few style questions measurement asks first are packed into `StyleFacts`, memoized by style pointer for the last 8; in test builds and under `layout-verify`, every hit is compared against a fresh computation.
4. **Work list reuse**: `PassScratch` gives each kind of temporary list a free stack, and layout allocations drop 16.8 → 9.8 per box. This is the largest step of this round.
5. **Fix the scoped pass's regression**: after the first four steps, a row-height change on the page with an open Select went from 2 913 µs to about 4 000 µs. The cause was not layout code: the scoped pass puts the node inputs it reads into a dense table indexed by id, and when the edit is near the top of the page that table grows from a low id to the page's highest id and is reallocated by doubling every frame (that was already so before the change); once the view carried the style tag and check bits, each slot went 56 → 64 B, and on the 62 001-node page the table reached 4.46 MB, crossing the system allocator's size class, so every frame started on freshly mapped pages (`time -l` shows about 370 page faults per edit), which also slowed stages that were not changed, such as extraction and hit testing. The modal's input moves out of the view (the two sites that use it ask on the spot), slots go back to 56 B with a test guarding it, and own-style check bits are only computed when measurement asks.

Combined (interleaved 8 rounds with 3637f6c8c, reading the minimum):

| Page | Boxes | Layout µs / box | Tree building + layout µs / box | Resident / box | Layout allocations / box | Taffy layout / tree building + layout / resident |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain boxes, product first frame | 2 101 | 1.20 → 1.03 | 2.31 → 2.19 | 3 025 → 2 840 B | 16.9 → 9.9 | 0.70 / 0.71 µs / 944 B |
| Plain boxes, product first frame | 21 001 | 1.41 → 1.24 | 2.60 → 2.44 | 2 727 → 2 541 B | 16.8 → 9.8 | 0.70 / 0.77 µs / 1 506 B |
| Plain boxes, product first frame | 67 201 | 1.77 → 1.56 | 3.02 → 2.79 | 2 798 → 2 791 B | 16.8 → 9.8 | 0.75 / 0.81 µs / 944 B |
| Plain boxes, full `layout_document` | 21 001 | 1.29 → 1.11 | 2.47 → 2.30 | 2 546 → 2 360 B | — | — |
| Control page, product first frame | 3 101 | 1.26 → 1.12 | 3.00 → 2.82 | 3 161 → 2 963 B | — | — |
| Control page, product first frame | 31 001 | 1.74 → 1.52 | 3.58 → 3.35 | 3 211 → 3 014 B | — | — |

Instructions per box (product first frame, 21 001 boxes): layout 28.6k → 24.8k, tree building 22.9k unchanged, total 51.6k → 47.7k (Taffy 17.5k); control page layout 30.0k → 26.9k. At 67 201 boxes the ids exceed `DENSE_LIMIT` (65 536), and the excess goes into an overflow hash table, so residency barely drops. Incremental edits (`edits 2000`, full-frame minimum, µs): height 1 034 → 1 034, width 116.2 → 114.6, transform 14.4 → 13.8, scale 14.1 → 14.1, `CONTROLS=1` text 21.0 → 20.9, `SCROLLED=1` 1 196 → 1 187, `ROTATED=1` height / transform 1 126 / 14.4 → 1 132 / 14.2, `OPEN_SELECT=1` 2 913 → 2 682; `insert` mounting and the frame after it are −3% to +5%, both directions occur, and a separate 10–12 round re-measurement (`wide` 100 / 400, `deep` 100 / 400) shows both directions too. The 16 temporarily exported page-and-edit groups (plain boxes height / width / transform / scale, control page text / height, MIXED, OPEN_SELECT, SCROLLED (plain boxes and control page), ROTATED height / transform, `insert` deep / wide / under-deep / under-deep-at-once) produce per-frame layout boxes, `LayoutResult`, Scene, hit grid, and accessibility tree on every commit that are byte-identical to 3637f6c8c.

Tried and reverted: **line box memo as a dense table** (`NodeMap<bool>` → `DenseNodeMap<bool>`): residency −11 B / box, cold layout instructions −0.1k, but a row-height change on the page with an open Select went 1 125 → 1 313 µs over 10 rounds, so it was reverted. **Reserving an extra eighth by the highest id** (to leave room for new nodes mounted into an already laid-out page): `insert` shows no difference beyond noise while residency rises 23 B / box, so it was reverted.

Gap to Taffy (21 001 boxes): layout 2.0 → 1.8×, tree building plus layout 3.4 → 3.2×, residency 1.8 → 1.7×, instructions 2.9 → 2.7×. What remains is not in the data layout: even a small tree (everything in cache) costs 1.03 µs and over 24k instructions per box, with IPC already around 4 — the gap is what each node does: the intrinsic facts measurement records for the incremental path, measurement plans and container plans (built for every container on every pass, and on the first layout for the next frame), line box and baseline answers, the used-size memo looked up by `MeasurementKey`, the branches for formatting contexts and visuals, and publishing a `LayoutResult` after layout (about a fifth of layout time, a 280 B struct plus slices per node). Taffy only computes boxes and keeps none of this.

Cold layout, fourth round: what each node does (2026-11-09). Measured first: with the `benchmark` feature's phase clocks, the product first frame (21 001 boxes) spends per box 300 ns measuring, 470 ns placing, 128 ns in the engine's tail and 248 ns publishing (195 building results, 36 storing them). Then each part was switched off and measured again (only to split the bill; the results are wrong), at 1.24 µs per box of layout for 21 001 boxes: no measurement plans −8%, no container plans −9%, neither −17%; no intrinsic facts recorded −10%; no publish −20%; all four −28%. In `sample`'s publish call tree `layout_anchors` takes 14% (four tenths of it growing the anchor memo's hash table); the tail holds the consulted hash set, the next_generation table hashed by content, and rehashing and whole-block copies of retained tables growing from empty by doubling. Counters show a pass measures about 1.5 used sizes and intrinsic facts per node (2 on the control page), so the two tables reserved by node count double at the end of the pass.

1. **The first layout's tail and publish** (0cabf13e5): the pass's used sizes and intrinsic facts reserve twice the node count; a pass with no affected node (a full one) builds no consulted set; an empty retained measurement plan table takes the pass's table whole; an empty retained container plan table reserves once; with no retained intrinsic facts every content takes the first generation directly; the first publish reserves the result table and the anchor memo once. Against 2cd0eea21, alternated 8 rounds: layout at 2 101 / 21 001 boxes 1.05 / 1.25 → 0.98 / 1.18 µs, control page at 3 101 / 31 001 boxes 1.13 / 1.56 → 1.06 / 1.50; layout instructions per box 24.6k → 23.6k, IPC 3.57 → 3.70; residency 2 541 → 2 523 B.
2. **The first publish builds without comparing first** (497170eb9): with the result table empty it sorts no table of covered children, asks no held result whether it still holds and compares with none, and counts children the same way as before. Against the previous commit, alternated 8 rounds: layout at 2 101 / 21 001 boxes 1.01 / 1.28 → 0.98 / 1.23 µs, control page at 3 101 boxes 1.12 → 1.07 (flat at 31 001); layout instructions per box 23.7k → 23.3k (control page 25.9k → 25.2k).

Together (against 2cd0eea21, alternated 8 rounds, minimum read, machine load 4–11):

| Page | Boxes | Layout µs / box | Build + layout µs / box | Resident / box | Layout instructions / box |
| --- | ---: | ---: | ---: | ---: | ---: |
| Plain boxes, product first frame | 2 101 | 1.06 → 0.96 | 2.22 → 2.16 | 2 840 → 2 809 B | — |
| Plain boxes, product first frame | 21 001 | 1.33 → 1.24 | 2.54 → 2.49 | 2 541 → 2 523 B | 24.6k → 23.3k |
| Control page, product first frame | 3 101 | 1.18 → 1.06 | 2.96 → 2.88 | 2 963 → 2 953 B | — |
| Control page, product first frame | 31 001 | 1.68 → 1.59 | 3.56 → 3.48 | 3 014 → 3 012 B | 26.9k → 25.2k |

Incremental edits run the same instructions per edit as the previous commit (height 21.8M, width 2.46M, height on the page with an open Select 47.0–47.2M; two measurements of the same binary differ by ±0.2M), and their timings under load go either way; `insert` mounting and the frame after it are within noise. The temporary export of 16 groups of pages and edits is byte-identical, frame by frame (layout boxes, `LayoutResult`, Scene, hit grid, accessibility tree and updates), with each commit's parent.

Tried and withdrawn: **no plans for single-child containers** (a container with one in-flow child records no measurement or container plan, and retires the old ones when it drops to one child): plain page layout at 21 001 boxes 1.39 → 1.22 µs (−12%), residency 2 523 → 2 163 B, layout instructions per box 23.9k → 21.3k, the frame after `insert` −2% to −12%, control page unchanged (its cells have two children), exports byte-identical; but 9 incremental contracts judged by `WorkCounters` went red: repeated resizes within one bucket used to measure nothing and now measure 2 boxes; a row growing in a million rows measures 3 boxes instead of at most 2; a child list edit walks every child instead of replaying; the plans down a deep chain no longer take the leaves. The single-child plans are what keep those edits at zero measurement, so it was withdrawn. Not building plans at all on the first layout fails the same way (upper bound −17%): the first edit in every region would scan its siblings. **Reserving the anchor memo for every covered node on every publish**: a height edit covers over ten thousand nodes, so every frame allocated and cleared a table that large; only the first publish reserves. **Reserving the retained facts table by fact count**: there are half again as many facts as contents, the table landed one size class up, +51 B per box resident; removed.

Gap to Taffy (21 001 boxes): layout 1.8× (1.24 against 0.70 µs), tree building plus layout 3.2×, residency 1.7×, instructions per box 46.4k against 17.5k (2.7×, 23.3k of it layout). What remains is still what each node does, and the three largest parts are kept for incremental edits: plans (17%), intrinsic facts (10%), and the published `LayoutResult` (20%). The first two are the price of the incremental contracts (the withdrawn step above is the evidence); the published result is a public contract, a 280 B struct per node, and saving more on it means building results lazily or on demand, which touches every place Scene, hit testing and accessibility read results. A like-for-like comparison of block, flex and grid under the same CSS, what the extra cost buys, and what the capabilities Taffy has no counterpart for cost, is in [Taffy reference benchmark](/reference/taffy-reference).

Incremental edit timings (2026-11-03): `edits` changes one place per frame inside the product frame of `RuntimeDocument::flush`, reads the minimum of the `Layout` stage in the frame profile and of the whole flush, and compares against Taffy's `compute_layout` after changing the same leaf (`edits 200,2000,6400`; `CONTROLS=1` changes label text; `PRINT_COUNTERS=1` prints the layout counters of the last edit). `width` changes the width of a leaf in the sixth cell of a middle row and moves only that row; `height` changes its minimum height, so every row below moves. The first measurement:

| Boxes | Edit | Layout | Full frame | Taffy layout |
| ---: | --- | ---: | ---: | ---: |
| 4 201 | width | 364 µs | 2.2 ms | 47 µs |
| 42 001 | width | 6.3 ms | 41.7 ms | 1.0 ms |
| 134 401 | width | 28.0 ms | 152.8 ms | 5.4 ms |
| 134 401 | height | 240.8 ms | 680.5 ms | 5.3 ms |

On the control page, changing label text takes 11 µs at both 9 301 and 93 001 boxes (full frame 19 / 26 µs) and does not grow with scale. `width`'s counters are small (5 measured, 12 placed, 2 000 memo hits), yet time grows with scale; there were two O(N) terms:

- **The line box memo was rebuilt every pass.** When the root is remeasured it has to answer whether it contains a line box (`holds_line_box_in`), and on the plain box page, where there are no line boxes at all, only a full tree walk can answer "no"; the memo table used to be built anew every pass. It now lives in `DocumentLayoutCache::line_boxes`: the answer depends only on a box and what is inside it, so a scoped pass clears it from every affected node up to the root (it does not stop at an ancestor with no answer: a box that has line boxes itself settles without asking its children), and the rest is reused. (Since 2026-11-05 it is cleared by the seed's change kind; see "Relative geometry closeout · Line box memo" below.) In test builds, worlds of up to 4 096 nodes compare every answer against a fresh walk. 134 401 boxes, `width`: layout 28.0 → 6.7 ms.
- **Child placements in the parent result made the parent count as "geometry changed".** A row is as wide as its content, so it widens when a cell widens (281 ↔ 289), and the root's result records every row's placement (`FlexChildPlacement` fragments and `ChildPlacement` parts), so the root was marked hit, render, and accessibility dirty and the hit index rebuilt the whole document from the root (`patch_hit_subtree`). Only layout writes these two kinds of records, and Scene, hit testing, and accessibility never read them, while a child whose position changed publishes and projects itself; `layout_result_projects_new_geometry` now excludes them from the comparison. 134 401 boxes, `width`: full frame 117 → 6.7 ms; 4 201 boxes 2.1 → 0.13 ms.

After both fixes:

| Boxes | Edit | Layout | Full frame | Taffy layout |
| ---: | --- | ---: | ---: | ---: |
| 4 201 | width | 111 µs | 131 µs | 46 µs |
| 42 001 | width | 0.83 ms | 0.85 ms | 0.46 ms |
| 134 401 | width | 6.7 ms | 6.7 ms | 5.1 ms |
| 42 001 | height | 22.2 ms | 71.2 ms | 0.48 ms |
| 134 401 | height | 118 ms | 346 ms | 5.0 ms |

`height` is still 20–70× behind, and the cause is the coordinate representation: NanaUI's layout boxes are document-absolute, so when a row grows, every box below changes coordinates (at 42 001 boxes, 21 001 placements, 20 997 of them moving only the origin), and then writeback, the hit index, accessibility, Scene primitives, and extraction each make their own pass over the changed boxes; Taffy stores positions relative to the parent, so the same edit only changes each row's own `location` after that row. Catching up would require retained boxes to be relative to the parent (or to the nearest compositing layer), with the whole `UiWorld → UiScene` path changing with it — a structural change, not done.

P0 and P1 of the translation design ([design](https://claude.ai/artifact/6VoxW6MKE6XVYkyfkjfHeF), option B) in the engine (2026-11-03): `WorkCounters` gains `layout_translations` and `layout_translated_nodes`; a container that only moved no longer walks its descendants through `place_node_body`, and `translate_moved_children` replays the plan's recorded offsets top-down from the new content origin, bit-identical to a fresh pass (the content origin uses the same addition order `origin + border + used padding + leading parts`, and first recomputes with the old origin and compares against the plan); if the subtree contains a box that cannot move this way (no retained box, a plan with positioned children or a lost offset, a dynamic row, an island, an inline context, a position that disagrees with the plan) the whole subtree falls back to the original path. issue259's cost gate becomes "placement + translation ≥ moving boxes". 42 001 boxes, row-height change: placements 21 001 → 1 012, with 19 989 carried by 1 008 translations; in samples, engine placement 424 → 248 and the layout stage about 15% less; instructions over the whole run 1.8% fewer.

What remains of the layout stage is mostly result publishing and writeback: padding box, content box, text lines, and parts in the result are all derived from the origin by addition, and translating them in place cannot be bit-identical to a cold rebuild, while the existing contract (`assert_matches_cold`) demands bit-identical output, so P1 did not skip result rebuilding and is still pending.

Results in local coordinates (2026-11-03): a result's own geometry — border box, padding box, content box, text lines, parts, baseline — is computed with the border box at the origin and then offset by the origin as a whole to give the outward result (`build_own_layout_result` → `place_layout_result`); child-related quantities (child placements, inline atom fragments, overflow boxes, scroll ranges) take the children's exact absolute boxes directly, without subtraction. Outward, it is still `layout_result(id) -> Option<&LayoutResult>` in absolute coordinates; the public interface and semantics are unchanged. Derived boxes now compute `origin + (border + padding)` instead of `(origin + border) + padding`, so the last bit can differ; cold rebuilds and incremental updates take the same path, so they agree bit for bit. The part solver's sites that read slot boxes (a list item's leading and trailing slots, a label value's action buttons) now measure from the same origin (`solve_content_parts_in`); the first version missed this and added the origin twice, and both the cold-rebuild comparison and the fast-path comparison were wrong in the same way, which the semantic snapshot caught.

A node in a subtree that only moved therefore does not need its result rebuilt at publish: its own geometry is unchanged, and it is only placed at the new origin and its children reattached. The result table records one more value per entry, `OwnGeometry`: `Empty` (a plain container), `PlainLine { baseline }` (one line of plain text, whose fragments and text part are the content box), everything else `Unkept`, rebuilt as usual on translation; list items and label values whose parts read slot positions are rebuilt as usual too. In test builds, every node that takes the fast path is built once more in full and compared bit for bit. Storing a full local result would add about 330–350 B per laid-out text, button, and list item, exceeding `node_memory`'s budget, so only this small enum is stored: each of the three node kinds grows 16 B (text 3 442 → 3 458 B). Against the engine's post-translation commit: a 42 001-box row-height change uses 1.7–2.0% fewer instructions over the whole run, width is unchanged, and first-frame layout uses 0.6% more instructions per box (32 766 → 32 970), with time within noise. In a row-height frame, the hit index, extraction, and accessibility stages still redo each moved box one by one; they are the subject of the next steps.

Hit index (2026-11-03): dirtied entries are routed first, and those that can be handled in place no longer go through subtree patching (`rebuild_hit_test_scoped`). When an entry is "plain" — unit transform, no clipping, no path clipping, no menu or painter, not escaped (`plain_hit_entry`) — the state it hands to its children does not depend on position: a node carried by a translation only copies its new world box into the entry; every other dirty node rebuilds only its own entry (`build_hit_entries_reaching` does not descend into children), and if the new entry is still plain, the child list and stacking are unchanged, and it was not turned `fixed`, the entry is replaced in place with its children keeping their old entries. Style changes that would affect child entries (pointer-events, visibility, transform, stacking) already mark the whole subtree INPUT-dirty, so children rebuild themselves. Entries handled in place refresh their bounds from the bottom up by depth, once per node; the bounds tree is paired by slot, so identical leaves give bit-identical internal nodes. The cold-rebuild comparison (`assert_matches_cold`) rebuilds the whole incremental index before comparing and so never exercises in-place patching, so in test builds every in-place handling on documents of up to 4 096 nodes also builds a complete index and compares these entries field by field against their parents in the index. The engine also records a translation when a moved container keeps its size and is not affected itself, so result publishing rebuilds one fewer result. 42 001 boxes, row-height change: hit stage 16.5 → 3.2 ms, full frame 65.6–68.8 → 52.4–52.8 ms, instructions over the whole run 13.6% fewer; 134 401 boxes, full frame 303–305 → 249–252 ms; width and label-text changes unchanged. What remains of the full frame is mostly accessibility (about 12 ms) and extraction (about 14 ms).

Scene (2026-11-03): primitives are built from layout boxes in absolute coordinates (plus ancestor transforms), and translating them bit-exactly like results would mean moving Scene primitives to local coordinates overall and changing GPU drawing, which was not done. What was done: when a node has no primitives at all in the Scene and its newly extracted record is identical to the old one apart from position and the outward result (`moved_only`; unchanged position or changed size is excluded directly, to avoid comparing child placements in wide container results one by one), primitives are not rebuilt. The state it hands to children is computed from the Scene's node presentation (`project_ancestor_state`), and projection is only read when drawing that node's own primitives, so skipping a node with no primitives does not affect others; debug builds rebuild every skipped node for real in scenes of up to 512 nodes and assert it still has 0 primitives. 42 001 boxes, row-height change: extraction stage about 14 → 9.6 ms, full-frame minimum 53.8 → 47.0 ms, instructions 7.3% fewer; 134 401 boxes 248 → 223 ms; nodes with painting are rebuilt as usual. What remains of the extraction stage is mostly the world side building an `ExtractedNode` for every dirty node (about 3.5 ms): a lightweight path that only sends "new box + new result" would first have to tell "dirty only because of a move" from "dirty for another reason too" (a theme change in the same frame, inline visibility, a confirm button's busy state, and so on are all handled during extraction), so it is left for later.

Nodes dirty only because of a move (2026-11-03): the node record gains two extraction epochs (`render_full_epoch` / `render_moved_epoch`), and the world's epoch increments after every `extract_nodes_reusing`. The distinction is recorded at the source of the dirtying: only nodes whose result publishing took the translation fast path (own geometry unchanged, placed again at the new origin) are recorded as "moved"; `WriteLayout` flags that only shifted position and flags projected from that node do not count as a cause; every other RENDER flag (style, text, theme, visibility, a confirm button's busy state, sticky, a rebuilt result, a new node, recovery after failure) records "other". When the epoch has only "moved" and no "other", and the Scene holds that node's previous record, the world no longer builds an `ExtractedNode` and sends only an `ExtractedMove` (new box + new result); the Scene's `apply_delta_with_moves` rewrites the record in place, computing the checks in the original loop that read the box or result (inherited geometry, transform and clip, clip chain, escape, stacking, inherited opacity, moves without primitives, group candidate counts) once before and once after the rewrite; anything uncertain takes the full extraction. Letting the world clone the Scene's old record and swap the box was tried first and was slower (world side 3.4 → 5.8 ms): an `ExtractedNode` is 1 160 B, and the Scene's old record is cold at extraction time, so reading it costs about as much as extracting again, so neither side reads the whole record. Comparison: under `layout-verify` (and in runtime test builds), every node that takes the lightweight path has its old record swapped to the new box and compared field by field against a full extraction (layout box bitwise), reached with `cargo test -p nana-ui-scene --features nana-ui-runtime/layout-verify`; `nana-ui-scene/tests/moved_extraction.rs` moves all rows in each frame while also changing label color, swatch fill, same-length text, or theme in the same frame, then compares every node's Scene record against cold extraction and its primitives against a cold Scene. Mutation experiments: with "other" flags ignored, the comparison reports `["source_style", "style"]`, and each of the four same-frame changes turns red on its own; a lightweight path that does not update the box, or a Scene that treats a painting moved node as having no primitives, also turns red. 42 001 boxes, row-height change (`PRINT_STAGES=1` now prints each stage's minimum over all rounds, interleaved): extraction stage 9.1–10.2 → 3.9–4.7 ms, full-frame minimum 44.3 → 40.4 ms, instructions over the whole run 26.1–26.4 G → 22.2–23.0 G (about 90 million fewer per frame); 134 401 boxes: extraction 50–55 → 21–25 ms, full-frame minimum 247 → 229 ms; stage times and instructions for width changes and control-page label-text changes are within noise (6.31–6.42 G, 8.84–8.93 G). `render_nodes_extracted` counts moves as well, and `extracted_text_spans` counts only fully extracted nodes.

Accessibility (2026-11-03): `AccessibilityNode::bounds` became relative to the parent (offsets measured from the parent's `bounds` origin, size unchanged; `clip` is still in viewport coordinates), and the AccessKit host sets the offset as the node's `transform` and the size as its local `bounds`. After a row grows, boxes that translate along with their parent keep the same relative values, are no longer projected, and do not enter native updates. The decision is made at layout publish time: for a box carried by an engine translation (`translated`), if the "own minus parent" offset and the size are bit-identical between the old and new results, and no container on the clip chain was moved or resized by this publish, the ACCESSIBILITY flag is not set (when layout writeback only shifted a box, that bit is left to the publish that follows immediately, and nodes that never reach a publish are marked by the next drain). INPUT flags set only because of a layout move carry no `DirtyMask::INPUT_ELSEWHERE` and go into `SystemWork::accessibility_moved` at drain; such nodes are no longer seeds for a whole subtree and project only themselves (if marked ACCESSIBILITY) and their direct children that layout did not move (for example a viewport-fixed `fixed` box); containers that clip descendants, or that have clipping visuals, are still handled as a whole subtree. The projection side checks again: the viewport box of the remaining node and of its parent must be exactly the layout box (no transform, no scroll offset, not a modal frame, not a control with virtual children such as a select, tree, or menu), otherwise it projects as usual. In test builds and under `layout-verify`, worlds of up to 4 096 nodes record the nodes every projection hands out, and every delta projects from scratch the nodes that "the old rule would have projected and the new rule leaves alone", comparing them field by field against what was handed out. 42 001 boxes, row-height change (interleaved 8 rounds with main after 12f5fb909, reading the minimum, load 12–24): accessibility stage 13.2 → 3.9 ms (median 14.8 → 4.5), full frame 48.1 → 36.9 ms (median 57.1 → 43.3); the layout stage costs 0.4–1 ms more because of the publish-time check (20.9 → 21.3 ms); instructions per edit (difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200) 409 M → 340 M (−17%; measured on the old baseline before the rebase, where only changing the seed rule without the publish check saved about half). Width changes 15.98 M → 15.81 M per edit, full frame 0.93 → 0.88 ms; control-page label-text changes unchanged at 24 µs full frame; cold first frames (the whole run, including tree building and the Taffy comparison) within ±0.5%. What remains of accessibility is the 1 000 moved rows themselves (their offsets relative to the parent really did change) and `accessibility_tree_is_plain`'s full scan.

Public API changes: the semantics of `AccessibilityNode::bounds` (relative to the parent); new `UiWorld::project_accessibility_in_viewport` and `project_accessibility_nodes_in_viewport` (viewport coordinates for automation and tests); `SystemWork::accessibility_moved` (a new field; behavior is unchanged when it is empty). Consumers that read `bounds` as screen coordinates must switch to viewport projection: the in-repo devtools agent export already has. In NanaLive (`nanalive-control/src/native_ui/`), the places that click or assert using the position from `project_accessibility(...).bounds` are `offscreen.rs` (offscreen automation clicks at the node center, the product path), `offscreen/receiver.rs`, `form/right_stack.rs`, `form/mode_rail.rs`, `mod.rs`, `kit/tests.rs`, `tools/content/tracking.rs`, and `tools/content/guide_model/tests.rs`; they need `project_accessibility_in_viewport`. Reads that only check whether width and height are positive are unaffected.

Primitive rebuild for painting nodes (2026-11-03): exactly translating existing primitives for moved painting nodes was evaluated and not done. Primitive positions are computed per kind in `build_node_primitives` (for example `(x + border) + padding`), adding a shift does not give bit-identical values, and recomputing from local geometry would mean copying a position formula for every primitive kind; Button's `component_geometry` is absolute to begin with and cannot enter `moved_only`. An upper-bound experiment — letting such nodes skip rebuilding entirely (wrong output, only to measure the ceiling) — saved only 3.3% of instructions on the control page (`CONTROLS=1 edits 1355 height`, 42 006 boxes, 6 779 such nodes), with a full-frame minimum difference on the order of noise; on the specified bare-box page the Scene has no primitives at all, so the gain was 0. The painter only iterates primitives after culling by `visible_operations`, so adding one origin per primitive costs the same regardless of scene size. The real hotspots of rebuilding itself are the ancestor walk and allocation of draw-order keys, which have nothing to do with coordinates and were changed: `ancestor_answer`'s walk table lives in a reused `RebuildScratch`; `group_prefix_memo` is answered in one step when the parent is already memoized, without collecting the chain; `order_stack` is built with one allocation, and on rebuild, if the slot's existing stack content is identical, that `Arc` is reused, and `insert_primitive` no longer swaps it when the key is unchanged. Control page, row-height change: 6.96–6.97 → 6.79–6.81 hundred million instructions per edit (−2.3%), extraction stage minimum 25.3 → 24.3 ms (8 interleaved rounds), full frame within noise; bare-box page instructions unchanged.

Layout stage constants (2026-11-03): for a 42 001-box row-height change, the layout stage is about 19–21 ms on ba8ba044f, while Taffy does the same edit in 0.44–0.48 ms. Only about 1 000 rows really relayout; the other ~20 000 nodes are translations. Split apart with the `benchmark` feature's segmented clock (temporary extra slots, since removed), taking the minimum of each segment over 40 edits (µs, before → after):

| Segment | Before | After | Content |
| --- | ---: | ---: | --- |
| Measurement | 1 500 | 1 500 | The root column is remeasured; 2 000 rows hit the cache row by row, about 0.7 µs/row |
| Placement · translation walk | 2 450 | 2 060 | `translate_moved_children`: about 9 table lookups per box, and one plan clone per container (`moved_to`) |
| Placement · write | 650 | 610 | Translation results written into `output` and this round's plan table |
| Engine tail · container plan recycling | 1 700 | 990 | 11 000 plans deduplicated, swapped into the retained table, old ones dropped |
| Engine tail · rest | 450 | 450 | Output sorting, writing retained boxes |
| Writeback comparison + view scan + commit | 1 750 | 1 750 | One `WriteLayout` per moved box, marked dirty at commit |
| Publish · prep | 700 | 530 | Ancestor closure, sorting, grouping by parent |
| Publish · building results | 4 200 | 3 950 | Every moved node rebuilds its absolute-coordinate result at the new origin: three slices + one `Arc` |
| Publish · geometry comparison | 330 | 330 | `geometry_eq` comparison against the old result |
| Publish · keep-offset decision | 700 | 650 | 47aa93fb2's accessibility decision; each translated node looks up its parent's old and new boxes |
| Publish · store and mark dirty | 2 130 | 1 060 | Install the new result, drop the old, mark INPUT/RENDER |
| Wrap-up | 550 | 550 | Mostly `scoped_scroll_retention_targets` walking the whole scope |

What changed (each item interleaved 3 rounds with the previous binary, instrumented build, minimum per segment):
- `dedupe_plan_rows`: up to 16 rows are compared pairwise directly instead of building a SipHash `HashSet` for every plan (11 000 allocations per frame); longer ones use `NodeSet`. Plan recycling 1 670–1 810 → 980–1 030 µs.
- `LayoutResult::placed_frame` takes by value: it used to clone the whole result (`..self.clone()`, four `Arc` refcounts) and then drop it. Building results 4 140–4 370 → 3 920–4 040 µs. The two together: layout 19.0–19.4 → 18.1–18.7 ms.
- Translation walk: a container's border, padding, and leading parts are read once and shared between the old and new origin (it used to walk `effective_layout_style` for each, including presence and overlay/menu branch checks), with the addition order unchanged; the work list lives in `LayoutInputMap` and is reused, so it no longer allocates and grows three `Vec`s per container. Translation walk 2 430–2 690 → 2 130–2 280 µs, layout 18.2–18.6 → 17.5–17.8 ms.
- Publish: `layout_result_projects_new_geometry` used to clone the whole old result for every published node and compare after patching; it now compares field by field and concludes directly when the node's own box changed. The store loop looks the old result up once (`entry`), "whether projection is requested" uses a cursor advancing with `published` instead of a binary search per node, and prep's `unique` does a stable sort starting from the input order (already sorted) instead of re-sorting from the set's hash order. Store and mark dirty 2 110–2 130 → 1 050–1 070 µs, prep 670–700 → 530–540 µs, layout 17.6–17.9 → 15.9–16.3 ms.

Combined (production build, interleaved 8 rounds with ba8ba044f, reading the minimum, load 4–5): layout stage 18.49 → 15.95 ms (−14%), full frame 31.69 → 29.35 ms; hit testing, accessibility, and extraction unchanged. Instructions per edit (difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200) 325 M → 296 M (−9%). Tried and not kept: merging writeback's three document cache lookups into one (−0.1 ms, and the three accessors became dead code); changing the plan's `by_child` to a shared slice (recycling saves 0.13 ms, the walk costs 0.15 ms more, and the atomic counts offset the saved allocations).

Of what remains, these can only go away by changing the coordinate representation: building results (about 4 ms, four allocations per moved node, and the old four blocks freed at store time), writeback and commit (about 1.75 ms), dirty marking in the store (about 1 ms, every node marked INPUT/RENDER once at commit and once at publish, two set inserts each time), and the keep-offset decision (0.65 ms) — all of them per "box whose coordinates changed", and once boxes store offsets relative to their parent, translating a subtree changes only the root offset and none of these happen. Candidates that do not need an architecture change: updating the translation plan in place (this round only recorded "where this plan moved" and did `get_mut` on the retained table in the tail for the new origin, avoiding the `moved_to` clone, the this-round table insert, and the old plan's drop; the dozen or so places that read `nodes.container_plans` would have to understand this state; estimated ≤1 ms); skipping `scoped_scroll_retention_targets` when the document has no `ScrollView` (about 0.5 ms, needs a count kept).

Remeasuring the root column (2026-11-03, continuing the previous section): that 1.5 ms was not a slow cache-hit path; the root column was not using a measurement plan at all. `Stack` defaults to `height: Shrink`, while the sequential plan (`sequential`, which adds a changed child's difference into the recorded total) requires the main-axis size to be `is_none()`; `Shrink` and `Auto` both resolve to no length in `resolve_axis` and `finish_intrinsic_size` takes the content-derived default, exactly as if nothing were written, so the requirement was relaxed to "not written, `auto`, or `shrink`". Other content size keywords (`min-content` and so on) and sizes with min/max are still excluded. In a frame changing height, the root column no longer hits the cache 2 000 times row by row (measurement hits 2 025 → 28, measurement plan reuse 1 → 2). The new test `content_growth_under_a_child_moves_its_siblings_when_the_container_shrinks` makes a `Shrink`/`Auto` column remeasure as many children as a column with no height (2; 14 before the change, and the mutation experiment confirms it turns red), comparing every step against a full recompute box by box. Interleaved 8 rounds with 275f9229c, reading the minimum (load about 5.5): row-height layout 16.33 → 15.17 ms, full frame 30.37 → 29.05 ms, instructions per edit 297 M → 286 M (−3.5%); width change (rows are `Shrink` too) layout 0.84 → 0.12 ms, already below Taffy's 0.46 ms; control page (`CONTROLS=1 edits 1355 height`) layout 20.9 → 19.7 ms. Re-measured after rebasing onto the first stage of relative geometry (35045d816) (6 interleaved rounds, reading the minimum, load about 6.6): row-height layout 3.81 → 2.29 ms (−40%), full frame 17.67 → 16.31 ms, instructions per edit 166 M → 155 M (−6.7%); width 0.78 → 0.11 ms. Once stage 1 took the translation cost away, this 1.5 ms root-column remeasurement became the largest single piece of layout for a row-height change.

Relative geometry (2026-11-03 to 11-05, design in [relative-geometry.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md)) changed layout, hit testing, extraction and Scene, the painter, and accessibility all to "what is inside a box lives in the box's own coordinates", so translating a subtree changes only the root's offset. The table below is the before-and-after overview (`perf/micro/stack-depth`, `PRINT_STAGES=1 edits 2000 …`, full-frame minimum, µs): "before" takes the baseline commit from the first time that page was measured, "now" the last round that measured it; the rows come from different rounds under different loads, so read magnitudes only, and the step-by-step A/Bs follow in the sections below.

| Page | Edit | Before (commit) | Before | Now |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | ba8ba044f | 30 424 | 1 094 |
| Control page 62 001 nodes | height | ba8ba044f | 85 988 | 1 180 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | 7fa3b81b4 | 34 237 | 1 180 |
| Plain page `SCROLLED=1` | height | 21f8058fa | 35 741 | 1 255 |
| Plain page `ROTATED=1` | height | 9f4fc87be | 27 075 | 1 216 |
| Plain page (root column) | transform | 54e3425e7 | 39 219 | 17.7 |
| Plain page `ROTATED=1` | transform / scale | 54e3425e7 | 37 619 / 38 038 | 17.8 / 17.8 |
| Plain page | width | ba8ba044f | 795 | 121.5 |
| Control page | text | ba8ba044f | 22.4 | 21.2 |

Instructions per row-height change: plain page 326 M → 23 M, control page 834 M → 25 M (ba8ba044f against 2f0479886). The sections below record each step in chronological order.

Relative geometry stage 1 (2026-11-03, design in [relative-geometry.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md)): layout boxes became offsets relative to a reference frame (parent, containing block, or page), stored locally in `NodeRecord`, the engine's retained boxes and plan origins, and `LayoutResult`; `translate_moved_children` was deleted, so a container that only moved writes only its own offset and touches no descendant, and results are neither rebuilt nor republished. The outward `layout_box` and friends accumulate down the reference-frame chain to absolute values. The hit index and Scene still read absolute boxes, so during the transition commit-time `spread_layout_moves` marks the descendants of a translated subtree as "only moved". 42 001 boxes (interleaved 8 rounds with 275f9229c, reading the minimum, load 5.5–7.1, µs):

| Edit | Stage | 275f9229c | Stage 1 |
| --- | --- | ---: | ---: |
| height | Layout | 16 391 | 3 911 |
| height | Hit testing | 2 917 | 3 793 |
| height | Accessibility | 3 392 | 3 379 |
| height | Extraction | 4 274 | 3 896 |
| height | Full frame | 29 928 | 17 794 |
| width | Layout / full frame | 780 / 800 | 771 / 787 |
| Control page text | Layout / full frame | 11 / 23 | 11 / 23 |

Instructions per edit (difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200, two rounds): height 295.7–295.9 M → 166.4 M (−44%), width 15.3–15.4 M unchanged, control-page text changes 0.27–0.36 M → 0.40–0.47 M (about 100k more). Whole run (`RUNS=40`, including tree building and the first frame): height 17.49 → 12.23 G, width 6.26–6.28 → 6.28 G (+0.3%), control page 8.77–8.80 → 8.84–8.85 G (+0.7%): what the cold path gained is the O(depth) sums that derive absolute boxes everywhere. The hit stage costs about 0.9 ms more: each of the 20 000 entries it moved has to derive an absolute box, and the 1 000 written-back rows each rebuild their own entry too (a writeback that only shifted a box cannot move the entry in place: the same commit may add or remove siblings, which changes entry order, as `layout-verify` caught); this disappears in stage 2 with local entries. `node_memory` is unchanged: the runtime reads results locally, page-placed results are only cached when `layout_result` is called from outside, and extraction shares local results directly.

Public interface changes: `UiWorld::layout_result` returns `Option<Arc<LayoutResult>>` (previously `Option<&LayoutResult>`); new `LayoutFrame`, `local_layout_box`, `layout_frame`, `local_layout_result`; `ExtractedNode::layout_result`, `ExtractedMove::layout_result`, and `StickyAncestor::result` became results in the node's own coordinates; `UiMutation::PlaceLayout` (hidden, used by the engine's writeback). `MutationQueue::write_layout` still takes document coordinates and converts them to local values by depth when the commit ends; children that are not written translate with the parent (previously they stayed put).

Relative geometry stage 2 (2026-11-03): hit entries became local — each entry lives in its own border box's coordinates and stores the `origin` from the hit parent's origin, subtracted level by level at query time (the same path as scrolling's `shift`); translating a subtree rebuilds only the root entry and leaves descendant entries alone. Entries that read geometry beyond their own offset (cross-reference-frame offsets, escape, component geometry, sticky) are recorded as pinned, and pinned entries below a dirty node are rebuilt individually. 42 001 boxes (interleaved 8 rounds with 35045d816, reading the minimum, load 3.2–3.9, µs):

| Edit | Stage | 35045d816 | Stage 2 |
| --- | --- | ---: | ---: |
| height | Layout | 3 792 | 3 779 |
| height | Hit testing | 3 617 | 660 |
| height | Accessibility | 3 230 | 3 003 |
| height | Extraction | 3 769 | 3 767 |
| height | Full frame | 17 325 | 13 789 |
| width | Layout / full frame | 760 / 776 | 762 / 778 |
| Control page text | Layout / full frame | 10 / 22 | 10 / 22 |

Instructions per edit (difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200): height 167.7 M → 139.5 M (−17%). What remains of the 0.66 ms hit stage: the 1 012 written-back rows each rebuild their own entry (`hit_test_entries_moved` 1 008, `hit_test_nodes_rebuilt` 1 012), ancestor bounds are refreshed by depth, and among the ~20 000 descendants still marked INPUT it skips those translating with their parent (`layout_followers`, removed along with those marks in stage 3).

Relative geometry stage 3 (2026-11-03): commits no longer mark the descendants of a translated subtree dirty (`spread_layout_moves` and `layout_followers` deleted) and send only the translation root. Scene places records along the reference-frame chain: records that draw nothing stay in the space they were extracted in and follow by transform, while records that draw are placed again in the Scene and rebuilt (the painter snaps pure translations to whole pixels; see [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene)); accessibility walks records down once from the translation root. 42 001 boxes (8 interleaved rounds with 715cc0331, swapping order every round and reading the minimum, load 3.3–4.1, µs; the Scene row is a separate temporary timing of the `apply_delta_with_moves` segment inside the extraction stage, 6 rounds):

| Edit | Stage | 715cc0331 | Stage 3 |
| --- | --- | ---: | ---: |
| height | Layout | 2 059 | 470 |
| height | Hit testing | 648 | 244 |
| height | Accessibility | 2 856 | 795 |
| height | Extraction (incl. Scene) | 3 735 | 80 |
| height | of which Scene | 1 954 | 48 |
| height | Full frame | 11 650 | 1 695 |
| width | Layout / full frame | 105 / 120 | 108 / 121 |
| Control page text | Layout / full frame | 10 / 22 | 10 / 22 |

Instructions per edit (difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200): height 127.1 M → 32.0 M (−75%). Of accessibility's remaining 0.8 ms, about 0.17 ms is the walk down; the rest is reprojecting the 1 008 translated rows themselves.

When a painting subtree translates (control page `CONTROLS=1 edits 2000 height`, 62 001 nodes, 10 labels plus buttons per row; reading the minimum, 3 rounds): with the visibility index (calling `visible_operations` after each frame, that is, with a painter) the full frame is 81.6 → 66.2 ms, Scene 54.6 → 50.7 ms, accessibility 9.7 → 2.9 ms; without the visibility index the headless run's Scene is 27.3 → 34.1 ms, the extra being `place_painted_records` scanning the whole primitive table (about 7.6 ms), while the path with a painter only looks at primitives under the root. Nodes with component geometry such as buttons are still fully re-extracted when they translate. The painter section (`nana-text-paint-benchmark --workload reflow`, new: the first label's height oscillates 0.5 px per frame and all labels below translate): 1 000 labels `batch` p50 0.15 ms, 10 000 labels 2.5 ms, unchanged; text instances are not rebuilt (`text_instance_rebuilds` 0), and each frame uploads only the translated rows per node (478 KB `text_presentation_upload_bytes` for 10 000 labels); `flush` 26.4 → 27.7 ms for 10 000 labels (every label is a directly translated root, with no subtree to skip).

Public interface changes: `ExtractedNode` gains `local_layout` and `layout_frame` (literal construction must add them); `ExtractedMove` gains the same two, sent only for a translation root; `SystemWork` gains `layout_moved`; `UiScene` gains `node_layout_bounds`. `UiScene::extracted_node(id).layout` and `node_bounds` are now the space the record lives in, so a record that draws nothing in a translated subtree may stay where it was extracted; use `node_layout_bounds` for the current position, and `draw_node_bounds` / `draw_primitive` for the drawing position.

Relative geometry stage 4 (2026-11-04): primitives, a node's own clipping, and `ExtractedNode.component_geometry` became node-local coordinates; the painter first places the geometry at the node origin (one addition per coordinate), then transforms, then snaps translations to whole pixels, so a layout translation is no longer treated as a scroll snap, and a subtree moved by a fractional pixel is pixel-identical to a cold scene (see [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene) and [Relative geometry · painter](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_6-painter)). `place_painted_records` and extraction's `component_geometry.is_none()` guard were deleted: a translated subtree rebuilds no primitives at all, button text fields are no longer re-extracted, and the culling index translates as a whole (`translate_subtree`); only nodes that read the page's visual (Select, MenuSurface, NativeMarkdown) and nodes that read their own position (`origin_bound`) are still handled per node. 8 interleaved rounds with 3e9c83b80, swapping order every round and reading the minimum (load 2–6.5, µs; VISIBLE=1 is a new `perf/micro/stack-depth` switch that calls `visible_operations` after each frame, that is, with a painter):

| Page | Edit | Stage | 3e9c83b80 | Stage 4 |
| --- | --- | --- | ---: | ---: |
| Control page 62 001 nodes, VISIBLE=1 | height | Extraction (incl. Scene) | 59 183 | 330 |
| Control page, VISIBLE=1 | height | Full frame | 66 494 | 4 076 |
| Control page, headless | height | Extraction (incl. Scene) | 41 292 | 122 |
| Control page, headless | height | Full frame | 48 846 | 3 731 |
| Control page | height | Layout / accessibility | 2 934 / 2 928 | 801 / 2 207 |
| Plain page 42 001 boxes, headless | height | Extraction / full frame | 81 / 1 782 | 58 / 1 728 |
| Plain page, VISIBLE=1 | height | Extraction / full frame | 86 / 1 736 | 60 / 1 748 (3 other rounds) |
| Plain page | width | Full frame | 122 | 119 |
| Control page | text | Full frame | 22 | 22 |

Instructions per edit (difference of `RUNS=240` and `RUNS=40` divided by 200): control-page row height 407 M → 54 M (headless), 597 M → 64 M (VISIBLE=1), plain page 31 M → 31 M. The control page's layout-stage drop comes from `settle_layout_moves` no longer marking the 20 000 buttons under a translation root RENDER. What remains is mostly accessibility (2.2 ms, `project_viewport_entries`; already down to sub-millisecond in the closeout step, see below).

The painter section (`nana-text-paint-benchmark --workload reflow`, 8 interleaved rounds reading the minimum, ms): 1 000 labels flush 2.07 → 1.76, batch 0.151 → 0.192, total 2.22 → 1.95; 10 000 labels flush 27.7 → 23.4, batch 2.47 → 2.59, total 30.1 → 26.0. Neither side rebuilds text instances (`text_instance_rebuilds` 0), and the translated rows upload the same amount; batch's extra ~20 ns/label is reading each node's placement origin (`DrawAttributes.origin`) when rebasing, and flush's saving is the Scene no longer rebuilding primitives for every translated label and the culling index no longer recomputing one by one. The snapshot suite's output is byte-identical to 3e9c83b80 (680 images; `gallery-sidebar-collapsed-dark` is a known jitter). NativeMarkdown was first tried in local coordinates: the whole document's layout accumulates from the content box, and accumulating locally then placing on the page differs in the last bit from accumulating in page coordinates; Mermaid curves in two `gallery-rich-text` snapshots differed by level 1 in 7 pixels, so it stays in page coordinates (re-extracted on translation).

Public interface changes (stage 4): `ScenePrimitive`'s geometry (`bounds`, batch rectangles, stroke points, `Path` / `Chart`'s `origin`, `LayerBegin`'s clip, `LayerMask.origin`, a node's own clipping) became node-local coordinates (except visuals that read the page); page positions use the new `UiScene::primitive_layout_bounds` / `place_rect`. `SceneDraw` gains an `origin` field plus `place`, `placed_bounds`, and `geometry_transform`; `transform` no longer includes the origin, and `clips` are all in page coordinates. `ExtractedNode.component_geometry` is local (`UiWorld::component_geometry` is still page coordinates, unchanged); new `StandardVisual::geometry_reads_page`. `UiScene::node_bounds` is back to the current page box (the same as `node_layout_bounds`).

`component-gallery`'s semantic snapshots have 8 other unrelated differences (the `line_height` of dialog, empty-state, and similar text changed from `Relative` to `normal`), coming from b1bc92819, which also appear on trees without these changes; the baseline was not re-recorded.

Relative geometry closeout · accessibility (2026-11-04): below a translated root, records are no longer walked one by one (`reach_moved_subtree` used to walk each root's whole subtree); instead the nearest translation root above is found from the pinned boxes recorded in `NodeStore` (reference frame is not the parent, clipping, non-identity, Select/TreeView/MenuSurface/ModalFrame) (`reach_moved_pins`, O(pins × depth)); when the root's chain to the page is non-identity it still walks the whole subtree. What remains is reprojecting the translation root itself: `axis_aligned` used to fall back to computing the transform chain from scratch whenever there was any visual (one `hit_motion_layout` and transform composition per child on the control page); now it looks at the boxes on the chain one by one; the asked-about box and its hidden check no longer enter the memo table (previously two hash inserts per child and repeated table growth), and child filtering reserves by child count. The approach is in [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍). 8 interleaved rounds with 6838427fb, swapping order every round and reading the minimum (load 11–17, µs, absolute values higher than on a quiet machine):

| Page | Edit | Stage | 6838427fb | This round |
| --- | --- | --- | ---: | ---: |
| Control page 62 001 nodes | height | Accessibility | 3 165 | 635 |
| Control page | height | Full frame | 5 783 | 2 567 |
| Plain page 42 001 boxes | height | Accessibility | 1 113 | 362 |
| Plain page | height | Full frame | 2 454 | 1 460 |
| Plain page | width | Full frame | 127 | 128 |
| Control page | text | Full frame | 24 | 24 |

Segments (temporary timings, load 4–8, reading the minimum): the control page's walk down 680 µs → 16 µs, projection of the 1 013 translated rows 1 833 → 595 µs; plain page 210 → 14 µs, 635 → 344 µs. Instructions per edit (difference of `RUNS=240` and `RUNS=40` divided by 200): control-page row height 55 M → 26 M, plain page 31 M → 23 M. What remains of accessibility is each translated row's own projection (the relative offset really did change): the node itself, hidden and alignment checks for ten children, and the whole `AccessibilityNode` put into the delta.

Relative geometry closeout · page-reading nodes (2026-11-04): NativeMarkdown, closed Select, and MenuSurface component geometry became local coordinates and are reused on translation; only an open Select / MenuSurface is recorded in `NodeStore`'s `page_bound` index (a separate set at the time; since be0a703c8 maintained by the node space class `NodeSpace::page_bound`), and `settle_layout_moves` scans only this set (it used to scan every node with a visual on each translation). The approach is in [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene). 8 interleaved rounds with 7fa3b81b4, swapping order every round and reading the minimum (load 7–14, µs, `RUNS=40 PRINT_STAGES=1 edits 2000 height`):

| Page | Stage | 7fa3b81b4 | This round |
| --- | --- | ---: | ---: |
| Control page 62 001 nodes | Layout | 650 | 529 |
| Control page | Full frame | 1 525 | 1 389 |
| Control page VISIBLE=1 | Full frame | 1 760 | 1 579 |
| Mixed page (`MIXED=1`, `rich-text`) | Extraction | 24 818 | 183 |
| Mixed page | Layout | 2 946 | 1 106 |
| Mixed page | Accessibility | 4 280 | 3 688 |
| Mixed page | Full frame | 34 237 | 5 802 |
| Mixed page VISIBLE=1 | Full frame | 45 030 | 5 690 |
| Plain page 42 001 boxes | Full frame | 1 212 | 1 245 |
| Control page | text full frame | 23.2 | 23.5 |

The mixed page replaces every odd cell's button on the control page with a closed Select and each row's first cell with a Markdown paragraph (the Markdown cell needs a temporary build with `--features rich-text`; the committed benchmark only carries Select). The plain page has no visuals, so both sides take the same path and the difference is within noise. The mixed page's accessibility is still the bulk: Select derives virtual nodes from geometry, and at that point it was a pinned box (a closed Select stops being pinned from the "Accessibility scrolling and Select" step below).

Relative geometry end to end (2026-11-04): ba8ba044f before the change against the final 2f0479886, each built in release from the same `perf/micro/stack-depth`, interleaved 8 rounds, swapping order every round and reading the minimum (load 2.4–3.1, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | ba8ba044f | 2f0479886 |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Layout | 17 623 | 465 |
| Plain page | height | Hit testing / accessibility / extraction | 2 763 / 3 118 / 4 150 | 238 / 298 / 53 |
| Plain page | height | Full frame | 30 424 | 1 149 |
| Plain page | width | Full frame | 795 | 125 |
| Control page 62 001 nodes | height | Layout | 31 238 | 501 |
| Control page | height | Hit testing / accessibility / extraction | 4 986 / 8 741 / 36 643 | 247 / 309 / 57 |
| Control page | height | Full frame | 85 988 | 1 233 |
| Control page VISIBLE=1 | height | Full frame | 86 539 | 1 441 |
| Control page | text | Full frame | 22.4 | 22.9 |
| Control page VISIBLE=1 | text | Full frame | 22.5 | 24.0 |

Instructions per row-height change (`/usr/bin/time -l`, `RUNS=240` minus `RUNS=40` divided by 200): plain page 326 M → 23 M, control page 834 M → 25 M. Row-height changes on both pages drop from tens of milliseconds to about 1.2 ms, the same order as Taffy doing layout alone (about 0.48 ms); width changes 0.80 → 0.13 ms; control text changes unchanged (differences of 1–1.5 µs, noise level). The control page's ba8ba044f row includes the cost of re-extracting every button and text field on each translation at that time. The mixed page (`MIXED=1`) does not exist on ba8ba044f, so there is no end-to-end number; see the previous section.

Relative geometry closeout · accessibility scrolling and Select (2026-11-04): on chains with only translation, scrolling, and positive scale, relative bounds are computed from the local offset (`(a·(x − sx), d·(y − sy), a·w, d·h)`), the cold projection and the retained path use the same formula, and under a scrolled container or a scaled ancestor only the root is projected while pins are examined below; scrolling no longer pins, and a closed Select no longer pins; a clip seed whose box did not change (a scroll container whose content grew) no longer reprojects the whole subtree (new `SystemWork::layout_reshaped`); when projecting one row under scrolling, whether a child has a viewport box is decided directly from the parent's transform. The approach is in [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍). The benchmark adds `SCROLLED=1`: rows go into a viewport-tall scroll container, and after the first frame it scrolls to 12 345.5 px and then edits. 8 interleaved rounds with 21f8058fa (the same benchmark source with the `SCROLLED` patch), swapping order every round and reading the minimum (load 6.6–10.8, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 21f8058fa | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Accessibility | 312 | 301 |
| Plain page | height | Full frame | 1 241 | 1 168 |
| Control page 62 001 nodes | height | Accessibility | 333 | 363 |
| Control page | height | Full frame | 1 305 | 1 408 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Accessibility | 2 410 | 341 |
| Mixed page | height | Full frame | 3 657 | 1 316 |
| Control page | text | Full frame | 22.8 | 23.5 |
| Plain page `SCROLLED=1` | height | Accessibility | 17 362 | 935 |
| Plain page `SCROLLED=1` | height | Full frame | 35 741 | 17 672 |
| Control page `SCROLLED=1` | height | Accessibility | 30 151 | 1 622 |
| Control page `SCROLLED=1` | height | Full frame | 63 522 | 35 128 |

The control page's row-height change was rotated 8 more rounds on its own (load 5–7): accessibility 383 → 354, full frame 1 410 → 1 346; the difference in the table above is noise (in that round, stages that were not changed, such as layout and extraction, also slowed together). The mixed page here has no Markdown (the committed benchmark only carries Select), so it is not the same page as the 3.7 ms in the previous section, but it is the same cost of the Select pin.

Relative geometry closeout · scroll containers (2026-11-04): a translated row inside a scroll container rebuilds its own hit entry in place (the condition relaxes from "plain entry" to "the transform handed to children and the scroll did not change", seeded with the parent entry's current transform and converted back into index space by the `shift` above), whereas a scroll container with clipping and a `shift`, and rows whose transform carries the scroll, used to rebuild the whole scroll container subtree for one row-height change; culling is decided by the primitive's own opacity group (opening a Popover no longer over-reports the trigger), translation amounts go through the inherited transform's linear part, and accessibility child filtering asks the chain once per parent. The approach is in [Relative geometry · Hit index](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_4-命中索引), [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene), and [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍). 8 interleaved rounds with 40b898c12 (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 3.9–7.4, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 40b898c12 | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Hit testing / accessibility | 249 / 296 | 257 / 281 |
| Plain page | height | Full frame | 1 210 | 1 210 |
| Control page 62 001 nodes | height | Hit testing / accessibility | 266 / 344 | 275 / 312 |
| Control page | height | Full frame | 1 360 | 1 339 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Full frame | 1 339 | 1 327 |
| Control page | text | Full frame | 21.5 | 21.5 |
| Plain page `SCROLLED=1` | height | Layout / hit testing / accessibility / extraction | 1 127 / 16 013 / 987 / 212 | 598 / 263 / 462 / 68 |
| Plain page `SCROLLED=1` | height | Full frame | 18 771 | 1 561 |
| Control page `SCROLLED=1` | height | Layout / hit testing / accessibility / extraction | 1 352 / 31 838 / 1 532 / 237 | 658 / 287 / 525 / 72 |
| Control page `SCROLLED=1` | height | Full frame | 35 515 | 1 766 |

The scrolled page's layout and extraction also drop by more than half: it used to rebuild 40–60 thousand hit entries per edit, and the allocation and cache pressure landed on the other stages of the same frame. The plain page's hit testing costs about 8 µs more (each row rebuilt in place does one more upward `shift` lookup and escape check), with the full frame within noise. The scrolled page still costs about 0.35 ms more than the plain page: the whole-tree refill of the scroll container's content range (O(rows)) and each translated row's accessibility projection under the scroll chain.

Relative geometry closeout · content range (2026-11-04): the scroll container's content range index (`ContentBoundsIndex`) reads only moved children, writes leaves in place and merges upper nodes once per level, instead of refilling the whole tree when more than half the children moved; an entry that only swapped its own box settles in place with one lookup; accessibility child filtering hoists the parent's transform out of the loop; and `holds_line_box_in` reads each box's children once. The approach is in [Relative geometry · Layout](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_3-布局). 8 interleaved rounds with 9600e796a (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 4.4–5.7, µs, `PRINT_STAGES=1 edits 2000 …`); instructions per edit are the difference of two `time -l` runs, `RUNS=2040` and `RUNS=40`, divided by 2 000, done twice:

| Page | Edit | Stage | 9600e796a | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Full frame | 1 218 | 1 225 |
| Plain page | height | Instructions per edit | 21.9 M | 21.9 M |
| Control page 62 001 nodes | height | Full frame | 1 307 | 1 306 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Full frame | 1 331 | 1 315 |
| Plain page | width | Full frame | 126.3 | 125.5 |
| Control page | text | Full frame | 22.5 | 22.0 |
| Plain page `SCROLLED=1` | height | Layout / accessibility | 596 / 462 | 554 / 440 |
| Plain page `SCROLLED=1` | height | Full frame | 1 577 | 1 449 |
| Plain page `SCROLLED=1` | height | Instructions per edit | 28.5 M | 26.4 M |
| Control page `SCROLLED=1` | height | Layout / accessibility | 639 / 496 | 582 / 481 |
| Control page `SCROLLED=1` | height | Full frame | 1 693 | 1 540 |
| Control page `SCROLLED=1` | height | Instructions per edit | 30.3 M | 27.9 M |

The scrolled page's excess over the plain page drops from about 0.36 ms to about 0.22 ms (plain page) and 0.39 → 0.23 ms (control page). What remains is one content range leaf write per moved row, the viewport box decision for children under the scroll chain, and the line box a list baseline asks about (once per row) — all O(rows) constants; the reasons are in [Relative geometry · Known limits](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_10-已知限制).

Relative geometry closeout · accessibility transforms (2026-11-04): `AccessibilityNode` carries its own `transform` (AccessKit's `transform`), so rotated, skewed, and flipped boxes publish their own transform and descendants are placed by local offset inside it; under a rotated ancestor only the root is projected, and the rotated box and the boxes below it are no longer pins. The approach is in [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍). The benchmark adds `ROTATED=1` (rows inside a column rotated by 0.05 rad). 8 interleaved rounds with 9f4fc87be (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 4.5–5.8, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 9f4fc87be | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Accessibility / full frame | 264 / 1 130 | 266 / 1 133 |
| Control page 62 001 nodes | height | Accessibility / full frame | 283 / 1 218 | 284 / 1 214 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Accessibility / full frame | 292 / 1 250 | 279 / 1 207 |
| Plain page `SCROLLED=1` | height | Accessibility / full frame | 420 / 1 367 | 442 / 1 396 |
| Plain page `ROTATED=1` | height | Accessibility | 9 951 | 750 |
| Plain page `ROTATED=1` | height | Full frame | 27 075 | 16 786 |
| Control page | text | Full frame | 23.0 | 22.3 |

Control page `CONTROLS=1 ROTATED=1` was also interleaved 8 rounds with 66cb370c0 (load 3.3–4.9): accessibility 17 668 → 1 286, full frame 50 706 → 34 715. The `SCROLLED=1` row is another 8-round interleaving (load 5.3–5.8): accessibility about 20 µs more, instructions per edit 26.47 M → 26.93 M (difference of `RUNS=1040` and `RUNS=40` divided by 1 000, +1.7%); `sample` shows no new hotspot, and the extra falls on each moved row's projection itself (nodes gained a 28 B `transform`), so it is recorded for now. What remains of the rotated page is mostly hit testing (about 15 ms, the same on both sides): in-place hit rebuild is limited to entries whose linear part is identity, so under rotation a translation still patches the whole subtree ([Relative geometry · Hit index](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_4-命中索引)). — Resolved by the next section, closeout · hit transforms.

Relative geometry closeout · hit transforms (2026-11-05): a box whose own 2D transform has scale, rotation, skew, or flip (with no perspective and no 3D context anywhere on the chain) hands its layout coordinates to its children (`HitEntry.local_children`), so descendant hit entries carry neither it nor the transforms above it; in a rotated subtree a translation rebuilds only the root entry, and a rotated box that is stretched taller (its transform origin moves with it) rebuilds only itself. The approach is in [Relative geometry · Hit index](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_4-命中索引). 8 interleaved rounds with 1e9322016 (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 2.0–2.7, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 1e9322016 | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Hit testing / full frame | 247 / 1 134 | 246 / 1 129 |
| Control page 62 001 nodes | height | Hit testing / full frame | 262 / 1 207 | 260 / 1 209 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Hit testing / full frame | 255 / 1 206 | 258 / 1 217 |
| Plain page `SCROLLED=1` | height | Hit testing / full frame | 244 / 1 383 | 262 / 1 382 |
| Plain page `ROTATED=1` | height | Hit testing | 14 704 | 243 |
| Plain page `ROTATED=1` | height | Layout / accessibility | 841 / 779 | 456 / 446 |
| Plain page `ROTATED=1` | height | Full frame | 16 688 | 1 330 |
| Control page `CONTROLS=1 ROTATED=1` | height | Hit testing | 29 815 | 251 |
| Control page `CONTROLS=1 ROTATED=1` | height | Full frame | 32 516 | 1 440 |
| Plain page | width | Full frame | 127.0 | 126.3 |
| Control page | text | Full frame | 21.4 | 21.3 |

For a row-height change, the rotated page rebuilds 42 002 → 1 013 hit entries (`PRINT_COUNTERS=1`, only one more than the plain page's 1 012, the rotated column itself). The rotated page's layout and accessibility also drop by about 0.35 ms: the code did not change, but the same process no longer rebuilds 42 000 entries and writes the index every frame, so cache and allocator state differ; recorded as an observation. The `SCROLLED=1` hit testing excess of about 18 µs measured 261 → 263 in another 8-round interleaving, and the full frame was flat in both rounds, so it is treated as noise. In the first version of the patch, in-place rebuild and the patch's seed walked up twice (`find_hit_transform` and `child_shift`), costing the plain page about 16 µs more in hit testing; after merging them into one (`live_child_seed`) the re-measurement was flat (the table above).

Relative geometry closeout · changing transforms (2026-11-05): changing a box's `transform` style marks only that box. Hit testing rebuilds only the entry that hands out local coordinates, accessibility projects only that box and the pins below it when the space handed to descendants is unchanged (rotated before and after, or the same pair of upright scale factors) (`SystemWork::transform_kept`), and Scene already re-extracts only it. The approach is in [Relative geometry · Hit index](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_4-命中索引), [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene), and [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍). The benchmark adds `edits 2000 transform` / `scale`: each edit changes the column the rows are in (`ROTATED=1` is the rotated column, angle 0.05 ↔ 0.06 rad; the plain page is the root column, 0.01 ↔ 0.02 rad; `scale` changes the scale 1.01 ↔ 1.02). 8 interleaved rounds with 54e3425e7 (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 2.3–2.9, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 54e3425e7 | This round |
| --- | --- | --- | ---: | ---: |
| Plain page `ROTATED=1` | transform | Hit testing / accessibility | 17 636 / 15 489 | 0 / 20 |
| Plain page `ROTATED=1` | transform | Full frame | 37 619 | 24.6 |
| Plain page `VISIBLE=1 ROTATED=1` | transform | Full frame | 37 866 | 24.0 |
| Plain page (root column) | transform | Hit testing / accessibility | 17 340 / 17 004 | 3 / 15 862 |
| Plain page (root column) | transform | Full frame | 39 219 | 16 168 |
| Plain page `ROTATED=1` | scale | Hit testing / accessibility | 17 720 / 15 330 | 3 / 14 643 |
| Plain page `ROTATED=1` | scale | Full frame | 38 038 | 14 956 |
| Plain page 42 001 boxes | height | Full frame | 1 135 | 1 130 |
| Control page 62 001 nodes | height | Full frame | 1 235 | 1 223 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Full frame | 1 230 | 1 223 |
| Plain page `SCROLLED=1` | height | Full frame | 1 412 | 1 375 |
| Plain page `ROTATED=1` | height | Full frame | 1 334 | 1 336 |
| Plain page | width | Full frame | 123.3 | 121.2 |
| Control page | text | Full frame | 21.7 | 21.5 |

The root column and scale changes only save hit testing: the page root comes from the viewport's bounding box, so when it rotates its descendants rotate with the bounding box; and when the scale factor changes, a descendant's bounds is an offset multiplied by the factor, so both still reproject accessibility for the whole subtree (about 15 ms). Previously about 4.5 ms of the full frame outside hit testing and accessibility marked and drained 42 000 nodes one by one, and that went away with the whole-subtree marking.

Relative geometry closeout · line box memo (2026-11-05): the line box memo is only cleared upward along seeds that could change the answer (`may_move_line_boxes`): seeds that only change size, spacing, alignment, scrolling, transforms, or pointer input do not clear it, and neither do nodes brought into the frontier by dependency edges; structural seeds (topology, visibility, unknown reason) still clear along all affected nodes. After every layout pass, test builds and `layout-verify` compare every remaining memo against a fresh walk. The approach and guards are in [Relative geometry · Layout](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_3-布局), and the mutation experiments are in d8eabffea's commit message. Temporary counters (boxes resettled per edit, `RUNS=41`): row-height changes plain page 3 → 0, `SCROLLED=1` 5 → 0, control page 3 → 0, `CONTROLS=1 SCROLLED=1` 5 → 0, mixed page 3 → 0, `ROTATED=1` 3 → 0; width changes 2 → 0; control text changes 2 → 2 (text is an input to the answer); settling one list reads its 2 000 rows, and `holds_line_box_in` in `sample` drops from 30 samples to 0. 8 interleaved rounds with 29f4c3e4d (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 1.9–2.1, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 29f4c3e4d | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Layout / full frame | 467 / 1 148 | 437 / 1 106 |
| Control page 62 001 nodes | height | Layout / full frame | 495 / 1 204 | 493 / 1 197 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Layout / full frame | 478 / 1 198 | 494 / 1 203 |
| Plain page `SCROLLED=1` | height | Layout / full frame | 523 / 1 372 | 482 / 1 353 |
| Plain page `SCROLLED=1` | height | Accessibility | 438 | 427 |
| Plain page `ROTATED=1` | height | Layout / full frame | 444 / 1 311 | 454 / 1 322 |
| Plain page `ROTATED=1` | transform | Full frame | 24.6 | 24.9 |
| Plain page | width | Full frame | 121.0 | 121.9 |
| Control page | text | Full frame | 21.5 | 21.4 |

The mixed page's and `ROTATED=1`'s layout differences measured 486 → 474 and 462 → 462 in another 8-round interleaving (load 1.8–2.0), so they are treated as noise; in that round `SCROLLED=1` layout was 530 → 505 and full frame 1 388 → 1 347, and the plain page's full frame 1 118 → 1 114. The full-frame gap between `SCROLLED=1` and the plain page is still about 0.23 ms: accessibility about 165 µs more (the viewport box decision for every child under the scroll chain), layout about 45 µs more (one content range leaf write and `record_layout_origin` per moved row), and hit testing about 15 µs more.

Relative geometry closeout · viewport frames and scale (2026-11-05): boxes that come from the viewport (the page root, `fixed`, `sticky`, escaped absolute positioning) publish their own `transform` when they are rotated or scaled on the 2D chain; a positively scaled box publishes `diag(a, d)`, and descendants all use local offsets without the factor, so changing their angle or scale factor projects only them and the pins below; child filtering under the scroll chain takes the mapping once per parent, the transform chain takes the parent's mapping directly for plain boxes, and the memo table is given capacity before projection. The approach and guards are in [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_7-无障碍), and the mutation experiments are in the commit messages of 02c09ef2a and 736e1b51b and in relative-geometry.md at the time. 8 interleaved rounds with d8eabffea (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 2.3–3.1, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | d8eabffea | This round |
| --- | --- | --- | ---: | ---: |
| Plain page (root column) | transform | Accessibility / full frame | 15 614 / 15 935 | 14 / 17.7 |
| Plain page (root column) | scale | Accessibility / full frame | 14 339 / 14 646 | 14 / 17.9 |
| Plain page `ROTATED=1` | transform | Accessibility / full frame | 20 / 24.4 | 14 / 17.8 |
| Plain page `ROTATED=1` | scale | Accessibility / full frame | 14 643 / 14 956 | 14 / 17.8 |
| Plain page 42 001 boxes | height | Accessibility / full frame | 268 / 1 132 | 235 / 1 095 |
| Control page 62 001 nodes | height | Accessibility / full frame | 279 / 1 201 | 252 / 1 184 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Accessibility / full frame | 279 / 1 201 | 251 / 1 178 |
| Plain page `SCROLLED=1` | height | Accessibility / full frame | 439 / 1 360 | 323 / 1 261 |
| Plain page `ROTATED=1` | height | Accessibility / full frame | 455 / 1 321 | 337 / 1 223 |
| Plain page | width | Full frame | 122.5 | 121.5 |
| Control page | text | Full frame | 21.3 | 21.4 |

The accessibility gap between `SCROLLED=1` and the plain page drops from about 171 µs to about 88 µs. Measured separately (`SCROLLED=1`, 6 interleavings in the same round): only changing child filtering and the transform-chain fast path gives about 391–399 µs, and then giving the memo table capacity brings it to 330–337 µs. The hit stage on `SCROLLED=1`, the mixed page, and `ROTATED=1` costs about 7–12 µs more on a row-height change (255 → 266, 261 → 271, 238 → 250), though the hit code did not change; the build that does not reserve capacity is flat against the baseline, so this is recorded as an observation of allocator state, and full frames are still each about 0.1 ms less.

Relative geometry closeout · child placements and extraction records (2026-11-05, 6e21c8579): a layout result whose child placements alone changed is also marked RENDER (a correction: the Scene used to keep the old child placements indefinitely). The approach is in [Relative geometry · Layout](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_3-布局). 8 interleaved rounds with 736e1b51b (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 4.3–5.1, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 736e1b51b | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Extraction / full frame | 55 / 1 091.5 | 53 / 1 092.3 |
| Control page 62 001 nodes | height | Extraction / full frame | 58 / 1 171.2 | 59 / 1 174.9 |
| Plain page `SCROLLED=1` | height | Extraction / full frame | 56 / 1 251.6 | 56 / 1 254.2 |
| Control page | text | Full frame | 21.3 | 21.6 |

All within noise: on these pages the edited row's parent had its own box changed too, so it was re-extracted anyway.

Relative geometry closeout · open Select (2026-11-05): an open Select's component geometry became field-local, so a translation re-extracts only when the menu surface's box changed (flipping sides or truncating). The approach is in [Relative geometry · Extraction and Scene](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/relative-geometry.md#_5-提取与-scene). The benchmark adds `OPEN_SELECT=1` (used with `CONTROLS=1 MIXED=1`: the edit is in the second row, the fourth row's second cell is an open Select that is inside the viewport and translated by every edit, about 2 000 rows moving; `OPEN_SELECT=closed` is the same page with it closed). 8 interleaved rounds with 6e21c8579 (the same benchmark source, each built in release in its own target), swapping order every round and reading the minimum (load 3.0–4.5, µs, `PRINT_STAGES=1 edits 2000 …`):

| Page | Edit | Stage | 6e21c8579 | This round |
| --- | --- | --- | ---: | ---: |
| Plain page 42 001 boxes | height | Accessibility / extraction / full frame | 234 / 54 / 1 101 | 234 / 55 / 1 094 |
| Control page 62 001 nodes | height | Accessibility / extraction / full frame | 253 / 58 / 1 190 | 247 / 60 / 1 180 |
| Mixed page (`CONTROLS=1 MIXED=1`) | height | Accessibility / extraction / full frame | 247 / 58 / 1 172 | 254 / 60 / 1 180 |
| Plain page `SCROLLED=1` | height | Accessibility / full frame | 328 / 1 255 | 324 / 1 255 |
| Plain page `ROTATED=1` | height | Accessibility / full frame | 345 / 1 214 | 345 / 1 216 |
| Mixed page `OPEN_SELECT=1` | height | Accessibility / extraction / full frame | 881 / 230 / 3 716 | 922 / 235 / 3 767 |
| Mixed page `OPEN_SELECT=closed` | height | Accessibility / extraction / full frame | 914 / 232 / 3 737 | 908 / 245 / 3 792 |
| Control page | text | Full frame | 21.8 | 21.2 |

On both builds the open-versus-closed difference is within noise (the difference between the two page kinds within one build is ±50 µs, the same order as the difference between the two builds); with the difference of two `time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200, each edit is about 47.5 million instructions, and the open-versus-closed difference fluctuates ±300 thousand between the two measurements, so one node cannot be measured. The gain shows in the test's counters: an open Select with 3 options used to rebuild 11 primitives on every translation that did not flip sides, now 0.

Node space classes and in-place resizes (2026-11-05): the node space class `NodeSpace` (be0a703c8) is a refactor whose accessibility deltas and whole trees, hit grids, Scene primitives and draws, culling, and work counters exported over 81 + 13 representative frames are byte-identical to 1ab1855eb, with `stack-depth` flat over 8 interleaved rounds; treating in-place resizes of a box with its own transform as a traversal root (596930d22) is a correctness fix, with `stack-depth` flat over 8 interleaved rounds. Afterwards, splitting `world/accessibility/`, `world/hit_test/`, and `nana-ui-scene/src/scene/` (e443a2a1e, e41729572, 138c79333) only moved code, leaving the public API and snapshot output unchanged.

A container that only moved skips its body (2026-11-09): when a parent's re-arrangement moves a container that did not itself change, `place_node_head` writes its new offset and `keeps_moved_subtree`, finding nothing the body reads changed, skips `place_node_body` (conditions in [Relative geometry · Translating a subtree](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/relative-geometry.md#_3-2-translating-a-subtree)). Plain-box containers used to re-check their affected children against the plan, and containers without a plan — inline formatting contexts (paragraphs), floats, `display: contents` and the like — laid all their children out again: about 3.6 µs per moved paragraph in the Taffy reference benchmark, see [Taffy reference · Observations](/reference/taffy-reference#observations). In tests and `layout-verify` builds, every box under a kept subtree must equal the full layout's bit for bit (`verify::note_kept_subtree`); `world/line_box_memo/moved_paragraphs.rs` moves paragraphs holding every kind of out-of-flow child and compares each step with a cold layout, and turns red when the "no child in another frame" condition is removed. Against 0b60695fd, 8 interleaved rounds swapping the order each round, least of each (load 9–18, µs; instructions are the difference of two `time -l` runs at `RUNS=240` and `RUNS=40`, over 200):

| Page | Edit | Stage | 0b60695fd | Now |
| --- | --- | --- | ---: | ---: |
| inline, 100 paragraphs | one run's text | layout / frame | 169 / 212 | 34 / 77 |
| inline, 1 000 paragraphs | one run's text | layout / frame | 1 779 / 2 101 | 181 / 481 |
| plain page, 42 001 boxes | height | layout / frame | 476 / 1 152 | 380 / 1 044 |
| plain page, 42 001 boxes | height | instructions per edit | 21.9 M | 20.4 M |
| plain page | width | layout / frame | 114.8 / 128.9 | 111.8 / 126.2 |
| plain page | transform / scale | frame | 15.5 / 15.5 | 15.5 / 15.6 |
| controls page, 62 001 nodes | text | layout / frame | 11.0 / 23.2 | 11.2 / 23.5 |
| `insert` deep / wide / under-deep 400 | mount | frame after | 4 031 / 1 886 / 4 721 | 3 875 / 1 881 / 4 359 |

In the row-height edit the 2 000 moved rows no longer re-check their plans' affected children, about 20% less layout. The controls page's 0.2 µs on a text edit is noise (`time -l` gives 0.46 M → 0.30 M instructions per edit, and a single measurement itself varies by about ±0.3 M). Cold layout (3.94 → 3.96 µs and 69.8k → 69.9k instructions per box on the inline page) and mounting are unchanged.

Done (2026-11-02): in the publish stage, each result's four slices are built in a reused scratch table and allocated once at their final length, empty slices allocate nothing (`Arc::<[T]>::default()`), and the per-parent groups become one sorted table instead of a `Vec` per parent; the intrinsic cache key's environment hash moves from SipHash to a fixed-seed foldhash, and `seeded_intrinsic` moves to hashbrown; `ConstraintClass` equality and hashing read the normalized tag and value in one pass instead of normalizing twice; `RetainedIntrinsicMetrics` grows each content's variant table by one (the first push used to reserve 4, using 80 B of 320 B); `MeasurePlanSlots`' two slots are boxed (each container used to have 528 B inline in the table, and usually only one slot is used); the full-page pass reserves capacity for its tables by node and container counts; and the anchor memo table no longer allocates when the parent has already settled.

Scrolling changes only the scroll container (2026-11-09, a2b52d862): a scroll used to be a whole-subtree seed in accessibility, and children's bounds held their parent's scroll, so every node in a scroll container projected again on every scroll; in hit testing every sticky branch was patched whole. Now `AccessibilityNode::scroll` holds the scroll and children's bounds do not depend on it, a scrolled container projects only itself and the pins below it (`SystemWork::accessibility_scrolled`, `reach_moved_pins`), and sticky hit entries are rebuilt in place first; see [Relative geometry · Accessibility](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/relative-geometry.md#_7-2-translating-a-subtree). The sticky / fixed scene of `perf/micro/taffy-reference` (the flex page in an 800 px scroller, every tenth row `sticky`, 20 `fixed` badges), alternated with 0b60695fd for 8 rounds, swapping order each round, minimum (load 10–18), one scroll's whole frame and in-process instructions:

| Page | Before | After |
| --- | ---: | ---: |
| 2 101 boxes, no sticky / fixed | 449 µs, 8.64 M | 4.3 µs, 0.07 M |
| 21 001 boxes, no sticky / fixed | 6 429 µs, 85.8 M | 10.2 µs, 0.20 M |
| 2 101 boxes, with sticky / fixed | 504 µs, 9.57 M | 34.5 µs, 0.52 M |
| 21 001 boxes, with sticky / fixed | 7 375 µs, 94.5 M | 353 µs, 4.05 M |

Without pins what is left is the scroll container's own projection (its child list is rebuilt with it, O(direct children)); with pins the cost follows the 120 pins and the stuck sticky branches (a stuck branch is patched whole in the hit index, the sticky offset being in the entry's transform), not the number of boxes. Cold first frame 3.29 → 3.29 µs / box. `perf/micro/stack-depth`, alternated the same way: whole frame for height 1 130 → 1 157, `SCROLLED=1` 1 285 → 1 296, `ROTATED=1` height / transform 1 259 / 16.0 → 1 254 / 15.8, `OPEN_SELECT=1` 4 841 → 4 606, `VISIBLE=1` 1 142 → 1 152 µs, cold layout 2.70 → 2.74 µs / box, all within noise; instructions per height edit (difference of `time -l`) 21.89 → 21.89 M, `SCROLLED=1` 24.41 → 24.55 M (+0.6%), `ROTATED=1` 24.17 → 24.05 M.

Sticky boxes rebuild only themselves (2026-11-09, 62a76afe6): after the section above, one scroll of the sticky / fixed page still took 353 µs, nearly all of it in hit testing: the sticky offset was folded into the entry's transform, which child entries inherit, so a stuck row could not be rebuilt in place and every stuck branch was patched whole (about 48 rows × 21 entries on the 21 001-box page). Now the sticky offset goes into the entry's `origin`, like the layout offset; child entries are stored relative to it, and sticking or sliding rebuilds that one entry. All sticky entries are built first and replaced at once, with one bounding-box refresh; under a pure scroll the stored transform stays, so a transform built against the live scroll that differs in the last bit does not send the entry back to a branch patch (see [Relative geometry · Hit index](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/relative-geometry.md#_4-hit-index)). This also fixes a missed projection left by a2b52d862 (36c731159): in the frame that scrolls back to the origin a previously stuck `sticky` node's bounds change, but the parent's scroll offset is already zero and `keeps_relative_bounds` took it as unchanged; it now keeps nothing whose parent is a scroll container. Profile (77f2b6f65, 21 001 boxes, load about 2): hit testing 344 µs, accessibility 58 µs, extraction 6 µs; the accessibility share did not grow with descendants: 121 pins project one node each (about 0.17 µs to find, 0.27 µs to project), plus the scroll container's own child list (about 10 µs for 1 020 children, paid by the page without pins too). Alternated with 77f2b6f65 for 8 rounds, swapping order each round, minimum (load 3.4–4.3), one scroll's whole frame and in-process instructions:

| Page | Before | After |
| --- | ---: | ---: |
| 2 101 boxes, with sticky / fixed | 30.5 µs, 0.52 M | 16.1 µs, 0.33 M |
| 21 001 boxes, with sticky / fixed | 312.5 µs, 4.03 M | 84.8 µs, 1.92 M |
| 2 101 boxes, no sticky / fixed | 3.8 µs, 0.07 M | 3.8 µs, 0.07 M |
| 21 001 boxes, no sticky / fixed | 8.9 µs, 0.20 M | 9.0 µs, 0.20 M |

What is left follows the sticky boxes and pins (10 sticky rows on the 2 101-box page, 100 on the 21 001-box one, plus 20 `fixed`): one hit entry per sticky box (about 0.3 µs; the sticky offset has to be computed to know whether it changed) and one accessibility node per pin (the node ships with its child list, which is AccessKit's contract: an updated node is given whole). A pin's cost does not grow with the boxes it holds. The incremental tree, a full projection and a cold build scrolled straight there are equal to the bit, and a 64 × 48 hit grid matches (12 uneven scrolls each at 2 101 and 21 001 boxes, including back to the origin; 2 101 boxes also under `layout-verify`). Everything else is unchanged: editing one run of the inline paragraphs, layout 159.6 → 159.5 µs, whole frame 437.0 → 433.8 µs, 10.03 → 10.04 M; cold first frame 2.34 → 2.35 µs / box; `perf/micro/stack-depth` whole frame height 948 → 944, `SCROLLED=1` 1 092 → 1 079, `ROTATED=1` height / transform 1 038 / 14.1 → 1 024 / 14.2, `OPEN_SELECT=1` 950 → 949, `VISIBLE=1` 950 → 975 µs (the layout stage is +6 µs too, with layout code untouched; instructions 20.29 → 20.22 M); instructions per height edit (difference of `time -l`) 20.07 → 20.28 M, `SCROLLED=1` 22.75 → 22.59 M, `ROTATED=1` 22.07 → 22.29 M, `OPEN_SELECT=1` 20.29 → 20.19 M, all within one measurement's ±0.3 M.

**What one node costs does not follow the size of the tree (2026-11-10).** Two paths used to, both by walking whole tables:

- Removing a node: `forget_subtree` ran `retain` over a dozen framework tables that hold one entry per component (the five reprojection registries, both directions of the keyed-child tables, slot keys and so on), so removing one node visited every node; and `AssemblyScope` finishes with one `despawn_node` per dropped child, each paying that twice.
- Keyed assembly: recording each new child copied its parent's whole key table (`String` keys included), inserted into the copy and rebuilt the whole reverse index, so a parent's first assembly cost the square of its children.

The first now works from the shorter side: a table with more entries than there are removed nodes is asked for each of them (a keyed parent's table is thinned once per affected parent), and only a shorter table is walked, so removing one node and tearing a whole tree down both cost what is removed. The second inserts into the table as it stands. Release, aarch64 macOS, minimum read:

| Case | Before | After |
| --- | ---: | ---: |
| Remove one of 1 000 buttons | 20 µs | 1.3 µs |
| Remove one of 10 000 buttons | 232 µs | 2.4 µs |
| Remove one of 100 000 buttons | 2 335 µs | 12 µs |
| First assembly of 1 000 keyed children | 48 ms | 2.3 ms |
| First assembly of 4 000 | 398 ms | 7.1 ms |
| First assembly of 16 000 | 6 302 ms | 30 ms |
| Drop 100 of 16 000 keyed children | 62 ms | 5.8 ms |

The 5.8 ms in the last row is about what declaring the remaining 15 900 children again takes (5.5 ms): an assembly now removes the children it no longer declares in one commit (`despawn_nodes`, 2026-11-11). It used to be a commit per child, and every commit that takes a child out of a parent settles the remaining siblings again, so dropping 100 took 19 ms. A keyed child placed under one of its siblings is removed before that sibling, so both go when neither is declared any more.

`nana-ui-runtime/tests/tree_size_cost.rs` guards these: dropping 60 children takes as many commits as dropping one, and the other two are held against a ratio of sizes rather than a budget: a child among 3 200 allocates no more than 3 times what a child among 200 does (10 times before, 1.003 times now); the shortest removal of one node among 32 000 takes no more than 8 times that among 1 000 (39 times before in a debug build, 1.15 times now). In the runtime's own tests `forget_subtree` re-checks itself with the old full walk every time, confirming that nothing the old implementation would have dropped is left.

Not changed: removing a child that is not the last still moves the siblings after it (about 0.25 ms to remove the first of 100 000 siblings; the world's child order). Most of that was renumbering the siblings after it, which no longer happens (see "Child numbers are written on the shorter side only" below); what is left is shifting the `Vec`.

**A reactive list changed in one place no longer visits every key (2026-11-11).** Every change to an `each` list put all keys into hash sets to find the added and removed ones and looked every row's roots up again, and `reconcile_children` then checked every child for existence and repeats, so inserting or removing one row cost the length of the list. The head and tail the old and new key orders share are now found by comparing, and only the part between is hashed (`Changes::between`; it falls back to the whole list when a head or tail key shows up in between), the unchanged rows' roots are taken in one piece from the previous result, and `reconcile_children` checks and reorders only the children between the common head and tail. Release, aarch64 macOS, minimum read, per operation when inserting and then removing one row:

| Rows | At the end, before | After | At the start, before | After |
| ---: | ---: | ---: | ---: | ---: |
| 1 000 | 30 µs | 3.7 µs | 31 µs | 5.6 µs |
| 10 000 | 280 µs | 15 µs | 300 µs | 32 µs |
| 100 000 | 4 628 µs | 109 µs | 5 575 µs | 489 µs |

What still follows the row count is reading the keys, comparing them and a few `memcpy`s (about 1 ns per row); inserting at the start also renumbered the siblings after it then; now only the shorter side is written (below). The unit tests of `view::structural` hold the partial comparison against the whole-list one over 20 000 random edits, and assert that inserting one row in the middle of 1 000 looks the row table up once.

Two more in the same batch:

- **Children are renumbered once per commit.** Every node records its index in its parent, and each edit of a child list renumbered the siblings after it; a commit that moves many children (a list reversed, every other row filled back in) renumbered after each move. While a commit is applied, only the lowest affected index per parent is now recorded, and the numbering happens once at the end; an index that is behind in the meantime is detected by `child_index`, which scans instead, so no wrong index is read. Reversing 16 000 rows 0.64 s → 0.28 s, filling 8 000 rows back in 114 ms → 52 ms. These two whole-list operations were still quadratic then (each move finds its place in a `Vec` and shifts it), with less than half the constant, and have since become one reorder (below); a single insertion costs what it did.
- **A removed node's tooltip is looked up.** The `tooltips` table has an entry per control with a tooltip, and removing a node walked all of it to see whether a tooltip's overlay or anchor was among the removed; a second table from overlay / anchor to control is now kept and asked about the removed nodes.

**A whole-list reorder is one mutation (2026-11-11).** Moving many children at once used to be one `Insert` per child: validation and application each find its place in the child list and shift the siblings after it, so reversing or sorting a list cost the square of its length. There is now `UiMutation::ReorderChildren { parent, order }` (`MutationQueue::reorder_children`): `order` must be exactly the children the parent has when the batch reaches the mutation, each once, or the whole batch fails with `InvalidChildOrder`; the list is checked once and written once. It brings no child in and leaves none out: insert the new ones and remove the unwanted ones first. The world treats as moved only the children between the common head and tail that lie outside one longest run keeping its order, and invalidates them as an `Insert` under the same parent does (input, render and accessibility dirty marks, published layout results cleared); the other siblings are not touched, and no subtree's mount state, depth or document membership is visited. The tests in `world/child_order.rs` hold one reorder against an `Insert` per moved child, comparing the work left for the next frame node by node, and then against the layout built cold in that order.

`reconcile_children`, the finish of a keyed `mount` and `each` lists choose by cost: one moved child, or two children exchanged across a long list, are still `Insert`s (a reorder looks at every child between the first and the last that moved, up to about 75 ns each; an `Insert` costs about 2 ns per child of the whole list plus 1 µs); everything else is one reorder, with children from elsewhere appended first. Release, aarch64 macOS, minimum read:

| Operation | Before | After |
| --- | ---: | ---: |
| Reverse an `each` list of 1 000 rows | 1.45 ms | 0.04 ms |
| Reverse an `each` list of 16 000 rows | 292 ms | 1.0 ms |
| Fill 8 000 rows back in between 8 000 rows of an `each` list | 52 ms | 20 ms |
| Turn 17 consecutive children round in the middle of 100 000 | 2.4 ms (16 `Insert`s) | 0.13 ms |
| Turn all of 100 000 children round | not measured (about 100 000 `Insert`s) | 3.0 ms |

What is left of the fill, 20 ms, is creating those 8 000 rows (about 2 µs each). In test builds the reads and writes of a child list are bounded: reversing, shuffling and filling in 4 000 children each stay within 6 times the list's length, where an `Insert` per child took 40 million steps.

**Child numbers are written on the shorter side only (2026-11-11).** Every node records its index in its parent, and putting a child first, or removing the first, wrote the index of every sibling after it: inserting the first of 100 000 siblings spent 83% of its time writing indices, the rest shifting the `Vec`. A child now records a number counted from a base its parent keeps (`Hierarchy::children_base`), and its index is the number minus the base (wrapping `u32`): siblings shifted as one block are not rewritten one by one, because moving the base renumbers all of them. Of the stretch at the start and the stretch at the end that an edit left alone, the longer keeps its numbers and the rest is written; when the list is as long as it was (a reorder, one child taken out and one put in), both keep theirs and only the middle is written. So an insertion or removal at either end writes only the child it changed, and an edit in the middle writes at most half the list (it wrote everything after it). During a commit the range is still only recorded and the numbering happens once at the end: per parent, how many children at the start and at the end no edit reached (`renumber_children_between`). The base sits in alignment padding `NodeRecord` already had, which stays 256 B. Release, aarch64 macOS, minimum, old and new alternating; unless noted, the total for inserting one child among 100 000 siblings and removing it again:

| Operation | Before | After |
| --- | ---: | ---: |
| At the start | 526 µs | 76 µs |
| At one quarter | 337 µs | 133 µs |
| In the middle | 265 µs | 285 µs |
| At three quarters | 196 µs | 201 µs |
| At the end | 13 µs | 13 µs |
| One commit removing the middle child and putting one in its place | 229 µs | 136 µs |
| Moving the first child to the end (once) | 346 µs | 358 µs |
| `each` list of 100 000 rows, inserting or removing a row at the start (once) | 499 µs | 172 µs |

The middle, three quarters and "first to the end" write as many numbers as before; their differences are noise. Inserting and removing at the start of 10 000 siblings 29 µs → 9.3 µs, of 1 000 siblings 5.4 µs → 3.2 µs. Not solved: an edit in the middle still writes half the list (about 2 ns each, about 0.1 ms per edit among 100 000 siblings); moving a child from one end to the other shifts everything between by one, and those are still rewritten one by one (only the untouched start and end are recorded, not that the middle moved as a block); what is left of an insertion at the start is shifting the `Vec` (172 µs for `each` against 108 µs at the end).

`world/child_numbering.rs` holds this: with 4 000 children, each kind of edit asserts how many numbers it wrote (1 for an insertion at the start or the end, 0 for removing the first or the last, 1 001 at one quarter, 1 for a child replaced in place); and 40 random edit sequences (400 commits each, mixing insertions, moves within and across parents, removals in runs and scattered, and stretches reversed in one commit) check after every commit that every child is found at its index by its number, without the scanning fallback.

**The frame after one row of a long list changes (2026-11-11).** The paragraphs above measure the commit; the frame after it is the larger cost. `nana-dirty-frame-benchmark --shape layout --dirty 1` (each row a box and a label, one row's height changed), release, aarch64 macOS, minimum, old and new alternating over four rounds:

| Rows | First row changed, before | After | Last row changed |
| ---: | ---: | ---: | ---: |
| 1 000 | 1.31 ms | 1.00 ms | 0.044 ms |
| 4 000 | 6.45 ms | 4.78 ms | 0.13 ms |
| 16 000 | 33.9 ms | 27.6 ms | 0.45 ms |

When the first row changes, every row after it moves: about 1.7 µs per row now (two nodes), spread over layout (about half), hit testing (about a fifth), accessibility (a little over a tenth) and extraction, with no single hot spot in a sampling profile. What this round removed is hashing in the layout frontier: the dozen-odd tables keyed by node in `LayoutDependencyGraph`, `LayoutFrontier` and the dependency walk used the standard library's default SipHash, read or written about ten times per sibling moved, and every node's adjacency carried an index by target of its own (one more allocation). They all hash with `IdHasher` now, an adjacency is scanned up to 8 links and indexed past that, and the pass's per-node sets (`affected`, `measure`, `metrics_changed`) changed with them. A content-sized container (`layout-auto`, 4 000 rows) 6.54 ms → 5.01 ms; frames that move no sibling, a label or a background changed, are as they were (13 µs, 2.7 µs). The six accessors `LayoutFrontier::nodes()` and its kin therefore return `&NodeSet` where they returned `&HashSet<StableNodeId>`; no call in the repository had to change.

Changing the last row also costs by the row count, about 28 ns per sibling, and was left alone: in the profile 65% is the container's `LayoutResult` rebuilt whole (it holds a placement, a fragment and a part per child, the public contract of #197), 17% the accessibility projection walking the container's children, and 9% a flex container's dependency graph chaining every sibling. All three are a result that carries the whole child list; lowering them takes a structure that shares its prefix. A list of thousands of rows should be virtualised.

**`IdHasher` for the rest of `nana-ui-runtime`'s node tables: did not measure, reverted (2026-11-11).** Following the round above, the node-keyed tables in `store.rs` (the thirty-odd tables of `NodeStore`, `space_pins` / `page_bound`), `layout_engine.rs` (retained container plans, envelopes, line solves, inline objects, `AffectedIndex::reaching`, `consulted`, `derived_baselines`) and the std tables left in `framework.rs` (the commit path's focus / retired-document / deleted sets, the `removed` set through `forget_subtree`, virtual list and table `activity`, `focus_within`) were all switched to `IdHasher`, measured, and reverted. The reason first: `store.rs` and `layout_engine.rs` use `hashbrown::HashMap`, whose default hasher is foldhash, not SipHash, so the switch traded one folded multiply for one multiply; only those few `framework.rs` sites were still on SipHash, and they count the changes in one commit or the subtree removed, not nodes per frame.

x86_64 Windows, release, old and new binaries alternating:

- `nana-dirty-frame-benchmark --shape layout --position head --rows 4000 --dirty 1 --samples 100 --warmup 20`, 20 pairs: minimum 8.28 → 8.70 ms, median paired ratio 1.010 (p50 1.009). 16 000 rows over 8 pairs 0.973, `layout-auto` 4 000 rows 0.975, 1 000 rows 0.997, last row changed 0.989, `nested` 0.959 (0.020 → 0.019 ms), `paint` 1.000. The direction is mixed and the size is inside this machine's run-to-run spread (±10%).
- `nana-framework-benchmark` (p50), 6 pairs old first: virtual list materialize 0.957, virtual table materialize 0.955, 5 000-node layout 0.947; 6 pairs new first: 0.981, 0.985, 1.008. The 5% of the first set was run position; averaged over both orders it is 2–3%, still in the noise.

So these sites are no longer listed as work. To try again, first find a scene in `framework.rs` whose cost follows the node count (tearing down a subtree of tens of thousands of nodes) and measure it on its own.

**Nodes that were removed no longer slow every frame (2026-11-11).** A layout cause is queued per node (`pending_layout_invalidations`) and taken per document. Removing a node first records a topology cause for it, and then the node is gone; the drain for one document (`take_system_work_for`, which the product frame uses) cannot tell which document it belonged to and kept it as another document's. So every node ever removed left an entry in the table, and every frame emptied the whole table, looked each entry up and put it back: a frame cost by how many nodes the session had removed so far. A removed node now takes its cause with it, and a drain drops the causes of nodes no longer in the world (another document's still wait for its own). `perf/micro/stack-depth` has `BURN=n` (create and remove n nodes before building the page); one edit per frame on a 1 051-box page, least whole frame (release, aarch64 macOS):

| Nodes removed before | Width edit, before | After | Label text edit (controls page, 1 551 boxes), before | After |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 32.7 µs | 27.5 µs | 21.4 µs | 21.6 µs |
| 10 000 | 278 µs | 27.0 µs | 263 µs | 15.6 µs |
| 70 000 | 3 169 µs | 28.8 µs | 3 328 µs | 17.0 µs |
| 300 000 | 16 212 µs | 28.0 µs | 18 300 µs | 16.9 µs |

Before, every node removed added about 54 ns to each frame after it; an application that scrolls a virtualised list or opens and closes menus reaches hundreds of thousands in a while. `world/pending_causes.rs` holds two things: a removed node has no cause left in the table; and draining one document drops the causes of nodes no longer in the world while keeping another document's.

**Tables indexed by id follow the nodes in use, not the highest id (2026-11-11).** Node ids are handed out in order and never reused. The node table and the ten-odd layout tables (`DenseNodeMap`) index by raw id, one vector as long as the highest id seen: every id ever issued left about 0.5 KB of empty slots across them, and past id 65 536 every new node went to an overflow hash map. A table is now one vector from zero to begin with (`Slots` in `store/paged.rs`), and when the entries in use thin out to under an eighth of its slots the whole table moves to pages: 64 slots a page, only pages that hold something allocated, an emptied page given back, and a directory that covers no more than the lowest page in use to the highest. While dense, a lookup is still one bounds check and one slot, as the vector was (that half has to be small enough to inline wherever a node is looked up; written in one function with the pages it was left as a call and cold layout took 3% more instructions); only ids past `PAGED_IDS` (2²⁴) go to the hash map. The same round cleared two tables that kept an entry per removed node: the set of nodes whose layout result is withheld (`layout_results_suppressed`) and the memo of which boxes hold a line box (`line_boxes`, swept only when the `boxes` table passed twice the node count, though it grows on its own; each retained table is now swept by its own size).

A 10 501-box page, with n nodes created and removed before it is built (`BURN=n`, release, aarch64 macOS, minimum):

| Nodes removed before | Held per box, before | After | Cold layout per box, before | After | Whole frame for one row's height (1 051 boxes), before | After |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 2 457 B | 2 457 B | 1.22 µs | 1.23 µs | 57.7 µs | 57.6 µs |
| 30 000 | 4 794 B | 2 531 B | 1.35 µs | 1.39 µs | 75.4 µs | 62.0 µs |
| 70 000 | 7 715 B | 2 781 B | 1.75 µs | 1.34 µs | 115 µs | 59.6 µs |
| 300 000 | 9 213 B | 4 279 B | 1.71 µs | 1.31 µs | 134 µs | 62.9 µs |

(The held column includes what those n nodes left queued before any frame took it, which is why it still grows with n afterwards; the figure with frames running is below.) With no removals behind it nothing changes: cold layout instructions +0.6% (`reps 1000 5`, 4.93 G → 4.96 G), and the seven cells of `nana-dirty-frame-benchmark` within ±0.5% over six alternating rounds. Once a table is on pages small frames are a little slower: after 70 000 removals a width edit's whole frame 28.6 → 30.1 µs, a label's text 16.8 → 17.7 µs.

`nana-ui-scene/tests/session_churn.rs` holds the session-level amount: on a page of 40 rows, "add 10 rows, a frame, remove them, a frame" over and over; after 18 000 nodes have come and gone the heap holds no more than 4 bytes per node more (what is left is each table's page directory, 8 bytes per 64 ids, under 2 bytes a node in all). Before, the same test left about 41 bytes per node, on top of the 0.5 KB of empty slots above. With either of the two clearings removed the test goes red (8 and 28 bytes a node).

**The live heap is not the physical memory the process is charged with (2026-11-11).** "Resident per box" in the tables above is the live heap throughout: allocations less frees, in requested bytes. `FOOTPRINT=1 target/release/nana-stack-depth reps <rows> 1 [taffy]` reports beside it what the process's physical footprint (`phys_footprint`, the number Activity Monitor shows) grew by over the same stretch. After building the page and one cold layout, per box (release, aarch64 macOS, the range over repeated readings):

| Page | NanaUI live heap | NanaUI physical | Taffy live heap | Taffy physical |
| ---: | ---: | ---: | ---: | ---: |
| 10 501 boxes | 2 323 B | 3 490–3 680 B | 1 507 B | 1 175 B |
| 21 001 boxes | 2 319 B | 3 340–3 550 B | 1 507 B | 1 020–1 070 B |
| 67 201 boxes | 2 576 B | 3 660–3 740 B | 945 B | 942 B |

The two numbers differ in two directions:

- **Capacity nothing has written to takes no physical memory.** The unwritten tail of a table that grew by doubling is address space only; that is why Taffy's physical footprint is below its live heap. The node table's empty slots, listed under "To do" for several rounds (21 001 nodes in 32 768 slots, about 140 B / box), are of this kind: with the node table reserved to the exact count (a temporary change), the live heap drops by 144–243 B per box and the physical footprint by 55–185 B (single readings; the same configuration varies by about 100 B from run to run). It is no longer a memory item.
- **Large blocks that were freed are still charged to the process.** At 21 001 boxes `vmmap -summary` reports 48.5 MB in use in the heap and 53.5 MB dirty (5.1 MB of fragmentation, 10%), and another 14.8 MB of large blocks already freed (`Malloc Large (empty)`): the system allocator keeps them, and with the page held for 30 seconds the footprint does not move (69 MB). Those blocks are the working tables of one cold layout pass, allocated in the pass and freed at its end (named by allocation stack): the pass's measurement memo `PassIntrinsicCache` (reserved for twice the node count, 7.7 MB and 2.4 MB, about 480 B / box); the `MutationQueue` layout is written back through (one `PlaceLayout` per node, 240 B per `UiMutation`, a vector doubling up to 7.9 MB, about 370 B / box); the pass's hash table of container plans (3.1 MB); the node input table (1.2 MB). A pass frees about 1.6 KB of large blocks per box, and the build another 0.7 KB or so (mostly the node table's earlier copies: its records are aligned to 64 bytes, for which Rust's `realloc` allocates anew, copies the whole block and frees the old one). They are not a leak, and later allocations can use them again, but the process's physical footprint stays at that high-water mark: 45% or so above the live heap.

So the gap to Taffy is 3 to 4 times by physical footprint, and 1.5 to 2.7 times by live heap. What there is to bring down is a pass's working tables, not the retained ones; see "To do".

**Three of a cold layout pass's working tables went (2026-11-11).** A table a pass fills only to copy into a retained one and free is memory needed on top of the page. Three of them are gone:

- **Layout is no longer written back through `UiMutation`.** A first layout queued one 240 B `PlaceLayout` per node; it now hands over a batch of 40 B placement records (`LayoutPlacement`: id, box, frame), which the commit path applies one by one in the same order. Validation (the whole batch is refused when a node is missing or a box is invalid), the report and the world state are the same as for the queued form (the three tests in `world::placement_commit` compare the two item by item).
- **The pass's table of container plans goes to the retained cache whole.** When the retained table is empty (a first layout), it takes the pass's table instead of copying it entry by entry and freeing it; later scoped passes merge as before.
- **The end of a pass no longer collects every measurement fact into a list** before retaining them; they are read where the pass holds them.

Besides, `text_nodes` went from a hash table to a dense table indexed by id (as the node records are): the hash table held an entry per node, and each time it grew it built a new table beside the old one, which the allocator then kept.

After building the page and one cold layout (`FRAME_PATH=1`, the way the product's first frame runs), physical footprint per box (release, aarch64 macOS, the range over repeated readings):

| Page | Before | After |
| ---: | ---: | ---: |
| 5 251 boxes | 3 650–4 197 B | 3 264–3 482 B |
| 21 001 boxes | 3 647–3 779 B | 3 114–3 362 B |
| 67 201 boxes | 3 945–3 967 B | 3 216–3 224 B |
| Controls page, 31 001 boxes | 6 480–6 771 B | 5 618–5 657 B |

The live heap does not change, except on the controls page, from 4 309 B to 4 119 B per box (`text_nodes`). By allocation stack, the large blocks a pass frees go from 1 576 B to 1 048 B per box, and the build's from 715 B to 532 B; the peak while laying out 21 001 boxes goes from 59.2 MB to 54.2 MB. `first_layout_working_memory` in `nana-ui-scene` holds this: in the first layout of 5 001 nodes, the most a pass holds beyond what it leaves goes from 466 B to 150 B per node, and the test allows at most 300 B (the queued write-back alone was about 390 B).

Time went down too: `nana-stack-depth wide` alternated with the previous build, 6 rounds each, minimum read. Cold layout goes from 1.09 to 0.97 µs per box at 21 001 boxes, from 1.22 to 1.02 µs at 42 001, and from 1.35 to 1.12 µs on the 31 001-box controls page; build plus layout from 2.36 to 2.21, 2.52 to 2.29 and 3.29 to 2.97 µs. The single edits of `edits` (4 201 and 42 001 boxes, width and height) stay within run-to-run variation for both layout and the whole frame.

**The pass's measurement memo: reserving twice the node count costs nothing, the size of each fact does (2026-11-11).** The facts a pass actually writes, 31 001 for 21 001 nodes on the wide page and 61 001 for 31 001 nodes on the controls page, fit in the capacity reserved for twice the node count, and the unwritten tail takes no physical memory. Starting empty and growing with use instead (a temporary change) leaves the wide page's physical footprint where it was and adds 30 B per box at 67 201 boxes and 125 B per box on the controls page (old blocks left by growth copies), so the reservation stays. What can come down is the size of each fact (`PassFact`): it held both what lookups answer and the recording handed to the retained cache, 64 B each, and the first is the second normalized with its generation cleared; on both pages not one fact had them differ. A fact now holds one copy, and a recording that differs goes to a side table (one place per fact at most). Each fact goes from 184 B to 128 B, held by a compile-time assertion; a test in `intrinsic_cache` takes the two-copy form as its reference and compares lookups and the recordings handed over at every step of 4 000 mixed writes (including recordings that differ from the answer and keys that normalize to another), and both mutations tried turn it red. Physical footprint (alternated with the previous build, 6 rounds each): 3 110 → 3 070 B per box at 21 001 boxes, 3 178 → 3 138 B at 67 201, 5 540 → 5 389 B on the 31 001-box controls page; the live heap, cold layout and edit times do not change.

**Two retained duplicates (2026-11-11).** After attributing the heap by allocation stack again (`HOLD=1` with `malloc_history -allBySize`, 21 001 boxes, product first frame, 2 731 B per box in all), two changes, neither touching a public interface:

- **Dense tables' slots no longer hold the id.** Each slot of a `DenseNodeMap` held `(StableNodeId, V)`, though the slot's index is the id. Since ids became `NonZeroU64` the `Option` tag uses their zero, but the 8 B id and its alignment padding remained: `far_start` (2 B) and `contexts` (1 B) took a 16 B slot each, `placements` 24 → 32, `boxes` 20 → 24, the intrinsic table 112 → 120, `text_nodes` 144 → 152. A slot now holds the value alone, and iteration rebuilds the id from the index (items go from `(&id, &V)` to `(id, &V)`; no read or write site changes). That is what "merging the retained tables into one" under "To do" was to save: the five small tables' values add up to 55 B, held apart without ids they take 67 B, merged into one record about 60 B, so merging would save little more while changing many read and write sites, and they stay apart. The randomized check of `paged` now compares indices too, for retaining and iterating by value.
- **A `Stack`'s style is shared with its node's record.** The component view `Box<Stack>` held a 192 B `NodeStyle` by value, beside the interned `Arc<NodeStyle>` in the node's record. `Stack` now holds an `Arc<NodeStyle>` (a private field; builders write through `make_mut`); on creation it goes to the world's style interner as layouts do (`ComponentView::share_style`, a hidden default method) and is replaced with an equal one written recently, and the record written from it then finds that very `Arc`. The interner compares the layout by pointer and the other fields one by one (`==` on `Arc<LayoutStyle>` compares the same layout deeply again; the destructuring names every field, so an added field fails to compile). The public `style` fields of `Text`, `Button` and the like are untouched. A `framework` test checks that three equal cells and their records point to one style and a different one has its own; dropping the sharing on creation turns it red.

Together (alternated with 2feb0e3a7, 5 rounds each, `FRAME_PATH=1`, release, aarch64 macOS):

| Page | Live heap, before | After | Physical, before | After |
| ---: | ---: | ---: | ---: | ---: |
| 21 001 boxes | 2 456 B | 2 214 B | 3 073–3 082 B | 2 812–2 813 B |
| 67 201 boxes | 2 713 B | 2 469 B | 3 138–3 158 B | 2 830–2 832 B |
| Controls page, 31 001 boxes | 4 119 B | 3 944 B | 5 392–5 402 B | 5 158–5 163 B |

The dense tables account for −58 to −91 B per box of live heap and `Stack` for −184 B (wide page; the controls page's leaves are `Text` and `Button`, −96 B). Cold layout time per box does not change; build plus layout is level on the wide page (2.27 against 2.28 µs at 21 001 boxes, 2.35 against 2.33 µs at 42 001) and goes from 3.00 to 3.05 µs on the controls page (8 alternated rounds, minimum). The cost is in the `Stack` change: each `Stack` created makes one more `Arc` allocation and frees one after interning, and `reps 1000 5` retires 2.3% more instructions on the wide page (about 1.1k per box, half from holding the style in an `Arc`, half from sharing it on creation) and 0.7–1.1% more on the controls page; the dense tables change no instruction count. Incremental edits (`edits`) stay within run-to-run variation for layout and the whole frame.

**Children's boxes are no longer copied into the parent's result (2026-11-11, breaking).** A parent's `LayoutResult` held three copies of each child's local box: one in `child_placements` (32 B), a child fragment at the head of `fragments` (80 B) and a `ChildPlacement` part in `parts` (32 B), 144 B per child, and whenever a child moved the parent's result was rebuilt, published, marked RENDER and extracted again. All three can be derived from the child's own record at any time, and only layout's own tests and two Scene tests read them. A result now holds only the node's own fragments and parts; children's placements and fragments are read from the children when `UiWorld::layout_child_placements` / `layout_child_fragments` (page coordinates) or their `local_` versions (the parent's local coordinates) are asked. Deciding at publish whether a parent's result still holds compared every child placement; it now only checks that the covered children are still this node's, have this node as their reference frame and stay inside `overflow`; a child measured from another frame joins the parent's `overflow` only on the page, and the result placed on the page is cached by the local result's identity, so when this publish covers such a child and the parent's local result is the same but its `overflow` on the page is not, the parent is published and extracted anew (an equivalent recompute with the escaping child unmoved still keeps the result; a `fixed` child is covered on every pass). A `world` test checks that the parent's local result pointer stays the same while a child moves inside it, and that `overflow` grows when the child reaches out and shrinks back; dropping the "box still inside the overflow" check turns it red. An issue200 test hangs a child from an outer containing block so that, measured from there, it lies inside its parent's box while on the page it is above the parent, and narrows the block so the child follows; dropping the "this node is the reference frame" check or the comparison on the page turns it red (the first version missed the latter, and three issue200 tests caught the stale `overflow` on the page; the second republished unconditionally, and the equivalent-recompute tests of issue257 and issue384 caught the extra publishes).

To migrate: `result.child_placements` → `world.layout_child_placements(id)` (or `local_layout_child_placements`); child fragments picked out of `result.fragments` by `node.is_some()` → `world.layout_child_fragments(id)`; `LayoutPartKind::ChildPlacement` is gone, and `parts` holds only the node's own parts.

Alternated with 28d4867bf, 5 rounds each (release, aarch64 macOS, minimum; editing one row is `edits`, in µs):

| Page | Live heap, before | After | Physical, before | After |
| ---: | ---: | ---: | ---: | ---: |
| 21 001 boxes | 2 214 B | 2 006 B | 2 765–2 811 B | 2 540–2 550 B |
| 67 201 boxes | 2 469 B | 2 261 B | 2 785–2 831 B | 2 571–2 572 B |
| Controls page, 31 001 boxes | 3 944 B | 3 580 B | 5 151–5 158 B | 4 695–4 703 B |

| Measure | Before | After |
| --- | ---: | ---: |
| Cold layout per box, 21 001 / 42 001 boxes | 0.97 / 1.03 µs | 0.91 / 0.97 µs |
| Cold layout per box, controls page 31 001 boxes | 1.12 µs | 1.05 µs |
| Build plus layout per box, 21 001 boxes / controls page | 2.27 / 3.05 µs | 2.20 / 2.96 µs |
| 42 001 boxes, width edit: layout / frame | 89.2 / 102.9 | 54.6 / 67.4 |
| 42 001 boxes, height edit: layout / frame | 348.9 / 968.4 | 323.9 / 939.9 |
| 4 201 boxes, width edit: layout / frame | 22.2 / 33.8 | 18.0 / 29.0 |

## To do

- **Cold layout throughput and per-box memory**: after the fourth round on 2026-11-09 (see the fourth round of cold layout under "CPU time"), 21 001 boxes cost 1.24 µs per box of layout, 2.52 KB resident and 23.3k layout instructions. Building no plans on the first layout (including only for single-child containers) breaks the incremental contracts and is no longer listed: doing it first needs those contracts to reach zero measurement without plans. Keeping the scoped pass's node input table capacity across passes was not done this round: since the third round's fix an edit takes 0–2 page faults, and what remains, the table's growth copies, does not show in the measurements; merging the retained tables into one was not done either (an estimated 30–60 B per box, with many read and write sites to change; on 2026-11-11 dense tables' slots stopped holding the id, which is what merging was to save, so the tables stay apart, see "Two retained duplicates"). What follows is the record after the third round. After the third round on 2026-11-06 (see the third round of cold layout under "CPU time" and "Resident memory"), the 21 001-box wide tree (product first frame) costs 1.24 µs per box for layout, 2.44 µs for tree building plus layout, and 2.54 KB resident; Taffy 0.70 µs, 0.77 µs, 1.5 KB; instructions per box 47.7k against 17.5k (tree building 22.9k, layout 24.8k). Layout hot data already sits in dense tables indexed by node, and a small tree (everything in cache) still costs 1.03 µs per box; what remains is what each node does, not where the data lives: every container builds measurement and container plans on every pass (on the first layout for the next frame; `MeasuredChild` / `PlannedChild` is one entry with an `Arc` per child), intrinsic facts and the used-size memo by `MeasurementKey` (a 56 B key each), line box and baseline answers, and publishing a `LayoutResult` after layout (about a fifth of layout). Possible next steps: do not build plans on the first layout and build them on demand in the first scoped pass instead (first proving that the scoped pass rebuilds the same plans that are left now); merge the retained intrinsic, box, padding, and context tables into one table indexed by node (each slot carries a 16 B id and tag, saving about 60 B / box in total; replaced by dense tables no longer holding the id); the scoped pass's node input table still grows by doubling to the page's highest id and is reallocated every frame when the edit is near the top of the page (that was already so before the change, about 4 MB for the 62 001-node page), so capacity could be kept across passes or it could become hashed; changing the line box memo to a dense table was reverted (17% slower on the page with an open Select, cause not found). The rest as before: the node table's empty slots (21 001 nodes sit in 32 768 slots of 256 B, about 140 B / box; measured on 2026-11-11, slots nothing has written to take no physical memory, see "The live heap is not the physical memory the process is charged with", so this is no longer listed); one `text_nodes` entry per node (190 B / box; building on demand needs all write sites changed to "get or build" and proof that a box with no entry is equivalent to one with an empty stamp); the `NodeStyle` stored by value in component views (224 B per `Box<Stack>`; on 2026-11-11 `Stack` came to share one with its record, while the public `style` fields of `Text` and the like are untouched); cells whose layout is written with a `with_layout` closure still copy once and then compare; `LayoutResult` stores one fragment and part per child (removed on 2026-11-11; `UiWorld` gives them on demand, see "Children's boxes are no longer copied into the parent's result").
- **The working tables of a cold layout pass**: after building a page and one cold layout, the process's physical footprint is about 45% above the live heap (21 001 boxes: 3.4 KB per box against 2.3 KB); the excess is large blocks a pass allocates and frees, see "The live heap is not the physical memory the process is charged with". On 2026-11-11 the `UiMutation` queue for writing layout back, the copy of the pass's container plan table and the list collecting measurement facts went (see "Three of a cold layout pass's working tables went"), bringing 21 001 boxes to 3.1–3.4 KB physical per box. The measurement memo's reservation for twice the node count was measured and costs nothing, and each fact went from 184 B to 128 B (see "The pass's measurement memo"). The large blocks that remain: the node input table (1.2 MB); the node table's earlier copies left by doubling during the build (its records are aligned to 64 bytes, so `realloc` cannot grow them in place). Only measured with the system allocator on macOS.
- **The frame after mounting deep**: it is linear now (see the two sections "The frame after mounting" and "After mounting twice" under "CPU time"): the frame for `under-deep` 100 / 200 / 400 is 2.9 / 3.3 / 4.1 ms, and `deep` 1.24 / 2.03 / 3.67 ms. What remains is mostly layout: when the chain contains a word wider than its box, the mounting frame lays out twice (text is shaped first with no box, and only after layout is it known to be unbreakable), which is also why mounting the same tree in one go (`under-deep-at-once`) is slower than mounting it in two steps; in-place hit rebuild costs about 0.6 µs per entry, twice a fresh build. When many roots are mounted at once under a scrolled index, every patch root still walks the ancestors for a translation (patching changes the index, so skip records cannot be reused across patches).
- **Mobile devices**: the iOS and Android branches of `stack` have only been cross-`cargo check`ed (`aarch64-apple-ios`, `aarch64-linux-android`, the latter after `source scripts/android-env.sh`), and the deep-tree gates have not been run on a device.
