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
neither affects the exit code. Both are currently clear — which is recent, and
the sections below are kept because how they were cleared is the part worth
reusing.

### `machine_verdict: fail` — none left

Each fixture's `*.evidence.txt` carries a `runtime_failed:` line naming the
clause that failed, and a clause with sub-parts names the part:
`segmented_contract_ok[selection(expected=option[0] actual=option[2])]`. Before
that it printed only `fail`, and finding out which of ~22 conjuncts had tripped
meant opening `evidence.rs` beside it — which is why this sat at 66 for weeks
without moving. Naming the clauses is what cleared it: 66 → 0.

**64 of the 66 were the harness holding a contract the tree had already moved
past** — read the component's own contract before suspecting the component. The
last two were the opposite, and worth the wait: a real defect in the scene
builder that only a fixture refusing to be reclassified could have found.

| what it turned out to be | count |
| --- | ---: |
| `hit_ok` modelled passive/interactive; the real split is leaf/container | 32 |
| `geometry_ok` list missing four components that have no `ComponentGeometry` by design | 12 |
| `segmented_contract_ok` asserting the pre-self-driving contract | 8 |
| `textarea_geometry_ok` reading the caret-line highlight as a stray selection | 6 |
| four passive displays missing from the hit-test list | 8 |
| `tooltip_state` routing `tooltip-delay` into the "open" arm | 2 |
| `segmented_geometry_ok` demanding a focus ring the component never requests | 2 |
| `action_applied` reading "focus did not move" as "the state failed to apply" | 2 |
| **an icon button's tooltip never reaching the scene at all** | 2 |

Three of those are worth reading in full because the fix was a contract change
or a code change, not a reclassification:

**Self-driving segmented control.** `pointer-request`, `a11y-radio`,
`atomic-reconcile` and `controlled-commit` each asserted that activating an
option leaves `selected` where it was — `controlled-commit` even named the
variable `remained_controlled`. The control commits the activated option
itself now; the application publishing a selection is still supported, it is
just no longer required. The exerciser asserts the current contract, and the
sub-clause prints both sides when it does not hold
(`selection(expected=… actual=… before=…)`).

**Focus that moved before the state ran.** `create_tabs_fixture` focuses the
first tab, then `apply_runtime_state("focused")` focused it again;
`focus_node` answers "did focus move", so the second call said no and the
harness read that as the state failing. It now accepts either — moved, or
already there.

### The tooltip that was never in the scene

`icon-button/{dark,light}/tooltip-edge` was the last pair, and it was right.
Two things were wrong behind it.

**The clock stopped too early.** The fixture hovered, read
`next_animation_deadline()` once and advanced to it. That deadline is the hover
fade, one frame out; an icon button's tooltip is due at
`TooltipConfig::default().delay_ms` — 350ms — and never got there. The pending
`tooltip-delay` fixture beside it asked only that *a* deadline existed, so the
same drive satisfied both and the pair looked identical by construction. They
are driven apart now: `tooltip-delay` steps one deadline and asserts the
overlay is still closed, `tooltip-edge` pumps until it opens.

**And it still painted nothing.** `is_descendant_of_icon_visual` suppressed
every descendant of a node with an `Icon` standard visual. That rule is right
for what it was written for — an `<i>` bound to `IconGlyph` keeps its `path`
children, and the atlas glyph already draws them — but an `IconButton` parents
its *tooltip* to itself, and the walk swallowed that too. The tooltip node was
mounted, visible, laid out at `54.0,22.4 70.80x23.20`, and produced zero scene
primitives. The `Tooltip` component's own fixtures were blank for the same
reason — `tooltip/{dark,light}/{open,edge}` had committed baselines of an empty
frame, and `machine_verdict` passed them because their clause never asked
whether anything was drawn.

The walk now stops at the first out-of-flow box: the icon's own children are in
flow and still skipped, a surface the icon merely hosts is not.
`an_icon_visual_still_paints_the_surface_it_only_hosts` pins it beside
`icon_visual_skips_vector_children`, which still passes.

Six pixel and four semantic baselines moved, and `tooltip-edge` finally shows
what its name promised: Left requested, no room at 20px from the edge, flipped
to the right and clamped inside the viewport.

### `FLAT` — none left

A `FLAT` snapshot paints nothing but the clear colour. It agrees with any
baseline recorded from it and therefore proves nothing, which is the whole
reason the suite calls it out separately: a green count can be made of blank
frames.

`overlay-host/{dark,light}/stacked` was the last pair. The fixture adopted one
parked `Text` into an `OverlayHost` and activated nothing. A host paints only
the child its `OverlayHostState.active` names — `box_visible` is gated on
`overlay_branch_active` — so with no active overlay it painted nothing, and the
node came out `0.00x0.00`.

The state is called "stacked", and the contract line says *exclusive* stacking
order. One surface and no activation demonstrated neither half. It now adopts
two labelled `Panel` surfaces and activates one: the frame shows "In front",
"Behind" contributes no primitives, and the host measures `380x120`.

`segmented-control/{dark,light}/empty` used to be here too — its track asked
for a 1px border and painted none, so the whole fixture came out the colour of
the page behind it. See below.

**What both had in common is worth more than either fix.** A fixture's own
clause never asked whether anything was drawn, so a blank frame passed. That is
also what let four `tooltip/*` baselines sit as empty frames. `machine_verdict`
now carries `overlay_exclusive_ok`, which names both halves of the claim: that
the active overlay draws, and that every adopted sibling does not. Run against
the old fixture shape it says

```
runtime_failed: overlay_exclusive_ok[active(none)]
```

which is the defect it existed to miss. Checking a new clause against the state
it should have caught is cheap and is the only thing that separates a clause
from a comment. When you write a fixture whose name is a claim, give the claim
a clause.

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

## Two things a green run still does not tell you

The gate answers "does this frame match the one committed". It does not answer
"does this frame show anything", or "is this frame different from the one next
to it" — and those two questions are where every defect in this tree has
actually lived.

### Hash the tree; identical files are the cheapest defect probe there is

Group the committed baselines by md5 and look at what collides. It needs no
adapter, no build, and one read per file. Run over 615 snapshots it found, in
one pass:

- **11 of 14 components rendered keyboard focus identically to another state** —
  three to their resting state, three to hover, one to selection. A focus
  indicator that looks like something else is not an indicator, and every
  baseline agreed with itself.
- `gallery-sidebar-tools` was byte-identical to `gallery-controls`. It moved
  the pointer to a point nothing occupies, and the Gallery scene path never
  advanced the animation clock, so even a hit would have painted at t=0 before
  the fade started. It also named tools this Gallery's sidebar does not have.
- `gallery-workspace-dock-preview` was built from exactly the same two messages
  as `gallery-workspace` and started no drag.
- `motion-*-0520` and `motion-*-0660` were both taken in the same breath as the
  `activate_overlay` / `dismiss_overlay` beside them, catching the dialog at
  progress zero. The sequence proved the dialog is open at 590 and nothing
  about how it arrives or leaves; 540 and 680 now carry that.

Two of those are pinned as tests — `no_two_top_level_scenes_are_the_same_image`
and `a_focused_fixture_never_looks_like_a_state_the_keyboard_did_not_cause` —
and both were checked against the baselines that preceded them, where they
report 4 and 18 collisions. A guard nobody has seen fail is a comment.

The focus one compares what is **painted**, not what is recorded: a colour on an
edge of zero width normalises to `none` first. `sidebar-row` wrote
`border=#7bb9f0ff border_width=0.00` for focus, which draws nothing, and a plain
text comparison would have called that a difference and let it through.

### Sort by painted fraction; a frame can be almost as empty as `FLAT`

`FLAT` is the zero case, and it is reported. One step above it is a frame that
paints a rounding error. `pane-tree/*/nested` was 310 painted pixels in a
480x240 canvas — the two words "left" and "right" — because `PaneTree` draws no
chrome of its own and the fixture's leaves had no surface. Neither the split
nor its ratio was observable in either baseline.

Giving the leaves a surface made two things visible at once, and both are open:

- **`PaneTree` does not nest.** `project_slots` walks the tree but only ever
  emits leaves, each as a direct child of the root with a `flex_grow`; an inner
  split's axis is discarded and its ratio overwrites the parent's. A nested tree
  renders as one flat row. Every call site in the repo is depth-1, so nothing
  had noticed.
- **`PaneTree` cannot size its leaves at all**, so the split ratio has never
  had any effect. `project_leaf_slot` writes width, height and flex onto the
  *content* node, and that node's own `ComponentView::project` overwrites them
  from its authored style in the same pass. Whoever writes last wins, and it is
  never the tree: with the leaves left alone they shrink to their text
  (37.96 and 46.74 of 440 for a 0.4 split), and with the leaves asked to fill
  they come out 220/220. Neither is 176/264.

  Both defects want the same thing — `PaneTree` owning a container node per
  split and per leaf, instead of writing style onto nodes the host also owns —
  which is why neither is patched here.

The fixture is therefore still named `nested` and still cannot nest. It is left
that way deliberately, the same way `tooltip-edge` was: renaming it would hide
the finding, and fixing it means changing either the flex engine or the slot
contract, which is not a snapshot-suite change.

## Adding an adapter

Run `--bless` on that machine and commit the new directory. Adapters are
independent; adding one does not affect the others. A software rasteriser
encodes its version in the adapter name and therefore in the key, so upgrading
it produces a new key with no baseline rather than a mysterious pixel diff.
