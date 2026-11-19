---
title: Runtime Retained Tree
---

# Runtime retained tree: `UiWorld`

This chapter goes into the heart of `nana-ui-runtime`: **how the `UiWorld` retained tree stores nodes, how it receives batched mutations atomically, and how it turns dirty marks into deterministic system work within a frame**. After reading it you will understand: what the dense table and sparse side tables behind a `StableNodeId` look like; how `UiMutation` and `MutationQueue` are validated as a batch at the commit boundary; how `ComponentRegistry` unifies built-in and extension controls; and the strict order of the phases (`style → text → layout → hit-test → a11y → extract`) when the host drives `RuntimeDocument::flush`. This chapter follows [Modules and Layers](/learn/modules-and-layers) and is the common foundation for [Layout Engine](/learn/runtime-layout) and [Input Path](/learn/runtime-input).

## Responsibilities and boundaries

- **The single retained-tree authority**: `UiWorld` (`crates/nana-ui-runtime/src/world.rs:628`) is the authority for the identity, topology, and logical state of all retained nodes. The internal node table (`NodeStore`) is deliberately kept out of the public API, so optimizing the underlying storage does not break diagnostic records, snapshots, or persisted data.
- **Atomic batched mutation**: outside code cannot tamper with the node table directly. It can only enqueue `UiMutation`s into a `MutationQueue`, which `UiWorld::commit` validates as a whole batch and then applies in one step (`crates/nana-ui-runtime/src/world/mutation.rs:2915`).
- **A clear line between authority and projection**: layout output (`LayoutResult`) is a projection, and the in-frame `MutationQueue` is a staging buffer awaiting commit. Neither is a second tree authority.
- **Zero CSS parsing at runtime**: the dependency table of `nana-ui-runtime` contains no `nana-ui-css`. Styles always enter as Style Model data (`LayoutStyle`) (see [Dependencies and neighbors](#dependencies-and-neighbors) below).

## Dependencies and neighbors

`nana-ui-runtime` sits at level 5 (L5). Required dependencies point downward to `nana-ui-core`, `nana-text`, `nana-ui-input`, `nana-ui-view-schema`, and `nana-diagnostics`, plus optional feature-gated `nana-ui-charts`, `nana-ui-diagram`, and `nana-ui-view-macros`. It **does not depend** on `nana-ui-css` or any GPU crate:

| Neighbor crate | Relationship |
| --- | --- |
| `nana-ui-core` | Provides node style, theme, and geometry contracts (`LayoutStyle` inside `NodeStyle`, `CompiledTheme`, and others) |
| `nana-text` | Provides text node measurement and the retained text layout authority; `TextLayoutStore` is attached in `NodeStore` |
| `nana-ui-scene` | `RuntimeDocument` holds an `AppContext` and a `UiScene` and drives the bounded per-frame `flush` transaction (`crates/nana-ui-scene/src/runtime_document.rs:18`) |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `UiWorld` | `crates/nana-ui-runtime/src/world.rs:628` | The single retained-tree authority: node table + dirty mark sets + generation numbers + indexes and caches |
| `StableNodeId` | `crates/nana-ui-runtime/src/world.rs:70` | Stable external node identity (`u64` wrapper, `0` reserved for empty, destroyed IDs are never reused) |
| `NodeStore` | `crates/nana-ui-runtime/src/store.rs:379` | Dense node table + sparse side tables (replaces the earlier `bevy_ecs::World`) |
| `NodeRecord` | `crates/nana-ui-runtime/src/store.rs:81` | Dense per-node record: `hierarchy`, `style`, `resolved_layout`, `layout`, `interaction`, `dirty`, and more |
| `Hierarchy` | `crates/nana-ui-runtime/src/store.rs:46` | `parent` / `children: Arc<Vec<StableNodeId>>` / `index_in_parent` / `children_base` |
| `NodeKind` | `crates/nana-ui-runtime/src/world.rs` | Four node kinds: `Document` / `Element` / `Text` / `Comment` |
| `UiMutation` | `crates/nana-ui-runtime/src/mutation.rs:15` | Enum of atomic retained-tree mutations (covering create, insert, unmount, style, text, focus, animation, and more) |
| `MutationQueue` | `crates/nana-ui-runtime/src/mutation.rs:306` | Frame-local command queue written by declarative views, component assemblers, and host adapters |
| `CommitReport` | `crates/nana-ui-runtime/src/world.rs:388` | Statistics receipt for one `commit` (created / inserted / detached / reparented / despawned) |
| `ComponentRegistry` | `crates/nana-ui-runtime/src/component_registry.rs:249` | Unified component type registry: three indexes `by_id` / `by_tag` / `by_rust` |
| `DirtyMask` | `crates/nana-ui-runtime/src/schedule.rs:9` | 9-bit per-node invalidation mask (`STYLE`, `TEXT`, `LAYOUT`, `INPUT`, `FOCUS_IME`, `RENDER`, `ACCESSIBILITY`, `STATE`, `TRANSFORM`) |
| `SystemWork` | `crates/nana-ui-runtime/src/schedule.rs:64` | Deterministic phased work list produced by one `take_system_work` |
| `AppContext` | `crates/nana-ui-runtime/src/framework.rs:1072` | Holds typed component state and event listeners; the underlying authority is still `UiWorld` |
| `RuntimeDocument` / `RuntimeFrameUpdate` | `crates/nana-ui-scene/src/runtime_document.rs:18` / `:26` | Wrapper for a document + incremental `UiScene` transaction, and the envelope for a single frame's update result |

## Internal structure

### 1. Node identity and storage: dense array + sparse side tables (`NodeStore`)

`StableNodeId` (`crates/nana-ui-runtime/src/world.rs:70`) is the single external node identifier (`0` maps to `None`). It pairs with a dedicated lightweight integer-spreading hash `IdHasher` (`:95`), so high-frequency animation lookups do not pay SipHash cost for increasing integer IDs.

`store.rs` uses a two-tier layout: **"hot fields go in the dense table, feature-specific fields go in sparse side tables"** (`crates/nana-ui-runtime/src/store.rs:300-445`):

- **Paged storage `Slots<T>` (`store/paged.rs:267`)**: Backing tables use `Slots<T>`. Contiguous mounts run directly over a linear run; when sparse unmounts drop live occupancy below 1/8 of the ID span, entries migrate automatically into 64-slot `Paged` pages and emptied pages are immediately freed, keeping wide sparse heaps at `943 B/box` (`634 B/box` for a 10k `Stack`).
- **`NodeRecord` (`:81`, `<= 152 B`) and resolve-on-write**: stores each node's most frequently accessed fields, including `hierarchy` (`24 B`, with `index_in_parent` counted from the parent's `children_base` to avoid linear sibling scans), `mount: MountState`, the author's original `style: NodeStyle`, and the pre-resolved `resolved_layout: Arc<LayoutStyle>`. Because `LayoutStyle` is about 4.8 KB, it is resolved once on write or theme switch and shared via `Arc` (`:87`).
- **Sparse side tables (remaining `NodeStore` fields, `:379-445`)**: store `visuals`, `custom_render`, `event_listeners`, `component_types`, `overlay_hosts`, `editors`, and separate `text_nodes` / `text_layouts` tables on demand.
- **Heap-iterative mount trampoline `ViewBuilder::run_trampoline` (`view/node.rs:491`)**: mounts deeply nested declarative views via an explicit heap task queue (`ViewTask::Build` / `ViewTask::LeaveEl`) rather than growing native call stack frames with tree depth.

### 2. Batched mutation commit: `UiMutation` / `MutationQueue` / `commit`

Every change to the tree is expressed as a variant of the `UiMutation` enum (`crates/nana-ui-runtime/src/mutation.rs:15-330`):

- **Topology operations**: `Create`, `Insert` (also used to move; `before = None` means append at the end), `ReorderChildren` (put the children a parent has into a given order in one edit, at the cost of the list's length rather than its length per move), `Detach` (leave the parent but stay `Mounted`), `ParkSubtree` (deactivate and hide a whole subtree while keeping node identity and internal state), `DespawnSubtree` (destroy completely; the IDs are retired permanently).
- **Style, theme, and text**: `SetStyle`, `SetPresetTheme`, `SetThemeTokens` (carries an `Arc<CompiledTheme>` as a whole), `SetText`, `SetRichText`, `SetLanguage`, `SetLocale`, `SetLocalizedText`.
- **Layout write-back and scrolling**: `WriteLayout` (only for the layout engine writing back results and for tests), `PatchPlacement`, `SetScrollOffset`.

`MutationQueue` (`:306`) collects the current frame's mutation sequence. When `UiWorld::commit` (`crates/nana-ui-runtime/src/world/mutation.rs:2915`) is called, it first validates the integrity of the whole queue (catching `UiWorldError`s such as duplicate IDs, retired IDs, cross-document references, parent-child cycles `Cycle`, and exceeding the maximum depth `TreeTooDeep`). **Only when everything passes is it written atomically into `NodeStore`**. A half-built tree is never left behind.

### 3. Three generation counters and precise invalidation isolation

`UiWorld` maintains three independent monotonic generation counters, so each kind of change wakes only the downstream consumers that actually care about it:

1. **`generation`**: incremented on any authoritative state change. The host uses it to tell whether the world has changed.
2. **`layout_generation` (`world.rs:731`)**: incremented only when layout geometry output actually changes. Pure paint or compositor animation frames do not increment it, so consumers that depend on layout geometry (Scene geometry, hit testing, accessibility bounds) are not woken by pure color or opacity changes.
3. **`layout_source_epoch` (`world.rs:735`)**: incremented only when **layout inputs** (style, text content, tree structure) change. When the layout engine writes computed results back to the tree through `WriteLayout`, **it is left unchanged** (`world/mutation.rs:2704`), so "writing back layout results" is not mistaken for "layout inputs changed again", which would cause an endless relayout loop.

### 4. `ComponentRegistry`: one registry for built-in and extension controls

`ComponentRegistry` (`crates/nana-ui-runtime/src/component_registry.rs:119`) maintains three indexes: `by_id`, `by_rust`, and `by_tag`. Static identities and tag metadata for built-in controls come from `component_descriptors::BUILTIN_COMPONENT_IDS`, while extension controls registered with `register_component` implement the `RegisterableComponent` trait (`:113`), providing `TYPE_ID` and `TAGS`; each type's behavior (`ComponentView::BEHAVIOR`) is installed on demand when first created. `UiWorld` also maintains `nodes_by_component: HashMap<ComponentTypeId, HashSet<StableNodeId>>` (`world.rs:894`), which keeps global queries by component type at $O(1)$.

### 5. `DirtyMask`, `SystemWork`, and the convergence order of `RuntimeDocument::flush`

Each `commit` marks a 9-bit `DirtyMask` on affected nodes (`crates/nana-ui-runtime/src/schedule.rs:9`: `STYLE`, `TEXT`, `LAYOUT`, `INPUT`, `FOCUS_IME`, `RENDER`, `ACCESSIBILITY`, `STATE`, `TRANSFORM`).

When the host calls `RuntimeDocument::flush` (`crates/nana-ui-scene/src/runtime_document.rs:109`), the frame transaction advances in this strict order:

1. **Drain reactive and async tasks**: `AppContext::take_system_work` (`framework/frame.rs:13`) first runs `poll_tasks()` and `flush_reactive()`, then sorts `SystemWork` out of the sorted dirty entities from `UiWorld::take_system_work` (`world.rs:2286`).
2. **Style → text shaping → incremental layout loop (capped at `MAX_FRAME_PASSES = 8`)**: runs `resolve_styles` → `shape_text` → merge the new layout seeds produced by text → `layout_document_for_viewport` / `layout_document_with_frontier` in order, and when needed runs at most `POST_LAYOUT_SHAPE_PASSES = 4` rounds of "reshape after layout fixes widths → layout again" feedback until it converges (if it has not converged after 8 rounds, it returns `FrameworkError::FrameDidNotSettle` and restores the dirty work).
3. **One-shot finish (after Settle)**: once layout has fully converged, it runs in one pass: hit-test index update (`apply_hit_test_work`) → incremental accessibility projection (`project_accessibility_delta`) → render extraction (`extract_nodes` + `scene.apply_delta`) → compositor presentation binding (`apply_presentation`).

## Collaboration and data flow

```text
Application / view functions / components ──write──▶ MutationQueue (in-frame staging)
                                         │ UiWorld::commit (whole-batch validation + atomic apply, world/mutation.rs:2915)
                                         ▼
                                 UiWorld (NodeStore + DirtyMask + LayoutFrontierSeed)
                                         │ RuntimeDocument::flush → AppContext::take_system_work
                                         ▼
                         SystemWork { style, text, layout_seeds, input, a11y, render }
                                         │ resolve_styles → shape_text → layout_document_*
                                         ▼
                         extract_nodes → UiScene::apply_delta (see the Scene Extraction chapter)
```

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-runtime/src/world.rs` | The `UiWorld` body, `StableNodeId` / `IdHasher`, `CommitReport`, `take_system_work` |
| `crates/nana-ui-runtime/src/store.rs` | `NodeStore`, `NodeRecord`, `Hierarchy`, and all sparse side tables |
| `crates/nana-ui-runtime/src/store/paged.rs` | `Slots<T>` and 64-slot sparse-compressed pages (immediate empty-page reclamation) |
| `crates/nana-ui-runtime/src/view/node.rs` | `ViewBuilder::run_trampoline` heap-iterative view mounting trampoline |
| `crates/nana-ui-runtime/src/mutation.rs` | The `UiMutation` enum vocabulary and `MutationQueue` |
| `crates/nana-ui-runtime/src/world/mutation.rs` | `UiWorld::commit` / `commit_with_mount_lifecycle`, scroll offsets, and layout result publishing |
| `crates/nana-ui-runtime/src/component_registry.rs` | `ComponentRegistry`, `RegisterableComponent`, `ComponentTypeId` |
| `crates/nana-ui-runtime/src/schedule.rs` | The 9-bit `DirtyMask` invalidation mask and `SystemWork` |
| `crates/nana-ui-runtime/src/framework.rs` | `AppContext` (typed component view state and event table) |
| `crates/nana-ui-runtime/src/framework/frame.rs` | Phased frame driver entry points: `resolve_styles`, `shape_text`, `layout_document_*` |
| `crates/nana-ui-runtime/src/world/extraction.rs` | `extract_nodes` (incremental dirty-node extraction) and `extract_document` (full snapshot) |
| `crates/nana-ui-scene/src/runtime_document.rs` | `RuntimeDocument::flush`: the bounded per-frame convergence transaction |

## Further reading

- [Layout Engine](/learn/runtime-layout): how the `LayoutFrontierSeed`s produced by `commit` are consumed in incremental layout
- [Input Path](/learn/runtime-input): how pointer and keyboard events are routed after `SystemWork.input_hit_test` is updated
- [Scene Extraction](/learn/scene-extraction): how the `ExtractedNode`s produced by `extract_nodes` are composed into `UiScene`
- [Contracts and Invariants](/learn/contracts-and-invariants): core architecture red lines such as "there is only one authoritative tree"
