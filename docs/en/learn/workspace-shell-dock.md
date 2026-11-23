---
title: "Workspace: Shell, Workspace, Dock and Overlay"
---

# Workspace: Shell, Workspace, Dock and Overlay

The workspace layer (Shell / Workspace / Dock / AreaLayout / Overlays) is the Runtime subsystem that provides the "desktop application skeleton": it introduces no new rendering mechanism, but assembles the retained tree, the layout engine and input routing into a custom-drawn title bar, a multi-region workspace, splittable dock panels and modal/non-modal overlays. The through-line of this chapter is **separating the logical model from view presentation**: region sizes, collapse transitions, the Dock recursion tree, the AreaLayout split model and appearance settings all live in serializable pure-data models, and component projection only translates model state into style and topology mutations.

## Responsibilities and boundaries

- **`nana-ui-core` holds the backend-neutral logical models**: `WorkspaceModel` / `WorkspaceLayout` / `RegionState` (`workspace_model.rs`, `layout.rs`), `AppearanceSettings` / `SettingsModel` (`settings.rs`) and the versioned-envelope view state persistence `ViewStateStore` (`persist.rs`). They depend on no window or rendering backend (`crates/nana-ui-core/src/workspace_model.rs:83`).
- **`nana-ui-runtime` holds component assembly and interaction orchestration**: `Workspace` / `DesktopShell` / `AppShell` / `AppTitleBar` (`workspace.rs`, `shell.rs`), `DockNode` / `DockWorkspace` / `Dock` (`dock.rs`, behind the Cargo feature `dock`; `Workspace` and the Shell are always compiled), the DCC-style split `AreaLayout` (`area_layout.rs`) and the property inspector `PropertyGrid` (`property_grid.rs`), overlay activation and routing (`framework/overlay.rs`, `framework/modal.rs`), plus the sidebar, split-pane and notification-stack components (`sidebar.rs`, `panel.rs`, `pane.rs`, `pane_section.rs`, `toast_stack.rs`, `overlay_surfaces.rs`).
- **The host layer (`nana-ui`) only does a thin adaptation**: `WorkspaceController` only translates the `Instant` clock and window pointer events into `WorkspaceMutation` and never stores a second copy of region state (`crates/nana-ui/src/workspace.rs:74`).
- **Business content and the serialization boundary**: what business content a region or pane shows and how navigation is routed belong entirely to the application; only the pure-data logical model is written to the save file, and child node handles mounted at runtime, together with overlay activation state, never enter the persisted JSON (`crates/nana-ui-runtime/src/dock.rs:555`).

## Dependencies and neighbors

| Layer / Module | What it provides |
| --- | --- |
| `nana-ui-core` | Region model, appearance and settings models, `ViewStateStore`, motion timing (`MotionTiming`) |
| `nana-ui-runtime` | Component assembly and projection, three-phase drag-resize interaction, overlay focus transactions, Dock and AreaLayout tree operations |
| Input path ([Input path](/learn/runtime-input)) | Pointer press/move enters entries such as `begin_workspace_resize` and `begin_dock_split_resize` through the `dispatch.rs` chain of responsibility |
| Layout engine ([Layout engine](/learn/runtime-layout)) | Solves region grid tracks, Dock splitters, AreaLayout nested Flex containers and overlay scrim geometry |
| `nana-ui` host | Window creation, native title-bar control buttons, floating Dock window reconciliation, when settings are saved |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `WorkspaceMutation` | `crates/nana-ui-core/src/workspace_model.rs:13` | The 12 kinds of logical region mutation (`ToggleRegion` / `ResizeMove` / `SetViewport`, etc.) |
| `WorkspaceModel` | `crates/nana-ui-core/src/workspace_model.rs:85` | The only authority for region layout, collapse transitions, drag state and viewport facts |
| `RegionState` / `RegionId` | `crates/nana-ui-core/src/layout.rs:94` / `:15` | The structure and persistence contract of a single region (`placement` / `scope` / size constraints) |
| `WorkspaceLayout` | `crates/nana-ui-core/src/layout.rs:386` | `Vec<RegionState>` and the default preset of the standard six regions |
| `Workspace` | `crates/nana-ui-runtime/src/workspace.rs:114` | The workspace component in the tree, holding the model snapshot and the track chrome slots |
| `DockNode` / `DockWorkspace` | `crates/nana-ui-runtime/src/dock.rs:59` / `:296` | The recursive dock tree (`Item` / `Tabs` / `Split`) and the product-level multi-surface dock authority |
| `DockDropZone` | `crates/nana-ui-runtime/src/dock.rs:49` | Drop zones: `Left` / `Right` / `Top` / `Bottom` / `Tab` |
| `AreaLayout` / `AreaLayoutModel` | `crates/nana-ui-runtime/src/area_layout.rs` | DCC-style non-overlapping recursive region split component and its pure-data model |
| `PropertyGrid` / `PropertySection` | `crates/nana-ui-runtime/src/property_grid.rs` | Inspector-style dense two-column property panel (collapsible groups + label column width dragged in sync across the grid) |
| `RuntimeOverlayKind` | `crates/nana-ui-runtime/src/framework/overlay.rs:14` | The five overlay interaction behaviors `Dialog` / `Panel` / `Menu` / `Tooltip` / `Status` |
| `OverlayHost` | `crates/nana-ui-runtime/src/view_components.rs:3478` | The overlay mount point component (node tag `overlay-host`) |
| `OverlayVisibility` | `crates/nana-ui-runtime/src/overlay_visibility.rs:55` | The auto-hide state machine for media control bars and floating chrome |
| `PaneTree` / `PaneSection` | `crates/nana-ui-runtime/src/pane.rs:616` / `pane_section.rs:41` | The content split-pane tree and a compound pane section with a tab row |
| `ViewStateStore` | `crates/nana-ui-core/src/persist.rs:146` | View state persistence store with a `schema_version` version envelope |
| `AppearanceSettings` / `SettingsModel` | `crates/nana-ui-core/src/settings.rs:101` / `:572` | Appearance configuration (radius tiers, window material, frosted-glass targets) and the settings page model |

## Internal structure

### 1. Separating logical state from presentation

The type documentation of the `Workspace` component (`crates/nana-ui-runtime/src/workspace.rs:108`) spells out the core rule: **"Snapshot the model; `project` never owns a clock"** — region visibility, overlay state, current width and drag highlight are all queried directly from `Self::model`, with no mirror cache maintained. Every interaction calls `Workspace::apply(mutation, now)` (`:156`) uniformly, and the `now` timestamp is passed in from outside. When `Workspace` is mounted under the page-level desktop shell `DesktopShell` (`crates/nana-ui-runtime/src/shell.rs:884`), each successful mutation syncs the latest model snapshot back to the parent Shell through `write_back_shell_model` (`workspace.rs:1185`).

### 2. The standard six regions and collapse transition sampling

`WorkspaceLayout` (`crates/nana-ui-core/src/layout.rs:390`) provides the standard six regions by default:

| Region ID | Default size | Min / max constraints | Description |
| --- | --- | --- | --- |
| `GlobalNavigation` | `56 px` | `44 / 96 px` | The leftmost global navigation bar |
| `Resources` | `260 px` | `180 / 520 px` | Left resource panel; collapsible and drag-resizable |
| `PrimaryToolbar` | `34 px` | — | Toolbar at the top of the primary region (`Primary` scope) |
| `Primary` | Fills adaptively | Minimum `320 px` | The core primary work area with `fill_priority = 1` |
| `Inspector` | `280 px` | `200 / 560 px` | Right property inspector; collapsible and drag-resizable |
| `Diagnostics` | `200 px` | `96 / 520 px` | Diagnostics and console panel at the bottom of the primary region |

The region collapse animation `RegionTransition` (`workspace_model.rs:44`) interpolates `from_extent` and `to_extent` with `EaseInOutCubic` over `WORKSPACE_REGION_TRANSITION_DURATION = 260 ms` (`:10`); when geometry is sampled, `with_transient_extents` (`layout.rs:450`) writes the transient width into a temporary layout snapshot, and the persisted model itself is never contaminated by intermediate transition values.

### 3. Three-phase pointer drag interaction (Workspace / Dock / AreaLayout)

Every drag-resize interaction in the workspace layer follows the same `begin / update / end` three-phase shape, driven by `dispatch_pointer` in the input path:

| Interaction | Three-phase entry anchors | Core behavior |
| --- | --- | --- |
| Workspace region resize | `begin_workspace_resize` (`workspace.rs:1041`) / `update :1074` / `end :1100` | Applies `ResizeStart` + `ResizeMove`, and runs `request_focus` + `capture_pointer` in the same transaction (`:1063-1064`) |
| Dock splitter drag | `begin_dock_split_resize` (`dock.rs:2602`) / `update :2647` / `end :2701` | The single split-ratio formula `dock_split_ratio_from_pointer` (`:796`), clamped to `0.05..=0.95` |
| Dock pane cross-region drag and drop | `dock.rs:2771` / `:2808` / `:2851` | `DockDropZone` (`Left/Right/Top/Bottom/Tab`) decides whether to split or merge into a Tab |
| AreaLayout splitters and corner-block splits | `crates/nana-ui-runtime/src/area_layout.rs` | A `2 px` gap owns a grab hot zone of at least `6 px`; dragging inward from the top-left/top-right corner block of a region header splits off a new region, while dragging toward a sibling half absorbs and merges |

### 4. The Dock recursion tree, split-pane slots and DCC region splitting

- **`DockWorkspace` and floating-window events**: `DockNode` (`dock.rs:59`) is a three-state recursion tree of `Item` / `Tabs` / `Split`; `DockWorkspace` (`:296`) manages the main tree and multiple floating window surfaces (`DockSurfaceSpec`, `:329`). When the user drags a pane out to become an independent window, `float_item_at` (`:396`) removes that item from the tree and returns `DockWorkspaceEvent::OpenFloating`, which the host turns into a native window (the window key is derived deterministically by `dock_surface_window_key:251`); Runtime itself never touches a platform window handle directly.
- **`PaneTree` and `PaneSection`**: `PaneTree` (`pane.rs:616`) allocates an independent `PaneSlot` (`:485`) for each split and leaf, so that a split width is not written directly onto a business content node and wiped out by business reprojection; `PaneSection` (`pane_section.rs:41`) wraps the compound card of "title + tab row + flexible body".
- **`AreaLayout` and `PropertyGrid`**: aimed at 3D and animation tools, `AreaLayout` maps the region tree onto nested Flex containers; when the structure changes it rebuilds only the split containers and splitters, while regions that still exist keep their header, content and the corner block being dragged in place; when a workspace preset is switched (`set_area_layout`), region children that do not appear are hidden and kept waiting rather than destroyed. The accompanying `PropertyGrid` provides a shared label column with a `6 px` synchronously dragged splitter, collapsible groups, mixed values for multi-selection (`mixed`) and a disabled-row mask with a reason.

### 5. Overlays: focus migration in the same transaction and auto-hide

`AppShell` (`crates/nana-ui-runtime/src/shell.rs:704`) assembles the custom-drawn title bar `AppTitleBar` (`shell.rs:82`, whose window buttons emit the typed `WindowChromeAction:38`), the filling body and the `OverlayHost` (`view_components.rs:3478`).

`overlay_kind_for_role` (`framework/overlay.rs:44`) maps an overlay to one of five `RuntimeOverlayKind` values according to its accessibility role: `Dialog` and `Menu` block pointer input to the layers below and trap Tab focus; `Panel` receives pointers only for itself and does not lock the layers below; `Tooltip` and `Status` are entirely passive and never take focus (`ToastStack` lives in `toast_stack.rs:43`, shows only after layout completes, and pauses its countdown while hovered or focused). Activating an overlay always calls `AppContext::activate_overlay` (`framework/modal.rs:221`): **it activates the overlay and moves focus into its subtree within the same Runtime transaction**, completely removing the intermediate frame where "the dialog is shown but focus is still in the input below".

### 6. Version-envelope persistence (`ViewStateStore`)

Region layout (`WorkspaceModel::layout_json`), the Dock tree (`DockWorkspace::save_to_store`) and appearance settings (`AppearanceSettings::save_to_store`) are all written through `ViewStateStore` (`crates/nana-ui-core/src/persist.rs:146`) into a version envelope carrying a `schema_version`. When reading, a version mismatch or corrupt JSON automatically deletes the bad entry, increments the `SCHEMA_MISMATCHES` / `CORRUPTIONS` diagnostic counters and falls back gracefully to the default layout (`:174-185`).

## Collaboration and data flow

Take "dragging the inspector edge to adjust its width" as an example:

```text
Pointer press → the input dispatch chain hits the handle → begin_workspace_resize (workspace.rs:1041)
  → WorkspaceModel applies ResizeStart / ResizeMove (size clamping)
  → request_focus + capture_pointer + write_back_shell_model in the same transaction
  → the Workspace component reprojects, writing the new width into MutationQueue
  → UiWorld::commit → RuntimeLayoutEngine incrementally relayouts neighboring tracks
  → at the frame boundary RuntimeDocument::flush drains system work → UiScene::apply_delta → on screen
```

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-core/src/workspace_model.rs` | `WorkspaceMutation`, `WorkspaceModel`, collapse transition sampling and JSON serialization |
| `crates/nana-ui-core/src/layout.rs` | `RegionId`, `RegionState`, `WorkspaceLayout`, `NarrowBehavior` |
| `crates/nana-ui-core/src/settings.rs` | `AppearanceSettings`, `SettingsModel`, `SettingsState` |
| `crates/nana-ui-core/src/persist.rs` | `ViewStateStore` version envelope, `LocalStorageAdapter` |
| `crates/nana-ui-runtime/src/shell.rs` | `AppTitleBar`, `AppShell`, `DesktopShell`, `WindowChromeAction` |
| `crates/nana-ui-runtime/src/workspace.rs` | The `Workspace` component, track assembly, three-phase resize and hooks |
| `crates/nana-ui-runtime/src/dock.rs` | `DockNode`, `DockWorkspace`, the `Dock` component, drag/keyboard splitting and persistence |
| `crates/nana-ui-runtime/src/area_layout.rs` | `AreaLayout` and `AreaLayoutModel` (DCC-style region splitting, merging and preset switching) |
| `crates/nana-ui-runtime/src/property_grid.rs` | `PropertyGrid`, `PropertySection`, `PropertyRow` (inspector-style property panel) |
| `crates/nana-ui-runtime/src/framework/{overlay,modal,auto_overlay}.rs` | `RuntimeOverlayKind`, same-transaction focus migration in `activate_overlay`, auto-hide routing |
| `crates/nana-ui-runtime/src/{overlay_surfaces,overlay_visibility,panel,pane,pane_section,sidebar,toast_stack}.rs` | Dialog/drawer, media bar auto-hide, named panels, the split-pane tree, the sidebar and the notification stack |
| `crates/nana-ui/src/workspace.rs` | The `WorkspaceController` host adapter |

## Further reading

- [Retained Tree: UiWorld](/learn/runtime-tree): how mutations produced by component projection commit atomically, and how in-frame system work is drained
- [Input path](/learn/runtime-input): how drag handles, splitters and overlay pointers are routed to this chapter's interaction entry points
- [Layout engine](/learn/runtime-layout): how region tracks, Dock splits and AreaLayout box geometry are solved
- [Workspace, dock and region layout (Reference)](/reference/workspace)
