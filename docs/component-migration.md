# Component migration contract

NanaUI migrates components independently while preserving their product behavior. The public
read-only catalog is available through `component_catalog()` and `component_support()`. Consumers
use it as diagnostic and acceptance metadata; it never appears in product UI or creates parallel
application state. NanaUI derives its internal default-backend route from the same declaration so
there is no second hand-maintained list of qualified components.

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
paths are independently checked against the component's design contract, including component/text
bounds, padding, baseline, wrapping, clipping and hit area. For each state the snapshot runner
writes `iced.png`, `runtime.png`, `side-by-side.png`, `difference.png`, and `evidence.txt` under
`target/ui-snapshots/component-migration/<component>/<theme>/<state>/` (or the corresponding
`NANA_UI_SNAPSHOT_OUTPUT` root). These images are diagnostic evidence, not a pixel-similarity gate,
and the runner never promotes a catalog entry automatically.

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

`Text`, `Button`, `TextInput`, and `Checkbox` are `RuntimeQualified`; their root, `components`
aggregate, and Vue hosted default routes use Runtime while their Iced adapters remain under
`nana_ui::compatibility`. A hosted Runtime Scene never silently rebuilds a qualified component
through Iced when retained state is missing. Button passed
all semantic kinds, sizes, loading, activation, focus and accessibility review. TextInput passed
placeholder, shaped selection/caret, secure, invalid, read-only/loading, keyboard, IME preedit and
native input-purpose review. Text passed wrapping, clipping, alignment, typography and
accessibility review. Checkbox passed checked/off, hover, pressed, focused, disabled, invalid,
pointer, keyboard and accessibility activation states in dark and light. Workspace, Dock,
Sidebar, Overlay and other professional components remain `Compatibility` unless their individual
catalog entry says otherwise.

## Current second batch

`IconButton`, `Switch`, `Card`, `ListItem`, and `RangeField` are `RuntimeQualified`; their root and
aggregate public exports route to Runtime, while their Iced adapters live under
`nana_ui::compatibility`. IconButton keeps hover, pressed, external focus and persistent selected
feedback visually distinct. Switch separates its complete-row hover/pressed state layer from the
track and focus ring, rather than flattening all three into one track paint. Iced output remains a
reference rather than an oracle: an `evidence.txt` entry records the expected design result, each
backend's observed result and verdict, and the reason for an intentional divergence. No automatic
fallback or parallel tree is introduced.

## Current third batch

`StatusBadge`, `ValidationMessage`, `EmptyState`, `LabeledValue`, and `SegmentedControl` are
`RuntimeQualified`; their root and aggregate public exports route to Runtime, while their Iced
adapters live under `nana_ui::compatibility`. Iced is a migration-era reference, not a visual
oracle. Qualification is design-correctness of behavior, layout, rendering semantics, and
human-reviewed visuals; SSIM or pixel equality with Iced is not required.

Visual review accepted the Runtime frame for this batch. Iced differences are not
defects: weaker EmptyState Primary, nearly invisible Segmented focus, and a
LabeledValue action beside the value instead of the trailing edge.

SegmentedControl keeps RadioGroup/Radio semantics, one sequential tab stop, controlled selection,
disabled-item skipping, wrapping horizontal keyboard navigation, and fixed-content intrinsic
layout. StatusBadge and ValidationMessage keep tone/intent geometry on the shared Quad/Text/Icon
path. EmptyState owns intrinsic icon/title/message content, measures wrapped text through the host
shaper, and accepts only an application-provided action child. LabeledValue remains a
non-interactive summary with an optional mounted action child. Tabs, vertical orientation,
reorder, drag, close, and cross-surface behavior stay outside this batch.

## Current candidates

`Textarea` is `RuntimeCandidate`. Its Runtime path now uses one retained editing authority for
multiline selection, caret, IME preedit, Unicode grapheme deletion, soft wrapping, clipping and
caret-driven scrolling. Dark and light fixtures cover placeholder, multiline, focused, single- and
multi-line selection, invalid, disabled, clipped and scrolled states. Candidate status does not
change the public or Vue default route: the compatibility `Textarea` remains the default until
Hosted IME/platform and affected-consumer evidence complete the qualification gate.

`HostedTextarea` remains a separate `Compatibility` component and is not implied by the Runtime
`Textarea` candidate.

## Current fourth batch

`Progress`, `Spinner`, `FormField`, `InteractiveCard`, `Skeleton`, and `LevelMeter` are
`RuntimeQualified`; their root and aggregate public exports route to Runtime, while their Iced
adapters live under `nana_ui::compatibility`. `Tabs` is also `RuntimeQualified` for the
selection-strip Runtime path (`SegmentedControl::tabs()`). The public `nana_ui::Tabs` constructor
remains the Iced adapter so reorder, drag, close, and cross-surface transfer stay available.

Visual review accepted the Runtime frame. Iced differences are not defects: Tabs keep selected
surface only and do not paint a 2px focus ring; FormField uses the shared TextInput surface
rather than Iced's handler-less Disabled fill.

Runtime Progress is determinate only: Nana theme track/fill (Subtle + Accent, 6px girth,
radius 3), optional label, no cancel control. Runtime Spinner reuses the Scene spinner
primitive; phase is host-sampled and does not create a timer. FormField is a non-interactive
label/hint/error wrapper; the control remains an application-owned child. InteractiveCard is a
selectable surface with selected/hover/pressed/disabled layers. Skeleton is a Subtle rounded
placeholder. LevelMeter is a determinate tone-colored meter with configurable girth.

Workspace, Dock, Sidebar, overlay hosts, selects, charts, GPU views, and other professional
components remain `Compatibility` unless their catalog entry says otherwise.
