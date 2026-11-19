# Performance

With no changes, `flush` is a no-op of about 0.0001 ms, independent of tree size. The host should not hand out an empty frame at that point.

Wakeups for animation, live GPU content and ordinary UI are separate. One piece of moving content should not force a full Runtime update. Compositor tracks only advance time in steady state and do not write back to `UiWorld`. Static windows stay at `FrameDemand::OnDemand`. When a window is occluded or minimized, the host still calls `prepare_window_frame` according to the program's own `FrameDemand`, but then does not flush, acquire the Surface, or present.

`width` and `height` go through layout. Every sample triggers another layout pass. Do not use them for scaling.

A window that needs a redraw goes roughly through these steps: drain `dispatch_program` messages, `prepare_window_frame`, `RuntimeDocument::flush`, acquire the Surface, `SceneWgpuPainter::paint_target`, submit and present, and only then `window_frame_presented`. When the compositor needs its own tick, only that window is moved to `FrameDemand::Continuous`. That does not force style or layout recomputation. A zero-size window is still prepared; producer encoding runs only when the size is drawable.

## Post-refactor baseline snapshot

Following the five-phase architecture baseline reset, Apple M4 `release` / `dist` measurements are (methodology and commands in [Resource budgets](../../reference/resource-budgets.md) and [Taffy reference benchmark](../../reference/taffy-reference.md)):

- **Stack safety with zero `stacker`**: both `measure_node` and `mount_view*` (`ViewBuilder::run_trampoline`) run iteratively on heap work stacks (`0 B` measure stack per level), while placement bounds recursion via `recursion::SPAN` + `Boundary` queues; a 500-level deep tree finishes a full frame on a **`52 KiB`** thread stack (vs. `900 KiB` for Taffy).
- **Compact per-node memory**: with `NodeRecord <= 152 B`, `Hierarchy == 24 B`, paged `Slots`, and shared `Arc<NodeStyle>`, 10k laid-out `Stack` nodes stay at `634 B` each, and `stack-depth wide` across `21,001` boxes measures **`943 B / box`** (vs. `1,506 B / box` for Taffy).
- **Local scaling & zero-bloat counters**: 100k-node local frame `p95` is `30.2 µs`, a **`1.17×`** ratio against 1k nodes (`25.8 µs`); `WorkCounters` and `HeadlessTurn::work` enforce incremental budgets in CI with zero per-node memory overhead.
- **Precompiled shaders & dist binary size**: Apple loads offline precompiled compressed MSL (`0 B` resident `naga` IR), while non-Apple hosts load compacted `NAGA1` IR; combined with `[profile.dist]`, the `reactive-sfc` binary shrinks from `15.80 MiB` to **`5.56 MiB`** (`-64.8%`).

## One pointer event

Dispatch itself is constant. The measurement is the P50 of the `window` shape on macOS / Apple Silicon, `--release`, 60 warm-up moves plus 400 moves. Production windows go through `prepare_window_frame`. The `headless` shape goes through `flush_scene_frame`, which production windows do not use.

After the per-component-type index was fixed, Rust L3 is 0.0003 ms at every size from 250 to 2,000 rows (before the fix, 2,000 rows took 0.0819 ms).

## Frames that actually changed

The constant above holds because nothing in that tree changed. Once something changes, the window frame's work comes back. Paint-only changes scale with the amount changed; frames that touched layout used to walk up to the document root and re-resolve styles for every sibling, and both of those have been fixed.

When the deferred work itself is constant, the cost is constant: 0.008–0.011 ms from 502 to 8,002 nodes. When every row below has to shift, it is still O(node count).

For long lists use `each_virtual`: mounting, laying out and windowing 50,000 rows takes 2.4 ms, against 175 ms for an `each` that builds everything at once, with about 2.3 KB resident per row. See [Lists](../essentials/list.md) for how to write it.

To stay aligned with the UI per frame, go through the host's single submission:

- Do not call `request_device` again for the UI
- Do not let live content submit through a separate Queue
- Do not read GPU content back to the CPU and paste it in again
- Do not draw another layer over the Surface after the UI is painted
- Controls do not take a window handle to call system APIs

New code enters through `nana_ui::runtime`. A `GpuTextureView` on the same string slot still takes part in layout, clipping and hit testing in this tree.

For the headless comparison, replace `--shape window` with `--shape headless`. For the scrolling version, add `--scroll`. The row count must match the one used to generate the bundle.

These numbers come from hovering on one machine; they are not a CI gate. The clock used to be frozen at 0; at that time L3 with 2,000 rows recorded 0.0242 ms, about 3.4 times more optimistic than the later 0.0819 ms. Each event now advances 16 ms, and the 0.0003 ms above is the figure after the index fix.

Measurement commands and per-phase timings are in [Cost of one pointer event](../../reference/input-cost.md).

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/accessibility">
    <p class="next-step-link">Accessibility</p>
    <p class="next-step-caption">How hidden nodes stay in the tree, and where names come from.</p>
  </a>
  <a class="next-step" href="/guide/essentials/list">
    <p class="next-step-link">Lists</p>
    <p class="next-step-caption">Rows outside the viewport need not be built all at once.</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">Enter and leave go through the compositor; steady state does not write back logical style.</p>
  </a>
</div>
