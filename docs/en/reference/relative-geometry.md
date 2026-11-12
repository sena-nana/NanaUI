# Relative geometry

A subtree changes position and only one offset at its root changes: layout, the hit index, extraction and Scene, the painter, and accessibility all store what is inside a box in that box's own coordinates, and absolute positions are summed along the reference frame chain when read. The cost of one edit is therefore proportional to what actually changed, not to the number of nodes that moved.

This page describes the current rules: who each subsystem handles on translation, resize, and transform change, which cases are still handled per node or per whole subtree and why, where the code is, and the cold comparison to keep when changing it. Per-stage benchmark numbers are in [Resource budgets · CPU time](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-time); the evolution is at the end.

## 1. Principles

### 1.1 One addition

```text
abs(child) = abs(frame) + local(child)
```

Every box stores "offset relative to the origin of its reference frame + size" (`NodeRecord::local_layout`) and the reference frame itself (`NodeRecord::layout_frame: LayoutFrame`). Absolute coordinates are obtained by **adding level by level from the topmost reference frame down** along the axis. `UiWorld::record_layout_origin` in Runtime, `origin_in` / `content_origin_in` in the engine, `page_boxes` in `RuntimeLayoutEngine::layout_document`, `UiScene::placed_origin` in Scene, and `AccessibleTransform::origin` in accessibility use the same addition order. A cold solve likewise computes the local offset first and then accumulates this way, so the incremental result and the cold comparison are **bitwise equal**; translation needs no delta addition, so there is no last-bit drift from "old value + displacement".

This convention is the precondition for every bitwise cold comparison. Changing any one place to another summation order (for example adding the displacement first, or accumulating from the bottom up) makes the cold comparison report a last-bit difference at unaligned steps (tests usually use 1.37 px).

### 1.2 The reference frame `LayoutFrame`

Defined in `crates/nana-ui-runtime/src/layout_result.rs`:

| Reference frame | Origin | Used by |
| --- | --- | --- |
| `Parent` | Origin of the parent's border box | Boxes in normal flow; the containing block is the parent's `absolute` box |
| `Block(id)` | Origin of the containing block's border box | An `absolute` box whose containing block is above the parent (escaped absolute positioning) |
| `Page` | Document origin | The root, `fixed` boxes, `absolute` boxes whose containing block is the initial containing block, and items of a triggered menu (attached to the viewport). Under `Page` the local value is the absolute value |

The engine writes the reference frame back (the hidden `UiMutation::PlaceLayout { id, layout, frame }`); the external `WriteLayout` infers it from style (`UiWorld::layout_frame_from_styles`, following the same rules as the clip chain). When `position` changes but layout has not rerun, the end of commit moves boxes whose reference frame no longer matches their style to the new reference frame, keeping the page position (`reframe_layouts`).

### 1.3 Public API: semantics unchanged, absolute values derived

| API | Coordinates | How it is obtained |
| --- | --- | --- |
| `UiWorld::layout_box(id)` | Document | Summed along the reference frame chain, O(depth), not cached |
| `UiWorld::local_layout_box(id)` / `layout_frame(id)` | Reference frame | The record itself |
| `UiWorld::layout_result(id)` | Document | Returns `Option<Arc<LayoutResult>>`; results placed on the page are cached in `page_layout_results`, and staleness is decided by the local result's `Arc` pointer and origin (bitwise) |
| `UiWorld::local_layout_result(id)` | The node itself | Border box at (0, 0) |
| `UiWorld::component_geometry(id)` | Document | Computed fresh from the current box each time; hit testing, IME, and the pointer read it |
| `layout_origin(id)`, `layout_box_from(owner, child)` (crate-internal) | Document / some ancestor | Page origin; a child's box relative to some ancestor |
| `UiScene::primitive_layout_bounds` / `place_rect`, `SceneDraw::placed_bounds` / `place` | Page | The primitive's local geometry plus the node origin |
| `SceneDraw::geometry_transform` | — | The composed matrix of "place at the origin, then transform", for readers that do not snap to the grid |
| `AccessibilityNode::bounds` + `transform`, `AccessibilityPlacements` | The parent's accessibility space | A node to its parent is `translate(bounds.x, bounds.y) · transform` (`AccessibilityNode::to_parent`); `AccessibilityPlacements` accumulates the viewport transform from the relative tree |

`RuntimeLayoutEngine::layout_document` / `layout_document_with_frontier` still return page coordinates; framework write-back uses the crate-internal `layout_document_with_frontier_local`. `MutationQueue::write_layout` takes document coordinates, and at the end of commit converts them to local values from the top down by depth, so the result does not depend on write order; children that were not written move with their parent.

"Not cached" is across frames: extraction and accessibility projection remember, in one read-only pass, the sum of the reference frames a deep node has already walked (`PageOriginScope`, `world/page_origins.rs`; depths up to 32 still walk directly), and the boxes below continue from it — the sum is exactly the accumulated value of adding level by level down to that reference frame, so the result is the same bits. It is dropped when the traversal ends; meanwhile the world is borrowed and boxes cannot be written.

Runtime internals do not go through the `layout_result` page cache: `canonical_layout_box`, `component_content_box`, part queries, and the clip chain all read the local result and add the origin; the cache is filled only for external calls, so no node keeps a second copy of its result (the `node_memory` budget guards this).

## 2. Node space classification `NodeSpace`

`crates/nana-ui-runtime/src/node_space.rs`. Every consumer that retains "what is inside a box" has to ask the same questions: where does layout measure it from, is its own transform only a translation or an upright scale or a turn, does it open a 3D context, does it clip, and does its visual read the page. `NodeSpace` is those answers, computed once per node from **its own** resolved style, reference frame, kind, and visual, and stored on `NodeRecord::space`. It holds only what the node's own record can decide, so however a box above changes, no subtree walk is needed to update it.

### 2.1 Bits

| Bit | Meaning | Source |
| --- | --- | --- |
| `OFF_PARENT` | The reference frame is not the parent (`Block` or `Page`) | `layout_frame` |
| `FIXED` / `STICKY` | `position: fixed` / `sticky` | Style |
| `MOVES` / `SCALES` / `TURNS` | Its own 2D `transform`: the linear part is identity / a positive scale on every axis (not all 1) / rotation, skew, flip, or non-finite; at most one of the three | Style |
| `TRANSFORM_3D` / `PERSPECTIVE` / `PRESERVE_3D` | Its own 3D context | Style |
| `CLIPS` | Clips overflow on either axis | Style |
| `DOCUMENT` | Document node | Kind |
| `VIRTUAL` | The visual derives accessibility nodes from geometry or rewrites its own accessibility box: open Select, TreeView, MenuSurface, ModalFrame. A closed Select projects only its field and does not count | Visual |
| `MODAL` | ModalFrame: the accessibility box is the surface, and hit testing clips children to it | Visual |
| `READS_PAGE` | The visual's geometry reads the page (`StandardVisual::geometry_reads_page`: only an open MenuSurface) | Visual |
| `MENU_ROOM` | Open Select: the menu fits into the space on the page, and moving a box above can change that space | Visual |

### 2.2 Derived answers

| Method | Answers | Read by |
| --- | --- | --- |
| `depth()` | Opens or joins a 3D context | Accessibility (does not hand offsets), in-place rebuild (refuses) |
| `opens_3d_context()` | `perspective` or `preserve-3d`; children draw inside a 3D context | Building hit entries, passing the accessibility transform chain down |
| `transformed()` / `frames_itself()` / `upright()` | Has a 2D transform / the transform is more than a translation / the transform leaves the box upright | Accessibility space, hit testing, deciding in-place resize |
| `places_by_offset()` | Accessibility bounds can take the local offset: the reference frame is the parent, not `fixed` / `sticky`, and no own 3D | Accessibility |
| `hands_offsets()` | Boxes below can be placed by offset in its space: no 3D | Accessibility |
| `pins_accessibility()` | When a box above translates, it or what is below it may not keep accessibility bounds: not offset-placed, clips, does not hand offsets, or `VIRTUAL` | Accessibility pin index |
| `keeps_space(after)` | The space handed to descendants is the same before and after the transform change: neither side has 3D, and either both sides `frames_itself` or neither does | Deciding `TRANSFORM_KEPT` at commit |
| `aligned_by_style()` | Style keeps the accessibility box where the parent's coordinates put it | Accessibility `axis_aligned` |
| `hit_frames_itself(local)` | For a hit entry, the box itself hands its layout coordinates to children | Building hit entries |
| `page_bound()` | `READS_PAGE` or `MENU_ROOM` | Extraction (`settle_layout_moves`) |

### 2.3 Maintenance and indexes

`NodeStore::note_node_space` (`crates/nana-ui-runtime/src/store.rs`) recomputes the node when a field that decides the classification is written: insertion (`NodeStore::insert`), visual set (`NodeStore::set_visual`; opening and closing a Select both change the visual), style resolution (`note_layout_anchor_change`), write-back or reframe (`write_local_layout`, `reframe_layouts`). Node removal deletes it from the indexes.

Two indexes are maintained with the classification and match it one for one:

- `space_pins`: nodes for which `pins_accessibility()` is true. Accessibility walks up from here to find the translation root (`reach_moved_pins`, `reach_turned_pins`), at O(pins × depth). Scroll writes do not change the classification and do not touch this index.
- `page_bound`: nodes for which `page_bound()` is true. `settle_layout_moves` scans only this set; `NodeStore::open_select_menus` is a `MENU_ROOM` filter over it, not a separate set.

Guard: `NodeStore::check_node_spaces` (tests and `layout-verify`, within 4 096 nodes) compares every node's classification with a fresh `NodeSpace::of` at the end of every commit and at the start of every accessibility projection, and checks that the two indexes are neither short nor long.

### 2.4 Answers that depend on the whole chain are not stored

The classification looks only at the node itself. The following look at the whole chain above, so they are **not stored and are derived each time**, memoized per projection or index-building call:

- **`AccessibleSpace`** (`world/accessibility/space.rs`): the node's accessibility space, with three fields, `local` (offset-placed), `unit` (per-axis scale when the box is upright in the space), and `framed` (the space is some box's local layout coordinates, not a viewport translated into place). Derived by `accessible_space` along the chain and recorded in `ProjectionMemo`.
- **`moved_root_hands_offsets`** (`world/accessibility/delta.rs`): whether the translation root's box is upright in its space, whether there is no modal on the chain, and whether globally there is no animation overlay, length animation, or clipping visual. The fast path reads only the recorded `hands_offsets()` and `upright()` along the way; when that fails it asks `accessible_space`.
- **The hit 3D context**: `build.rs` carries `parent_blocks_3d` down from the seed when building entries and combines it with `opens_3d_context()` into the child's `child_blocks_3d`; `local_children` requires no perspective along the way and the child not inside a 3D context.
- **The effective style for hit testing**: entries are built from the effective style. When the effective style is not the resolved style (animation overlays, overlay-host branches), `NodeSpace::of_style` classifies the effective style afresh and does not read `NodeRecord::space`.

### 2.5 Differences kept on purpose

A planar `transform_3d` answers differently on the two sides, and the classification records them separately (`TRANSFORM_3D` is not merged with `PERSPECTIVE` or `PRESERVE_3D`):

- **Hit testing treats it as 2D**: `hit_frames_itself` looks at the homography `TRANSFORM_3D` produces; when there is no perspective row and the linear part is not identity, the entry still hands out local coordinates (`boxes_moved_under_a_planar_3d_turn_answer_as_a_fresh_index`).
- **Accessibility treats it as 3D**: `depth()` counts all three as 3D and falls back to the bounding box of the projected corners.

Reason: hit testing needs only an invertible affine, and a planar 3D transform provides exactly that; the `transform` accessibility hands to the platform is a 2D affine, and the current output for a planar 3D transform is a bounding box — unifying on 2D would change what the platform receives. Both sides stay as they are; a change updates both together and refreshes the snapshots and the accessibility cold comparison.

Another difference: `pins_accessibility` counts `fixed` and clipping as pins; `HitEntry::pinned` does not. Hit testing hangs `fixed` at a parentless place and keeps clipping in the entry's own coordinates; accessibility bounds are relative to the tree parent, and `clip` is in viewport coordinates.

## 3. Layout

### 3.1 Rules

- The origins of the engine's `DocumentLayoutCache.boxes` and `ContainerPlan` are relative values, and a non-parent reference frame is recorded separately in `DocumentLayoutCache.frames`. `place_node_body` always places children with `Point::ZERO` as this container's coordinate origin, and the box `place_node_head` writes is the offset relative to the reference frame.
- Cross-reference-frame reads exist in only a few places: the containing block's padding box for a positioned child is in the containing block's own coordinates; the static position is converted to the containing block's coordinates through `content_origin_in` along the reference frame chain; items of a triggered menu are placed in page coordinates, and the menu's own page origin comes from `origin_in`; resolving parts in an inline context uses the border box in the box's own coordinates.
- A statically positioned escaped absolute box moves with the boxes between its containing block and its container: `place_remote_overlays` rechecks unplaced containers on every pass by comparing "did the static position change after conversion to the containing block's coordinates", at O(escaped entries × depth).
- `LayoutResult` is stored in the node's own coordinates (border box at the origin). a result copies none of its children's boxes, and children's placements and fragments are read from the children when `UiWorld::layout_child_placements` / `layout_child_fragments` are asked (before 2026-11-11 a result held each child's local box three times); `overflow` and `scroll_extent` merge in only children whose reference frame is this node, while `fixed` boxes, escaped absolute positioning, and menu items merge in only when placed on the page.
- `ContentBoundsIndex` (`world/scroll_bounds.rs`) stores each entry at its own node's origin, and the range it carries toward its containing block in the coordinates of the containing block where it lands. Two cases can change the landing point (whether `position` establishes a containing block changes, a subtree changes parent), and then entries carrying a range inside that subtree re-read their children (`reland`). A parent refresh reads only the children that moved, leaves are written in place, and the levels above merge once (`MaxTree::rederive`).
- Sticky positioning is computed from the local box and the local result; `StickyAncestor::result` is the result in the ancestor's own coordinates.

### 3.2 Translating a subtree

When a parent re-arranges, it writes only the offsets of its direct children: a container that only moved fails `subtree_unchanged` at its parent; once `place_node_head` has written its new offset, `keeps_moved_subtree` finds that nothing its body reads has changed (it is no seed, not on the measure frontier, its writing mode and scroll did not change, and it is in the affected closure only because its parent or a sibling in the same formatting context moved it; nothing under it is affected; its border-box size and used padding are last pass's; it takes part in no Dynamic Layout line, inherits no grid tracks, is neither a modal nor a triggered menu, is not on the remote-overlay recheck list, and has no child in a frame other than its own), so it never enters `place_node_body` and not one child is touched; its result is unchanged and publish judges it the same by `geometry_eq`. An inline formatting context (a paragraph) has no container plan and used to pack its line boxes again on every move; it now writes only its offset too. In tests and `layout-verify` builds, every box under a kept subtree must equal the full layout's bit for bit. Commit marks only the boxes written back (`settle_layout_moves`): a root whose origin changed gets `DirtyMask::MOVED` and enters `SystemWork::layout_moved`; no descendant is marked.

At publish (`publish_layout_results`):

- When the result projects new geometry (`layout_result_projects_new_geometry`: its own box, padding box, content box, baseline, overflow, scroll, clipping, participation, its own fragments and parts), mark INPUT | RENDER | ACCESSIBILITY; when the change goes beyond the scroll extent, overflow, and baseline (`layout_result_reshapes`), also mark `DirtyMask::RESHAPED` and enter `SystemWork::layout_reshaped`, on which accessibility decides whether the clip box still clips by the old box.
- A child that only moves inside its parent's box (the first row of a fixed-height column grew) leaves the parent's result as it was: the result records no child placements, and `layout_result_geometry_current` only checks that the covered children are still this node's, have this node as their reference frame and stay inside `overflow`; when that holds the previous result is kept, not rebuilt and not marked dirty. A child reaching out or back changes `overflow` and projects as in the previous point. A result rebuilt with nothing new of its own to project still **marks RENDER only**: the record extraction hands it carries the result, and otherwise Scene would keep the old one.

The line box memo ("does this box hold an in-flow line box", `holds_line_box_in`, stored in `DocumentLayoutCache::line_boxes`) is kept across frames and cleared by the seed's change class: `may_move_line_boxes` (`layout_engine.rs`) answers "do not clear" for seeds that change only size, spacing, alignment, scroll, transform, or pointer input, and whose reason does not involve flow; a node brought in by a dependency edge that itself did not change is also not cleared. Structural seeds (topology, visibility, unknown reason, `moves_line_boxes_below`) are cleared up to the root along every affected node as before, because a hide derived from an ancestor changes the answer for the whole subtree.

### 3.3 Changing a transform

A transform is not a layout input: a changed `transform` in style marks no layout dirt and clears no line box memo. Commit calls `keeps_space` on the two `NodeSpace::of_style` values before and after: when it holds, `mark_transform_kept` marks only `TRANSFORM | TRANSFORM_KEPT | INPUT | RENDER` (INPUT without `INPUT_ELSEWHERE`) and lists it in `SystemWork::transform_kept`; when it does not, it marks `TRANSFORM | INPUT | RENDER` and lists it in `SystemWork::transform`. Both mark only the box itself; descendants are handled by each subsystem under the rules below. A stacking change still marks the whole subtree INPUT | RENDER.

### 3.4 Still handled wholesale

- Clearing the line box memo for a structural seed goes up to the root along every affected node (see above).
- When a subtree changes parent, or `position` switches between establishing a containing block and not, entries in `ContentBoundsIndex` that carry a range re-read their children.
- In a scroll container, every moved row still writes one leaf of the content bounds (see [Known limitations](#_10-known-limitations)).

## 4. Hit index

### 4.1 Entries

Each entry (`HitEntry`) is in **its own border box's coordinates**: `layout` is `(0, 0, width, height)`, and clipping, path clipping, the menu, and the transform pivot are all stored in these coordinates.

- `origin`: the box's offset from the hit parent's origin, converted by the linear part of the transform the parent hands to children (`linear_offset`). Sources: when the reference frame is the parent and the hit parent is the tree parent, take `local_layout_box` directly; a hit root with the `Page` reference frame also takes the local value directly; the rest (absolute positioning whose containing block is elsewhere, escaped entries hung on a clipping parent) take the difference from `layout_box_from` / `layout_origin`, and such entries are pinned.
- `IndexedHit::shift`: the scroll translation on a scroll container; `HitEntry::scroll` is the scroll offset the index puts children at.
- A query (`open_hit`, `query.rs`) first subtracts `origin` to enter the entry's own space; an entry that hands out local coordinates then goes through one inverse transform (`transformed_point`; when singular, children are unreachable), then subtracts `shift` to enter the children's space.
- A parent boundary tree's bounding box is "itself ∪ children (plus `shift`)" plus `origin` (`entry_bounds`; an entry that hands out local coordinates uses `turned_bounds`, taking the bounding box of the children's bounding box through its own transform). Cold build and incremental refresh use the same formula, bitwise.
- When the parent's transform carries perspective, the offset cannot be applied outside: the offset goes into the entry's own transform and `origin` is zero.
- **The sticky offset** (`sticky_translation`) is, like the layout offset, a translation before the entry's own transform: without perspective in the parent it goes into `origin`, not `transform`. Sticking or sliding changes only that entry's `origin`; child entries are stored relative to it and do not inherit it. A modal body's own clip (`self_clips`) is stored in the parent's space, so its stored transform takes off the converted sticky offset, and once a query has subtracted `origin` the clip is still where the frame laid the body out. Under a parent with perspective the sticky offset goes into the entry's transform with the layout offset.
- **Handing out local coordinates** (`HitEntry::local_children`): an entry whose own 2D transform has scale, rotation, skew, or flip (`hit_frames_itself`), with no perspective along the way, and whose children are not inside a 3D context, hands children only "subtract scroll" (`scroll_seed`); children are built in its local layout coordinates, without any transform from it or above. So everything in a turned subtree is upright. Reads that follow: `inherited_shift` and `child_shift` accumulate scroll upward to the nearest such entry; seeds for patching and in-place rebuild all go through `live_child_seed`; places that cross between the window and an entry (`pointer_layout_position`, `layout_pointer_position`, `menu_region_contains`, `escaped_hit_at`) go through `entry_space`.
- **Pinned entries** (`HitEntry::pinned`, the set `HitIndex::pinned`): entries that read geometry other than their own offset — offsets differenced across reference frames, #201's escaped bridge transform (`clip_bridge_transform_from`), component geometry (Select menu, hover card connector, modal surface / body clipping), sticky positioning. When patching, pinned entries below a dirty node are rebuilt even when not marked dirty (`pinned_hit_entries_under`, O(pins × depth)).

A query while a compositor layer is up (`visit_presentation_hits`) computes the transform from page coordinates each time and does not read the entry's `transform` or bounding box.

### 4.2 Translating a subtree

Only the root entry is rebuilt (`rebuilt_hit_entry_in_place`, `in_place.rs`, replaced in place; a changed `origin` counts in `hit_test_entries_moved`), then ancestor bounding boxes are refreshed by depth; descendant entries are untouched except pinned ones. The root entry does not change `origin` in place: the same commit may add or remove siblings, and the order must come from the rebuild.

In-place rebuild requires that "what is handed to children did not change": the new entry is built on its own from the parent entry's current seed (`live_child_seed`: the parent entry's transform plus the `shift` above minus the parent's current scroll), minus the `shift` above to return to the space the index stores it in, and then:

- the entry has no escape, perspective is zero, and for an entry that does not hand out local coordinates the linear part is identity;
- the hit parent is unchanged, `clip_escape` is empty, the parent entry has no perspective, and the node has no own 3D (`space.depth()`);
- exactly one new entry is built, nothing is deferred; the transform (when not handing out local coordinates), `HitEntry::scroll`, the child list (the same `Arc`), and z-index equal the old entry's, and it did not become `fixed`; an entry that hands out local coordinates still does so before and after.

Clipping, the menu, and the painter belong to the entry alone and may change freely. `IndexedHit::shift` is kept as is, and children are still queried at the scroll they were built with plus `shift`.

### 4.3 Resize, transform change, scroll

- A turned box that grows taller or is translated: the transform origin (the center by default) moves with it, but descendants are in its local coordinates, so only its own entry is rebuilt.
- Transform change (`TRANSFORM` or `TRANSFORM_KEPT` marks only the box itself): when it hands out local coordinates both before and after, only it is rebuilt, and pinned entries below the dirty node are rebuilt as usual; in the other cases (pure translation, becoming or ceasing to be such an entry, perspective, 3D) in-place rebuild does not hold, and `patch_hit_subtree` patches its branch whole.
- Scroll: `shift_hit_test_scroll` changes only `shift` and refreshes ancestor bounding boxes, O(depth); on an entry that hands out local coordinates it converts by the identity linear part. When the same frame has both scroll and other dirty nodes, the driver drops the scroll increment and leaves it to the patch; the scroll container's `scroll` then differs from the current value, so it goes through the subtree patch. The fast path (`hit_test_work_is_scroll_only`) requires every INPUT-dirty node to be a container scrolled this frame and none to be in `non_scroll_hit_dirty`: that set records INPUT marks other than scroll from `mark` / `mark_moved` / `mark_ancestors` (scroll only goes through `mark_scroll_compatible`), so a container whose own box changed and that scrolled in the same frame goes through the patch; it is removed when entries are built from current geometry — a whole-tree rebuild deletes this document's, a range rebuild deletes the passed dirty nodes and pinned entries, and a sticky entry rebuilt during scroll deletes itself — and nodes marked INPUT after the work was taken are kept until the next frame. A sticky box (`sticky_nodes.affected`) changes its own offset as the content scrolls: the sticky offset is in `origin` and the entries below are in its coordinates, so each sticky box rebuilds only its own entry in place (`rebuilt_hit_entry_in_place`), stuck or not; all of them are built first and replaced at once, and ancestor bounding boxes refresh once. The branch is patched whole (`patch_hit_subtree`, outermost first) only when it holds a pinned entry or its parent has perspective (the offset is then in the transform). Under a pure scroll nothing above the entry changed in index space, so the transform built against the live scroll and brought back by `shift` differs from the stored one by rounding alone: the stored transform stays (the `scrolled` parameter) and the clip transforms move by the same amount, so the in-place rebuild does not fall back to a branch patch over last-bit differences. A scroll's hit cost is therefore one entry per sticky box, whatever the sticky box or the scroll container holds.

### 4.4 Still patched whole

Branches under perspective and 3D contexts, a transform switching between "hands out local coordinates" and "does not", `HitEntry::scroll` disagreeing with the current scroll, and a changed child list or stacking all go through `patch_hit_subtree`. Reason: their descendants' transforms contain theirs (perspective does not enter the affine; a 3D context must pass the transform down), so a change in one changes all.

## 5. Extraction and Scene

### 5.1 Records and component geometry

- `ExtractedNode` carries `local_layout` and `layout_frame` (the same as `UiWorld::local_layout_box` / `layout_frame`); `layout` is the page box at the moment of extraction, for reference only. `layout_result` is the local result, sharing the same `Arc`. `ExtractedMove` likewise carries `local_layout` / `layout_frame` / `layout_result`, and is sent only for a translation root.
- `ExtractedNode::component_geometry` is in the node's own coordinates. `UiWorld::local_component_geometry` derives it with the same `derive_component_geometry`: it sets `geometry_frame` to this node, and `record_layout_origin` stops when the sum along the chain reaches it (its origin is recorded as 0); if the chain does not reach it, or it reads the viewport, it records `geometry_read_page` and asserts in debug builds. The public `component_geometry` sets no frame and is still in page coordinates.
- **A visual that reads the page** (`READS_PAGE`): only an open MenuSurface is left (menu items' reference frame is the page). It derives in page coordinates and is recorded on the page; when it or a box above translates, `settle_layout_moves` finds it from `page_bound` and marks RENDER, re-extracting the whole node. A closed MenuSurface cannot reach menu items (`closed_menu_hides`), a closed Select does not read the viewport when deriving, and NativeMarkdown lays out from the local content box down, so both derive in local coordinates and keep their records on translation.
- **An open Select** (`MENU_ROOM`): the menu is derived in the field's local coordinates. The page is read in exactly one place: the space `select_menu_room` gives (the viewport and clip chain, stored in the visual's `menu_room`). `local_select_menu_room` subtracts the field's page origin, and after that only `select_menu_surface` (`select.rs`) decides the surface box; everything else is decided by this box and the field's own state. So the surface box is bitwise identical, and the whole record is bitwise identical. On translation the field is the translation root, and `extract_nodes_reusing` compares once before reusing the record (`keeps_select_menu`: the surface the renderer holds is compared bitwise with the `local_select_menu_surface` computed now); when the field is below a translation root, `settle_layout_moves` marks RENDER only and records `render_room_epoch` (`mark_keeping_render_reason`, not counting as "another reason"), and extraction compares once the same way. Only when it flips sides, is cut short, or the space itself changes (scroll, clipping, a window change; `stale_select_menu_rooms` changes the visual) is it re-extracted.

### 5.2 Primitives and painting

- A primitive is built with `UiScene::record_box` as its `bounds` (the local record is `(0, 0, width, height)`, the page record is the page box at extraction), and `bounds`, batch rects, stroke points, `Path` / `Chart` `origin`, layer clipping, and the node's own clipping are all in the node's own coordinates; `transform` is the transform on the page (ancestors' transforms, scroll, the node's own transform about its current position), excluding the origin.
- The node's current page origin is summed along the reference frame chain (`placed_origin`, `placement.rs`, memoized by `placement_epoch`; before the first translation it is taken directly from `layout`, `placement_live`). `paint_origin` is where the primitive lands: `placed_origin` for a local record, 0 for a page record.
- `draw_primitive` (`attributes.rs`) gives `SceneDraw { origin, transform, clips }`: geometry is first offset by `origin`, then transformed; `clips` are all on the page, inherited ones computed fresh by `fold_ancestor` from the ancestors' current page boxes (`placed_layout`), the node's own plus `origin` (cached in `placed_clips`). A cold scene uses the same formula. A retained projection uses the origin recorded when the primitive was built (`RetainedPrimitive::origin`; a debug build asserts it equals the fresh computation); the rebased origin and inherited clips are recorded in `DrawAttributes`.

### 5.3 Translating a subtree

- A translation root changes only its local box, reference frame, and result, raises `attribute_epoch` and `placement_epoch`, and leaves primitives alone; only a changed size or result rebuilds them. A re-extracted whole node that only moved and has the same result (`moved_in_place`) is likewise not rebuilt. Descendants are not in the delta.
- **Nodes that read their own position** fall into two sets (`offset_bound` / `origin_bound` on `UiScene`):
  - `offset_bound`: read only their own offset in the parent's coordinates. A small icon aligned with the adjacent text of its parent (`icon_y_aligned_to_adjacent_text` answers `PlacementRead::Offset`) is rebuilt only when it is itself a translation root, O(translation roots).
  - `origin_bound`: read the page position, and are rebuilt whole when they or a box above translate, O(set size × depth). Members are listed under [Known limitations](#_10-known-limitations).
- **Cull index**: a translation root translates its whole subtree in the visibility index (`translate_subtree`, the same path as scrolling; a nested root only fills in its own extra segment), and the root's own primitives are recomputed; boxes whose reference frame is not the parent, or that are sticky (`unfollowing`: escaped absolute positioning, `fixed`, menu items, `sticky`), recompute themselves and their subtree when below a translation root. All translations happen first, then any recomputation. The translation is passed through the linear part of the root's inherited transform first; when the inherited transform carries perspective, the whole subtree is recomputed (`refresh_subtree`). A leaf's bounds are judged unknown by the primitive's own opacity group (`primitive_opacity_groups`, the same rules as the painter).

### 5.4 Transform changes and child placement changes

- A transform change re-extracts only the box itself, and descendants' primitives are rebased by `current · inverse(base)`. `DrawAttributes`'s `rebased` records the projection when the primitive was built and the current projection; the primitive's and the node's own clipping are drawn directly in the current projection without going through an inverse matrix, so going from turned back to a pure translation leaves the linear part exactly 1 and the painter snaps as usual. The cull index `refresh_subtree`s those roots when an ancestor's projection changed.
- A child moving inside its parent's box does not change the parent's result, and the parent is not extracted again (see 3.2); one that changes `overflow` projects as usual, and a result rebuilt with nothing new to project marks RENDER, the node is re-extracted, and the result Scene holds is swapped accordingly.

### 5.5 Public semantics

`ScenePrimitive` geometry is in the node's own coordinates (except for a visual that reads the page); the position on the page is obtained with `UiScene::primitive_layout_bounds` / `place_rect`, or `SceneDraw::placed_bounds` / `place`. `node_bounds` and `node_layout_bounds` are the same: the current page box.

## 6. The painter

`crates/nana-ui/src/scene_paint/`. For each primitive the painter:

1. **Places it at the origin first**: geometry plus `SceneDraw::origin` (`placed_bounds`, `place`, stroke points into a reused scratch area, `origin` of `Path` / `Chart` / layer mask);
2. **Then transforms**: after that it is exactly the same as for a page-coordinate primitive;
3. **Then snaps**: `paint_transform` (`scene_paint/clip.rs`) snaps only the pure translation part of `transform` to whole device pixels (#223's scroll semantics); a glyph's whole-pixel and subpixel phase comes from the placed page position.

A layout translation is therefore not a translation that needs snapping: a label moved by a fractional pixel has its phase recomputed at the new position, and one moved by whole pixels only switches the run's `origin` and reuses its instances. No per-node transform buffer is needed: each frame the painter walks only the primitives left after `visible_operations` clipping, and each primitive costs one addition and one origin read.

## 7. Accessibility

### 7.1 Semantics

`AccessibilityNode::bounds` is relative to the parent's **children's space**, and `transform: Option<PaintTransform>` corresponds to AccessKit `Node`'s `transform`; the node's own box is `(0, 0, width, height)` in its own space. `AccessibilityNode::scroll` is how far the node scrolls its content (its scroll offset times its space's `unit`): its children's space is its own space moved by `-scroll` (`children_to_parent`), so a scroll changes the scroll container's node alone and the bounds of what it holds stay. It is `[0, 0]` on a node that does not scroll, on one whose children are placed from their viewport boxes (under a 3D context, a modal frame or a motion overlay, where `unit` is `None` and the scroll stays in the children's bounds) and in viewport projections. The AccessKit host sets `to_parent · translate(-scroll)` as the node's `transform` and its local box moved by `scroll` as its `bounds` (its own box stays where it was), the representation Chromium uses too: a scroll container carries its scroll offset, its children are relative to it. `clip` is still in viewport coordinates. Automation and tests that need viewport coordinates use `project_accessibility_in_viewport` / `project_accessibility_nodes_in_viewport`, or accumulate along the chain with `AccessibilityPlacements`.

Every node's space (`AccessibleSpace`, see 2.4):

- **Offset-placed** (`offset_placed`, `local_placement`, `offset_bounds_in_parent` in `frames.rs`): when `places_by_offset()` holds, the parent's space leaves it upright (`unit` has a value), and there is globally no animation overlay or clipping visual, bounds are computed from the local box: `(a·x, d·y, a·w, d·h)`, where `(a, d)` is the parent space's scale; the parent's scroll is in the parent's own `scroll`, not in its children. When its own transform is only a translation it is folded into the offset and `transform` is `None`; a positive scale emits `diag(a·a', d·d')`; rotation, skew, and flip emit `diag(a, d) · L`. The latter two are said to frame themselves (`framed`): its space is its local layout coordinates, and below it boxes are offset-placed by the unscaled factor. All of this is independent of the page position.
- **From the viewport, framing itself** (`frames_from_viewport`, `viewport_frame_in_parent`): a box that is not offset-placed (page root, `fixed`, `sticky`, escaped absolute positioning), on a 2D chain, where the parent-space-to-viewport mapping is invertible, and that has been turned or scaled, gets `to_parent = V_P⁻¹ · M · translate(origin)` (`M` is the whole chain from `accessibility_ancestor_transform`, `V_P` is `accessible_space_to_viewport`). The translation comes from the viewport, the linear part is handed to descendants, and offset-placed boxes below are independent of the page position.
- **Everything else from the viewport bounding box**: 3D, a modal surface, everything under an animation overlay or clipping visual, and when the parent space is not invertible. When the parent space has a frame, it emits the linear part of its inverse (`turned_space_inverse`), aligning its space with the viewport. A 3D context falls back to the bounding box only and emits no perspective: the platform has only 2D affine.

### 7.2 Translating a subtree

At write-back, a box whose local offset changed is marked ACCESSIBILITY; descendants that translate with their parent are not. INPUT marked only because layout moved carries no `INPUT_ELSEWHERE` and enters `SystemWork::accessibility_moved`. `project_accessibility_delta` (`delta.rs`) for each translation root:

- **Find from pins**: when `moved_root_hands_offsets` holds, the root is not walked whole; instead `reach_moved_pins` (`pins.rs`) looks up from each pin in `space_pins` to the nearest translation root. It stops at a box whose reference frame is the page (it did not move along); a box whose reference frame is some block records that block, and the pin counts as moving with the root only if it passes through it before reaching the root; a pin that still hands offsets (scale, `sticky`, absolute positioning measured from a block that translates with the root) does not block the way. A found pin is walked from itself by `reach_moved_subtree`'s rules: ones that did not move along enter the affected set, a clipping one is reprojected whole, and ones not offset-placed go one by one through `keeps_relative_bounds`. O(pins × depth).
- **Walk one by one**: when `moved_root_hands_offsets` does not hold (the root's space is not upright, the root is a modal, there is an animation overlay, length animation, or clipping visual), `reach_moved_subtree` decides exactly from the root down, stopping at another translation root.
- A translation root's own relative offset really changed and must be reprojected. A box that layout marked INPUT but that did not enter the affected set (its relative offset did not change) asks `keeps_relative_bounds` once more, and is reprojected if it cannot keep them.

**Scrolling**: a scroll marks only the scroll container's INPUT, with `DirtyMask::INPUT_SCROLLED` (and no `INPUT_ELSEWHERE`), and the drain lists it in `SystemWork::accessibility_scrolled`; a container something else marked INPUT on in the same frame, that layout moved or reshaped, or whose transform changed, is not listed. `project_accessibility_delta` projects the container itself (its `scroll` and scroll axes changed); where `moved_root_hands_offsets` holds, what it holds is treated as content that moved and handed to `reach_moved_pins`, with the scrolled containers as roots and stops: what the scroll does not carry (`fixed`, an absolute box whose containing block is outside the container), what is placed from its viewport box (`sticky`, an absolute box measured from another block; when the parent is a scroll container (it clips overflow, or its scroll offset is not zero) `keeps_relative_bounds` no longer keeps a box because both viewport boxes are layout boxes — testing the offset alone misses a `sticky` box that was stuck in the frame that scrolls back to the origin) and the whole subtree of a clip (descendants' `clip` is in viewport coordinates, nested scroll containers included) project again, and nothing else. A scroll container inside another one that scrolled projects whole (its clip moved on the viewport); where `moved_root_hands_offsets` does not hold the subtree projects whole, as before. The cost is O(pins × depth) plus the container's own child list.

**Clip boxes**: a clipping seed is in principle reprojected whole (the viewport-coordinate `clip` comes from it), but when it is not in `layout_reshaped` (a scroll container that stayed in place and only changed its scroll extent) it clips by the old box and projects only itself.

**Resize in place** (since 596930d22): when a box changed size in place and is not in `layout_moved`:

- A box that frames itself (2D transform, `transform_3d`, or `perspective`) is always a traversal root (`resized_roots`, combined with `layout_moved` into `walk_roots`): the transform origin moves with the size, and its space moved as if it had translated. If it is a 2D transform and `moved_root_hands_offsets` holds, it goes to `reach_moved_pins` (only pins below are reprojected, for example a `fixed` two levels down); otherwise (animation overlay, clipping visual, 3D, modal) it goes to `reach_moved_subtree` and is decided exactly one by one below, not just direct children.
- For a box that does not frame itself, when there is a box that does not hand offsets along the way, or its own space is not upright, reproject the direct children that did not move along (the bounding box origin moves with the size); otherwise do nothing.

### 7.3 Transform changes

A box in `SystemWork::transform_kept` must additionally: not have been translated by layout this frame, not clip itself, `moved_root_hands_offsets`, `style_frames_its_space` (a box whose style frames itself really does frame its space; a scaled box under a 3D context has a non-invertible parent space that is the viewport's scale, and changing the factor would change descendants). When it holds, only it is projected, then `reach_turned_pins`: each pin walks up, and the first such root reprojects the pin whole, while the first other pin is left to that pin. A turn carries everything below it, so unlike translation this does not look at the reference frame. Boxes that do not satisfy this, and boxes in `SystemWork::transform`, are reprojected whole.

### 7.4 Fast paths in the projection itself

- Child filtering: the mapping a parent hands to children and the page origin are taken once per parent (`HandedMap`), and offset-placed children without their own transform read only the record and two corners (`HandedMap::finite_under`); the parent chain's three answers (hidden, aligned, hands offsets) are asked once per projected node (`ChildChain`). When the parent is the document, the full check runs (`0 + x` and `x` differ at `-0`).
- On a page with only layout styles and records, the transform chain takes the parent's mapping directly for in-flow, relatively positioned boxes without their own transform (`plain_transform_step`).
- `axis_aligned` looks only at each box on the chain (scroll, transform, `fixed` / `sticky`, 3D, parent is an open triggered menu), and globally keeps only animation overlays and clipping visuals.
- Before projecting, `ProjectionMemo` is reserved by the number of affected nodes.

## 8. Code map

`crates/nana-ui-runtime/src/`:

| File | Responsibility |
| --- | --- |
| `node_space.rs` | `NodeSpace` bits and derived answers |
| `store.rs` | `NodeRecord::space`, `note_node_space`, the `space_pins` / `page_bound` indexes, `check_node_spaces` |
| `layout_result.rs` | `LayoutFrame`, the local `LayoutResult` |
| `world.rs` | `layout_box`, `layout_origin`, `record_layout_origin`, `layout_box_from`, `local_component_geometry`, `publish_layout_results`, `layout_result_projects_new_geometry`, `layout_result_reshapes` |
| `world/mutation.rs` | `write_local_layout`, `reframe_layouts`, `settle_layout_moves`, `mark_transform_kept` |
| `world/extraction.rs` | `extract_nodes_reusing`, `keeps_select_menu`, `check_records_kept_under_moves` |
| `world/geometry.rs` | `derive_component_geometry`, `local_select_menu_room`, `local_select_menu_surface` |
| `world/scroll_bounds.rs` | `ContentBoundsIndex`, `MaxTree` |
| `layout_engine.rs`, `layout_engine/placement.rs` | `may_move_line_boxes`; `origin_in`, `content_origin_in`, `place_remote_overlays` |
| `layout_engine/verify.rs` | Layout recomputation comparison and `retained_line_boxes_are_fresh` |

`crates/nana-ui-runtime/src/world/accessibility/` (root file `accessibility.rs`: `ProjectionMemo` and the public projection entry points):

| File | Responsibility |
| --- | --- |
| `space.rs` | `AccessibleSpace`, `AccessibleTransform`, `HandedMap`, `ChildChain`; space classification and the transform handed to children |
| `bounds.rs` | Viewport bounding box, ancestor transform chain (`accessibility_ancestor_transform`, `plain_transform_step`), and clipping |
| `project.rs` | Full projection of a single node |
| `virtual_nodes.rs` | Virtual nodes such as menu items, tree rows, and Select options |
| `frames.rs` | Bounds relative to the parent space and viewport frames (`offset_bounds_in_parent`, `turned_space_inverse`, `viewport_frame_in_parent`) |
| `delta.rs` | The incremental transaction `project_accessibility_delta`, `moved_root_hands_offsets`, `style_frames_its_space` |
| `pins.rs` | `reach_moved_pins`, `reach_turned_pins`, `reach_moved_subtree`, `keeps_relative_bounds` |
| `verify.rs` | Cold comparison in tests and under `layout-verify` |
| `relative_tests.rs`, `viewport_tests.rs` | Tests for relative bounds, pins, and viewport frames |

`crates/nana-ui-runtime/src/world/hit_test/` (root file `hit_test.rs`: modules and re-exports for `world`):

| File | Responsibility |
| --- | --- |
| `clip.rs` | Rounded-corner and path clipping decisions |
| `index.rs` | Entries, index structure, bounding boxes (`entry_bounds`, `turned_bounds`), and insert / remove / update (`insert_tree`) |
| `bounds.rs` | Ordered bounding-box tree of siblings |
| `space.rs` | Entry space, `shift`, child seeds, and window / layout coordinate conversion (`entry_space`) |
| `query.rs` | Index traversal and `open_hit` |
| `candidates.rs` | Paint keys, overlays and menus, candidates, and topmost hit |
| `scroll.rs` | Scroll `shift` |
| `build.rs` | Building entries and the forest, whole-tree rebuild |
| `patch.rs` | Subtree patching and withdrawal |
| `in_place.rs` | In-place rebuild (`rebuilt_hit_entry_in_place`) and range rebuild (`rebuild_hit_test_scoped`) |
| `verify.rs` | Moved-entry verification in tests / `layout-verify`, eligibility audit under `debug_assertions` |
| `moved_turned_tests.rs`, `moved_scroll_tests.rs`, `build_tests.rs`, `presentation_tests.rs`, `scroll_invalidation_tests.rs` | Tests |

`crates/nana-ui-scene/src/scene/` (root file `scene.rs`: modules, re-exports, the `UiScene` / `SceneNodes` structures and read-only access):

| File | Responsibility |
| --- | --- |
| `geometry.rs` | `SceneRect`, `AffineTransform`, `ClipRegion` |
| `slots.rs` | `PrimitiveId` and the slot namespace |
| `paint_order.rs`, `order.rs` | Group prefixes, paint order keys, and root layers; paint order projection |
| `kinds.rs` | `ScenePrimitive` and primitive kinds |
| `placement.rs` | Records where it now stands: `placed_origin`, `paint_origin`, `place_rect`, `primitive_layout_bounds` |
| `attributes.rs` | Shared projection attributes: `SceneDraw`, `DrawAttributes`, `draw_primitive` |
| `ancestors.rs` | Ancestor folding of transforms, opacity, and clip chains, and ancestor decisions |
| `groups.rs` | Opacity / filter / edge fade / inner shadow groups |
| `delta.rs` | Delta application: removal, translation, rebasing, and re-extraction; cull index translation and recomputation |
| `retained.rs` | Retained primitives looked up, added, and removed by node |
| `audit.rs` | Audit of retained and moved primitives |
| `visibility.rs` | Ordered bounding-box hierarchy for culling (`translate_subtree`, `refresh_subtree`, `bounds_match`) |
| `primitives.rs`, `primitives/` | Building primitives by visual (`offset_bound` / `origin_bound` are registered here) |
| `component_text.rs` | Component text primitives, icons aligned to text |
| `text_effects.rs`, `text_primitives.rs` | Text effects and glyph presentation; span backgrounds, editor marks, inline objects |
| `visual_quads.rs` | Quads, images, contrast rings, focus rings, strokes, and batch primitives |
| `presentation.rs` | Whether inline object animation and chart / glyph presentation are playing |
| `custom_paint.rs` | `Painter` recordings turned into primitives |
| `composition.rs`, `compositor.rs`, `sticky.rs` | Compositing projection, compositor layers, sticky |

## 9. Verification and tests

### 9.1 Cold comparison and audit

| Guard | Builds | Compares what |
| --- | --- | --- |
| `layout_engine::verify` (`retained_matches_full_layout`) | Tests, `layout-verify` | After every pass of retained layout it lays out from scratch again; boxes are bitwise equal |
| `retained_line_boxes_are_fresh` | Tests, `layout-verify` (also runs under `skip_layout_verify`) | Every line box memo left after a layout pass equals a fresh walk; reading a memo additionally compares on every read (up to 4 096 nodes) |
| `check_scroll_content_extent` | Tests, `layout-verify` | Every read of the content extent is bitwise equal to an index built from scratch |
| `assert_matches_cold` (`reflow_oracle.rs`) | Tests | Layout and hit geometry (including `entry_offset`) match a cold rebuild |
| `check_node_spaces` | Tests, `layout-verify` | The classification and the two indexes (see 2.3) |
| `check_moved_hit_entries` | Tests, `layout-verify` | After patching, entries replaced in place and every entry below them are compared field by field (including `origin`, `pinned`, `scroll`, `local_children`, bounding box) against a full rebuild; entries above that carry a `shift` are compared in their current space, within a rounding scale (`max(1e-4, 1e-6·|v|)`) |
| `audit_hit_eligibility` | `debug_assertions` | Pointer eligibility |
| `hit_frames_itself` assertion (`build.rs`) | Tests, `layout-verify` | The classification answer agrees bitwise with the matrix decision |
| `check_records_kept_under_moves` | Tests, `layout-verify` (up to 1 024 nodes) | Nodes whose box in this batch changed relative to the record the renderer holds: records below them that were not re-extracted are compared one by one with a cold extraction |
| `geometry_read_page` assertion | Debug | Locally derived component geometry did not read the page |
| `audit_moved_primitives` | Debug (up to 512 nodes) | After every delta carrying a translation, clone and rebuild everything, then compare the translation root's and its descendants' primitives, placed origins, pure-translation transforms, and clip rects |
| `audit_retained_projection` | Debug | The retained projection matches a fresh computation; when the cull index was translated it compares `placed_mismatch` (leaves plus the owed translation, counted in ulps of the box's magnitude, `bounds_match`), and checks that inner nodes cover their children |
| `check_kept_accessibility` | Tests, `layout-verify` (up to 4 096 nodes) | Each node that was not projected is compared with a cold projection (`ShippedAccessibility` records what was handed out) |
| `check_moved_pins` | Tests, `layout-verify` | Nodes and subtrees found from pins equal the result of walking the whole tree by the old rule; every pinned box is in the index |
| `check_offset_bounds` | Tests, `layout-verify` | The four corners of an offset-placed box through the parent space's mapping land on the positions of the layout box's four corners through the whole transform chain |
| `check_viewport_frame` | Tests, `layout-verify` | The same for a viewport box that frames itself |
| `offset_placed`, `moved_root_hands_offsets`, `plain_transform_step`, `HandedMap` assertions | Tests, `layout-verify` | The fast path and the slow path are bitwise equal |

### 9.2 Main tests

Twin-page tests compare the incremental world with a twin page that only does full rebuilds, bitwise, every frame:

- `crates/nana-ui-scene/tests/moved_extraction.rs`: translation, changing colour / text / theme in the same frame; records against a cold extraction, primitives against a cold scene, bitwise; `moved_controls_that_read_little_of_the_page_keep_what_they_built`, `an_open_select_moved_keeps_its_menu_until_it_flips`, `a_moved_subtree_that_reads_the_page_paints_like_a_cold_scene`.
- `crates/nana-ui-scene/tests/turned_container.rs`: a container changing its turn, scale, and origin continuously, with translation, scrolling, and insertion in between; the hit grid, the accessibility tree, and the scene are compared with the twin page; `rows_moved_under_a_turn_with_a_popover_open_keep_a_cold_projection`, `rows_grown_under_a_3d_turn_keep_a_cold_projection`.
- `crates/nana-ui-scene/tests/viewport_frames.rs`: the page root, `fixed`, `sticky`, and escaped absolute positioning turning and scaling continuously; `AccessibilityPlacements` checks the bounding boxes.
- `crates/nana-ui-scene/tests/child_placement_records.rs`: after children move inside their parents, every record in the Scene is compared with a fresh extraction.
- `crates/nana-ui-scene/tests/select_popup_hit.rs`: primitives are placed onto the page (`place_rect`) and then compared with the page geometry.
- `crates/nana-ui-runtime/src/world/hit_test/moved_turned_tests.rs`: translation under nine turned ancestors; a 60 × 36 pointer grid is compared with the twin page and with a compositor-layer walk (`visit_presentation_roots`); `boxes_moved_under_a_planar_3d_turn_answer_as_a_fresh_index`. `moved_scroll_tests.rs`: translation inside a scrolling list; `stuck_sticky_boxes_are_built_again_alone`: a sticky header holding a row of cells and a modal frame whose body is sticky, scrolled by uneven steps — each step rebuilds only the two sticky entries, the pointer grid matches a cold build, and the stuck body takes the pointer only inside the body box the frame laid out.
- `crates/nana-ui-runtime/src/world/accessibility/relative_tests.rs`: unaligned steps, scrolling, scaling, turning, pins, Select, resize in place; `a_scroll_projects_the_scroller_and_its_pins_alone` scrolls a list holding a sticky header, a `fixed` box, an absolute box and a nested scroll container by uneven steps, scrolls both containers in one frame, scrolls while rows move and finally scrolls back to the origin: every frame's delta equals a full projection to the bit, and only the container and its pins project; `placed_corners_are_the_turned_corners_of_a_cold_projection` is the equivalence test.
- `crates/nana-ui-runtime/src/world/line_box_memo.rs`: the line box memo is cleared by seed category.
- `crates/nana-ui/src/scene_paint/tests.rs`, `rows_a_reflow_moves_by_a_fraction_paint_what_a_cold_scene_does`: a row moving 0.37 px in one frame at 1, 1.1, 1.2, and 1.75×; compared with a cold scene pixel by pixel.

### 9.3 What must run on a change

```bash
cargo fmt --all -- --check
cargo test -p nana-ui-runtime --lib
cargo test -p nana-ui-scene --features nana-ui-runtime/layout-verify,rich-text
cargo test -p nana-ui --features hosted,gpu --lib rows_a_reflow_moves_by_a_fraction
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
```

- The runtime's unit tests are already `cfg(test)`, so the cold comparisons all run; the Scene integration tests run the runtime's cold comparisons inside only with `nana-ui-runtime/layout-verify`; `rich-text` turns on the Markdown pages.
- `nana-ui`'s `scene_paint` sits behind `gpu`; without the feature, matching 0 tests still shows as passing.
- The snapshot expects zero pixel change (`gallery-sidebar-collapsed-dark` is a known flake).
- Cost: under `perf/micro/stack-depth`, `PRINT_STAGES=1 cargo run --release -- edits 2000 height`, plus the `width`, `transform`, and `scale` pages with `CONTROLS=1`, `MIXED=1`, `SCROLLED=1`, `ROTATED=1`, `VISIBLE=1`, and `OPEN_SELECT=1`; alternate with the baseline commit for 8 rounds, swapping the order each round and reading the minimum, and take instruction counts as the difference between two `/usr/bin/time -l` runs, `RUNS=240` and `RUNS=40`, divided by 200. `PRINT_COUNTERS=1` prints the work counters of the last edit. Results go into [Resource budgets](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-time).

### 9.4 Mutation experiments

Every retention rule added must be shown to be guarded by a test: break the rule temporarily (for example, make the decision always answer "keep", drop one marking, change the summation order, or make the fast path always hold), run the tests above, confirm that at least one turns red, then revert. If none turns red, either add a test or write in a code comment that the guard is defensive and why it cannot be constructed. When a rule affects only cost and not output, mutation experiments cannot see it; use a benchmark or work counters.

## 10. Known limitations

The following are deliberately still handled per node or per whole tree, and each has been checked against the current code.

- **3D and perspective** (`transform_3d`, `perspective`, `preserve-3d`): hit testing does not hand out local coordinates and does not rebuild in place (`rebuilt_hit_entry_in_place` refuses perspective and `space.depth()`), and translation and transform changes patch whole; accessibility falls back to the bounding box and decides node by node below a translation root; the cull index recomputes whole when an inherited transform carries perspective. Reason: perspective does not enter the affine, a 3D context must pass the transform down, and the platform's accessibility `transform` is only 2D affine.
- **Modals, animation overlays, length animation, clipping visuals**: accessibility translation roots and turned boxes resized in place both fall back to node-by-node decisions (`moved_root_hands_offsets` does not hold). Reason: an overlay can transform any box, a clipping visual clips every box below it, and a modal's accessibility box is the surface.
- **An open MenuSurface**: still derived from the page (`READS_PAGE`), and when it or a box above translates the whole node is re-extracted. Reason: the surface is the menu items' bounding box, menu items' reference frame is the page and layout repositions them on every translation (flipping against the viewport), and the local surface is "menu item page box − trigger origin"; after both sides move by the same amount the subtraction differs in the last bit. Only one menu is open at a time.
- **A Painter with a non-identity local transform** (`origin_bound`, `custom_paint.rs`): the local transform is about the corner point recorded on the page (`about_origin`), and the translation amount is a function of page coordinates. To make it local, the painter would have to apply the local transform before adding the origin, and the composed matrix would round in a different order, so turned painter content would differ in the last bit and even in antialiasing. Only painter nodes with a non-identity local transform enter the set.
- **An icon whose reference frame is not its parent** (`PlacementRead::Page`): the difference is taken between the two page origins. To be independent of translation it would have to walk both reference frame chains to a common ancestor and subtract there, which would change the icon's `y` in the last bit. Only appears for an absolutely positioned icon under a text host, or when the host is an open MenuSurface.
- **Chart hover lines**: snapping to the page's half pixel is the desired behaviour (a line that does not snap is blurry), and it exists only while the pointer hovers.
- **Inline objects with per-glyph presentation**: `present_inline_objects` writes a scale about the page centre into the primitive's transform, the same structure as the painter (the transform comes after the origin), and it exists only while a presentation animation runs.
- **Text in a transition viewport**: the viewport is the intersection of a clip on the page with the text box, and the clip does not necessarily follow the translation; it exists only during enter and exit transitions, and returns to an ordinary local record afterwards.
- **Accessibility of an open Select**: still a pin (`VIRTUAL`), and on translation the field, the list box, and the visible options are reprojected. Reason: the list box's and options' bounds come from intersecting component geometry on the page with a space on the page and mapping that through the viewport into the field's space, so after a translation the last bit changes; not pinning would mean rewriting this projection to compute from local geometry and local space (the output differs in the last bit) and adding a "menu box unchanged" skip condition to the pin. Only one menu is open at a time, and the node count is in the single digits to low teens.
- **A viewport-framed box that follows its parent's translation** (`sticky` inside a turned space, absolute positioning measured from a moving row, a `fixed` that turns itself): it reprojects itself (`keeps_relative_bounds` answers "do not keep" for `framed`), bounds come from mapping the page position, and the last bit changes with the translation; offset-placed boxes below it do not move.
- **Still handled whole on a transform change**: switching between framing itself and only translating (the space changed); a box that clips itself (descendants' `clip` is in viewport coordinates); a scale under a 3D context (`style_frames_its_space` does not hold); the whole-patch branch for hit entries that do not hand out local coordinates; and `refresh_subtree` for subtrees whose ancestors' projection changed in the cull index, where after a turn the bounds are not obtainable by a translation (the cost has not been measured separately).
- **A constant O(moved rows) remains for translation inside a scroll container**: the content extent gets one leaf write per moved row (lazy range translation cannot be bitwise: a leaf is `fl(child extent + offset)`, and "old leaf + displacement" differs in the last bit); accessibility does one viewport box decision, one clip lookup, and one transform memo per moved row (whether a child "has a viewport box" depends on whether the child's own page box through the mapping is finite, which the parent cannot decide once).
- **What a scroll still costs**: the scroll container itself projects again, and its child list is rebuilt with it (O(direct children), about 0.2 M instructions on the 21 001-box page); each sticky box rebuilds its own entry in the hit index and projects its own node in accessibility (stuck or not: the sticky offset has to be computed to know whether it changed, and the node ships with its child list, as above); a scroll container nested in one that scrolled, and any clipping box in a scroll container, project whole in accessibility (`clip` is in viewport coordinates). All of these follow the pins; a pin's cost grows neither with the boxes it holds nor with the number of boxes.
- **The translation root itself**: its relative offset really changed, so accessibility, hit testing, and extraction must each handle every translation root; this is the lower bound.

## 11. Checklist for a new feature

When adding a layout feature (a new positioning mode, reference frame, or clipping mode) or a new visual:

1. **Classification**: does it decide a new kind of spatial relationship from the node's own style, reference frame, kind, or visual? If so, add a bit to `NodeSpace` and set it in `of_style` / `of_visual` / `of`. Answers that depend only on the chain above are not stored as bits; put them with the derived answers in 2.4.
2. **Maintenance points**: if the field deciding this bit is written outside `note_node_space`'s existing call sites (insertion, `set_visual`, `note_layout_anchor_change`, `write_local_layout` changing the reference frame, `reframe_layouts`), add a call site. `check_node_spaces` reports stale when one is missed.
3. **Derived answers**: go through the table in 2.2 one by one — does it change the answers of `places_by_offset`, `hands_offsets`, `pins_accessibility`, `keeps_space`, `aligned_by_style`, `hit_frames_itself`, or `page_bound`? In particular:
   - If it makes the box read geometry other than its own offset → hit testing must pin it (`HitEntry::pinned`), and accessibility must enter `pins_accessibility`;
   - If its visual reads the page → `geometry_reads_page` or `MENU_ROOM`, entering `page_bound`; if it reads only its own offset → `offset_bound`; if it reads its page position → `origin_bound`;
   - If it changes children's coordinate space → the hit index's `local_children` / in-place rebuild conditions, accessibility's `AccessibleSpace`, and the Scene cull's translation amount all need reviewing.
4. **Layout**: is its field classified in `may_move_line_boxes`; do all cross-reference-frame reads use the same summation order; do newly added fields in the result enter `layout_result_projects_new_geometry` / `layout_result_reshapes`.
5. **Cold comparison tests**: add a page to the twin-page tests (the matching group in `moved_extraction.rs`, `turned_container.rs`, `viewport_frames.rs`, or `relative_tests.rs`), translate by an unaligned step (1.37 px) with scrolling and same-frame changes in between, and compare frame by frame; run the commands in 9.3 with `layout-verify`; run a mutation experiment for every retention rule added.
6. **Limitations**: anything still handled per node goes into section 10 with its reason.

## 12. Evolution

| Commit | Content |
| --- | --- |
| 3d2f6f02e | Design and phased plan |
| 35045d816 | Phase 1: layout authority becomes relative, the public API relies on derivation |
| 715cc0331 | Phase 2: hit entries become local, a translation rebuilds only the root entry |
| 3e9c83b80 | Phase 3: only translation roots are sent; extraction, Scene, and accessibility no longer handle descendants one by one |
| 6838427fb | Phase 4: primitives and component geometry become local, the painter places at the origin before snapping |
| 7fa3b81b4, f47021db4, 2f0479886, 21f8058fa | Wrap-up: accessibility found from pins; page-reading nodes become local; accessibility last bit; end-to-end benchmarks |
| 40b898c12, dd3179b0f, 917a1112e, 16a42ab4f, 66cb370c0 | Wrap-up: accessibility scrolling and Select; in-place hit rebuild in scroll containers, cull corrections and child filtering; content extent |
| a2539876a, 1e9322016, 54e3425e7, 29f4c3e4d | Wrap-up: accessibility frames its own transform; hit testing hands out local coordinates; a transform change marks only the box itself |
| d8eabffea, 02c09ef2a, 6e21c8579, 1ab1855eb | Wrap-up: line box memo cleared by category; viewport frames and scaling; a change to child placement alone also marks RENDER; an open Select derives locally |
| be0a703c8 | Node space classification `NodeSpace` and the `space_pins` / `page_bound` indexes |
| 596930d22 | A box that frames its own transform and is resized in place is always a traversal root |
| e443a2a1e, e41729572, 138c79333 | Split `world/accessibility/`, `world/hit_test/`, and `nana-ui-scene/src/scene/` |
| a2b52d862 | A scroll changes the scroll container alone: `AccessibilityNode::scroll` and `SystemWork::accessibility_scrolled`, a scrolled container projects itself and its pins; sticky hit entries rebuilt in place |
| 2572617f5 | Restore the `non_scroll_hit_dirty` decision and clearing lost in merge 3fab98a65: a scroll container changing its box in the same frame no longer takes the scroll fast path |
| 36c731159, 62a76afe6 | A previously stuck sticky node projects again when the scroll returns to the origin; the sticky offset goes into the hit entry's `origin`: a stuck sticky box rebuilds only its own entry, and a scroll's hit cost no longer grows with the boxes in sticky branches |

Benchmarks for each phase are in [Resource budgets · CPU time](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/resource-budgets.md#cpu-time). How each item was done and its mutation experiments are recorded in the commit messages and in this document as it stood then (`git show <commit>:docs/reference/relative-geometry.md`).
