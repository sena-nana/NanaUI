//! Virtual accessibility nodes: context-menu items, tree rows and their
//! toggles, and an open Select's options.

use super::*;

impl UiWorld {
    /// The items of an open context menu under it, at the bounds its
    /// geometry gives them on the page; `pages` takes those page boxes.
    pub(super) fn project_virtual_menu_items(
        &self,
        menu: &AccessibilityNode,
        pages: &mut Vec<LayoutBox>,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(menu_visual) = self.widget_visual_as::<crate::MenuSurfaceVisual>(menu.id) else {
            return Vec::new();
        };
        if menu_visual.kind != crate::MenuSurfaceKind::ContextMenu || !menu_visual.open {
            return Vec::new();
        }
        let rows = &menu_visual.rows;
        let highlighted = menu_visual.highlighted;
        let options = self
            .menu_surface_geometry(menu.id)
            .map_or_else(Vec::new, |geometry| geometry.options);
        let count = rows.len().max(1) as f32;
        let slots = crate::menu_row_slots(rows);
        let surface = self.component_layout_box(menu.id).unwrap_or(menu.bounds);
        rows.iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let id = crate::virtual_menu_item_id(menu.id, slots[index])?;
                let page = options
                    .get(index)
                    .map(|option| option.bounds)
                    .unwrap_or_else(|| LayoutBox {
                        x: surface.x,
                        y: surface.y + surface.height * index as f32 / count,
                        width: surface.width,
                        height: surface.height / count,
                    });
                let bounds = self
                    .accessibility_viewport_bounds(menu.id, page, memo)
                    .unwrap_or(page);
                pages.push(page);
                Some(AccessibilityNode {
                    id,
                    parent: Some(menu.id),
                    children: Vec::new(),
                    role: row.mark.role(),
                    label: Some(
                        row.accessible_name
                            .clone()
                            .filter(|name| !name.is_empty())
                            .unwrap_or_else(|| Arc::clone(&row.label)),
                    ),
                    value: None,
                    description: row.hint.clone(),
                    role_description: None,
                    disabled: row.disabled,
                    display_only: false,
                    inert: menu.inert,
                    checked: row.mark.checked(),
                    mixed: false,
                    orientation: None,
                    selected: row.mark.selected(),
                    level: None,
                    expanded: None,
                    active_descendant: None,
                    position_in_set: None,
                    set_size: None,
                    multiselectable: false,
                    has_popup: false,
                    multiline: false,
                    editable: false,
                    selection: None,
                    modal: false,
                    busy: false,
                    invalid: false,
                    live: None,
                    numeric_minimum: None,
                    numeric_maximum: None,
                    numeric_step: None,
                    numeric_value: None,
                    scroll_x: None,
                    scroll_y: None,
                    focused: highlighted == Some(index),
                    bounds,
                    transform: None,
                    scroll: [0.0, 0.0],
                    clip: menu.clip,
                    writing: menu.writing,
                })
            })
            .collect()
    }

    /// The visible rows of a tree as `TreeItem` nodes under it, each followed
    /// by its toggles as `Checkbox` nodes under the row (#353). The row the
    /// tree is on is focused while the tree has the focus.
    ///
    /// Each takes the bounds the tree's geometry gives it on the page;
    /// `pages` takes those page boxes.
    pub(super) fn project_virtual_tree_items(
        &self,
        tree: &AccessibilityNode,
        pages: &mut Vec<LayoutBox>,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(visual) = self.widget_visual_as::<crate::TreeViewVisual>(tree.id) else {
            return Vec::new();
        };
        let rows = &visual.rows;
        let active = &visual.active;
        let column_labels = &visual.column_labels;
        let geometry = self
            .tree_view_layout(tree.id)
            .map(|layout| layout.rows)
            .unwrap_or_default();
        let count = rows.len().max(1) as f32;
        let whole = self.component_layout_box(tree.id).unwrap_or(tree.bounds);
        let mut nodes = Vec::with_capacity(rows.len() * (1 + column_labels.len()));
        for (index, row) in rows.iter().enumerate() {
            let Ok(slot) = u16::try_from(index + 1) else {
                break;
            };
            let Some(id) = crate::virtual_tree_item_id(tree.id, slot) else {
                break;
            };
            let row_geometry = geometry.get(index);
            let page = row_geometry
                .map(|row| row.bounds)
                .unwrap_or_else(|| LayoutBox {
                    x: whole.x,
                    y: whole.y + whole.height * index as f32 / count,
                    width: whole.width,
                    height: whole.height / count,
                });
            let toggles = column_labels
                .iter()
                .enumerate()
                .filter_map(|(column, label)| {
                    if nana_ui_core::tree::toggle_hidden(row.hidden_toggles, column) {
                        return None;
                    }
                    let toggle = crate::virtual_tree_toggle_id(tree.id, slot, column)?;
                    // without geometry, a square of the row's height per
                    // column at its end
                    let icon = row_geometry
                        .and_then(|row| row.toggles.get(column))
                        .map(|(_, icon, _)| *icon)
                        .unwrap_or_else(|| {
                            let side = page.height.min(page.width);
                            let from_end = (column_labels.len() - column) as f32;
                            LayoutBox {
                                x: page.x + page.width - side * from_end,
                                y: page.y,
                                width: side,
                                height: side,
                            }
                        });
                    Some((toggle, column, label, icon))
                })
                .collect::<Vec<_>>();
            let bounds = self
                .accessibility_viewport_bounds(tree.id, page, memo)
                .unwrap_or(page);
            pages.push(page);
            nodes.push(AccessibilityNode {
                id,
                parent: Some(tree.id),
                children: toggles.iter().map(|(toggle, ..)| *toggle).collect(),
                role: crate::AccessibilityRole::TreeItem,
                label: Some(Arc::clone(&row.label)),
                value: None,
                description: None,
                role_description: None,
                disabled: row.disabled,
                display_only: false,
                inert: tree.inert,
                checked: None,
                mixed: false,
                orientation: None,
                selected: Some(row.selected),
                level: Some(u32::from(row.depth) + 1),
                expanded: row.branch.then_some(row.expanded),
                active_descendant: None,
                position_in_set: None,
                set_size: None,
                multiselectable: false,
                has_popup: false,
                multiline: false,
                editable: false,
                selection: None,
                modal: false,
                busy: false,
                invalid: false,
                live: None,
                numeric_minimum: None,
                numeric_maximum: None,
                numeric_step: None,
                numeric_value: None,
                scroll_x: None,
                scroll_y: None,
                focused: tree.focused && active.as_ref() == Some(&row.id),
                bounds,
                transform: None,
                scroll: [0.0, 0.0],
                clip: tree.clip,
                writing: tree.writing,
            });
            nodes.extend(toggles.into_iter().map(|(toggle, column, label, icon)| {
                pages.push(icon);
                let icon = self
                    .accessibility_viewport_bounds(tree.id, icon, memo)
                    .unwrap_or(icon);
                AccessibilityNode {
                    id: toggle,
                    parent: Some(id),
                    children: Vec::new(),
                    role: crate::AccessibilityRole::Checkbox,
                    label: (!label.is_empty()).then(|| Arc::clone(label)),
                    value: None,
                    description: None,
                    role_description: None,
                    disabled: row.disabled,
                    display_only: false,
                    inert: tree.inert,
                    checked: Some(row.toggles.get(column).copied().unwrap_or(false)),
                    mixed: false,
                    orientation: None,
                    selected: None,
                    level: None,
                    expanded: None,
                    active_descendant: None,
                    position_in_set: None,
                    set_size: None,
                    multiselectable: false,
                    has_popup: false,
                    multiline: false,
                    editable: false,
                    selection: None,
                    modal: false,
                    busy: false,
                    invalid: false,
                    live: None,
                    numeric_minimum: None,
                    numeric_maximum: None,
                    numeric_step: None,
                    numeric_value: None,
                    scroll_x: None,
                    scroll_y: None,
                    focused: false,
                    bounds: icon,
                    transform: None,
                    scroll: [0.0, 0.0],
                    clip: tree.clip,
                    writing: tree.writing,
                }
            }));
        }
        nodes
    }

    /// An open Select, Dropdown or SearchDropdown menu, read as a WAI-ARIA
    /// combobox reads: a list box under the field with one option per row
    /// the menu shows, each known by its value, at the bounds the menu
    /// paints and hits it (the field's [`crate::SelectMenuGeometry`], under
    /// the field's transform). A row scrolled out of the menu, or cut away by
    /// a clip above the field, is not there to read; a row cut in part reads
    /// as the part shown. The field says whether it is expanded and which
    /// option the keyboard is on; the options' set is the rows the menu
    /// lists, so a filtered SearchDropdown counts only its matches.
    ///
    /// `pages` takes the boxes on the page the list box and its options
    /// stand at, in the order they are returned.
    pub(super) fn project_select_menu(
        &self,
        field: &mut AccessibilityNode,
        pages: &mut Vec<LayoutBox>,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(crate::SelectVisual {
            opened,
            options,
            highlighted,
            checkable,
            ..
        }) = self.widget_visual_as::<crate::SelectVisual>(field.id)
        else {
            return Vec::new();
        };
        if field.role != AccessibilityRole::ComboBox {
            return Vec::new();
        }
        // Open is `aria-expanded`; the `selected` the field keeps in its
        // state paints it, and a combobox is not itself chosen.
        field.expanded = Some(*opened);
        field.selected = None;
        if !*opened {
            return Vec::new();
        }
        let (
            Some(crate::SelectGeometry {
                menu: Some(menu), ..
            }),
            Some(listbox),
        ) = (
            self.select_geometry(field.id),
            crate::virtual_listbox_id(field.id),
        )
        else {
            return Vec::new();
        };
        // What the window and the clips above the field leave of the menu,
        // in the field's layout px, cut as hit testing cuts it.
        let room = match self.select_menu_room(field.id) {
            Some(room) => Some(LayoutBox {
                x: room.left,
                y: room.top,
                width: room.right - room.left,
                height: room.bottom - room.top,
            }),
            None if self.document_viewport_of(field.id).is_some() => return Vec::new(),
            None => None,
        };
        let seen = |bounds: LayoutBox| match room {
            Some(room) => intersect_layout_boxes(bounds, room),
            None => Some(bounds),
        };
        let Some((surface_page, surface)) = seen(menu.surface).and_then(|local| {
            Some((
                local,
                self.accessibility_viewport_bounds(field.id, local, memo)?,
            ))
        }) else {
            return Vec::new();
        };
        let set_size = u32::try_from(options.len()).ok();
        let slots = crate::menu_row_slots(options);
        let mut rows = Vec::with_capacity(options.len());
        let mut row_pages = Vec::with_capacity(options.len());
        for (index, (option, row)) in options.iter().zip(&menu.options).enumerate() {
            // Only the part of a row inside the menu's viewport is drawn and
            // takes the pointer.
            let Some((page, bounds)) = intersect_layout_boxes(row.bounds, menu.viewport)
                .and_then(seen)
                .and_then(|local| {
                    Some((
                        local,
                        self.accessibility_viewport_bounds(field.id, local, memo)?,
                    ))
                })
            else {
                continue;
            };
            let Some(id) = crate::virtual_menu_item_id(field.id, slots[index]) else {
                continue;
            };
            if *highlighted == Some(index) {
                field.active_descendant = Some(id);
            }
            row_pages.push(page);
            rows.push(AccessibilityNode {
                id,
                parent: Some(listbox),
                children: Vec::new(),
                role: AccessibilityRole::ListBoxOption,
                label: Some(
                    option
                        .accessible_name
                        .clone()
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| Arc::clone(&option.label)),
                ),
                value: None,
                description: option.hint.clone().filter(|hint| !hint.is_empty()),
                role_description: None,
                disabled: option.disabled,
                display_only: false,
                inert: field.inert,
                checked: None,
                mixed: false,
                orientation: None,
                selected: Some(option.mark.checked().unwrap_or(false)),
                level: None,
                expanded: None,
                active_descendant: None,
                position_in_set: u32::try_from(index + 1).ok(),
                set_size,
                multiselectable: false,
                has_popup: false,
                multiline: false,
                editable: false,
                selection: None,
                modal: false,
                busy: false,
                invalid: false,
                live: None,
                numeric_minimum: None,
                numeric_maximum: None,
                numeric_step: None,
                numeric_value: None,
                scroll_x: None,
                scroll_y: None,
                focused: false,
                bounds,
                transform: None,
                scroll: [0.0, 0.0],
                clip: field.clip,
                writing: field.writing,
            });
        }
        field.children.push(listbox);
        pages.push(surface_page);
        pages.extend(row_pages);
        let mut nodes = Vec::with_capacity(rows.len() + 1);
        nodes.push(AccessibilityNode {
            id: listbox,
            parent: Some(field.id),
            children: rows.iter().map(|row| row.id).collect(),
            role: AccessibilityRole::ListBox,
            label: field.label.clone(),
            value: None,
            description: None,
            role_description: None,
            disabled: false,
            display_only: false,
            inert: field.inert,
            checked: None,
            mixed: false,
            orientation: None,
            selected: None,
            level: None,
            expanded: None,
            active_descendant: None,
            position_in_set: None,
            set_size,
            multiselectable: *checkable,
            has_popup: false,
            multiline: false,
            editable: false,
            selection: None,
            modal: false,
            busy: false,
            invalid: false,
            live: None,
            numeric_minimum: None,
            numeric_maximum: None,
            numeric_step: None,
            numeric_value: None,
            scroll_x: None,
            scroll_y: None,
            focused: false,
            bounds: surface,
            transform: None,
            scroll: [0.0, 0.0],
            clip: field.clip,
            writing: field.writing,
        });
        nodes.extend(rows);
        nodes
    }
}

impl UiWorld {
    /// The row of `menu` that holds accessibility `slot` now.
    /// A select field's open menu counts as its menu: its options are rows.
    pub(crate) fn virtual_menu_row_index(&self, menu: StableNodeId, slot: u16) -> Option<usize> {
        if let Some(crate::SelectVisual {
            opened: true,
            options: rows,
            ..
        }) = self.widget_visual_as::<crate::SelectVisual>(menu)
        {
            return crate::menu_row_index(rows, slot);
        }
        if let Some(menu) = self.widget_visual_as::<crate::MenuSurfaceVisual>(menu) {
            return crate::menu_row_index(&menu.rows, slot);
        }
        None
    }

    /// The visible row of `tree` that holds accessibility `slot` now.
    pub(crate) fn virtual_tree_row(
        &self,
        tree: StableNodeId,
        slot: u16,
    ) -> Option<crate::tree_view::TreeRowData> {
        let index = usize::from(slot).checked_sub(1)?;
        self.widget_visual_as::<crate::TreeViewVisual>(tree)
            .and_then(|visual| visual.rows.get(index).cloned())
    }

    /// The visible row of `tree` in accessibility `slot`, when the tree has
    /// toggle `column`.
    pub(crate) fn virtual_tree_toggle_row(
        &self,
        tree: StableNodeId,
        slot: u16,
        column: usize,
    ) -> Option<crate::tree_view::TreeRowData> {
        let index = usize::from(slot).checked_sub(1)?;
        self.widget_visual_as::<crate::TreeViewVisual>(tree)
            .filter(|visual| column < visual.columns.len())
            .and_then(|visual| visual.rows.get(index).cloned())
    }
}
