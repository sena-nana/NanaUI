# Gallery snapshot baselines

Committed reference frames for `ui-snapshots`. One PNG per snapshot, under a
directory named after the GPU adapter that produced it.

```
snapshots/<adapter-key>/<snapshot key>.png
```

## Why the tree is keyed by adapter

The comparison is exact: zero per-channel tolerance. That is affordable because
rendering is deterministic — rendering the whole suite twice on one adapter
produces byte-identical PNGs, and the offscreen target is single-sampled — and
it is necessary because no honest tolerance separates a rasteriser's edge noise
from a real regression. A one-step change to a single colour token moves 15
snapshots by `max_channel_delta=1`; a threshold loose enough to absorb backend
differences absorbs that too, and the gate stops meaning anything.

So a baseline is only valid for the adapter that recorded it. Running on an
adapter with no baseline is a failure, not a silent pass: the suite tells you
the key it looked for.

## The semantic tree

`snapshots/semantic/` is the same state matrix recorded as **resolved style**
rather than pixels: per component and mode, every fixture's layout box,
accessibility state and scene primitives, with the background, border, radius,
shadow, text colour, size and weight the theme resolved to.

It is deliberately not keyed by adapter. Nothing in it is rasterised, so there
is only one correct answer on every machine — which is the point: a reviewer
whose GPU has no PNG baseline can still tell whether light/dark and the
hover / pressed / focused / disabled / selected / checked / invalid states
still resolve to the values the design system intends. It is also what makes a
token change readable: the diff names the roles that moved instead of counting
pixels.

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic --bless
```

It does not replace the pixel suite. A semantic snapshot cannot see a
rasteriser bug, and the pixel suite cannot say *why* a colour moved. Issue #101
keeps both.

## Recording a baseline

Recording is always explicit. There is no fallback that writes a baseline from
the frame it was supposed to judge.

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --bless
```

Pass one or more key prefixes to re-record only part of the suite; everything
outside the prefix stays under verification, so an intended change to one
component cannot quietly bless a regression in another:

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --bless component-migration/chip/
```

Blessing also deletes baselines the suite no longer renders, so the tree cannot
accumulate references nobody checks.

### Before blessing a large set, bound what *you* changed

A prefix keeps other components under verification, but it does not tell you
whether the change inside your prefix is yours. Baselines go stale: this tree
was recorded once at `352bd2492` and 338 commits later 421 of 615 snapshots
disagreed with it, none of them from the branch that noticed. Blessing then
means signing for two weeks of other people's work.

Bound your own footprint with an A/B on one machine:

```bash
git checkout <base>            # then build and run
cp -R target/ui-snapshots "$SCRATCH/base"
git checkout <your branch>     # rebuild and run
# compare "$SCRATCH/base" against target/ui-snapshots by md5, not against the
# committed baseline — that difference is yours and nobody else's.
```

Two traps this catches:

- **`--bless <prefix>` over-matches.** `.../settings-page` also takes
  `settings-page-full`; a bare `.../segmented-control` takes every state under
  it. Diff `git status` against your A/B list afterwards and revert the extras.
- **A fixture can be platform-dependent even here.** Anything reading
  `WindowChrome::platform_default()` renders differently per OS. Pin it in the
  fixture (`AppTitleBar::new(..).native_controls(false)`), or the baseline is
  only true on the machine that recorded it. The semantic tree has the same
  exposure — it is adapter-independent, not `cfg(target_os)`-independent.

### Reading a large diff without opening 355 images

Sort by changed-pixel fraction. A convergence of spacing or type tokens lands
around 1–2% with the bbox hugging text runs; something structural shows up as a
large fraction or a changed image size. Check the outliers individually and the
tail statistically. The 2026-09-20 re-record did exactly this — median 1.29%,
no size changes, four outliers inspected by eye — and the per-snapshot table is
in `docs/performance-data/gallery-pixel-rerecord-2026-09-20/`.

## Two advisories the suite prints, and neither is a pixel failure

Both are the fixture's own contract check, not the pixel comparison, and
neither affects the exit code.

### `machine_verdict: fail` — 14 fixtures

Each fixture's `*.evidence.txt` carries a `runtime_failed:` line naming the
clause that failed. Before that it printed only `fail`, and finding out which
of ~22 conjuncts had tripped meant opening `evidence.rs` beside it — which is
why this sat at 66 for weeks without moving. Every one triaged since has turned
out to be the harness holding a contract the tree had already moved past, not a
product defect.

| clause | count | finding |
| --- | ---: | --- |
| `segmented_contract_ok` | 8 | **the harness asserts the pre-self-driving contract** |
| `segmented_geometry_ok` | 2 | `segmented-control/focused`, unexplained |
| `tooltip_state` | 2 | `icon-button/tooltip-edge` never opens its tooltip |
| `action_applied` | 2 | `tabs/focused` |

**`segmented_contract_ok`.** The four failing states — `pointer-request`,
`controlled-commit`, `a11y-radio`, `atomic-reconcile` — are four of the five
states that expect exactly one activation request. The fifth,
`selected-repeat-request`, re-activates the option that is *already* selected
and passes. The difference is `selection_ok`, which demands
`selected_after == selected_before`: the contract from before
`SegmentedControl` became self-driving. The control now commits the selection
itself, the pixel baselines were re-recorded for that change, and this
exerciser was not.

**`segmented_geometry_ok`.** Only `focused`. The arithmetic all checks out by
hand against the semantic dump — options 55.94 + 52.86 + 73.82, two 2px gaps
and the 6px track inset give exactly the recorded 192.62 track width, option
heights are the expected 26, and each label sits inside its option. Whatever
fails is one of the `SelectionOption` geometry or slot lookups, and finding it
needs the clause broken down further the way `runtime_ok` was.

**`tooltip_state`.** `icon-button/tooltip-edge` reports `tooltip=Some(..)` with
`active_overlay=None` — the pending state, not an open one. Its sibling
`tooltip-delay` had the identical observation and has been routed to the delay
contract, which it satisfies. `tooltip-edge` reads as wanting an *open* tooltip
at a viewport edge, so the fixture is probably not advancing the hover clock to
the deadline. That is a fixture question, not a contract one.

Cleared since: 32 `hit_ok` (leaf / container, above), 12 `geometry_ok`
(`SidebarFrame` / `SidebarFooter` / `SidebarSection` / `OverlayHost` have no
`ComponentGeometry` by design and were missing from the list), 6
`textarea_geometry_ok` (slot 1 is the caret-line highlight when there is no
selection — `multiline && focused && selection.is_none()` — and the check
predated it), 2 `tooltip_state`, 8 hit-test misclassifications.

### 2 snapshots paint nothing but the clear colour (`FLAT`)

`overlay-host/{dark,light}/stacked`: the node is `0.00x0.00` with no scene
primitives at all. An `OverlayHost` shows only the overlay its
`OverlayHostState.active` names, and the fixture adopts one child without ever
making it active, so the state called "stacked" stacks nothing.

`segmented-control/{dark,light}/empty` used to be here too. It is not any more:
its track asked for a 1px border and painted none, so the whole fixture came
out the colour of the page behind it. See below.

### The border that never painted

`LayoutStyle::paint_border_edges` zeroes any side whose colour is absent — CSS's
rule, and the right one for a colour written in CSS. But an L3 component names
its border as a `SemanticColorRole`, so the colour resolves onto the *computed*
style and never reaches the `LayoutStyle`. The scene then took the width from
that CSS-only view and the colour from the semantic one: an `Outlined` card
supplied `border_width: Some(1.0)`, supplied a colour, and painted neither.

168 primitives across 15 component families were affected. The fix is
`paint_border_edges_with(colour)` — the caller passes the colour it is actually
going to stroke with, so width and colour come from one decision. Pinned by
`a_border_coloured_outside_the_layout_still_strokes_when_the_caller_names_it`,
which also checks that `border-style: none` and a zero width stay zero.

## Adding an adapter## Adding an adapter## Adding an adapter

Run `--bless` on that machine and commit the new directory. Adapters are
independent; adding one does not affect the others. A software rasteriser
encodes its version in the adapter name and therefore in the key, so upgrading
it produces a new key with no baseline rather than a mysterious pixel diff.
