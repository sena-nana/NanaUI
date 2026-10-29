# Theme audit baseline

`theme-hardcoding.json` is the maintained inventory used by the repository gate.
The reports under `archive/docs-notes/performance-data/` are historical phase
evidence and keep their original source population.

The 2026-10-17 baseline records the source at
`891edf1d985f0bf20867117e28762d5030a851dc`. The September Phase 1 report scanned
131 components; the current tree has 146, including AreaLayout, PropertyGrid,
ScrubField, TimelineEditor and newer feedback controls. Their constructor
defaults, shared geometry, animation helpers and constants changed the inventory:

| Category | September phase report | Current inventory |
| --- | ---: | ---: |
| Component design numbers | 21 | 25 |
| Component motion values | 2 | 3 |
| Shared design numbers | 73 | 78 |
| Shared motion values | 19 | 27 |
| Module constants | 87 | 111 |
| Color literals, component and shared | 0 | 0 |

This records existing debt; it does not remove hardcoding or change product
appearance. The test still rejects growth in component/shared color literals,
design numbers, motion values, and module constants. Prefer moving a value to
the Theme. A population change requiring a new baseline must include a reviewed
inventory diff and its reason; do not rewrite a historical report or update the
baseline merely to hide an unexplained failure.

Regenerate and verify from the repository root:

```sh
python3 scripts/audit-theme-hardcoding.py --output scripts/baselines/theme-hardcoding.json
python3 scripts/audit-theme-hardcoding.py --check scripts/baselines/theme-hardcoding.json
python3 -m unittest discover -s scripts/tests -v
```

## 2026-10-20 re-record

The gate had been failing on `main` since the chrome marks (`84599a4db`), the
public animation-run builder (`ff9e4c895`) and the native Mermaid engine
(`7b3b98b38`, `62d4b5e9d`): shared motion 27 → 28 and module constants
111 → 114.

- Module constants are back to 111 without a new baseline: the keycap height and
  edge in `view/marks.rs` and the diagram label size in `markdown_diagram.rs`
  now read the default theme scales (`type_scale::LINE + 2 × space::XS`,
  `space::XXS`, `type_scale::BODY`), the same pixels as before.
- Shared motion 28 is recorded: the new one is `Easing::Linear` in
  `AnimationSpec::run`, the documented "linear until `with_curve`" default of a
  one-off run, not a design value a component picks.
- Shared design numbers drop 78 → 65: `markdown_drawing.rs` 14 → 2 because the
  Mermaid SVG path was removed and the new layout engine lives in the
  `nana-ui-diagram` crate, outside this scan's root; `rich_text.rs` 5 → 4.
  `markdown_diagram.rs` and `view/marks.rs` join the shared inventory with role
  reads only. Components scanned 146 → 148.

## 2026-10-29 re-record

The gate had been failing on `main` since `faf41be3c` (sena-nana/NanaUI#371).
The inventory diff against the previous baseline source (`efb90d29b`), each
entry reviewed:

- Shared design numbers 63 → 84 (+23, −2). All 23 additions are zeros, not
  design values: the empty box a migrated component's geometry falls back to
  when layout published no part (`world/geometry.rs`, 12), zero-width carets
  and runs (`text_edit.rs`, `layout_engine.rs`, `layout_engine/inline.rs`,
  `layout_engine/overlay.rs`, `inline_content.rs`, `store.rs`, `menus.rs`), a
  zero initial `gap_after`/`height` in `layout_engine/parts.rs` and `measure.rs`,
  and the reset `border_width = Some(0.0)` before AreaLayout's header divider.
  The two removals are the 14 × 14 Card spinner box `world/geometry.rs`
  placed by hand before Card's header became published parts (`faf41be3c`).
- Shared motion 28 → 30: the touch-scroll velocity window and frame interval
  in `framework/touch_scroll.rs`, sampling constants of the gesture physics.
- Module constants 113 → 126: the ten touch-scroll physics constants (slop,
  rest thresholds, deceleration, rubber band, fling), the animation channel id
  `animation::ATTENTION`, and SettingsRow's `ROW_CONTROL_MIN` (96 px) and
  `FLASH_HOLD` (0.2). The last two are real design values and belong on the
  Theme; they are recorded here and left to #371.
- The test modules `world/issue201.rs` and `world/issue205.rs` were scanned as
  shared code because they lacked an inner `#![cfg(test)]`; they now carry
  one, which removed their fixtures from the inventory.
- Component totals are unchanged (design numbers 25, motion 3). Components
  scanned 149 → 156.
