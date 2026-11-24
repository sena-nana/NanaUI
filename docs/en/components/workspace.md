# Workspace

`Workspace` binds your content to regions. The content is child nodes in the tree. Collapse, size, visibility, and restore go through `nana_ui::runtime::WorkspaceModel` and `WorkspaceMutation`.

## Basic usage

When you do not need an IDE-style shell, use `List` or `AppShell`.

`Workspace::new()` equals `from_model(&WorkspaceModel::new(), [])`. `from_model(model, slots)` brings existing `WorkspaceRegionSlot`s. `.slot(id, content)` binds a piece of content by `nana_ui::RegionId`; writing the same id again replaces it. On a view it is `.region(id, view)`.

The control table has no `<Workspace>`. `.region` takes two arguments, a region id and a view, and cannot be written as `<template #region>`.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::widget;
use nana_ui::runtime::Workspace;
use nana_ui::RegionId;

widget(Workspace::new()).region(RegionId::Primary, view! { <Column /> })
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::Workspace;
use nana_ui::RegionId;

widget(Workspace::new()).region(RegionId::Primary, column())
```

:::

`assemble_workspace` is the `slot_assembler`. It runs when the view commits. After placing slots by hand, call it once yourself. Individual writes do not trigger reassembly automatically.

## Regions

Built-in ids are `GlobalNavigation`, `SectionNavigation`, `Resources`, `PrimaryToolbar`, `Primary`, `Inspector`, `Diagnostics`, plus `Custom(String)`. `register` rejects duplicate ids.

The field `workspace_corners` decides whether the primary region has rounded corners; rounded by default. When `DesktopShell` assembles, it copies its own `workspace_corners` onto the workspace it builds. If you change the node the framework built, the next shell assembly overwrites it.

## Model and splitters

`apply(mutation, now)` pushes a mutation into the model. `refresh_from_model` updates only fields derived from the model and does not swap your slots.

The splitter hit area covers the region edge, 8px wide, and takes no grid track. Collapse has a 260ms transition.

## Geometry

`WorkspaceGeometry` maps the same layout to logical and physical rectangles for GPU viewports. It creates no windows and no GPU resources.

`WorkspaceController` only turns pointer and clock input into mutations; do not keep a second copy of region state in an adapter.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `Workspace::new` | — | Equals `from_model(&WorkspaceModel::new(), [])` |
| `model` | — | The model of `from_model`. `Workspace::new()` equals `from_model(&WorkspaceModel::new(), [])` |
| `slots` | `WorkspaceRegionSlot` | `from_model(model, slots)` brings existing slots |
| `workspace_corners` | — | Decides whether the primary region has rounded corners; rounded by default |

## Events

No control events of its own. `WorkspaceController` turns pointer and clock input into mutations.

## Slots

| Slot | Description |
| --- | --- |
| `.slot(id, content)` | Binds a piece of content by `nana_ui::RegionId`; writing the same id again replaces it |
| `.region(id, view)` | The binding on a view. Takes two arguments, a region id and a view, and cannot be written as `<template #region>` |

## See also

[AreaLayout](area-layout.md) (DCC-style area splitting and workspace presets) · [Overview](index.md) · [Controls](../reference/components.md) · [Workspace](../reference/workspace.md)
