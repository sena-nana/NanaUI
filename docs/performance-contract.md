# NanaUI Performance Contract

This is the Issue [#8](https://github.com/sena-nana/NanaUI/issues/8) contract that
CI, benches, and runners must cite. It is not a claim that every gate already
runs. Status below matches this workspace on 2026-08-19.

**#8 acceptance is Nana-owned.** Work-counter / catalog / hotspot gates, plus
CI that fails on an abnormal Nana regression. Living close is §12; leftovers
are §15 / §16. Iced or GPUI same-batch numbers are Issue
[#12](https://github.com/sena-nana/NanaUI/issues/12) observation, not a
completion condition. Relative multipliers (P50 1.15× / P95 1.20× /
P99 1.25× / memory 1.20×) are **not** #8 DoD. Animation is **in-force**
on `animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8`
(live dump 1/64; self-test `considered=1` / `scanned=64` failed).
`due==1` is attendance, not the pass.

Historical Gallery numbers stay in [`performance-baseline.md`](performance-baseline.md).
Do not use those numbers to claim a Nana gate passed.

Shared Scenario files and runners: [`perf/README.md`](../perf/README.md).

## 1. Same Nana scenario, not a lone absolute frame time

Nana gates must hold the catalog workload constant:

```text
same Scenario id and params
same node data
same interaction script
same Release/LTO configuration
same sampling window
```

Workload parameters live in `perf/scenarios/*.json`. Runners must not invent
their own tree size. Timing CI also needs a fixed device / OS / window / DPI
(see §8.2). Iced and GPUI runners may emit the same Scenario envelope for
Issue #12 observation.

## 2. Relative Iced/GPUI multipliers — not #8 acceptance

The old P0 table versus the faster of Iced and GPUI was:

| Metric | Historical multiplier |
| --- | --- |
| P50 CPU frame cost | ≤ 1.15× |
| P95 | ≤ 1.20× |
| P99 | ≤ 1.25× |
| steady-state memory | ≤ 1.20× |

If comparison returns, it belongs with
[#12](https://github.com/sena-nana/NanaUI/issues/12). `--evaluate-relative` is
honesty fail-closed observation, not multiplier CI.
`relative_gate_enforceable` stays **False**. Weekly GHA is not a named fixed
machine. Fake numbers stay forbidden. Do not wait on GPUI in #8 CI.

#8 still requires these **Nana** invariants (independent of any multiplier):

- static UI with no mutation / animation / external frame must not keep producing UI frames;
- a single-node paint or hover mutation must not default to full-tree layout;
- large lists must be virtualized;
- idle must not scan/execute every ECS system.

## 3. Shared Scenario catalog

Harness files (runners consume these today):

| id | kind | params |
| --- | --- | --- |
| `static-tree-100` | `StaticTree` | `nodes: 100` |
| `static-tree-1k` | `StaticTree` | `nodes: 1000` |
| `static-tree-5k` | `StaticTree` | `nodes: 5000` |
| `static-tree-10k` | `StaticTree` | `nodes: 10000` |
| `static-tree-50k` | `StaticTree` | `nodes: 50000` |
| `mutation-paint-only` | `Mutation` | `tree_nodes: 5000`, `kind: PaintOnly` |
| `mutation-text` | `Mutation` | `tree_nodes: 5000`, `kind: Text` |
| `mutation-layout-style` | `Mutation` | `tree_nodes: 5000`, `kind: LayoutStyle` |
| `mutation-visibility` | `Mutation` | `tree_nodes: 5000`, `kind: Visibility` |
| `mutation-transform` | `Mutation` | `tree_nodes: 5000`, `kind: Transform` |
| `mutation-a11y` | `Mutation` | `tree_nodes: 5000`, `kind: Accessibility` |
| `hover` | `Hover` | `nodes: 10000`, `size_change: false` |
| `virtual-list-10k` | `VirtualList` | `items: 10000`, `visible: 40`, `overscan: 8`, `text_len: 32` |
| `virtual-list-100k` | `VirtualList` | `items: 100000`, `visible: 40`, `overscan: 8`, `text_len: 32` |
| `virtual-tree-10k` | `VirtualTree` | `items: 10000`, `visible: 40`, `overscan: 8`, `text_len: 32` |
| `virtual-tree-100k` | `VirtualTree` | `items: 100000`, `visible: 40`, `overscan: 8`, `text_len: 32` |
| `text-table` | `Table` | `rows: 10000`, `columns: 100`, `visible_rows: 40`, `visible_columns: 16`, `overscan_rows: 8`, `overscan_columns: 2`, `short_cell_len: 8`, `wrapped_cells: 4`, `wrapped_cell_len: 256` |
| `animation` | `Animation` | `active: 1` |
| `ime` | `Ime` | `scripts: ["latin", "zh", "ja", "ko"]` |
| `dock-workspace` | `DockWorkspace` | `panes: 8` |
| `overlay` | `Overlay` | `kinds: ["tooltip", "context_menu", "modal", "popup"]` |
| `text-editor` | `TextEditor` | `document_chars: 100000`, `visible_lines: 40` |
| `gpu-scene-ui` | `GpuScene` | `composition: UiOnly`, `viewport: [800, 480]`, host_texture 640×360 `content`, `ui_nodes: ["list", "text", "gpu-texture-view", "button"]` |

`virtual-list-1m` has a Scenario file and Nana mapping, but it stays out of `harness_ids` so default `--all` is not exit 2. The framework 1M row is skipped unless `NANA_PERF_SCALE=large`.

`virtual-tree-1m` is the same env-gated path for `virtual_scales` `kind=tree`. Iced/GPUI stay unsupported.

`text-table` is in `harness_ids` and maps onto the Nana framework table 10k×100 scale (wrapped cells in the catalog).

`animation` / `ime` / `dock-workspace` / `overlay` / `text-editor` / `gpu-scene-ui` are in `harness_ids`. IME / Dock / Overlay / TextEditor map onto isolated Nana CPU contexts and are §8.1 gated from live dirty counters. `animation` is §8.1 gated on `animations_considered` and `animation_deadlines_scanned` from the sparse `advance_animations` deadline-index walk (`due==1` is attendance, not the pass). `gpu-scene-ui` materializes UiOnly from JSON on `nana-gpu-scene-benchmark`; missing adapter is honest exit 2. Live2D compositions stay out of `harness_ids` (no encode path; do not emit 0).

`VirtualList` parameters are the issue’s example shape:

```text
Scenario::VirtualList { items, visible, overscan, text_len }
```

Other workstreams that add a 100k/1M bench **must** use
`virtual-list-100k` / `virtual-list-1m` and those params, not a private name.

Record sets follow the issue: construction, reconciliation, style, text
shaping, layout, render extraction, render encode, memory, allocations for
trees; dirty-propagation counters for mutations; scroll CPU, P99, live entity
count, text shape count, GPU upload, draw/batch count for lists.

## 4. Runners

| Runner | Command | What it actually does | Status |
| --- | --- | --- | --- |
| Nana | `python3 perf/runners/nana/run.py --scenario <id>` | Thin map onto existing `nana-runtime-benchmark`, `nana-framework-benchmark`, `nana-scene-benchmark`, `nana-gpu-scene-benchmark` | **partial** |
| Iced | `python3 perf/runners/iced/run.py --scenario <id>` | #12 observation via `engine/iced` `scenario-bench`. Not a Nana #8 gate. Visibility / Transform / Accessibility and StaticTree 50k stay **unsupported**. Gallery `--from-report` stays a legacy wrap | **#12 observation** |
| GPUI | `python3 perf/runners/gpui/run.py --scenario <id>` | #12 observation via excluded `engine/gpui-scenario-bench`. Not a product renderer and not a #8 gate | **#12 observation** |

Exit codes: `0` ok, `1` error, `2` unsupported. CI must distinguish 2 from 1.

Nana mapping (existing binaries are not dedicated Scenario processes):

| Scenario | Binary | Extracted fields | Gaps |
| --- | --- | --- | --- |
| StaticTree 100/1k/5k/10k | `nana-runtime-benchmark` + `nana-scene-benchmark` | enqueue, commit, initial systems (when present); `frames_after_idle` after settle; scene extraction/idle/frame-graph | memory/allocations not exported |
| StaticTree 50k | **unsupported** (incomparable) | Nana `nana-runtime-benchmark` is `kind=construction` only (enqueue/commit/paint/hover). Iced `scenario-bench` would otherwise run a full 50k layout+draw. Neither runner emits `ok` until both sides share that work definition. |
| PaintOnly | `nana-runtime-benchmark` `local_paint_*` at 5k | systems P50/P95/P99, `local_paint_work` WorkCounters including `layout_nodes` when present | invariant `layout_nodes == 0` is evaluable when the field is present; missing stays **not-evaluable** |
| Text / LayoutStyle | `nana-runtime-benchmark` `single_node_mutations.<kind>` at 5k | systems/commit/schedule + WorkCounters | 5k full case. Iced/GPUI `scenario-bench` Mutation is same-scenario for these kinds only (5k heap, single node) |
| Visibility / Transform / Accessibility | `nana-runtime-benchmark` `single_node_mutations.<kind>` at 5k | systems/commit/schedule + WorkCounters | Iced stays **unsupported** (exit 2). Height-0, Shadow, or widget Id is not Nana `hidden` / `PaintTransform.e` / `set_accessibility`. |
| Hover | `nana-runtime-benchmark` `pointer_hover_*` | only when the report has `nodes == 10000`; `pointer_hover_work` WorkCounters when present | otherwise **unsupported** (exit 2). Do not substitute a smaller tree. `layout_nodes == 0` is evaluable when the field is present. Iced/GPUI `scenario-bench` Hover is same-scenario at 10k |
| VirtualList 10k/100k/1M | `nana-framework-benchmark` `virtual_scales` (legacy `virtual_list_10k_*` fallback) | materialize/window; `live_ui_entities` when `virtual_scales` exists | Nana runner passes catalog window (`visible×extent` viewport, `overscan×extent` px; 10k/100k: 800 / 160 / 20). Standalone binary still defaults to 200px overscan. 1M needs `NANA_PERF_SCALE=large` or the scale row is skipped (runner exit 2). Iced/GPUI materialize only that catalog window |
| VirtualTree 10k/100k/1M | `nana-framework-benchmark` `virtual_scales` `kind=tree` | Fenwick `VirtualTreeLayout` construction (expanded parent+2-leaf forest) + window + `materialize_virtual_tree`; `live_ui_entities` | Same catalog list window as VirtualList (10k/100k: 800 / 160 / 20). Standalone / weekly default stays 200px overscan. 1M needs `NANA_PERF_SCALE=large` or the scale row is skipped (runner exit 2). Empty `status=ok` without `materialize_ms` / `live_ui_entities` is forbidden. Iced/GPUI stay **unsupported** (exit 2); a VirtualList window is not a disclosure tree |
| Table / `text-table` | `nana-framework-benchmark` `virtual_scales` `kind=table` 10k×100 | materialize/window, `live_ui_entities`, WorkCounters text shaping/cache (`text_shaped`, `text_shaped_runs`, `text_layout_cache_hits`/`misses`, `text_wrap_layouts`, `cache_eviction`, `glyph_cache_hits`/`misses` from the bench `MeasureTextShaper` em-width `GlyphCache`); catalog `wrapped_cells` copied | §8.1 requires `glyph_cache_hits ≥ 1` and `glyph_cache_misses ≥ 1` on that em-width cache, not the product `NanaTextShaper` hinter. wrap/eviction attendance is not this gate. Nana runner passes the catalog table window (`visible` 16×40, `overscan` 2×8 → 1280×800 / 160×160 px at 80×20). Standalone / weekly default stays 10-row overscan (`TABLE_OVERSCAN.y=200`). A leftover 200 px report KeyErrors. Iced `scenario-bench` Table is same-scenario on that catalog window (virtualized cells only; no invented `text_shaped`). GPUI same-scenario on that window. 100k table is extra binary coverage, not a catalog id. 1M table needs `NANA_PERF_SCALE=large` |
| Animation | `nana-runtime-benchmark` `catalog_animation` on an isolated UiWorld | idle/scheduled `next_animation_deadline` + `advance_animations`; records `due_animation_samples` / `scheduled_animations` plus `work.animation_deadlines_scanned` / `work.animations_considered` from the sparse due-index advance | Hotspots `animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8` (live dump 1 of 64). Full-table 64 fails even if the other counter stays 1; modest extra due samples must not. `due==1` is attendance, not the pass. Missing field stays **not-evaluable**. Iced/GPUI stay **unsupported**. A tweening widget is not that scheduler work. |
| Ime | `nana-framework-benchmark` `catalog_workloads` ime | `set_ime_preedit` / `commit_ime` on a focused TextInput for latin/zh/ja/ko | Hotspot `layout_nodes == 0` from the live dump. `ime_script_count==4` is attendance, not the pass. Full-tree IME layout must fail. Iced/GPUI stay **unsupported**. `text_input` typing is not that dirty work. |
| Dock | `nana-framework-benchmark` `catalog_workloads` dock-workspace | `assemble_dock` of `eight_pane_root` + `adjust_focused_dock_split`; catalog `panes=8` | Hotspot: `layout_nodes ≤ 40` and `render_nodes_changed ≤ 40` (live dump 25 / 33 of 45). Full-tree 45 fails; one extra chrome node must not. `panes==8` is not the pass. Iced stays **unsupported** (exit 2). Topology-only `pane_grid` is not Nana chrome rebuild. GPUI stays unsupported. |
| Overlay | `nana-framework-benchmark` `catalog_workloads` overlay | OverlayHost activate/dismiss + `toggle_popover` | Hotspot: `layout_nodes ≤ 4` and `entities_changed ≤ 4` (live dump 1 / 2 of 5). Full overlay-tree fails; do not pin `==1`/`==2`. `overlay_kind_count==4` is attendance. Iced/GPUI stay **unsupported**. Always-on tooltips are not that dirty work. |
| TextEditor | `nana-framework-benchmark` `catalog_workloads` text-editor | one TextArea, `editor_document(100000)`, caret-local `replace_text_area_selection` then `drain_text` | Hotspot `layout_nodes == 0`. `text_shaped<=1` is not the pass (one TEXT node stays green under full-tree layout). Iced stays **unsupported** (exit 2). A cached view+layout+draw after an untimed edit is not that dirty work. GPUI stays unsupported. Must not share the list-scroll AppContext (`input_hit_test==41`) |
| GpuScene `gpu-scene-ui` | `nana-gpu-scene-benchmark` from `perf/scenarios/gpu-scene-ui.json` | UiOnly materialization + encode/submit WorkCounters (`gpu_upload_bytes`, draw/batch) | §8.1 honest-ok for Nana encode envelopes. Missing adapter exit 2. Live2D stays out of `harness_ids` (no encode path; do not emit 0) |

Iced mapping: `engine/iced` `scenario-bench` builds StaticTree, Mutation (PaintOnly / Text / LayoutStyle only), and Hover as the same complete-binary-heap (`parent(i)=i//2`, element-div). StaticTree may export `frames_after_idle` for triangulation; a busy `request_redraw` probe must be non-zero before 0 is emitted. `--evaluate-invariants` skips Iced envelopes. Missing `work_counters.layout_nodes` on Iced Hover / PaintOnly stays **not-evaluable**, never envelope-ok. Visibility / Transform / Accessibility stay **unsupported**. VirtualList materializes only the catalog window (`visible` + `overscan` rows; 10k/100k: 800×160 px at 20 px). Table materializes only the catalog table window (`visible` 16×40, `overscan` 2×8 at 80×20 px). The Nana runner passes the same windows into `nana-framework-benchmark`. Those wired paths are `same-scenario` when the report declares that generation. StaticTree 50k is **unsupported** on both Iced and Nana until they share a work definition (Nana is still construction-only). Gallery `ui-benchmark` `--from-report` remains `closest-legacy-reference` (`static-tree-100` → `list-100`, `static-tree-1k` → `list-1000`). Animation, IME, dock, overlay, editor, GPU scene, and VirtualTree stay **unsupported** (exit 2). A VirtualList window is not a Fenwick disclosure tree. Topology-only Iced `pane_grid` is not Nana `assemble_dock` chrome. A cached Iced editor frame is not Nana `replace_text_area_selection` + `drain_text`.

`overscan_rows`: catalog Table (and list/tree) overscan is **8 rows**. Iced copies that catalog param; Nana `nana-framework-benchmark` writes `mounted − visible`. Do not equate the two fields — compare windows via `list_overscan_px` / `table_overscan_y_px`. Nana extract is `same-scenario` only when the dump declares the catalog window (`list_viewport_px` / `list_overscan_px` / `list_item_extent_px`, or the table viewport/overscan/extent px fields); missing fields stay `closest-legacy-reference`. `window_ms` is index arithmetic (Fenwick lookup may round to 0); judged work is `materialize_ms` + `live_ui_entities`, not `window_ms`.

GPUI is #12 observation (excluded `engine/gpui-scenario-bench`; not a product
renderer; not #8 DoD). Iced and GPUI live dumps go under
`target/performance/issue12/`. Unwired kinds stay exit 2 with no metrics
(`present_ms` / `frames_after_idle` omitted, not 0).

`--from-report` maps a JSON the binaries already wrote. `--print-plan` prints
the cargo command without running it.

## 5. Frame stages

Issue §4 requires a per-frame profiler at least:

```text
Input, Reconcile, Style, Text Shape, Layout, Hit Test, Accessibility,
Animation, Render Extract, Batch, GPU Upload, Command Encode, Submit,
CPU Total, GPU UI, GPU Live2D, GPU Effects, Present / frame latency
```

A single `frame_time` is not enough.

### Map onto current Runtime / Scene

| Issue stage | Current name | Evidence in this workspace |
| --- | --- | --- |
| Input | `SystemWork.input_hit_test` + `DirtyMask::INPUT` + `FrameStage::Input` | `FrameProfiler` can time it; not yet a required Frame N dump from every host |
| Reconcile | `MutationQueue` commit / `FrameStage::Reconcile` | commit timing in `nana-runtime-benchmark` |
| Style | `SystemWork.style` / `DirtyMask::STYLE` / `FrameStage::Style` | `resolve_styles` |
| Text Shape | `SystemWork.text` / `DirtyMask::TEXT` / `FrameStage::TextShape` | host shaper via `RuntimeDocument::flush` |
| Layout | `SystemWork.layout` / `DirtyMask::LAYOUT` / `FrameStage::Layout` | `RuntimeLayoutEngine` |
| Hit Test | `input_hit_test` / `rebuild_hit_test` / `FrameStage::HitTest` | scheduled from INPUT |
| Accessibility | `SystemWork.accessibility` / `DirtyMask::ACCESSIBILITY` / `FrameStage::Accessibility` | incremental projection |
| Animation | Runtime-owned animation sample / `FrameStage::Animation` | sparse sample in runtime bench |
| Render Extract | `SystemWork.render_extraction` / `DirtyMask::RENDER` / `FrameStage::Extract` | `extract_nodes`; Scene `apply_delta` |
| Batch | `FrameStage::Batch` | **Runtime-only: unsupported**. `SceneWgpuPainter` times it after encode |
| GPU Upload | `FrameStage::GpuUpload` | **Runtime-only: unsupported**. Observed `queue.write_buffer` on encode/submit |
| Command Encode | `FrameStage::Encode` | **Runtime-only: unsupported**. `status=ran` after `SceneWgpuPainter::paint` |
| Submit | `FrameStage::Submit` | **Runtime-only: unsupported**. Filled after host `record_submit` |
| CPU Total | `FrameProfile.cpu_total` / Gallery `cpu_total_ms` | **partial** |
| GPU UI / Live2D / Effects | weekly `ui-live2d-acceptance` totals | **partial**; not the same RenderPlan split |
| Present / frame latency | *(none)* | **missing** |

`nana_ui_core::{FrameStage, WorkCounters}` and `nana_ui_runtime::FrameProfiler`
exist. Batch / GPU Upload / Encode / Submit report `unsupported` with zero
duration on Runtime-only hosts. `SceneWgpuPainter` records `GpuWorkObservation`
and those stages after a real encode; missing adapter does not emit 0. A host
that never calls `FrameProfiler` still does not produce Issue §4 Frame N output.
`FOCUS_IME` is an extra dirty bit, not an Issue §4 stage name.

## 6. Dirty bits

Issue §6.2 requires at least:

```text
STATE, STYLE, TEXT_SHAPE, LAYOUT, TRANSFORM, HIT_TEST, PAINT, ACCESSIBILITY
```

Current `DirtyMask` / `SystemWork` (from `nana-ui-runtime` `schedule.rs`).
Mask is `u16` (was `u8` with 7 bits used).

| Issue bit | Current bit | Notes |
| --- | --- | --- |
| STATE | `STATE` (`1 << 7`) | `SystemWork.state`. Hover/press/focus/`SetInteraction`. STYLE only when interaction paints exist |
| STYLE | `STYLE` (`1 << 0`) | |
| TEXT_SHAPE | `TEXT` (`1 << 1`) | |
| LAYOUT | `LAYOUT` (`1 << 2`) | |
| TRANSFORM | `TRANSFORM` (`1 << 8`) | `SystemWork.transform`. Paint-transform `SetStyle`; not STYLE/LAYOUT. INPUT+RENDER because hit-test and extract read the matrix |
| HIT_TEST | `INPUT` (`1 << 3`) | |
| PAINT | `RENDER` (`1 << 5`) | paint-only style uses STYLE+RENDER; unit test `paint_only_style_change_does_not_schedule_subtree_layout` |
| ACCESSIBILITY | `ACCESSIBILITY` (`1 << 6`) | |
| *(not in issue)* | `FOCUS_IME` (`1 << 4`) | extra current bit |

A single `dirty = true` is already not the Runtime model. STATE and TRANSFORM
are distinct bits. Layout-stop when intrinsic or wrap height is unchanged is
in Runtime; wrapping `LayoutBox` constraints apply.

## 7. Work counters and allocation

Issue §5 is the PR-stable signal. Timing is noisy; counters are the algorithm
gate. Required counters include `entities_total`, `entities_changed`,
spawn/despawn, `style_processed`, `text_shaped`, `layout_nodes`,
`hit_test_candidates`, `input_targets`, `accessibility_nodes_updated`,
`render_nodes_extracted` / `changed`, `batch_rebuilds`, `draw_batches`,
`draw_calls`, allocations, allocated bytes, `gpu_upload_bytes`,
`gpu_buffer_reallocations`.

Current types:

- `nana_ui_core::WorkCounters` (`entities_total`, `entities_changed`,
  spawn/despawn, `style_processed`, `text_shaped`, `layout_nodes`,
  `hit_test_candidates`, `input_targets`, `accessibility_nodes_updated`,
  `render_nodes_changed`, `render_nodes_extracted`, `extracted_text_spans`,
  `allocations`, `allocated_bytes`, `text_shaped_runs`,
  `text_layout_cache_hits`, `text_layout_cache_misses`, `text_wrap_layouts`,
  `glyph_cache_hits` / `glyph_cache_misses` (`None` until `GlyphCache` is
  consulted), `cache_eviction`,
  GPU keys `None` until encode/submit);
- `SystemWork::counters()` and `UiWorld::last_work_counters()`;
- `FOCUS_IME`, `STATE`, and `TRANSFORM` on `SystemWork`, not on `WorkCounters`.

`allocations` / `allocated_bytes` are **CPU hot-path** payload counts Runtime
can observe without a global allocator hook: dirty drain Vecs, layout-input
children clones, document-order output, and text-shape temps. Empty idle
drains report 0. They are not process-wide malloc, allocator slack, or VRAM.

Text shaping/cache (Issue §3.5 / §11.4):

| Field | What it measures | Status |
| --- | --- | --- |
| `text_shaped` | nodes with TEXT dirty this drain | drain |
| `text_shaped_runs` | `TextShaper::shape` invocations (cache misses only) | `shape_text` / `shape_text_for_layout` |
| `text_layout_cache_hits` | `TextLayoutCache::lookup` hits | real cache, lookup |
| `text_layout_cache_misses` | `TextLayoutCache::insert` after a miss | real cache, insert |
| `text_wrap_layouts` | shape calls with `wrap: true` | shaping path |
| `cache_eviction` | `TextLayoutCache` FIFO evictions | `Some(n)` after a shaping pass; `None` until consulted |
| `glyph_cache_hits` / `glyph_cache_misses` | Runtime `GlyphCache` lookup/insert | `Some` after a shaping pass that consulted the cache. The `text-table` §8.1 gate is the bench `MeasureTextShaper` em-width lookup/insert, **not** the product `NanaTextShaper` hinter. `None` / omitted when `shape_cached` is not overridden. Never a fake 0. |

Still **off / unsupported** on CPU-only drains (do not invent numbers):

- process-wide allocations/frame and peak temporary bytes (no allocator hook);
- persistent UiWorld / RenderWorld / glyph-cache **memory bytes**;
- glyph-cache FIFO trim (do not treat `cache_eviction` as glyph trim);
- GPU keys (`batch_rebuilds`, `draw_batches`, `draw_calls`, `gpu_upload_bytes`,
  `gpu_buffer_reallocations`) stay `None` until `WorkCounters::record_gpu_work`
  after encode/submit. `SceneWgpuPainter` / `nana-gpu-scene-benchmark` record
  `Some` (including 0) only on that path. Issue §7 GPU resource memory is
  **missing** as exported metrics.

Runner stand-ins, not a substitute for PR invariants:

- `local_paint_work` / `pointer_hover_work` WorkCounters on
  `nana-runtime-benchmark` JSON when the drain is measured;
- `local_paint_work_nodes == 1` and `pointer_hover_work_nodes <= 2` for older
  reports without WorkCounters;
- framework `virtual_scales[].live_ui_entities` when present;
- scene local-update primitive rebuilds.

There is no perf history database. PR `runtime-work-invariants` still runs the
named cargo tests. §8.1 runner-JSON judging is
`python3 perf/contract.py --evaluate-invariants <envelope.json>…` on Nana
fixture envelopes (live release benches are not required on every PR). Do not
treat the weekly timing workflow as a fixed machine.

## 8. CI layering

### 8.1 Ordinary PR CI

Public GitHub runners must not block a PR on a small timing delta (issue:
not 5% timing). Prefer correctness, work counters, allocation counts, and
algorithmic invariants, for example:

```text
hover_without_size_change: layout_nodes == 0
single_text_patch:         text_shaped <= bounded_expected_count
virtual_list_100k:         live_ui_entities <= bounded_visible_cache
```

`static_ui: frames_after_idle == 0` is judged for Nana StaticTree 100/1k/5k/10k.
`frames_after_idle` is the count of UI frames scheduled after the tree has
settled (Nana: non-empty `take_system_work` drains). The settle frame itself
is not counted. `idle_schedule_ms` is a timing and must never be mapped to
this field. Missing or null stays **not-evaluable** / skipped, never treated
as 0. A busy tree (pending paint dirty) must be able to produce a non-zero
count; otherwise runners refuse to emit 0. StaticTree 50k stays
**unsupported** (Nana is construction-only). An empty `invariants` array must
never count as a §8.1 pass.

CI `--from-report` success envelopes are **Nana** live dumps, not
extractor-fixture JSON and not Iced comparison jobs. One Nana `bench_full`
dump (`perf/fixtures/nana-runtime-static-tree.json`) covers StaticTree
100/1k/5k/10k, Hover 10k, Mutation PaintOnly/Text/LayoutStyle plus
Visibility/Transform/Accessibility (`single_node_mutations` at 5k).
`catalog_animation` is mapped from that dump and §8.1 gated on
`animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8`
from the sparse `advance_animations` observation (`due==1` is
attendance, not the pass). A due-only increment while scanning the
full 64-entry table must fail.
IME / Dock / Overlay / TextEditor use the live
`nana-framework-benchmark` slice
`perf/fixtures/nana-framework-catalog-workloads.json`. Do not substitute
`perf/fixtures/catalog-workloads.json`. `text-table` uses the live table
slice `perf/fixtures/nana-framework-text-table.json`
(`MeasureTextShaper` em-width `glyph_cache_hits ≥ 1` and
`glyph_cache_misses ≥ 1`). `perf/fixtures/virtual-table-scales.json`
omits `glyph_cache_*` and stays **not-evaluable** / skipped, never
envelope-ok. Putting that skip into a directory that also has honest-ok
envelopes must fail the job.

```text
cargo run --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-runtime-benchmark -- \
  --output perf/fixtures/nana-runtime-static-tree.json
```

Those catalog assertions are evaluated from runner JSON by
`python3 perf/contract.py --evaluate-invariants <envelope.json>…`, which calls
the same `evaluate_invariants` path runners already attach. A missing or null
field stays **not-evaluable** and must never be treated as 0. Evaluated catalog
ids are honest Nana `ok` envelopes with a non-empty catalog row: Mutation
PaintOnly/Text/LayoutStyle, Visibility (`render_nodes_changed` and
`layout_nodes` ≤ 64; live dump layouts 12 — not `layout_nodes == 0`),
Transform / Accessibility (`layout_nodes == 0`), Hover 10k, VirtualList 10k/100k
(catalog window 800/160/20), VirtualTree 10k/100k (same catalog-8 cap 58),
`text-table` (catalog-8 cap 1334; MeasureTextShaper em-width
`glyph_cache_hits ≥ 1` and `glyph_cache_misses ≥ 1`; missing stays skipped),
StaticTree 100/1k/5k/10k (`frames_after_idle == 0`), IME / TextEditor
(`layout_nodes == 0`), Overlay (`layout_nodes` / `entities_changed` ≤ 4),
Dock (`layout_nodes` / `render_nodes_changed` ≤ 40), Animation
(`animations_considered` and `animation_deadlines_scanned` ≤ 8 from the
sparse due-index advance; `due==1` is not the pass), and Nana
`gpu-scene-ui` encode envelopes (`draw_calls >= 1` is the real bound;
catalog also has `gpu_upload_bytes >= 0`). Missing GPU keys / adapter skip,
never vacuous 0. GpuScene
Live2D / StaticTree 50k / GPUI / VirtualList 1M / VirtualTree 1M stay
**skipped**, not invariant-ok. A PR `invariants/` directory that skips or
omits a gated Nana id while other envelopes are ok is a failed job
(exit 1). Weekly ubuntu `weekly/` judges only the envelopes it mapped —
it does not include macos-only `gpu-scene-ui`, and must not fail for that
omission.
A gated id that is present but skipped in that weekly set still
fail-closes. Unsupported-only directories stay exit 2. `runtime-work-invariants`
in `.github/workflows/ci.yml` still runs the named cargo tests; it also runs
`--self-test` and `--evaluate-invariants` on Nana fixture-derived envelopes.
That is not a live release-bench dump from every PR. Do not treat the weekly
timing workflow as that gate.

### 8.2 Timing CI / fixed machine

Real timing must run on a **fixed** benchmark machine: fixed CPU/GPU, OS,
power mode, display/DPI, driver; no heavy background load; long-lived
baseline storage.

**Current stand-in (not a fixed machine):**
`.github/workflows/runtime-performance.yml` is a weekly cron
(`23 3 * * 1`) on `ubuntu-latest` and `macos-latest`. Shared GitHub-hosted
images move underfoot; this cron is **not** Issue §12.2. Weekly ubuntu
passes catalog windows via `--print-framework-window-args` and maps live
dumps through `--from-report` into `--evaluate-invariants`.

## 9. Exemption process

Any change that exceeds a threshold that *is* in force must:

1. Record the regression (scenario id, counters or percentiles, commit);
2. State the cause;
3. File or cite a follow-up issue, or record an explicit waiver;
4. **Not silent-merge.**

This is process text. There is no GitHub required status named
“performance-waiver” in this change. A waiver still applies to any existing
Nana `validate-runtime-performance.py` or §8.1 failure: explain it in the PR;
do not merge a red Nana gate quietly.

## 10. Native RHI same-RenderPlan A/B — NO-GO

Issue #8 §9 / §16 asks for WGPU vs Metal / D3D12 / Vulkan on one logical
`RenderPlan`. **This is not an Issue #8 blocker.**

[#7 Gate B](issue-7-phase-7.md) (Phase 7 Native RHI decision) concluded
**NO-GO**: do not start `nana-hal` / `nana-rhi`; WGPU stays the default
backend. Empty-pass Metal probes are not a product A/B. Reopen only if the
five Phase 0/7 conditions hold together, as a new evidence phase — not by
implementing Native RHI under #8.

## 11. Architecture invariants (issue §14)

Long-term, not currently all machine-checked:

1. No change → no UI frame.
2. Local change must not default to full-tree diff/layout/render.
3. ECS systems must not default to a full World scan every frame.
4. Text shaping/layout must have a stable, observable cache.
   Runtime `TextLayoutCache` lookup/insert fills `text_layout_cache_hits` /
   `misses` and `cache_eviction`. Runtime `GlyphCache` lookup/insert fills
   `glyph_cache_*` when `shape_cached` consults the cache. The `text-table`
   gate is `MeasureTextShaper` em-width lookup/insert, not product hintering.
   Hosts that skip that override leave them **`None` (omitted)**.

5. Large List/Tree/Table must be virtualized.
6. UiWorld → RenderWorld must support incremental extraction.
7. Allocation / GPU upload / draw/batch counts must be measurable.
   CPU hot-path `allocations` / `allocated_bytes` are measurable. GPU upload
   and draw/batch are measurable on Scene encode/submit (`GpuWorkObservation`).
   Runtime-only drains omit GPU keys.

8. Critical Nana workloads must keep being measured on the catalog ids.

## 12. Definition of done vs this workspace

**Living #8 acceptance** (supersedes historical GitHub §16 checkboxes): Nana-owned
work-counter / catalog / hotspot gates, plus CI that fails on an abnormal Nana
regression. Iced/GPUI same-batch numbers are observation, not a completion
condition. Status matches this workspace; this is not a claim that every
historical box is done.

| Living #8 DoD | Status |
| --- | --- |
| Documented Performance Contract | **done** (this document; Iced/GPUI comparison demoted) |
| Nana catalog runners + work-counter / hotspot gates | **in-force** for mapped ids whose judged rows use dirty/sparse counters (IME / Dock / Overlay / TextEditor dirty-cap; Animation `animations_considered` + `animation_deadlines_scanned`; Hover / mutation / virtualization / StaticTree idle; text-table GlyphCache hit+miss; `gpu-scene-ui` encode). Identity attendance (`due==1`, `panes==8`, `ime_script_count==4`, `overlay_kind_count==4`, `text_shaped<=1`) is **not** the pass. |
| Animation catalog hotspot | **in-force**: `animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8` (live dump 1/64; self-test `considered=1` / `scanned=64` failed). A due-only `considered` cap is not this gate. `due==1` is attendance, not the pass. |
| CI fails on abnormal Nana regression | **done** for in-force Nana gates (`runtime-work-invariants` + `--evaluate-invariants` fail-closed). Weekly GHA is a stand-in, **not** a fixed machine, and is not this row. |

Historical issue §15 phases 0–4 and the original §16 list are **not** extra
close blockers. See §15 / §16.

## 13. Non-goals

NanaUI need not beat Iced/GPUI on any workload to close #8. Do not sacrifice
IME, accessibility, correctness, or API maintenance for a score; do not block
PRs on tiny public-runner timing noise; do not assume Native RHI is faster than
WGPU; do not treat an ECS/GPUI-like API as proof. Closing #8 also does not
require microbench, a real-window E2E runner, Gallery 10k-control stress,
Present/malloc, or Live2D encode.

## 14. Schema rules other workstreams must follow

1. New workloads get a `perf/scenarios/<id>.json` with `schema_version: 1`,
   `kind`, and the param keys in `perf/schema/scenario.schema.json`.
2. Stable ids in §3 / `catalog.json` are the names CI and benches cite.
   Do not rename `virtual-list-10k` to `list10k`.
3. Runner output uses the envelope in `perf/schema/run-report.schema.json`
   (`runner`, `status`, `scenario_id`, `equivalence`, `metrics`,
   `work_counters`).
4. `equivalence` is `same-scenario` only when the runner built that JSON.
   Mapping an existing binary is `closest-legacy-reference`.
5. Relative Iced/GPUI multipliers stay off. `relative_gate_enforceable` stays
   False. Comparison, if any, is #12 observation.
6. Do not add a GPUI git submodule or invented timings.
7. Native RHI A/B is out of scope for #8 while Gate B is NO-GO.
8. Layout-stop / counter / scale benches should keep the work-counter names
   in §7 (`layout_nodes`, `live_ui_entities`, `text_shaped`, …) so PR
   invariants can be wired without another rename.

## 15. Out of #8

These were in the original GitHub issue body. They are **not** living close
conditions. Do not invent numbers to make them look done. Do not open a pile
of follow-up issues unless a later epic actually wants the work.

| Leftover | Where it lives |
| --- | --- |
| Iced/GPUI same-scenario triangle, relative P50/P95/P99/memory multipliers, fixed-machine timing | [#12](https://github.com/sena-nana/NanaUI/issues/12) (open; explicitly not #8 DoD) |
| Native RHI same-RenderPlan A/B | [#7](https://github.com/sena-nana/NanaUI/issues/7) Gate B **NO-GO** (epic already closed; do not reopen under #8) |
| Every Issue §4 stage independently profiled, including Present / frame latency | **out of #8**. `FrameProfiler` / Scene encode observations stay; Present is missing |
| Process-wide malloc / peak temporary bytes | **out of #8**. CPU hot-path `allocations` / `allocated_bytes` stay in-force |
| Microbench (issue §10) | **out of #8** / not implemented (`perf/micro/` reserved) |
| Macro / real-window E2E runner (issue §11) | **out of #8** / not implemented |
| Gallery 10k-control stress and NanaShader / NanaStudio / NanaLive app benches (issue §13) | **out of #8** / not implemented |
| Live2D encode (`gpu-scene-ui-live2d`) | **out of #8**. No encode path; stay out of `harness_ids`; do not emit 0 |
| 1M virtual list/tree unless `NANA_PERF_SCALE=large` | env-gated skip; not a close blocker |
| StaticTree 50k | **unsupported** (Nana construction-only); not a close blocker |

## 16. Historical GitHub §16 vs living close

Original issue §16 checkboxes are **historical**. Do not tick them as #8
complete. Living close is only §12. Animation is **in-force** on
`animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8` (live
dump 1/64; self-test `considered=1` / `scanned=64` failed). `due==1` is
attendance, not the pass.

| Original §16 item | Living status |
| --- | --- |
| Documented Performance Contract | **#8 DoD / done** |
| Iced / GPUI / NanaUI shared reference runner | **not #8 DoD** → #12 |
| Every critical frame stage independently profiled | **out of #8** (Present missing; GPU stages Scene-encode only) |
| Work Counter can locate algorithm regressions | **#8 DoD / in-force** for catalog hotspots that export counters; Animation **in-force** on `animations_considered ≤ 8` and `animation_deadlines_scanned ≤ 8` |
| ECS dirty/incremental automatic asserts | **#8 DoD / in-force** (unit tests + `--evaluate-invariants`) |
| virtualized list/tree/table scale gates | **#8 DoD / in-force** (10k/100k; 1M env-gated) |
| allocation, memory, GPU upload, draw/batch recorded | CPU hot-path + Scene encode **in-force**; process-wide malloc / Present **out of #8** |
| Fixed benchmark machine + history baseline | **not #8 DoD** → #12 |
| P50/P95/P99/max and frame-budget misses | **not #8 DoD** → #12 |
| Native RHI vs WGPU same RenderPlan | **not #8 DoD** → #7 Gate B NO-GO |
| #7 large architecture stages must pass this gate | process only; #7 already closed on its own §20 |
