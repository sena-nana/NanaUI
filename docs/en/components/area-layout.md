# AreaLayout

`AreaLayout` recursively splits a rectangle into non-overlapping areas. 3D and animation software often uses this kind of layout. Each area shows one editor, and the application provides both its content and its header. It has no template tag; put it into a view with `widget(AreaLayout::new(model))`.

This type is behind the Cargo feature `area-layout` (included in `components`).

## Basic usage

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{AreaAxis, AreaLayout, AreaLayoutChanged, AreaLayoutModel};

let mut model = AreaLayoutModel::single("outliner", "outliner");
model.split("outliner", AreaAxis::Horizontal, 0.25, "viewport");
model.set_editor("viewport", "viewport");

widget(AreaLayout::new(model))
    .on(|e: &AreaLayoutChanged| { /* e.change, e.model */ })
    .children((
        outliner_header().key("outliner:header"),
        outliner().key("outliner"),
        viewport_header().key("viewport:header"),
        viewport().key("viewport"),
    ))
```

View children are placed into areas by key:

- A child whose key is an area id is that area's content.
- A child whose key is `<area id>:header` goes into that area's header. The header is a strip at small control height that holds the application's own header controls.
- A child whose key is `editor:<type>` is the content of that editor type, and a child whose key is `editor:<type>:header` is its header controls; they go into the area currently showing that editor (see "Placing by editor type" below).

Children for areas not in the current layout, or for editors no area shows, are hidden and wait; they are not destroyed.

## Placing by editor type

Giving content by area id means the application has to know every area's id in advance. A new area the user cuts off from a corner gets its id from `fresh_id`, so the application cannot declare its content ahead of time. Giving content by editor type avoids this: whichever editor an area switches to, that editor's content moves there.

```rust
widget(AreaLayout::new(model).editor_select(AreaEditorSelect::new(["outliner", "viewport"])))
    .children((
        outliner().key("editor:outliner"),
        viewport_toolbar().key("editor:viewport:header"),
        viewport().key("editor:viewport"),
    ))
```

- An editor's content is in only one area at a time: the area that most recently selected it with `set_area_editor` (or the header dropdown) wins, then the area that currently holds it, then the first in leaf order; visible areas win (when an area is maximized, only it is visible). Other areas also showing that editor have empty content, and the application can switch them to other editors after receiving `AreaChange::Split` / `AreaChange::Editor`.
- Content keyed by area id wins: an area that has such content does not receive content placed by type.
- After `set_area_layout` replaces the layout, placement is decided again in the order above.

## Editor type dropdown

`.editor_select(AreaEditorSelect::new([types…]))` puts a small dropdown at the far left of each area header. Its options are these types, named from the `.labels(...)` name table (types without a name use the id); if an area's current editor is not in the list, it is listed as an option too. Choosing an option is the same as calling `set_area_editor` on that area and emits `AreaChange::Editor`. The dropdown's name for screen readers comes from `AreaEditorSelect::label`; when unset it uses the framework string `area_layout.editor_type` (default "Editor type"). On a language change, update the option names with `set_area_labels` and the dropdown itself with `set_area_editor_select`.

## Model

`AreaLayoutModel` is plain data with the following parts:

- `root`: the area tree. `AreaNode::Area { id }` is a leaf. `AreaNode::Split { axis, ratio, first, second }` cuts the rectangle in two. `AreaAxis::Horizontal` places them side by side, `AreaAxis::Vertical` stacks them top to bottom. `ratio` is the first half's share.
- `areas`: each area's `editor` (the application's editor type id) and `state` (a key the application uses to save the area's own state). NanaUI only stores these two values; what an area shows after switching editors is up to the application.
- `maximized`: the maximized area.

`split`, `join`, `can_join`, `set_editor`, `set_ratio`, `toggle_maximized` and `fresh_id` modify the model directly. `geometry(bounds, gap, min_size)` computes each area's and each divider's position without a document. `to_json` / `from_json` serialize it, and `from_json` checks with `validate` first: the tree and the areas correspond one to one, ids are unique, and the maximized area exists.

## Pointer

- **Dividers**: the gap between areas (`gap`, default 2 px) is the divider, and it can be dragged. However thin the gap is drawn, there is at least a 6 px wide range to grab: the divider extends into the edge of the adjacent area on both sides and sits on top of the areas. Neither side goes below `min_size` (default 48 px). Releasing emits `AreaChange::Resized`.
- **Splitting**: drag from the small square at the top-left or top-right corner of an area header into the area. Mostly horizontal movement splits left and right, mostly vertical movement splits top and bottom, and the cut lands at the pointer. During the drag an accent-colored cut line shows, and the split happens only on release. The new area shows the same editor, with an id generated by `fresh_id`. Emits `AreaChange::Split { area, new }`.
- **Joining**: drag from a corner into the adjacent area. The two must be the two halves of the same split. The area being dragged into is covered with an accent-colored shade, showing that it will be removed; on release, the area where the drag started takes its place. Emits `AreaChange::Joined { kept, removed }`. Dragging into a non-adjacent area does not join.
- Neither half may be smaller than `min_size`. If the pointer leaves the area, or moves less than 8 px, nothing is split or joined.

## Application calls

| Call | Description |
| --- | --- |
| `split_area(layout, area, axis, ratio)` | Splits the area and returns the new area's id |
| `join_areas(layout, kept, removed)` | Joins the two halves of the same split |
| `toggle_area_maximized(layout, Some(area))` | Maximizes; calling it again on an already maximized area restores it, and passing `None` also restores |
| `set_area_editor(layout, area, editor)` | Switches the editor type; content placed by type moves along, while the application switches content given by area id itself |
| `set_area_layout(layout, model)` | Replaces the whole layout, for example a workspace preset |
| `set_area_labels(layout, [(editor, name)])` | Changes each editor's name for screen readers and in the header dropdown, for example after the UI language changes; the layout does not change |
| `set_area_editor_select(layout, Some(dropdown) / None)` | Replaces the header's editor type dropdown or removes it; the layout does not change |
| `area_layout_boxes(layout)` | Each visible area's box after layout, for placing overlays on areas |

Except for `set_area_labels` and `set_area_editor_select`, every call emits one `AreaLayoutChanged` carrying the new model, which the application can save.

Each area is a `Region` in the accessibility tree. Its name is looked up by editor type id in `.labels(...)` / `set_area_labels`; when none is given it is the id itself. Names follow the UI language and are not written into `AreaLayoutModel`, so the model stays language-independent data.

When the view re-renders, the model in `AreaLayout::new` does not overwrite a layout the user has already dragged. To replace it wholesale, call `set_area_layout`.

## Workspaces

Workspaces are just several saved `AreaLayoutModel`s; switching workspaces calls `set_area_layout`. As long as the view still declares these content children, they are always the same set of nodes:

- Areas present in both layouts keep their content in place.
- Areas that appear only in the old layout have their content hidden and waiting.

So state such as scroll position and text being typed is not lost.

## Choosing between this and Dock

[Dock](https://github.com/sena-nana/NanaUI/blob/main/docs/components/dock.md) is IDE-style docking: panels can be dragged to an edge or into a tab group, or floated out as separate windows, and several panels can stack in one position. `AreaLayout` is DCC-style splitting:

- Areas never overlap, and an area shows only one editor.
- The layout changes by splitting or joining from corners, not by dragging panels.
- The application decides what goes in an area header.

Use Dock when there are many panels and you need tabs and floating windows. Use `AreaLayout` when the set of editor types is fixed and users should be able to split the screen casually, as in Blender.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `AreaLayout::new(model)` | `AreaLayoutModel` | Initial layout |
| `.gap(px)` | `f32` | Gap between areas, which is also the divider width; default 2 |
| `.min_size(px)` | `f32` | Minimum size on each side of a divider and of each half after a split; default 48 |
| `.chrome(appearance)` | `AreaChrome` | Each area's appearance: `frame_radius` is the area frame's radius tier (default `RadiusTier::Sm`), `header_background` is the header's background color role (default `Subtle`; the area frame itself is `Surface`; use `Header` to make the header a separate color band), and `header_divider` draws a thin line between header and content (off by default). `header_height` is the header height (default `None`, meaning small control height, which only fits one row of compact controls); set it taller than a control and the header content centers vertically, fits tool groups with bottom slots, and looks more like the area's title bar. With a larger gap, a `Surface` header, the divider line on and `Lg` corners, the areas become separate panels floating over the background |
| `.labels([(editor, name)])` | `(String, Arc<str>)` | Screen reader names for areas of each editor, also the option names in the header dropdown, keyed by editor type id; types not given use the id |
| `.editor_select(dropdown)` | `AreaEditorSelect` | The editor type dropdown at the far left of each area header: `editors` are the options (type ids, in order), `label` is the screen reader name |
| `.style(style)` | `NodeStyle` | Root node style; fills the parent by default |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `AreaLayoutChanged` | `change: AreaChange`, `model: AreaLayoutModel` | `Resized`, `Split`, `Joined`, `Maximized`, `Editor`, `Replaced` |

## Slots

| Slot | Description |
| --- | --- |
| Child keyed by area id | That area's content |
| Child keyed by `<area id>:header` | That area's header content |
| Child keyed by `editor:<type>` | Content of one area currently showing that editor |
| Child keyed by `editor:<type>:header` | Header content of that same area, placed after the editor type dropdown |

## See also

[Dock](https://github.com/sena-nana/NanaUI/blob/main/docs/components/dock.md) · [Workspace](https://github.com/sena-nana/NanaUI/blob/main/docs/components/workspace.md) · [Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
