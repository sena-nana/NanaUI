---
name: nanaui-workspace-ui
description: Maintain NanaUI's Workspace and UI system. Product path is Runtime / UiScene drawn by SceneWgpuPainter. Use when changing WorkspaceController, regions and layout, sidebar, settings, shell, themes, fonts, widgets, overlays, serialization, public exports, or LiliaUI visual and interaction parity.
---

# NanaUI Workspace UI

## Rules

- Read [`docs/how-it-works.md`](../../../docs/how-it-works.md), [`docs/workspace.md`](../../../docs/workspace.md), [`docs/look.md`](../../../docs/look.md), [`docs/components.md`](../../../docs/components.md), [`docs/window.md`](../../../docs/window.md), crate-boundary notes in [`docs/architecture.md`](../../../docs/architecture.md), and the matching LiliaUI source before
  changing structure or visuals.
- Keep application content and business state outside the framework. Pass content through public
  Region and message contracts; keep sample navigation and documents in Demo state.
- Treat Runtime / UiScene / `SceneWgpuPainter` as the product view path.
- Preserve stable Region/Settings identities, sizing constraints, serialization, and public export
  compatibility.
- Overlay title bars use `AppTitleBar::transparent(true)` and its mounted slots; keep drag ownership in NanaUI. Fullscreen consumers retain business slots with `drag_enabled(false)` and `show_window_controls(false)`, not a parallel pointer gesture.
- Centralize shared tokens and component states. Every visible action must update real Rust state.
- Windows custom title-bar controls follow the LiliaUI rounded icon-button pattern (28px square, control radius, 2px gaps, 6px horizontal group insets). Preserve normal hover/pressed feedback and Close danger feedback; hit bounds follow the inset buttons rather than covering the frame corner.
- Route GPU resources to `$nanaui-gpu-integration`, window handles to
  `$nanaui-window-materials`, and verification to `$nanaui-validation`.

## Validation

Test changed state transitions and layout contracts. Regenerate and inspect real snapshots for
visual changes; review consumers when public or persisted contracts change.
