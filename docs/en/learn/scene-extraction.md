---
title: Scene Extraction
---

# Scene extraction: from `UiWorld` to the retained drawing graph

This chapter goes into `nana-ui-scene`: **how a batch of dirty nodes in the retained tree becomes `UiScene`, a retained drawing graph that updates incrementally and is completely neutral to the graphics backend**. After reading it you will understand: what an `ExtractedNode` contains, and who maintains each of the four elements of a `ScenePrimitive` (position, transform, shared clip chain, total-order key); why the three-tier incremental invalidation rules of `apply_delta` are designed the way they are; how the `FramePlan` frame graph compiles one frame's render program without depending on any GPU type; and why hit testing and accessibility stay in `UiWorld` instead of moving into Scene. This chapter follows the `flush` transaction in [Retained Tree: UiWorld](/learn/runtime-tree) and is the direct upstream of [GPU and Frame Lifecycle](/learn/gpu-frame).

## Responsibilities and boundaries

- **The backend-neutral drawing graph authority**: `lib.rs` of `nana-ui-scene` (`crates/nana-ui-scene/src/lib.rs:1`) positions it as a "Backend-neutral render scene and frame graph". It holds no application business state, window handles, GPU devices, or render pipeline objects. Actual GPU painting is done by `SceneWgpuPainter` in `nana-ui`.
- **Why `nana-ui-scene` does not depend on WGPU**: this is a core architecture contract. All outputs of the drawing graph (`ScenePrimitive`, `FramePlan`, `RenderGraph`) are pure data structures: no texture handles, no render pass objects, no hardware pixel format enums. Resource contents and bindings are resolved by the painter only at consumption time ("a texture update does not compile a graph", `composition.rs:8`). This keeps GPU-free unit tests, scene snapshot comparison, and cross-backend evolution entirely free of any graphics API.
- **Retained incremental structure**: `UiScene` retains all primitives in a `BTreeMap` keyed by `PrimitiveId`. Each frame it rebuilds primitives only for dirty subtrees, and pure scroll changes take a linear translation fast path (`crates/nana-ui-scene/src/scene.rs:1672`).
- **What it explicitly does not do**: it does not parse CSS, does not touch WGPU, and **does not do pointer hit testing or accessibility tree projection** (both need the layout and component semantics authority and are already done by `UiWorld` in the first half of `RuntimeDocument::flush`).

## Dependencies and neighbors

In the dependency levels: `core(L1) → text/css/charts/input/window/gpu(L2) → runtime(L5) → scene(L6) → nana-ui(L7)`. `nana-ui-scene` depends on `nana-ui-runtime`, `nana-ui-core`, `nana-ui-charts`, and `nana-diagnostics`. It **does not depend** on `nana-gpu` or `nana-ui-css`:

| Neighbor crate | Relationship |
| --- | --- |
| `nana-ui-runtime` | `UiWorld::extract_nodes` produces `ExtractedNode`s (`crates/nana-ui-runtime/src/world/extraction.rs:652`) |
| `nana-ui` | `SceneWgpuPainter` consumes the `UiScene` primitive sequence, `frame_plan()`, and the `instance` generation number (see [GPU and Frame Lifecycle](/learn/gpu-frame)) |
| `nana-ui-core` | Provides low-level geometry primitives and packed Motion descriptor data |
| `nana-text` | Retained text layout handles are already loaded into `ExtractedNode.text_layout` (`crates/nana-ui-runtime/src/components.rs:4555`) |
| `nana-diagnostics` | `flush` records extraction and `apply_delta` time under `FrameStage::Extract`; failing to converge in 8 rounds is recorded as `FLUSH_DID_NOT_SETTLE` (`runtime_document.rs:203`) |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `ExtractedNode` | `crates/nana-ui-runtime/src/components.rs:4514` | Per-node extraction snapshot (shared `Arc<NodeKind>` and `Arc<ComputedStyle>`, layout box, text layout handle, custom recording, and more) |
| `UiScene` | `crates/nana-ui-scene/src/scene.rs:1357` | Retained drawing graph: node table + primitive table + total-order table + visibility and frame plan caches, with a monotonically increasing `instance` |
| `ScenePrimitive` | `crates/nana-ui-scene/src/scene.rs:1257` | A single drawing primitive: `id`, `bounds`, `transform`, shared clip chain `clips`, `paint_opacity`, `z_index`, `document_order`, `kind` |
| `ScenePrimitiveKind` | `crates/nana-ui-scene/src/scene.rs` | Enum of concrete primitive kinds: `Quad` / `Mesh` / `Icon` / `Text` / `Chart` / `HostTexture` / `Custom`, and more |
| `PrimitiveId` | `crates/nana-ui-scene/src/scene.rs:145` | `(StableNodeId, slot: u64)`; one node can expand into primitives in several slots (background, text, selection, and scrollbar each take one) |
| `AffineTransform` | `crates/nana-ui-scene/src/scene.rs:62` | `2×3` affine matrix + `[f32; 2]` projective row, fully covering 2D and 3D perspective transforms |
| `ClipRegion` | `crates/nana-ui-scene/src/scene.rs:114` | Clip box + its own transform + corner radii / polygon vertices |
| `SceneOrderKey` | `crates/nana-ui-scene/src/scene.rs:1299` | Primitive total-order key: stacking context stack prefix `stack` + `paint_layer` + `slot` + `node` |
| `SceneDelta` / `SceneDeltaStats` | `crates/nana-ui-scene/src/scene.rs:1315` / `:1334` | The incremental change set of one `apply_delta` and its rebuild statistics counters |
| `DrawAttributes` / `SceneDraw` | `crates/nana-ui-scene/src/scene/attributes.rs:5` / `:13` | Painter-facing inherited attribute cache and per-primitive draw query snapshot |
| `RuntimeDocument` / `RuntimeFrameUpdate` | `crates/nana-ui-scene/src/runtime_document.rs:18` / `:26` | Single-document transaction owner for the canonical host and the per-frame update receipt |
| `FramePlan` | `crates/nana-ui-scene/src/scene/composition.rs:8` | Structured frame program: `RenderOperation` sequence + offscreen `preparations` + `custom_nodes` |
| `RenderGraph` / `CompiledRenderGraph` | `crates/nana-ui-scene/src/graph.rs` | Directed acyclic graph (DAG) of render passes; `compile()` fills in read/write hazard edges and runs a Kahn topological sort |
| `VisibilityIndex` | `crates/nana-ui-scene/src/scene/visibility.rs:6` | Ordered bounding-box hierarchy index (nodes with projective transforms use a conservative containment strategy) |

## Internal structure

### 1. When extraction happens: the bounded convergence transaction of `RuntimeDocument::flush`

Extraction is driven by `RuntimeDocument::flush` (`crates/nana-ui-scene/src/runtime_document.rs:109`). The internal `flush_loop` (`:174`) runs at most `MAX_FRAME_PASSES = 8` rounds of system work draining, and after each layout allows at most `POST_LAYOUT_SHAPE_PASSES = 4` rounds (`:48`) of "wrapping changes box width → reshape → layout again" feedback.

Once the loop settles, `flush` shares the same `layout_generation` (`:28`) **within the same bounded transaction** and runs four finishing steps in order (`:264-330`):

1. Update the incremental hit index (`apply_hit_test_work`).
2. Produce the incremental accessibility transaction (`project_accessibility_delta`).
3. **Extract dirty nodes and update the drawing graph**: call `world().extract_nodes(&render_dirty)` (hidden nodes stay in the returned list with `visible = false` so that `UiScene` removes their old primitives, `world/extraction.rs:652`), then call `Arc::make_mut(&mut self.scene).apply_delta(extracted, render_removed)` (updates in place when no one shares the snapshot; the time is counted under `FrameStage::Extract` as well, `:291`).
4. Bind the compositor presentation layer (`apply_presentation`).

`ExtractedNode` (`crates/nana-ui-runtime/src/components.rs:4514`) makes wide use of `Arc` sharing: `kind: Arc<NodeKind>` and `style: Arc<ComputedStyle>` clone references directly; node-local paint output projected on demand by each control's `WidgetVisual::paint` is carried in `widget_paint: Option<Arc<WidgetPaintOutput>>` (`:4601`); `chrome_radii: ChromeRadii` carries corner radii already resolved against the current theme (`:4583`); `compositor: ExtractedCompositor` carries only layer promotion hints, and transient presentation values are not baked into logical style (`:4595`).

### 2. Primitive expansion and the slot namespace (`PrimitiveId`)

`UiScene` expands each `ExtractedNode` into one or more `ScenePrimitive`s (`crates/nana-ui-scene/src/scene.rs:1257`), each uniquely located by `PrimitiveId { node, slot }` (`:145`):

- **Node base slots (`0..=255`)**: such as the background surface (slot 0), border, main text, and so on.
- **Collection namespace slots (`namespace << 32 | index`, `collection_slot:152`)**: text line labels (1), diagnostic markers (2), document text selection (6), inline objects (7), editor caret (8), dropdown menu items (9–13), and so on.
- **Special global slots**: `MODAL_SCRIM_SLOT = 10` (`:208`, modal scrim), `TRIGGERED_OVERLAY_SURFACE = 0xFFFF_FFE0` (`:216`, popup menu and bubble surfaces are promoted to the root stacking context and leave the trigger's local clip chain), `SCROLLBAR_CHROME = 0xFFFF_FFF8` (`:226`, scrollbars are always drawn after all of the container's descendants).

Custom vector paths that an application records through the `Painter` API (`PaintRecording`, Issue #217) go through `i_overlay` boolean/fill solving and `lyon` tessellation (tolerance `TOLERANCE = 0.05` logical pixels, `MITER_LIMIT = 4.0`, `:45-48`) in `crates/nana-ui-scene/src/scene/custom_paint.rs` to produce a retained `PathMesh`. When the node is translated or scrolled, the cached mesh is reissued directly without retriangulation.

### 3. Ancestor transform composition, shared clip chains, and the total-order key (`SceneOrderKey`)

- **Ancestor transforms and scroll order (`compute_ancestor_state`, `scene.rs:2176`)**: `AffineTransform` (`:62`) contains `2×3` affine components and a `[f32; 2]` projective row. `compute_ancestor_state` walks the ancestor chain from root to leaf (on reaching a `PositionSpec::Fixed` ancestor it resets geometry and the clip chain but keeps the outer structural opacity), collecting the ancestors' `overflow` clip regions, modal clip regions, and `clip-path` in order. **Only at the end does it multiply in the container's own `scroll_offset`**. So scrolled content is correctly clipped by the container's own overflow box, while the container's clip box itself does not scroll away with the content.
- **Shared clip chains (`Arc<[ClipRegion]>`)**: the `clips: Arc<[ClipRegion]>` of each `ScenePrimitive` shares the same `Arc` slice within one clip scope (`:1257`), so the clip array is not cloned again for hundreds or thousands of child primitives in the same container.
- **Total-order key (`SceneOrderKey`, `scene.rs:1299`)**: made of the `(z_index, document_order)` stack prefix `stack` of each stacking context level from outside in (opacity groups, `isolation: isolate`, positioned nodes with a `z_index`) + `paint_layer` + `slot` + `node`. It guarantees that any isolation group or subtree with a `z-index` is strictly contiguous in the draw sequence and never interleaved with outside siblings.

### 4. The three-tier incremental invalidation rules of `apply_delta`

`UiScene::apply_delta(extracted, removals) -> SceneDelta` (`crates/nana-ui-scene/src/scene.rs:1672`) handles changes in three tiers by their nature:

1. **Pure scroll translation fast path**: when a node changes only its `scroll_offset` and the transform is not projective, the subtree's existing primitives are translated linearly (`scroll_translations`), with no primitive rebuild and no re-sorting.
2. **Single-node in-place update**: ordinary paint attribute changes update `delta.paint`; box position changes update `transforms`; clip changes update `clips`.
3. **Re-sorting and subtree rebuild**: when `paint_order_facts` (`z_index` or the stacking group flag) change, `stacking_changed` triggers a re-sort. When the inherited opacity of a non-isolated ancestor changes, `subtree_rebuild` is triggered. Any structural or frame plan change clears the `frame_plan` and `visibility` caches and advances the monotonically increasing `UiScene.instance`. In debug builds, `audit_retained_projection()` (`:1950`) also recomputes small trees from scratch and compares the result with the incremental one, catching any missed incremental invalidation automatically.

### 5. `FramePlan` and `RenderGraph` compilation

`UiScene::frame_plan()` (`crates/nana-ui-scene/src/scene/composition.rs:15`) compiles the retained primitive sequence into a structured `FramePlan { operations, preparations, custom_nodes }` (cached in a `OnceLock`):

- It builds offscreen preparation passes by renderer name (one `prepare:{renderer}` pass per renderer, not one pass per resource, which avoids the quadratic complexity of `RenderGraph::compile`).
- It scans primitives in `SceneOrderKey` total order: consecutive `Custom` nodes of the same renderer are merged into one `custom:{renderer}` operation, and standard UI primitives switch back to a `ui-standard` operation. **Document occlusion order is preserved exactly**.
- If two custom nodes declare the same host texture slot with different `(renderer, revision)`, `conflicting_external_resource()` (`:58`) rejects the whole frame with `GraphError::ConflictingExternalResource` before painting.

## Collaboration and data flow

```text
RuntimeDocument::flush (runtime_document.rs:109)
  ├─ 1. style → text shaping → incremental layout convergence (sharing this layout_generation)
  ├─ 2. UiWorld updates the incremental hit index + produces the incremental accessibility transaction
  ├─ 3. UiWorld::extract_nodes(&render_dirty) ──► Vec<ExtractedNode>
  ├─ 4. UiScene::apply_delta(extracted, removed) ──► SceneDelta (scroll translation / in-place update / re-sort)
  └─ 5. UiScene::apply_presentation (binds CompositorLayer and MotionDescriptor)
        │
        ▼
SceneWgpuPainter reads the Arc<UiScene> from shared_scene(), frame_plan(), and records draws in SceneOrderKey total order
```

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-scene/src/lib.rs` | Crate boundary declaration and public export list |
| `crates/nana-ui-scene/src/runtime_document.rs` | `RuntimeDocument`, `RuntimeFrameUpdate`, and the 8-pass bounded `flush` transaction |
| `crates/nana-ui-scene/src/scene.rs` | `UiScene`, `ScenePrimitive`, `SceneOrderKey`, `apply_delta`, ancestor transform and clip composition |
| `crates/nana-ui-scene/src/scene/order.rs` | `sort_primitives` and retained document order rebuild |
| `crates/nana-ui-scene/src/scene/visibility.rs` | `VisibilityIndex` ordered bounding-box hierarchy and conservative containment culling |
| `crates/nana-ui-scene/src/scene/composition.rs` | `FramePlan` construction and external resource conflict detection |
| `crates/nana-ui-scene/src/scene/compositor.rs` | Compositor layer promotion/demotion hysteresis (`16 ms / 120 ms`) and `CompositorPaintEncode` |
| `crates/nana-ui-scene/src/scene/custom_paint.rs` | `PaintRecording` vector boolean operations and `lyon` mesh tessellation (`PathMesh`) |
| `crates/nana-ui-scene/src/scene/attributes.rs` | The `DrawAttributes` and `SceneDraw` incremental draw query layer |
| `crates/nana-ui-scene/src/scene/primitives/` | `ScenePrimitiveKind` primitive expansion for each control family |
| `crates/nana-ui-scene/src/graph.rs` | `RenderGraph` DAG compilation, read/write hazard detection, and topological sort |

## Further reading

- [GPU and Frame Lifecycle](/learn/gpu-frame): how `UiScene` and `FramePlan` are recorded into the host's frame by `SceneWgpuPainter`
- [Retained Tree: UiWorld](/learn/runtime-tree): the upstream of the `flush` transaction and how `SystemWork` is produced
- [Runtime and Scene Render Extraction Contract (Reference)](/reference/runtime-scene)
