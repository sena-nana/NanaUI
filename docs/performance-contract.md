# NanaUI Performance Contract

This is the Issue [#8](https://github.com/sena-nana/NanaUI/issues/8) contract that
CI, benches, and runners must cite. It is not a claim that every gate already
runs. Status below matches this workspace on 2026-08-18.

Historical Gallery numbers stay in [`performance-baseline.md`](performance-baseline.md).
Do not use those numbers to assert that Runtime already meets the relative
Iced/GPUI gates in §2.

Shared Scenario files and runners: [`perf/README.md`](../perf/README.md).

## 1. Same scenario, not a lone absolute frame time

Every major comparison must hold all of the following constant:

```text
same device
same OS / power mode
same window size
same DPI
same font and font size
same node data
same interaction script
same Release/LTO configuration
same sampling window
```

and must implement:

```text
Scenario
├── NanaUI runner
├── Iced runner
└── GPUI runner
```

Demos that differ in application, data structure, or interaction script are not
a comparison. Workload parameters live in `perf/scenarios/*.json`. Runners must
not invent their own tree size.

## 2. Relative gates — not yet enforceable

P0 uses a relative reference, not an absolute leadership claim. Versus the
**faster of Iced and GPUI** on the same Scenario:

| Metric | Gate |
| --- | --- |
| P50 CPU frame cost | ≤ 1.15× |
| P95 | ≤ 1.20× |
| P99 | ≤ 1.25× |
| steady-state memory | ≤ 1.20× |

Also required by #8, independent of the multipliers:

- static UI with no mutation / animation / external frame must not keep producing UI frames;
- a single-node paint or hover mutation must not default to full-tree layout;
- large lists must be virtualized;
- idle must not scan/execute every ECS system.

These multipliers are an engineering starting line, not an industry standard.
They should tighten once both reference runners exist and a fixed machine is
recording history.

**Not yet enforceable.** GPUI is a stub (exit 2). Iced StaticTree uses
`engine/iced` `scenario-bench` on the same complete-binary-heap as Nana
(`same-scenario` when the report declares that generation). Gallery
`ui-benchmark` `--from-report` stays `closest-legacy-reference`. Relative
gates stay off until GPUI also emits real same-scenario numbers. Fake GPUI
or Iced numbers are forbidden.

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
| `text-table` | `Table` | `rows: 10000`, `columns: 100`, `visible_rows: 40`, `visible_columns: 16`, `overscan_rows: 8`, `overscan_columns: 2`, `short_cell_len: 8`, `wrapped_cells: 4`, `wrapped_cell_len: 256` |
| `animation` | `Animation` | `active: 1` |
| `ime` | `Ime` | `scripts: ["latin", "zh", "ja", "ko"]` |
| `dock-workspace` | `DockWorkspace` | `panes: 8` |
| `overlay` | `Overlay` | `kinds: ["tooltip", "context_menu", "modal", "popup"]` |
| `text-editor` | `TextEditor` | `document_chars: 100000`, `visible_lines: 40` |

`virtual-list-1m` has a Scenario file and Nana mapping, but it stays out of `harness_ids` so default `--all` is not exit 2. The framework 1M row is skipped unless `NANA_PERF_SCALE=large`.

`text-table` is in `harness_ids` and maps onto the Nana framework table 10k×100 scale (wrapped cells in the catalog). Glyph cache fields stay omitted.

`animation` / `ime` / `dock-workspace` / `overlay` / `text-editor` are in `harness_ids` and map onto isolated Nana contexts (not the shared list drain). `gpu-scene-ui` materializes UiOnly from JSON on `nana-gpu-scene-benchmark`; it stays out of `harness_ids` because CI has no GPU. Live2D compositions stay unmapped (no encode path; do not emit 0).

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
| Nana | `python3 perf/runners/nana/run.py --scenario <id>` | Thin map onto existing `nana-runtime-benchmark`, `nana-framework-benchmark`, `nana-scene-benchmark` | **partial** |
| Iced | `python3 perf/runners/iced/run.py --scenario <id>` | StaticTree → `engine/iced` `scenario-bench` (same complete-binary-heap as Nana). Gallery `ui-benchmark` `--from-report` stays a legacy wrap | **partial** (StaticTree same-scenario; other kinds exit 2) |
| GPUI | `python3 perf/runners/gpui/run.py --scenario <id>` | Same CLI/schema; returns `unsupported` | **stub** |

Exit codes: `0` ok, `1` error, `2` unsupported. CI must distinguish 2 from 1.

Nana mapping (existing binaries are not dedicated Scenario processes):

| Scenario | Binary | Extracted fields | Gaps |
| --- | --- | --- | --- |
| StaticTree 100/1k/5k/10k | `nana-runtime-benchmark` + `nana-scene-benchmark` | enqueue, commit, initial systems (when present); scene extraction/idle/frame-graph | memory/allocations not exported |
| StaticTree 50k | `nana-runtime-benchmark` `kind=construction` | enqueue/commit/paint/hover only | full systems pass missing |
| PaintOnly | `nana-runtime-benchmark` `local_paint_*` at 5k | systems P50/P95/P99, `local_paint_work` WorkCounters including `layout_nodes` when present | invariant `layout_nodes == 0` is evaluable when the field is present; missing stays **not-evaluable** |
| Text / LayoutStyle / Visibility / Transform / Accessibility | `nana-runtime-benchmark` `single_node_mutations.<kind>` at 5k | systems/commit/schedule + WorkCounters | remaining §3.2 kinds. 5k full case only; Iced/GPUI stay unsupported |
| Hover | `nana-runtime-benchmark` `pointer_hover_*` | only when the report has `nodes == 10000`; `pointer_hover_work` WorkCounters when present | otherwise **unsupported** (exit 2). Do not substitute a smaller tree. `layout_nodes == 0` is evaluable when the field is present |
| VirtualList 10k/100k/1M | `nana-framework-benchmark` `virtual_scales` (legacy `virtual_list_10k_*` fallback) | materialize/window; `live_ui_entities` when `virtual_scales` exists | overscan row count may differ from contract `overscan: 8`; 1M needs `NANA_PERF_SCALE=large` or the scale row is skipped (runner exit 2) |
| Table / `text-table` | `nana-framework-benchmark` `virtual_scales` `kind=table` 10k×100 | materialize/window, `live_ui_entities`, WorkCounters text shaping/cache (`text_shaped`, `text_shaped_runs`, `text_layout_cache_hits`/`misses`, `text_wrap_layouts`, `cache_eviction`); catalog `wrapped_cells` copied | `glyph_cache_*` omitted (`None`); Iced/GPUI stay **unsupported** (exit 2). 100k table is extra binary coverage, not a catalog id. 1M table needs `NANA_PERF_SCALE=large` |
| Animation / Ime / Dock / Overlay / TextEditor | `nana-runtime-benchmark` `catalog_animation` or `nana-framework-benchmark` `catalog_workloads` on isolated contexts | due samples / script counts / panes / overlay kinds / local-edit `text_shaped` | Iced/GPUI stay **unsupported**. Must not share the list-scroll AppContext (`input_hit_test==41`) |
| GpuScene `gpu-scene-ui` | `nana-gpu-scene-benchmark` from `perf/scenarios/gpu-scene-ui.json` | UiOnly materialization + encode/submit WorkCounters (`gpu_upload_bytes`, draw/batch) | Not in `harness_ids`. Missing adapter exit 2. Live2D compositions have no path; do not emit 0 |

Iced mapping: `engine/iced` `scenario-bench` builds StaticTree as the same complete-binary-heap (`parent(i)=i//2`, element-div, no text). That path is `same-scenario` when the report declares the generation. Gallery `ui-benchmark` `--from-report` remains `closest-legacy-reference` (`static-tree-100` → `list-100`, `static-tree-1k` → `list-1000`). Hover, mutation, VirtualList, Table, animation, IME, dock, overlay, editor, GPU scene, and StaticTree sizes without a scenario-bench run stay **unsupported** (exit 2). Relative gates stay off until GPUI also emits same-scenario ok.

GPUI: no crate, workspace member, or adapter exists. Plug-in path is
`perf/runners/gpui/adapter.py` implementing `run_scenario(scenario, args)`.

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

Current `DirtyMask` / `SystemWork` (from `nana-ui-runtime` `schedule.rs`):

| Issue bit | Current bit | Notes |
| --- | --- | --- |
| STATE | *(none)* | mutations go through `MutationQueue` / generation; not a dirty flag |
| STYLE | `STYLE` | |
| TEXT_SHAPE | `TEXT` | |
| LAYOUT | `LAYOUT` | |
| TRANSFORM | *(none)* | no dedicated bit; layout/render |
| HIT_TEST | `INPUT` | |
| PAINT | `RENDER` | paint-only style uses STYLE+RENDER; unit test `paint_only_style_change_does_not_schedule_subtree_layout` |
| ACCESSIBILITY | `ACCESSIBILITY` (A11Y) | |
| *(not in issue)* | `FOCUS_IME` | extra current bit |

A single `dirty = true` is already not the Runtime model. TRANSFORM and STATE
are still **missing** as distinct bits. Layout-stop when intrinsic or wrap
height is unchanged is in Runtime; wrapping `LayoutBox` constraints apply.

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
  `glyph_cache_hits` / `glyph_cache_misses` (`None`), `cache_eviction`,
  GPU keys `None` until encode/submit);
- `SystemWork::counters()` and `UiWorld::last_work_counters()`;
- `FOCUS_IME` on `SystemWork`, not on `WorkCounters`.

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
| `glyph_cache_hits` / `glyph_cache_misses` | glyph atlas | **`None` / omitted** — Runtime has no `GlyphCache` |

Still **off / unsupported** on CPU-only drains (do not invent numbers):

- process-wide allocations/frame and peak temporary bytes (no allocator hook);
- persistent UiWorld / RenderWorld / glyph-cache **memory bytes**;
- glyph-cache eviction (no glyph backend; do not treat `cache_eviction` as glyph trim);
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

There is no perf history database. `runtime-work-invariants` in
`.github/workflows/ci.yml` is a unit-test stand-in, not envelope JSON.

## 8. CI layering

### 8.1 Ordinary PR CI

Public GitHub runners must not block a PR on a small timing delta (issue:
not 5% timing). Prefer correctness, work counters, allocation counts, and
algorithmic invariants, for example:

```text
hover_without_size_change: layout_nodes == 0
single_text_patch:         text_shaped <= bounded_expected_count
virtual_list_100k:         live_ui_entities <= bounded_visible_cache
static_ui:                 frames_after_idle == 0
```

Those assertions are evaluated from runner JSON when `work_counters.layout_nodes`
is present. A missing or null field stays **not-evaluable** and must never be
treated as 0. `runtime-work-invariants` in `.github/workflows/ci.yml` is a
unit-test stand-in, not envelope JSON. Do not treat the weekly timing
workflow as that gate.

### 8.2 Timing CI / fixed machine

Real timing must run on a **fixed** benchmark machine: fixed CPU/GPU, OS,
power mode, display/DPI, driver; no heavy background load; long-lived
baseline storage.

**Current stand-in (not a fixed machine):**
`.github/workflows/runtime-performance.yml` is a weekly cron
(`23 3 * * 1`) on `ubuntu-latest` and `macos-latest`. Shared GitHub-hosted
images move underfoot; this cron is **not** Issue §12.2.

## 9. Exemption process

Any change that exceeds a threshold that *is* in force must:

1. Record the regression (scenario id, counters or percentiles, commit);
2. State the cause;
3. File or cite a follow-up issue, or record an explicit waiver;
4. **Not silent-merge.**

This is process text. There is no GitHub required status named
“performance-waiver” in this change. Until relative gates are enforceable,
a waiver still applies to any existing `validate-runtime-performance.py`
failure: explain it in the PR; do not merge a red gate quietly.

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
   `misses` and `cache_eviction`. `glyph_cache_*` stay **`None` (omitted)**.

5. Large List/Tree/Table must be virtualized.
6. UiWorld → RenderWorld must support incremental extraction.
7. Allocation / GPU upload / draw/batch counts must be measurable.
   CPU hot-path `allocations` / `allocated_bytes` are measurable. GPU upload
   and draw/batch are measurable on Scene encode/submit (`GpuWorkObservation`).
   Runtime-only drains omit GPU keys.

8. Critical workloads must keep being compared to Iced/GPUI on the same machine.

## 12. Definition of done vs this workspace

Copied from issue §16 so the contract does not drop DoD. Status is honest.

| DoD | Status |
| --- | --- |
| Documented Performance Contract | **done** (this document) |
| Iced / GPUI / NanaUI reference runners on shared workload | **partial** (Nana map + Iced scenario-bench StaticTree + Gallery wrap + GPUI stub) |
| Every critical frame stage independently profiled | **partial** (`FrameProfiler` + `FrameStage`; GPU stages ran on Scene encode, unsupported on Runtime-only) |
| Work counters can locate algorithm regressions | **partial** (`WorkCounters` includes `input_targets`, `render_nodes_changed`, CPU hot-path `allocations` / `allocated_bytes`, text shaping/cache, and GPU keys after encode; process-wide malloc stays omitted) |
| ECS dirty/incremental automatic asserts | **partial** (unit tests + binary asserts + weekly semantic gates); PR invariants in progress |
| Virtualized list/tree/table scale gates | **partial** (10k/100k `virtual_scales`; 1M harness id env-gated; virtual tree materializer + Fenwick in `nana-ui-core` / `AppContext::materialize_virtual_tree`; scale benches still list/table) |
| Allocation, memory, GPU upload, draw/batch recorded | **partial** (CPU hot-path `allocations` / `allocated_bytes`, text shaping/cache, and GPU encode observations; process-wide malloc and peak temp bytes stay **unsupported**) |
| Fixed benchmark machine with history | **missing**; weekly `ubuntu-latest`/`macos-latest` is not it |
| P50/P95/P99/max and frame-budget misses comparable | **partial** on existing binaries (`frame_budget_misses` on new reports); not same-Scenario vs Iced/GPUI |
| Native RHI vs WGPU same RenderPlan | **NO-GO / deferred by #7 Gate B** |
| #7 large architecture stages must pass this gate | process; not a substitute for the gaps above |

Issue §15 phases 0–4 remain open except the contract/schema/runner scaffolding
in this tree and whatever sibling agents land. Microbench (§10), macro/E2E
window runner (§11), Gallery 10k-control stress and real NanaShader/Studio/Live
app benches (§13) are **required by #8 / not implemented**.

## 13. Non-goals

Unchanged from the issue: NanaUI need not beat Iced/GPUI on every workload;
do not sacrifice IME, accessibility, correctness, or API maintenance for a
score; do not block PRs on tiny public-runner timing noise; do not assume
Native RHI is faster than WGPU; do not treat an ECS/GPUI-like API as proof.

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
5. Relative gates stay off until Iced and GPUI both emit `status: ok` with
   real metrics on the same id.
6. Do not add a GPUI git submodule or invented timings to “complete” the
   triangle.
7. Native RHI A/B is out of scope for #8 while Gate B is NO-GO.
8. Layout-stop / counter / scale benches should keep the work-counter names
   in §7 (`layout_nodes`, `live_ui_entities`, `text_shaped`, …) so PR
   invariants can be wired without another rename.
