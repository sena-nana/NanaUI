# Components

This page lists the **89 built-in controls and composed views** by family. You use them to attach nodes to the tree. The Rust entry point is `nana_ui::runtime`. Names and template tags come from the same control table: a misspelled tag or attribute is a compile error in `view!`.

Each item has its own page with a property table and an event table. The examples switch with the toggle at the top of the sidebar. For controls without their own template tag, the template side is written as `<Widget of={...}>`, and slots as `<template #name>`. The contract for behavior and events is still [Controls](../reference/components.md).

## Easily confused

- [Select](./select.md) is a single-value dropdown. [Dropdown](./dropdown.md) is single- or multi-value and shares Select's menu. [SearchDropdown](./search-dropdown.md) runs its query through a committed input box.
- [Dialog](./dialog.md) has its own body and footer. [ConfirmDialog](./confirm-dialog.md) is confirm and cancel. [Drawer](./drawer.md) also has the dialog role and enters from an edge.
- [AppShell](./app-shell.md) is a shell without a sidebar or Dock. [DesktopShell](./desktop-shell.md) is a desktop shell with a grid. [Workspace](./workspace.md) content is child nodes, and what goes in each region is up to the application; see [Workspace](../reference/workspace.md).

## Actions and input

- [Button](./button.md) — Emits `Activate` when pressed. The label is the accessible name.
- [IconButton](./icon-button.md) — A single icon. `label` is the accessible name. Activation does not change `selected` on its own.
- [TextInput](./text-input.md) — One line of text. `model` binds `value`.
- [TextArea](./text-area.md) — Multi-line text. Has `on_input`, no `on_submit`.
- [NumberInput](./number-input.md) — Numbers. Writing `value` is `assign`.
- [Checkbox](./checkbox.md) — Check box. `model` binds `checked`.
- [Switch](./switch.md) — Switch. Has one more state than a checkbox: `loading`.
- [RangeField](./range-field.md) — Slider. The template tag is `<Slider>` and the function is `slider(min, max, step)`.
- [TimelineEditor](./timeline-editor.md) — An editable time base: frame / second ruler and playhead, collapsible tracks, keyframe selection and dragging (snapped to frames), movable and trimmable clips, a Bézier curve view with tangent handles; content belongs to the application, and edits are reported through events.
- [PropertyGrid](./property-grid.md) — Inspector-style two-column property panel: collapsible groups, a shared draggable label column, mixed values, and disabled rows with a reason; controls are placed by row id.
- [ScrubField](./scrub-field.md) — A property field whose number changes as you drag left and right over it; click to type, with mixed-value support.
- [RangeSpanField](./range-span-field.md) — Two-thumb range, horizontal or vertical, with an optional live value marked on the track. The template tag is `<RangeSpan>` and the function is `range_span(min, max, step)`.
- [CurveField](./curve-field.md) — A polyline in a unit square; each point has a fixed horizontal position and you drag it up and down to change its height. The template tag is `<Curve>` and the function is `curve(step)`.
- [Select](./select.md) — Single-value dropdown.
- [Dropdown](./dropdown.md) — Single or multiple values. No `<Dropdown>` tag.
- [SearchDropdown](./search-dropdown.md) — The query goes through a committed `TextInput`.
- [FindReplaceBar](./find-replace-bar.md) — Find-and-replace toolbar; the application handles searching and editing.
- [SegmentedControl](./segmented-control.md) — Segmented selection. The options are its child nodes.
- [Tabs](./tabs.md) — Tabs. Selection, reordering, closing, and dragging between strips are reported from here.
- [XYPad](./xy-pad.md) — Two-dimensional pad. Press and move are `Input`; release is `Change`.
- [ColorField](./color-field.md) — Swatch, hex, and HSV, with native screen eyedropper support. The control assembles itself.
- [PathField](./path-field.md) — A path input plus an icon button. The control assembles itself.
- [DatePicker](./date-picker.md) — A header and 6×7 date buttons.

## Layout and text

- [Text](./text.md) — Text. Child text with interpolation recomputes with its signals.
- [RichTextView](./rich-text-view.md) — Text with styled ranges: font, color, stroke, shadow, decoration lines. The application owns the value; changing a color does not relayout.
- [RichTextEditor](./rich-text-editor.md) — Edits a `RichText` in place, breaking lines the same way as RichTextView. Toolbars send commands through `cx.rich_edit`.
- [Column](./column.md) — Vertical arrangement. The spacing is `.gap`, in logical pixels.
- [Row](./row.md) — Horizontal arrangement. Width shrinks to fit content, and children are centered on the cross axis.
- [Stack](./stack.md) — The container behind `row` / `column` / `bar` / `spacer` / `layers` / `overlay_layer`. Rows and columns are flex; `layers` stacks.
- [Divider](./divider.md) — Divider line. No label, and it takes no part in interaction.
- [IconGlyph](./icon-glyph.md) — Decorative icon. Takes no pointer input and cannot be focused.
- [ScrollView](./scroll-view.md) — Scrolling. The position is in `ScrollOffset`, the sizes in `ScrollMetrics`.
- [heading_row](./heading-row.md) — Group or card heading row with remaining-width title and overhanging tool buttons.
- [settings_group](./settings-group.md) — Grouped settings title and raised card container (`group_row_on_line`, `.inset()` tinted block).
- [scroll_foot_fade](./scroll-foot-fade.md) — Bottom scroll fade overlay that clears the scrollbar lane.

## Tables and trees

- [Table](./table.md) — Table (`TableRow` / `TableCell`). It does not compare data itself.
- [TreeView](./tree-view.md) — Tree. Does not read paths or watch the file system.
- [ReorderList](./reorder-list.md) — Drag to reorder. It reports the moved value and the value after it, and does not change the order itself.

## Display

- [Card](./card.md) — Card. Actions go on the controls inside; the card itself does not act as a button.
- [List](./list.md) — List. Screen readers read it as a list.
- [ListItem](./list-item.md) — One row. Selection is not toggled by the control itself.
- [FormField](./form-field.md) — Form row. The control is your child node.
- [DynamicForm](./dynamic-form.md) — Dynamic form driven by stable keys; the application owns state and validation.
- [InteractionRequestCard](./interaction-request-card.md) — A neutral request card; the application supplies fields and actions.
- [EmptyState](./empty-state.md) — Empty state. One action can go below the title with `.action`.
- [callout](./callout.md) — Notice strip tinted by `StatusTone`, with `.quiet()` and optional trailing action.
- [Progress](./progress.md) — Progress. Only `value` and `label`.
- [Skeleton](./skeleton.md) — Holds the size while content has not arrived.
- [Spinner](./spinner.md) — Spinner. An empty label is just the ring; with a label, it sits to the left of the text.
- [StatusBadge](./status-badge.md) — Status badge. Only describes a status; owns no action.
- [Chip](./chip.md) — Chip. Activation does not change `selected` on its own.
- [Avatar](./avatar.md) — Round avatar. Does not take part in hit testing by default.
- [Thumbnail](./thumbnail.md) — A host texture shown at its aspect ratio. Hits belong to the parent row; when `pickable`, it reports the pointer position within the image.
- [Texture](./texture.md) — A host texture that fills its own box.
- [Tooltip](./tooltip.md) — Tooltip. The role is tooltip, and the label is the displayed text.
- [ValidationMessage](./validation-message.md) — Validation text. Together with `invalid` on the control and `error` on `FormField`, it is one of three separate displays.
- [QrCode](./qr-code.md) — QR code. Not focusable; the role is image.
- [ImageViewer](./image-viewer.md) — Scrim, surface, stage, and caption row.
- [NativeMarkdown](./native-markdown.md) — Native Markdown with RaTeX math and native `nana-ui-diagram` rendering for all 34 Mermaid diagram types; copying takes a snapshot of its selection.
- [CalendarHeatmap](./calendar-heatmap.md) — Calendar heatmap. You supply dates, values, and titles.
- [Chart](./chart.md) — Charts: line, bar, pie and donut, scatter, radar, gauge. The option mirrors ECharts; you supply data and labels.
- [GraphCanvas](./graph-canvas.md) — Graph canvas. You keep the model and its persistence.
- [GraphMinimap](./graph-minimap.md) — Draws nodes and the viewport according to `GraphModel::bounds`.

## Overlays

- [Dialog](./dialog.md) — Title, body, and footer. No `<Dialog>` tag.
- [ConfirmDialog](./confirm-dialog.md) — Confirmation box. The role is alert dialog.
- [Drawer](./drawer.md) — Drawer. The role is dialog.
- [Popover](./popover.md) — Popover. Press, focus, Enter, Space, and the panel anchor all go through the trigger.
- [ActionMenu](./action-menu.md) — Action menu. Alignment changes to the start edge.
- [ContextMenu](./context-menu.md) — Context menu. The role is menu.
- [AppMenuBar](./app-menu-bar.md) — A menu bar drawn inside the application UI that follows the theme, with cascading submenus; selected items go through the same id queue as native menus.
- [CommandPalette](./command-palette.md) — Command palette. You do the actual dispatch and save shortcuts.
- [Toast](./toast.md) — Passive notification. Once assembled, it has progress, one action, and a close button; you remove it.
- [ToastStack](./toast-stack.md) — Notification corner. Measures before showing, queues, and animates in and out; placed above the modal layer, it stays pressable while a modal is open.

The framework places overlays in the window. For opening, closing, anchors, and focus return, see [Controls](../reference/components.md).

## Shell

- [AppShell](./app-shell.md) — Use it when you need no sidebar, inspector, or Dock.
- [DesktopShell](./desktop-shell.md) — Desktop shell. You still supply the content in the grid.
- [AppTitleBar](./app-title-bar.md) — Title bar. Its height is `TITLE_BAR_HEIGHT`.
- [Toolbar](./toolbar.md) — Toolbar. You place the buttons in it.
- [ToolGroup](./tool-group.md) — Tool group. A set of tool buttons on one shared trough, horizontal or vertical, read as single choice, toggles, or actions.
- [StatusBar](./status-bar.md) — Status bar. Changes are announced as live status.
- [MediaTransportBar](./media-transport-bar.md) — Play, progress, volume, settings, and fullscreen.
- [Workspace](./workspace.md) — Workspace. Content is child nodes in the tree.
- [SidebarFrame](./sidebar-frame.md) — Sidebar frame. No built-in product navigation.
- [SidebarSection](./sidebar-section.md) — Sidebar group. You supply the rows in the body.
- [SidebarRow](./sidebar-row.md) — One sidebar row. Drawn by reusing the list item's slots.
- [SettingsRow](./settings-row.md) — Settings row; `label_column` switches to an inspector-style aligned label column. Your state still owns the actual values.
- [Dock](./dock.md) — Docking. A pane's content is child nodes.
- [AreaLayout](./area-layout.md) — DCC-style region splitting: drag from a corner to split or join, regions can be maximized, headers and content are placed by key, and the layout is serializable.
- [SplitPane](./split-pane.md) — Split. Sizes come from `SplitPaneModel`.
- [PaneChrome](./pane-chrome.md) — A tab strip and a row of icon actions.

What goes in each region is up to your application; see [Workspace](../reference/workspace.md).

## Surfaces

Sizes, colors, fonts, and control surfaces come from the theme. Switching between dark and light changes colors. Single-line controls have three heights: small, medium, and large. For the exact values and how to switch themes, see [Look](../reference/look.md).

Some families need their own Cargo feature, such as `calendar`, `charts`, `graph-canvas`, and `rich-text`. `components` enables all optional families. See the table in [Application API](../reference/application-api.md).
