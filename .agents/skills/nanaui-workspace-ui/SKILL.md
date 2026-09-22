---
name: nanaui-workspace-ui
description: Maintain NanaUI's reusable workspace shell and visual controls. Use when changing Workspace, Dock, regions, layout composition, themes, widgets, overlays, title-bar slots, serialization, or public UI exports.
---

# NanaUI Workspace UI

Read [`docs/workspace.md`](../../../docs/workspace.md), [`docs/components.md`](../../../docs/components.md), [`docs/look.md`](../../../docs/look.md), and [`docs/how-it-works.md`](../../../docs/how-it-works.md). For crate ownership read [`docs/architecture.md`](../../../docs/architecture.md).

## Boundaries

- Workspace, Shell, Dock, Settings, regions, and controls provide reusable structure and interaction. Applications own the content, navigation, persistence policy, and business state inside regions.
- Use stable Region and Settings identities, shared theme tokens, public serialization fields, and the existing component registry. Every visible action must update real state.
- Keep overlays in Runtime: use `OverlayHost` plus `Panel` for non-modal task surfaces, and `Dialog`/`Drawer` only for modal interaction. Context menus use `OverlayHost` and the common dismissal/focus lifecycle.
- Let framework placement entries (`Panel::viewport`, `Toast::place_in`, reserved `PanelInsets`) determine transient geometry. Do not add application-specific outside-press or absolute-offset systems.
- Title-bar controls use `AppTitleBar` slots. Fullscreen consumers disable dragging and window controls while retaining business slots; do not reserve platform-specific hard-coded bands.
- Keep GPU resources, window handles, and validation in their dedicated skills.

Validate changed state transitions, layout contracts, serialization, and real visual behavior when appearance changes.
