# Component migration contract

NanaUI migrates components independently while preserving their product behavior. The public
read-only catalog is available through `component_catalog()` and `component_support()`. It is
diagnostic and acceptance metadata; it never chooses a renderer, appears in product UI, or creates
parallel application state.

## States

| State | Meaning |
| --- | --- |
| `Compatibility` | The Iced compatibility implementation remains the complete supported path. |
| `RuntimeCandidate` | A Runtime implementation exists, but behavior, strict layout, visual review, platform, or consumer evidence is incomplete. |
| `RuntimeQualified` | Runtime passed the component's functional, layout, reviewed visual, accessibility, performance, and affected-consumer gates and may become the default path. |

State changes are monotonic: `Compatibility → RuntimeCandidate → RuntimeQualified`. A regression
in a qualified component must be fixed; it must not be hidden by silently downgrading the catalog or
falling back to a second retained tree. Existing components remain supported through their current
qualified or compatibility implementation while another component advances.

## Promotion gate

Each component promotion uses one backend-neutral fixture state. The compatibility and Runtime
paths must produce the same behavior and satisfy the component's logical geometry contract,
including component/text bounds, padding, baseline, wrapping, clipping and hit area. The snapshot
runner writes the two images, a side-by-side image, a pixel-difference image, and the geometry
report to `NANA_UI_SNAPSHOT_OUTPUT` (or `target/ui-snapshots`). These images are diagnostic evidence,
not a pixel-similarity gate, and the runner never promotes a catalog entry automatically.

Iced preserves migration-era product behavior and design intent, but it is not an absolute visual
truth. Review differences against shared theme semantics, font metrics, component state contracts,
accessibility, hit testing and GPU composition. Runtime must retain its more accurate geometry and
must not copy an Iced defect, tune a fixture, blur output or change colors merely to reduce a pixel
difference. A component qualifies when its behavior, layout contract, rendering semantics and
human-reviewed visual result are correct; SSIM or exact pixel equality is not required.

Functional evidence covers every capability advertised by `ComponentSupport`, including pointer,
keyboard, focus, IME, accessibility, animation, overlay, GPU, or persistence behavior where
applicable. Rendering changes also retain one Runtime authority, one host-owned Device/Queue, and
the existing idle-redraw contract.

## Current first batch

`Text`, `Button`, `TextInput` (the compatibility `Input`) and `Checkbox` are
`RuntimeCandidate`. Their Runtime contracts exist, but candidate status remains until functional
interaction, strict layout, rendering semantics, accessibility and human visual review all pass.
Workspace, Dock, Sidebar, Overlay and other professional components remain `Compatibility` unless
their individual catalog entry says otherwise.
