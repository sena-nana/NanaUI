# Lilia QJS↔V8 pixel parity (2026-08-10)

## Verdict

On the **current Nana iced-view** path, same-session QuickJS↔V8 captures of
`home-light` / `settings-light` score **SSIM similarity = 1.0** (byte-identical
in this run). Historical ~0.92 / ~0.97 was **not** engine raster divergence.

## Root cause of legacy FAIL

| Factor | Evidence |
|--------|----------|
| Capture protocol skew | Docs used `paint-vello` for QJS and `paint-stub` for V8 |
| Bootstrap / data state | Legacy home QJS had heatmap+donut; V8 showed empty + magenta/cyan placeholder |
| Pipeline removed | `evidence-png` / paint-* features were dropped; restored as iced_wgpu offscreen |

## Closure strategy

1. **Hard gate = engine parity** via `evidence-png` same-session pairs (this doc + `baselines/l1`).
2. **Do not** promote sparse iced captures as visual acceptance vs `_accept-*`.
3. **Next:** densify L1 iced mapping (icons, cards, heatmap) until iced evidence
   approaches `_accept-nana-home-window.png` / settings; then add a separate
   fidelity gate (accept baseline ↔ regenerable iced candidate).

## Reproduce

See `docs/ui-snapshots/baselines/l1/README.md`.
