---
name: nanaui-workspace-ui
description: Maintain NanaUI's Workspace and UI system. Iced is the current compatibility view adapter. Use when changing WorkspaceController, regions and layout, sidebar, settings, shell, themes, fonts, widgets, overlays, serialization, public exports, or LiliaUI visual and interaction parity.
---

# NanaUI Workspace UI

## Rules

- Read [`architecture.md`](../../../docs/architecture.md),
  [`design-language.md`](../../../docs/design-language.md), and the matching LiliaUI source before
  changing structure or visuals.
- Keep application content and business state outside the framework. Pass content through public
  Region and message contracts; keep sample navigation and documents in Demo state.
- Treat Iced as the current compatibility view adapter, not the Workspace/UI product contract.
- Preserve stable Region/Settings identities, sizing constraints, serialization, and public export
  compatibility.
- Centralize shared tokens and component states. Every visible action must update real Rust state.
- Route GPU resources to `$nanaui-gpu-integration`, window handles to
  `$nanaui-window-materials`, and verification to `$nanaui-validation`.

## Validation

Test changed state transitions and layout contracts. Regenerate and inspect real snapshots for
visual changes; review consumers when public or persisted contracts change.
