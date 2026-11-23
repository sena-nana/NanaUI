# Dock

`Dock` is a pane tree that can be split and floated out. A pane's content is its child nodes. It does not take over the cross-window semantics of editor tabs; that is the `Tabs` contract.

There is no `<Dock>` in the control table.

This type is behind the Cargo feature `dock` (included in `components`); the host adapters `nana_ui::dock` and `runtime_dock_window_update` come with it.

## Basic usage

The framework computes ratios, floating windows, and hit strips.

`Dock::new(root)` takes a `DockNode` tree.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{Dock, DockNode};

view! {
    <Widget of={Dock::new(DockNode::item("stage", None)).primary("stage").title("stage", "Stage")} />
}
```

```rust rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{Dock, DockNode};

widget(Dock::new(DockNode::item("stage", None)).primary("stage").title("stage", "Stage"))
```

:::

The framework computes ratios and drop targets. What a pane shows after the drop is still the child node you bound to it.

## Nodes

`DockNode::item(id, content)` is one leaf. `content` is an optional child node id.

`DockNode::tabs(tabs, active, contents)` is a tab group.

`DockNode::split(axis, ratio, first, second)`: `axis` is `DockAxis::Horizontal` or `Vertical`, and `ratio` is the first leaf's share, clamped.

`.primary(id)` marks the central leaf that cannot be hidden.

`.title(id, title)` gives a leaf a title.

`.locked(true)` locks dragging.

`bind_content(id, node)` binds content to an existing leaf.

## Assembly

`assemble_dock_panels` is a `slot_assembler` that assembles the panels from the tree.

The view has no separate content slot method.

After you build nodes by hand and write them into a `DockNode`, call assembly yourself once.

Changing a title does not reassemble automatically.

## Floating

A floating pane goes through `DockWorkspaceEvent` and `runtime_dock_window_update` and becomes `WindowCommand::Open` or `Close`.

`DockController` and `DockAction` in `nana_ui::dock` are a host adapter that turns pointer, hover, and frame input into mutations.

Do not treat it as a second Dock.

## Visibility

`hide` takes a leaf out of the assembly; `show` puts it back.

The `primary` leaf cannot be hidden.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `Dock::new(root)` | `DockNode` | Takes a `DockNode` tree |
| `DockNode::item(id, content)` | — | One leaf. `content` is an optional child node id |
| `DockNode::tabs(tabs, active, contents)` | — | A tab group |
| `DockNode::split(axis, ratio, first, second)` | — | `axis` is `DockAxis::Horizontal` or `Vertical`. `ratio` is the first leaf's share, clamped |
| `.primary(id)` | — | Marks the central leaf that cannot be hidden |
| `.title(id, title)` | — | Gives a leaf a title. Changing a title does not reassemble automatically |
| `.locked` | — | `.locked(true)` locks dragging |
| `bind_content(id, node)` | — | Binds content to an existing leaf |
| `hide` | — | Takes a leaf out of the assembly. The `primary` leaf cannot be hidden |
| `show` | — | Puts it back |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `DockWorkspaceEvent` | — | A floating pane goes through `DockWorkspaceEvent` and `runtime_dock_window_update` and becomes `WindowCommand::Open` or `Close` |

## Slots

| Slot | Description |
| --- | --- |
| None | The view has no separate content slot method. A pane's content is its child nodes |

## See also

[AreaLayout](area-layout.md) (region splitting; that page covers when to use it instead of Dock) · [Overview](index.md) · [Controls](../reference/components.md) · [Workspace](../reference/workspace.md)
