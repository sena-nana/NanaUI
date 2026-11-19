# Per-Pointer-Event Cost and Dirty-Frame Analysis

This page records the measured cost of Rust L3 under high-frequency pointer movement (hover), how the component type index converged, and the dirty-frame cost when an event handler triggers a real view update. The raw performance archives are [`archive/docs-notes/performance-data/vue-hover-2026-09-08/`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/performance-data/vue-hover-2026-09-08) and [`archive/docs-notes/runtime-dirty-frame.md`](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/runtime-dirty-frame.md).

## Key findings

1. **Pointer event dispatch itself is $O(1)$ constant cost and does not grow with tree size.** On lists of 250 to 2,000 rows, a single Rust L3 pointer move is constant at **`0.0003 ms`**.
2. **When the outstanding work of a local change is itself constant, Runtime incremental layout and painting also take constant time** (`0.008–0.011 ms` from 502 to 8,002 nodes). Changing the size of the first row shifts every row below it; that $O(N)$ part is work that must be done.

## Measured data (macOS / Apple Silicon, `--release`, 60 warm-up + 400 moves, P50 in milliseconds)

The measured subject is a column of fixed-height `24 px` rows. The pointer moves down one row at a time and wraps around, flushing one frame after each move.

| Rows | Rust L3 |
| ---: | ---: |
| 250 | 0.0003 |
| 500 | 0.0003 |
| 1,000 | 0.0003 |
| 2,000 | 0.0003 |

### Two $O(N)$ fixes for dirty frames

After removing two $O(N)$ bottlenecks, "the layout dirty set broadcast to the accessibility projection up to the document root" and "the layout engine re-resolving the effective style for every sibling," the time for a purely constant local change converged from linear growth with node count to $O(1)$: **`0.008–0.011 ms`** from 502 to 8,002 nodes.

## Component type index on the Rust L3 path (`0.0819 ms` → `0.0003 ms`)

An early benchmark always passed `Duration::ZERO` as the event timestamp, so the path with an `8 ms` throttle ran only once. This hid an L3 scan cost that grew with node count (the real cost at 2,000 rows was `0.0819 ms`). After changing the benchmark to advance a fixed `16 ms` step per event, segmented timing found four linear hot spots that "scan the whole document's `document_order` to find a specific component":

- `active_runtime_overlays` (switched to the existing `UiWorld::overlay_hosts_by_document` index);
- `split_handle_near`, `sync_split_handle_hover`, and `clear_calendar_heatmap_hover`.

**Root-cause fix**: the single authoritative `UiWorld` maintains `nodes_by_component: HashMap<ComponentTypeId, HashSet<StableNodeId>>`, updated when `SetComponentType` is applied and when a node is despawned (maintained directly inside the single authoritative `UiWorld` so every path applying `SetComponentType` stays in sync without depending on an `AppContext`-layer wrapper). After the fix, a single L3 pointer event is stable at **`0.0003 ms`** at any size, and all 559 pixel baselines in `component-gallery` match.

## Measurement commands

Input routing alone (no flush, no layout, no Scene extraction):

```bash
cargo run --release -p nana-ui-devtools --features runtime-agent \
  --bin nana-input-benchmark -- --rounds 15 --events 4000
```

How one `RuntimeDocument::flush` varies with the number of dirty nodes and total nodes:

```bash
cargo build --release -p nana-ui-scene --features benchmark --bin nana-dirty-frame-benchmark
./target/release/nana-dirty-frame-benchmark --shape layout --position head --dirty 1 --rows 2000 --samples 150 --warmup 30
```

`--position tail` changes the last row, so no rows below need to shift; any cost that still grows with the document is over-invalidation. When comparing two builds, run them interleaved and read the minimum p50.
