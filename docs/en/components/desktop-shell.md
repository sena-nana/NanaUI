# DesktopShell

`DesktopShell` assembles a title bar, a workspace, navigation, an inspector and two overlay host layers into one desktop page. The content in each cell is still yours. When you do not need this shell, use `List` or `AppShell`.

The control table has no `<DesktopShell>`.

## Basic usage

`DesktopShell::new()` uses a fresh `WorkspaceModel`.

`DesktopShell::from_model(model)` reuses a model you already have.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::DesktopShell;

view! {
    <Widget of={DesktopShell::new().title("Nana").workspace_corners(true)}>
        <template #navigation>
            <Column />
        </template>
        <template #primary>
            <Column />
        </template>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, widget};
use nana_ui::runtime::DesktopShell;

widget(DesktopShell::new().title("Nana").workspace_corners(true))
    .navigation(column())
    .primary(column())
```

:::

## Title bar

`.title(text)` makes assembly build an `AppTitleBar` when no title bar is given.

`.title_leading`, `.title_center` and `.title_trailing` are slots in the title bar.

`.title_center_width` widens the middle column, for a breadcrumb that needs more width than a short title.

`.title_window_controls(false)` avoids hanging a second, display-only set of window buttons.

## Regions

`.navigation`, `.navigation_footer`, `.primary`, `.inspector` and `.bottom` go into the matching region.

Inspector tabs go in `.inspector`, not in `RegionId::PrimaryToolbar`.

`.region(id, content)` adds one more region.

## Overlays

`.overlay` pushes the `OverlayHost` used for dialogs, and `.status` pushes the second host layer used for toasts, so toasts can still show while a confirm dialog is open.

## Corners

Whether the primary area is rounded is written on `DesktopShell::workspace_corners`, rounded by default.

At assembly it is handed to the `Workspace` the shell builds.

Do not change the `Workspace` node the framework built; the next assembly paints over it with the value on the shell.

## Assembly

`assemble_desktop_shell` is the `slot_assembler`.

It runs when the view is committed.

After placing slots by hand, call it once yourself.

A write does not re-assemble automatically.

## Model

Product state lives in `nana_ui::runtime::WorkspaceModel` and `WorkspaceMutation`.

Collapse, size and visibility go through the model.

`WorkspaceController` only turns pointer and clock input into mutations.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `DesktopShell::new()` | — | Uses a fresh `WorkspaceModel` |
| `DesktopShell::from_model(model)` | — | Reuses a model you already have |
| `.title(text)` | — | Makes assembly build an `AppTitleBar` when no title bar is given |
| `.title_center_width` | — | Widens the middle column, for a breadcrumb that needs more width than a short title |
| `.title_window_controls` | — | `.title_window_controls(false)` avoids hanging a second, display-only set of window buttons |
| `.region(id, content)` | — | Adds one more region |
| `workspace_corners` | — | Whether the primary area is rounded, rounded by default. At assembly it is handed to the `Workspace` the shell builds |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| None | None | None |

## Slots

| Slot | Description |
| --- | --- |
| `.title_leading` | A slot in the title bar |
| `.title_center` | A slot in the title bar |
| `.title_trailing` | A slot in the title bar |
| `.navigation` | Goes into the matching region |
| `.navigation_footer` | Goes into the matching region |
| `.primary` | Goes into the matching region |
| `.inspector` | Goes into the matching region. Inspector tabs go here, not in `RegionId::PrimaryToolbar` |
| `.bottom` | Goes into the matching region |
| `.overlay` | Pushes the `OverlayHost` used for dialogs |
| `.status` | Pushes the second host layer used for toasts, so toasts can still show while a confirm dialog is open |

## See also

[Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md) · [Controls](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md) · [Workspace](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/workspace.md)
