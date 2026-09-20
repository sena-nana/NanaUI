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

### `machine_verdict: fail` — 58 fixtures

Each fixture's `*.evidence.txt` now carries a `runtime_failed:` line naming the
clause that failed. Before that it printed only `fail`, and finding out which
of ~22 conjuncts had tripped meant opening `evidence.rs` beside it — which is
why this sat at 66 for weeks without moving.

| clause | count | what it means |
| --- | ---: | --- |
| `hit_ok` | 24 | the fixture's hit-test expectation does not match what the tree does |
| `geometry_ok, hit_ok` | 12 | `sidebar-section` / `sidebar-frame` |
| `segmented_contract_ok` | 8 | segmented activation contract |
| `textarea_geometry_ok` | 6 | `textarea/{focused,invalid-focused,scroll}` |
| `tooltip_state` | 4 | `icon-button/{tooltip-delay,tooltip-edge}` |
| `segmented_geometry_ok` | 2 | `segmented-control/focused` |

The `hit_ok` group splits into two kinds, and telling them apart is the next
step for anyone picking this up:

- **Probably the harness.** Four components were passive displays missing from
  the passive list (`QrCode`, `TimeSeriesChart`, `KeyCaptureLayer`,
  `KeymapLayer`); they are in it now, which is where 66 − 58 went.
- **Probably the product, and worth a look.** `tabs/{selected,focused}` and
  `sidebar-footer/actions` report `hit=None` — a tab strip and a footer full of
  buttons should be clickable. `card/*` (18 fixtures) reports the opposite: the
  harness says a plain `Card` must not be the hit target — that is what
  `InteractiveCard` is for — but the card *is* hit, so it swallows pointer
  events. Both are product questions; neither was decided here.

### 4 snapshots paint nothing but the clear colour (`FLAT`)

They agree with any baseline recorded from them and prove nothing. Root cause
for each, so the fix is a decision and not an investigation:

- `segmented-control/{dark,light}/empty` — the track *does* paint: a 6×32
  rounded quad. It is invisible because its background is the palette's
  `background`, which is also the page behind it, and because the track's
  border renders at `border_width=0.00` even though `selection_chrome_style`
  sets `1.0`. The semantic tree already records the box and `role=RadioGroup`,
  so the pixel key adds nothing today. Worth checking why the 1px track border
  never reaches the quad — that is a visual defect in its own right, not only
  here.
- `overlay-host/{dark,light}/stacked` — the node is `0.00x0.00` with no scene
  primitives at all. An `OverlayHost` shows only the overlay its
  `OverlayHostState.active` names, and the fixture adopts one child without
  ever making it active, so the state called "stacked" stacks nothing.

## Adding an adapter## Adding an adapter

Run `--bless` on that machine and commit the new directory. Adapters are
independent; adding one does not affect the others. A software rasteriser
encodes its version in the adapter name and therefore in the key, so upgrading
it produces a new key with no baseline rather than a mysterious pixel diff.
