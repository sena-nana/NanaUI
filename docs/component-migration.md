# Component migration contract

NanaUI migrates components independently while preserving their product behavior. The public
read-only catalog is available through `component_catalog()` and `component_support()`. Consumers
use it as diagnostic and acceptance metadata; it never appears in product UI or creates parallel
application state. NanaUI derives its internal default-backend route from the same declaration so
there is no second hand-maintained list of qualified components.

## States

| State | Meaning |
| --- | --- |
| `Compatibility` | Runtime promotion is incomplete; this entry is not a product-default path. |
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
`NANA_UI_SNAPSHOT_OUTPUT` root). `runtime.png` is the current Runtime frame painted by
`SceneWgpuPainter`. `iced.png` is an archived baseline when present; the runner does not host
Iced widgets as the left/oracle column. These images are diagnostic evidence, not a
pixel-similarity gate, and the runner never promotes a catalog entry automatically.

Iced widget adapters (`nana_ui::compatibility`) were removed after leaf
qualification. Public types route through Runtime / UiScene. Historical batch
notes below may still mention those adapters as migration-era references; the
modules no longer exist.

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
aggregate, and Vue hosted default routes use Runtime. Iced widget adapters were removed after
leaf qualification. A hosted Runtime Scene never silently rebuilds a qualified component
through a second retained tree when state is missing. Button passed
all semantic kinds, sizes, loading, activation, focus and accessibility review. TextInput passed
placeholder, shaped selection/caret, secure, invalid, read-only/loading, keyboard, IME preedit and
native input-purpose review. Text passed wrapping, clipping, alignment, typography and
accessibility review. Checkbox passed checked/off, hover, pressed, focused, disabled, invalid,
pointer, keyboard and accessibility activation states in dark and light. Workspace, Dock,
Sidebar, Overlay, and the remaining catalog leaves are covered in later sections;
`component_catalog()` is the authority for each identity.

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

## Current candidate cutover

`Textarea`, `Tooltip`, `Dialog`, `ConfirmDialog`, `Drawer`, `Toast`, `XYPad`, `QrCode`,
`Select`, `Popover`, `ActionMenu`, `ActionMenuItem`, `AnchoredActionMenu`, `ContextMenu`,
and `OverlayHost` are `RuntimeQualified`. Their root and aggregate public exports route to
Runtime, while their Iced adapters live under `nana_ui::compatibility`.

Textarea keeps one retained editing authority for multiline selection, caret, IME preedit,
Unicode grapheme deletion, soft wrapping, clipping and caret-driven scrolling. Field states
change the field border (`border` / `border-strong` / `danger`); focused uses `border-strong`,
and invalid-focused is a `2px` danger border with no second ring. Vue hosted leaves paint
through Runtime Scene. Native IME preedit/commit updates that same Runtime state and emits
Vue `beforeinput`/`input`; a focused Runtime editor advertises one hosted `text_input_request`
so winit IME is not also fed to an Iced editor. `commit_text` refuses disabled and read-only
fields. Leftover native preedit on IME Disabled commits into the original field even if focus
moved. `HostedTextarea` is `RuntimeQualified`. Public `nana_ui::HostedTextarea`
is a Runtime `TextArea` that always carries a `"highlight"` request. Iced
`HostedTextarea` / `HostedSyntaxHighlighting` stay under
`nana_ui::compatibility`. Official syntax color is the registered
`TextPresenter` (`"highlight"`) on committed text. Enabling
`nana-ui/syntax-highlighting` (or `nana-ui-runtime/syntax-highlighting`)
installs `HighlightPresentation` on every `AppContext::new`. Call
`TextArea::highlight` / `TextInput::highlight` / `HostedTextarea::new(_, lang)`
to request spans. IME preedit stays solid. Vue `nana-textarea` with
`language` / `lang` / `syntax` projects `HostedTextarea`.

Tooltip is a compact label-only hover card. Hover delay, open/close, cursor tracking and
exclusive active/focus stay on the existing IconButton-hosted overlay lifecycle. Public
`nana_ui::Tooltip` is the Runtime overlay; Iced wrap-content tooltips stay under
`nana_ui::compatibility`. Vue `nana-tooltip` projects the Runtime label overlay.

Dialog, ConfirmDialog and Drawer use the shared `ModalFrame` scrim/surface/body, exclusive
overlay lifecycle, close policy and typed slots. Toast is an outlined tone card without a
timer. XYPad is a two-axis pad with Input/Change, Shift lock and keyboard steps. QrCode paints
a scanner-safe module matrix with a four-module quiet zone. Select keeps disabled options
visible in the opened menu. Popover and ActionMenu keep an in-flow trigger. ActionMenuItem is
the shared menu row. AnchoredActionMenu and ContextMenu pin that surface to a logical point and
hug their items. Runtime ContextMenu owns nested `parent/child` levels (`active_path`,
`back`, leaf `Select`) and optional query filter. OverlayHost is the exclusive overlay
lifecycle on Runtime; Iced stacking stays under `nana_ui::compatibility`.

Vue Scene paints Textarea, Select, Dropdown, SearchDropdown, Toast, Tooltip, ActionMenu,
ActionMenuItem, XYPad, QrCode, Dialog (including children), ConfirmDialog, Drawer, Popover,
Tabs, SegmentedControl, FormField, InteractiveCard, EmptyState, LabeledValue, and
ContextMenu — including searchable menus (≥6 options or `search` class) — through
`SceneWgpuPainter` subtrees. Runtime ContextMenu owns the search field on the same retained
`TextInputState` as its query filter. Runtime `QrCode::encode` builds the scanner-safe matrix from a Vue string payload;
empty or unencodable payloads still project a LabeledValue placeholder.

L2 wrappers exist for `NanaToast`, `NanaTooltip`, `NanaActionMenu`, `NanaXyPad`, and
`NanaQrCode`. Visual review accepts the Runtime frame. Iced differences are not defects.

## Current fourth batch

`Progress`, `Spinner`, `FormField`, `InteractiveCard`, `Skeleton`, and `LevelMeter` are
`RuntimeQualified`; their root and aggregate public exports route to Runtime, while their Iced
adapters live under `nana_ui::compatibility`. `Tabs` is `RuntimeQualified` for selection,
before-value reorder, close request, and the generation-lease drag contract. Runtime paints
each option with the same `SegmentedOption` / `SelectionChrome::Tabs` chrome as
`SegmentedControl::tabs()`. Public `nana_ui::Tabs` is the Runtime strip (`Arc<str>`
identities). Vue `WidgetKind::Tabs` stays on the selection-strip path
(`SegmentedControl::tabs()`). Close remains an application request: Iced `Tabs` does not
paint a close control. Generic Iced `Tabs<T, Message>` and `DraggableTabStrip` remain
under `nana_ui::compatibility` for generic `T` paint and Iced-hosted leases. Overlay is
not claimed.

Visual review accepted the Runtime frame. Iced differences are not defects: Tabs keep selected
surface only and do not paint a 2px focus ring; FormField uses the shared TextInput surface
rather than Iced's handler-less Disabled fill.

Runtime Progress is determinate: Nana theme track/fill (Subtle + Accent, 6px girth,
radius 3), optional label, and an optional cancel hit target (`ProgressCancelled`). Runtime Spinner reuses the Scene spinner
primitive; phase is host-sampled and does not create a timer. FormField is a non-interactive
label/hint/error wrapper; the control remains an application-owned child. InteractiveCard is a
selectable surface with selected/hover/pressed/disabled layers. Skeleton is a Subtle rounded
placeholder. LevelMeter is a determinate tone-colored meter with configurable girth.

## Current fifth batch

`Dialog`, `ConfirmDialog`, `Drawer`, `Toast`, `XYPad`, and `QrCode` were introduced as Runtime
leaves on the shared modal and leaf contracts. They are now `RuntimeQualified` with the
candidate cutover above.

## Current sixth batch

`Select`, `Popover`, `ActionMenu`, `ActionMenuItem`, `AnchoredActionMenu`, and `ContextMenu`
were introduced as Runtime overlay candidates. They are now `RuntimeQualified` with the
candidate cutover above. Searchable ContextMenu keeps the filter field on Runtime.

`Dropdown` and `TreeView` are `RuntimeQualified`. Public `nana_ui::Dropdown` is the
Runtime field (single or multiple `Arc<str>` values, hints, disabled options stay
visible). Iced generic `Dropdown<T>` stays under `nana_ui::compatibility`. Vue
`nana-dropdown` projects Runtime `Dropdown`.

`SearchDropdown` and `CommandPalette` are `RuntimeQualified`. Search uses committed
`TextInputState` plus the shared case-insensitive `query_matches` helper; it is not
an Iced `combo_box`. The opened SearchDropdown field shows the query, IME preedit
stays on that same state, and filtered options reuse the Select menu surface.
CommandPalette is one `StandardVisual::CommandPalette` (scrim, surface, search
field, windowed rows). Public `nana_ui::SearchDropdown` / `nana_ui::CommandPalette`
are the Runtime types. Iced generic `SearchDropdown<T>` and the Iced palette widget
stay under `nana_ui::compatibility`. Vue `nana-search` projects Runtime
`SearchDropdown`.

Runtime `TreeView` flattens visible disclosure rows onto one retained surface with
pointer, keyboard and accessibility. The Iced SidebarRow adapter stays under
`nana_ui::compatibility`. Navigation (`TreeNode`, `TreeViewEvent`, `tree_navigation_event`)
lives in `nana-ui-core`. Palette items and navigation
(`CommandPaletteItem`, `CommandPaletteEvent`, `ActionPickerNavigation`) live in
`nana-ui-core`.

Workspace, Dock, Sidebar, charts, GPU views, and other professional components
remain on their catalog state; none of the remaining leaves stay
`Compatibility` by default.

## Current sidebar and settings cutover

`sidebar-row` and `settings` (SettingsRow / SettingsCard leaf chrome) are
`RuntimeQualified`. Root `nana_ui::SidebarRow`, `nana_ui::SettingsRow`, and
`nana_ui::SettingsCard` route to Runtime. Iced adapters stay under
`nana_ui::compatibility` and the original `nana_ui::sidebar` /
`nana_ui::settings` modules.

Vue `nana-sidebar-row`, `nana-settings-row`, and `nana-settings-card` project
through Runtime. Scene paint reuses ListItem / Card / child Switch geometry;
SettingsRow hosts an application-owned control child the same way FormField
does. Vue Scene assigns `label_slot` / `hint_slot` / `copy_slot` from
`nana-settings-row__label` / `__hint`. `settings_page` / `settings_sidebar`
remain Iced compatibility composers.

`sidebar-frame`, `sidebar-section`, `sidebar-footer`, `appearance-section`,
`about-section`, and `settings-collapsible-card` are `RuntimeQualified`.
Root `nana_ui::SidebarFrame`, `SidebarSection`, `SidebarFooter`,
`SidebarFooterButton`, `SidebarSectionState`, `AppearanceSection`,
`AboutSection`, `AboutMetadata`, and `SettingsCollapsibleCard` are Runtime
(also via the `components` aggregate). Iced Element composers live under
`nana_ui::compatibility` (and `nana_ui::sidebar` /
`nana_ui::components::settings_sections`). DesktopShell and gallery Iced
trees keep using those Iced adapters.

Runtime types exist (`nana_ui::runtime::*`) with expand/collapse, press, and
`AppearanceEvent` forwarding. Runtime `SidebarFrame` treats `body` as a vertical
`ScrollView` so top/footer stay unscoped siblings of the scrollport. Vue projects
`nana-sidebar-frame__body` as that `ScrollView` and maps `WidgetKind::SidebarFrame`
to the catalog id. Hosted wheel updates Runtime `scroll_offset` without Iced
pending tasks. Vue Scene default-routing (`component_uses_runtime`) paints
qualified `sidebar-frame` through Runtime Scene; Iced no longer wraps that
runtime-owned body in `scrollable`.

`AppContext::assemble_appearance_section`, `assemble_about_section`, and
`assemble_settings_collapsible_card` mount qualified SettingsRow / control
children from the host snapshot. SidebarSection chrome is host-mounted slots
(header, disclosure, title, count, body, tools).

2026-08-16 windowed A/B **passed** for `sidebar-frame`, `sidebar-section`,
`sidebar-footer`, `sidebar-row`, `settings`, `appearance-section`,
`about-section`, and `settings-collapsible-card`. About and Appearance
Runtime frames are the accepted design (Iced tracked uppercase card title
and radius-without-track are Iced-side). Collapsed disclosure is
`Icon::ChevronRight`. Collapsible title stays body text; the card is the
single activation target.

## Current workspace-family cutover

`workspace`, `dock`, `dock-panel`, `split-pane`, `pane-chrome`, `pane-tree`,
`app-shell`, and `app-title-bar` are `RuntimeQualified`. Root
`nana_ui::Workspace`, `Dock`, `DockPanel`, `SplitPane`, `PaneChrome`,
`PaneTree`, `AppShell`, and `AppTitleBar` are the Runtime views. Iced widget
adapters were removed; hosted floating dock windows and native title-bar drag
stay on the Scene host.

2026-08-16 windowed A/B **passed** for this batch. Slot labels in fixtures
use a shared 8px content inset; Workspace/Dock/Split/AppShell chrome does not
add that inset.

`CalendarHeatmap`, `TimeSeriesChart`, `ReorderList`, `NativeMarkdown`,
`SelectableRichText`, `ImageViewer`, `KeyCaptureLayer`, and `KeymapLayer` are
`RuntimeQualified`. Root and `components` exports route to Runtime. Scene
paints these through Quad/Text geometry. Archived Iced output is a reference,
not a pixel oracle.

`GraphCanvas`, `GpuView`, and `GpuTextureView` are `RuntimeQualified`. The
graph model lives once in `nana-ui-core`. Root `nana_ui::GraphCanvas`,
`GpuView`, and `GpuTextureView` are the Runtime views. GraphCanvas paints Scene
Quad/Text geometry from `StandardVisual::GraphCanvas`: background grid,
cubic edges, node title bars, port discs and labels, hover/selected, and
live drag or connection preview. `RuntimeInputAdapter` routes pointer,
wheel and keyboard into the same graph events. GpuTextureView binds the
host-owned `"nana.host-texture"` slot. GpuView projects
`CustomRenderNode` `"gpu-view"`. `default_scene_gpu_renderers` and the Scene
host install the default `"gpu-view"` painter. Construction is not a draw:
the default painter still needs the host Device/Queue to emit GPU. Syntax highlighting is the
Runtime `"highlight"` presenter on `HostedTextarea` / `TextArea` / `TextInput`.
`SplitPane` handle drag applies `SplitPaneMutation` through the same
adapter.

## Current remaining work

`component_catalog()` lists 68 identities; every entry is `RuntimeQualified`
and none remain `RuntimeCandidate`. Leaf qualification is complete. There is
no further leaf batch.

Remaining work is host polish, not a new leaf batch.

2026-08-17 windowed A/B **passed** for Dock, SplitPane, AppShell,
SettingsPage, and CalendarHeatmap. Calendar hover titles hug `Tooltip`
metrics; Iced's fixed 176px popup is Iced-side.

- Gallery settings, six main sections, and overlays (command palette,
  confirm dialog, image viewer, context menu) paint retained Runtime
  documents. Windowed Gallery uses `run_runtime` / `RuntimeProgram`.
  `ui-snapshots` and the windowed A/B bins paint `UiScene` with
  `SceneWgpuPainter`.
- Gallery live dock state is Runtime `DockWorkspace`. Hide/show uses
  `DockWorkspace::{hide,show,is_visible}`; the workspace tree stays live.
  Floating events record host window commands. The Scene host does not open
  extra daemon windows.
- `default_scene_gpu_renderers` installs the `"gpu-view"` painter so Vue
  hosted GpuView keeps a product Scene GPU renderer.
- Vue maps Calendar `options` (weekday labels, level strategy, string
  `monthFormat` / `titleFormat` templates). JS `Function` formatters stay
  ignored — tree sync does not invoke the engine. GraphCanvas
  `viewport`/`selection`, and Workspace / Dock / SplitPane / AppShell /
  SettingsPage children map onto Runtime composers. After
  `sync_semantic_styles`, Vue binds Workspace / Dock / SplitPane /
  AppShell / NativeMarkdown / SettingsPage on `AppContext` and calls
  `assemble_workspace` / `assemble_dock` / `assemble_split_pane` /
  `assemble_app_shell` / `assemble_markdown` / `assemble_settings_page`.
- Runtime `assemble_markdown` attaches a hidden text child per mermaid,
  display-math, and code fence (`mermaid` / `math` / `highlight`).
  Applications still own mermaid/math paint.
- Windows/Linux real-device acceptance stays deferred.
