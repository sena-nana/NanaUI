//! Accessible projection from retained nodes.

use super::*;

#[derive(Default)]
struct ProjectionMemo {
    transforms: hashbrown::HashMap<StableNodeId, AccessibleTransform>,
    bounds: hashbrown::HashMap<StableNodeId, Option<LayoutBox>>,
    /// Ancestor-hidden answers. A deep tree otherwise rewalks the parent
    /// chain once per node and once per child filter.
    hidden: hashbrown::HashMap<StableNodeId, bool>,
    /// Nodes whose scroll and transform chain is the identity, so viewport
    /// bounds are the layout box.
    aligned: hashbrown::HashMap<StableNodeId, bool>,
    chain: Vec<StableNodeId>,
}

#[derive(Clone, Copy)]
struct AccessibleTransform {
    own: ([f32; 6], [f32; 2]),
    children: ([f32; 6], [f32; 2]),
    blocks_3d: bool,
}

impl UiWorld {
    /// Layout box and viewport box are the same when no ancestor scrolls or
    /// transforms. The answer is memoized; a skewed node forces the normal
    /// transform walk for itself and its descendants.
    fn axis_aligned(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        if let Some(aligned) = memo.aligned.get(&id) {
            return *aligned;
        }
        if !self.layout_length_tracks.is_empty()
            || !self.presentation.is_empty()
            || self.z_index_nodes != 0
            || self.nodes.has_visuals()
            || !self.overlay_host_nodes.is_empty()
            || self.clip_visuals != 0
        {
            memo.aligned.insert(id, false);
            return false;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut aligned = true;
        while let Some(node) = current {
            if let Some(&known) = memo.aligned.get(&node) {
                aligned = known;
                break;
            }
            memo.chain.push(node);
            let Some(record) = self.nodes.get(node) else {
                aligned = false;
                break;
            };
            let style = record.resolved_layout.as_ref();
            if record.scroll_offset != ScrollOffset::default()
                || style.transform.is_some()
                || style.transform_3d.is_some()
                || matches!(style.position, PositionSpec::Fixed | PositionSpec::Sticky)
                || style.css_perspective.is_some()
                || style.preserve_3d
            {
                aligned = false;
                break;
            }
            current = record.hierarchy.parent;
        }
        for node in memo.chain.drain(..) {
            memo.aligned.insert(node, aligned);
        }
        aligned
    }

    fn visible_accessibility_bounds(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        if let Some(bounds) = memo.bounds.get(&id) {
            return *bounds;
        }
        let bounds = self.compute_accessibility_bounds(id, memo);
        memo.bounds.insert(id, bounds);
        bounds
    }

    fn compute_accessibility_bounds(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        let local = if self.nodes.has_visuals()
            && matches!(
                self.nodes.visual(id),
                Some(StandardVisual::ModalFrame { .. })
            ) {
            match self.component_geometry(id) {
                Some(crate::ComponentGeometry::ModalFrame { surface, .. }) => surface,
                _ => self.component_layout_box(id)?,
            }
        } else {
            self.component_layout_box(id)?
        };
        let mut bounds = self.accessibility_viewport_bounds(id, local, memo)?;
        if self.clip_visuals == 0 {
            return Some(bounds);
        }
        let mut parent = self.nodes.get(id)?.hierarchy.parent;
        while let Some(ancestor) = parent {
            if matches!(
                self.nodes.visual(ancestor),
                Some(StandardVisual::EmptyState { .. })
            ) {
                bounds = intersect_layout_boxes(
                    bounds,
                    self.accessibility_viewport_bounds(
                        ancestor,
                        self.component_layout_box(ancestor)?,
                        memo,
                    )?,
                )?;
            }
            if matches!(
                self.nodes.visual(ancestor),
                Some(StandardVisual::ModalFrame { .. })
            ) && let Some(crate::ComponentGeometry::ModalFrame { surface, body, .. }) =
                self.component_geometry(ancestor)
            {
                bounds = intersect_layout_boxes(
                    bounds,
                    self.accessibility_viewport_bounds(ancestor, surface, memo)?,
                )?;
                if let Some(StandardVisual::ModalFrame { slots, .. }) = self.nodes.visual(ancestor)
                    && slots
                        .body
                        .is_some_and(|body_root| self.is_descendant_or_self(id, body_root))
                {
                    bounds = intersect_layout_boxes(
                        bounds,
                        self.accessibility_viewport_bounds(ancestor, body, memo)?,
                    )?;
                }
            }
            parent = self.nodes.get(ancestor)?.hierarchy.parent;
        }
        Some(bounds)
    }
}

impl UiWorld {
    // Match build_hit_forest's composition from the current retained state.
    // A committed transform may precede the hit projection; that projection
    // is not an authority for accessibility coordinates. Memoizing ancestors
    // also avoids repeating hit-index ancestor walks for every projected node.
    fn accessibility_ancestor_transform(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        if let Some(transform) = memo.transforms.get(&id) {
            return Some(transform.own.0);
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut cumulative = (IDENTITY_AFFINE, [0.0, 0.0]);
        let mut blocks_3d = false;
        while let Some(node) = current {
            if let Some(transform) = memo.transforms.get(&node) {
                cumulative = transform.children;
                blocks_3d = transform.blocks_3d;
                break;
            }
            memo.chain.push(node);
            current = self.nodes.get(node)?.hierarchy.parent;
        }
        while let Some(node) = memo.chain.pop() {
            let record = self.nodes.get(node)?;
            let bounds = self.component_layout_box(node)?;
            let layout = self.hit_motion_layout(node);
            if layout.position == PositionSpec::Fixed {
                // Viewport-relative; ancestors cannot scroll or clip this branch.
                cumulative = self.hanging_branch_transform(node);
                blocks_3d = false;
            }
            let local = self.input_local_scene_transform(node, &layout, bounds, blocks_3d);
            cumulative = then_hit(cumulative, self.with_sticky_translation(node, local));
            let own = cumulative;
            let scroll = record.scroll_offset;
            cumulative = then_hit(
                cumulative,
                ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
            );
            blocks_3d |= layout.fails_closed_3d_context();
            memo.transforms.insert(
                node,
                AccessibleTransform {
                    own,
                    children: cumulative,
                    blocks_3d,
                },
            );
        }
        memo.transforms.get(&id).map(|transform| transform.own.0)
    }

    /// AccessKit consumes document/viewport logical coordinates. Layout boxes
    /// deliberately stay unscrolled; compose current ancestor transforms so
    /// pointer and accessibility consumers agree after nested scrolling.
    fn accessibility_viewport_bounds(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        let transform = self.accessibility_ancestor_transform(id, memo)?;
        let [a, b, c, d, e, f] = transform;
        if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && e == 0.0 && f == 0.0 {
            return Some(bounds);
        }
        let corners = [
            (bounds.x, bounds.y),
            (bounds.x + bounds.width, bounds.y),
            (bounds.x, bounds.y + bounds.height),
            (bounds.x + bounds.width, bounds.y + bounds.height),
        ]
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
        if corners
            .iter()
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return None;
        }
        let x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let right = corners
            .iter()
            .map(|p| p.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = corners
            .iter()
            .map(|p| p.1)
            .fold(f32::NEG_INFINITY, f32::max);
        Some(LayoutBox {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

#[cfg(test)]
#[path = "accessibility_viewport_tests.rs"]
mod viewport_tests;

impl UiWorld {
    /// Whether a node or one of its ancestors is explicitly hidden from the
    /// accessibility tree. The retained hierarchy is still used for paint,
    /// layout, and input; this is only the semantic projection boundary.
    fn accessibility_hidden(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        if let Some(hidden) = memo.hidden.get(&id) {
            return *hidden;
        }
        let Some(node) = self.nodes.get(id) else {
            memo.hidden.insert(id, false);
            return false;
        };
        if node.accessibility.hidden || self.inline_box_omitted(id) {
            memo.hidden.insert(id, true);
            return true;
        }
        let parent = node.hierarchy.parent;
        let hidden = parent.is_some_and(|parent| self.accessibility_hidden(parent, memo));
        memo.hidden.insert(id, hidden);
        hidden
    }

    /// What `id` says to assistive technology as the name of a node it
    /// labels: its own label, or its text.
    fn accessible_text(&self, id: StableNodeId) -> Option<Arc<str>> {
        let node = self.nodes.get(id)?;
        node.accessibility
            .label
            .clone()
            .filter(|label| !label.is_empty())
            .or_else(|| {
                (!node.text.value.is_empty())
                    .then(|| self.inline_text(id, true).unwrap_or_default().into())
            })
    }

    /// Every retained node is a mounted, visible, untransformed box. Viewport
    /// bounds are the layout box, and every structural child stays in the
    /// accessibility tree. One scan, then the per-node walk does not re-check
    /// ancestors. A single scrolled or transformed node keeps the normal path.
    fn accessibility_tree_is_plain(&self) -> bool {
        if !self.layout_length_tracks.is_empty()
            || !self.presentation.is_empty()
            || self.z_index_nodes != 0
            || self.nodes.has_visuals()
            || !self.overlay_host_nodes.is_empty()
            || self.clip_visuals != 0
            || self.nodes.has_inline_content()
            || !self.detached_mounted.is_empty()
        {
            return false;
        }
        self.nodes.records().all(|node| {
            node.mount == crate::MountState::Mounted
                && node.scroll_offset == crate::ScrollOffset::default()
                && !node.accessibility.hidden
                && node.resolved.0.visible
                && node.resolved.0.box_visible
                && !matches!(node.kind.as_ref(), NodeKind::Comment)
                && crate::components::layout_style_is_default(&node.resolved_layout)
        })
    }

    fn project_accessibility_node(
        &self,
        id: StableNodeId,
        plain: bool,
        memo: &mut ProjectionMemo,
    ) -> Option<AccessibilityNode> {
        if !plain && (!self.is_mounted(id) || self.accessibility_hidden(id, memo)) {
            return None;
        }
        let (
            parent,
            children,
            kind,
            state,
            text_value,
            document,
            visible,
            box_visible,
            writing,
            scrolls,
            layout_box,
        ) = {
            let node = self.nodes.get(id)?;
            (
                node.hierarchy.parent,
                Arc::clone(&node.hierarchy.children),
                Arc::clone(&node.kind),
                if node.resolved.0.visible {
                    node.accessibility.clone()
                } else {
                    AccessibilityState::default()
                },
                if node.resolved.0.visible {
                    node.text.value.clone()
                } else {
                    crate::TextValue::default()
                },
                node.document,
                node.resolved.0.visible,
                node.resolved.0.box_visible,
                node.resolved.0.writing_context(),
                node.style.layout.overflow_x.scrolls() || node.style.layout.overflow_y.scrolls(),
                node.layout,
            )
        };
        // Keep a neutral structural container so visible descendants never
        // reference an omitted parent. Hidden leaves and omitted layout subtrees
        // contribute neither semantics nor structure.
        if !visible && (!box_visible || children.is_empty()) {
            return None;
        }
        if matches!(kind.as_ref(), NodeKind::Comment) {
            return None;
        }
        let role = if !visible {
            AccessibilityRole::Generic
        } else {
            match (state.role, kind.as_ref()) {
                (AccessibilityRole::Generic, NodeKind::Document) => AccessibilityRole::Document,
                (AccessibilityRole::Generic, NodeKind::Text) => AccessibilityRole::Text,
                (role, _) => role,
            }
        };
        let secure = self.nodes.has_visuals()
            && matches!(
                self.nodes.visual(id),
                Some(StandardVisual::TextInput { secure: true, .. })
            );
        // Scroll positions are part of the semantic node, rather than a
        // second visual-only state. Publish an axis only when the retained
        // geometry says it can actually move; this keeps ordinary containers
        // from advertising meaningless scroll actions.
        let scroll_metrics = (visible && (scrolls || self.nodes.has_visuals()))
            .then(|| {
                self.is_scroll_container(id)
                    .then(|| self.scroll_metrics(id))
            })
            .flatten()
            .flatten();
        let scroll_offset = scroll_metrics.and_then(|_| self.scroll_offset(id));
        // A node another one names (a settings row's label beside its
        // switch) keeps a label of its own first, as `aria-label` wins over
        // `aria-labelledby`; an empty one names nothing. Its own text (a
        // select's shown option) is its value then, not its name.
        let label = match (!self.labelled_by.is_empty())
            .then(|| self.labelled_by(id))
            .flatten()
        {
            Some(source) if visible => state
                .label
                .clone()
                .filter(|label| !label.is_empty())
                .or_else(|| self.accessible_text(source)),
            // A password field's text is its secret, not a name for it.
            _ => state.label.clone().or_else(|| {
                (!secure && !text_value.is_empty()).then(|| {
                    if self.inline_record(id).is_some() {
                        self.inline_text(id, true).unwrap_or_default().into()
                    } else {
                        Arc::<str>::from(text_value.as_str())
                    }
                })
            }),
        };
        let bounds = if plain && self.layout_results.is_empty() {
            layout_box
        } else if plain || self.axis_aligned(id, memo) {
            self.component_layout_box(id)?
        } else {
            self.visible_accessibility_bounds(id, memo)?
        };
        let mut children = if plain {
            children.as_ref().clone()
        } else {
            children
                .iter()
                .copied()
                .filter(|child| {
                    let child_id = *child;
                    self.nodes.get(child_id).is_some_and(|node| {
                        node.resolved.0.box_visible
                            && (node.resolved.0.visible || !node.hierarchy.children.is_empty())
                            && !matches!(node.kind.as_ref(), NodeKind::Comment)
                    }) && !self.accessibility_hidden(child_id, memo)
                        && (self.axis_aligned(child_id, memo)
                            || self.visible_accessibility_bounds(child_id, memo).is_some())
                })
                .collect::<Vec<_>>()
        };
        if self.nodes.has_visuals()
            && let Some(StandardVisual::MenuSurface {
                kind: crate::MenuSurfaceKind::ContextMenu,
                open: true,
                rows,
                ..
            }) = self.nodes.visual(id)
        {
            children.extend(
                crate::menu_row_slots(rows)
                    .into_iter()
                    .filter_map(|slot| crate::virtual_menu_item_id(id, slot)),
            );
        }
        if self.nodes.has_visuals()
            && let Some(StandardVisual::TreeView { rows, .. }) = self.nodes.visual(id)
        {
            children.extend(
                (0..rows.len())
                    .map_while(|index| u16::try_from(index + 1).ok())
                    .filter_map(|slot| crate::virtual_tree_item_id(id, slot)),
            );
        }
        Some(AccessibilityNode {
            id,
            parent,
            children,
            role,
            label,
            description: state.description.clone(),
            role_description: state.role_description.clone(),
            value: if !visible || secure {
                None
            } else if self.nodes.has_text_inputs() {
                self.nodes
                    .text_input(id)
                    .map(|input| input.value_shared())
                    .or_else(|| state.value.as_ref().map(crate::TextValue::from))
            } else {
                state.value.as_ref().map(crate::TextValue::from)
            },
            disabled: visible
                && (state.disabled
                    || self
                        .confirm_action_effect(id)
                        .is_some_and(|effect| effect.0)),
            display_only: state.display_only,
            checked: state.checked,
            mixed: state.mixed,
            orientation: state.orientation,
            selected: state.selected,
            level: None,
            expanded: None,
            active_descendant: None,
            position_in_set: None,
            set_size: None,
            multiselectable: false,
            has_popup: state.has_popup,
            multiline: state.multiline,
            editable: state.editable,
            selection: if visible && self.nodes.has_text_inputs() {
                self.nodes.text_input(id).map(|input| input.selection)
            } else {
                None
            },
            modal: state.modal,
            busy: state.busy,
            invalid: state.invalid,
            // A hidden container stays neutral: it is no live region either,
            // as a `visibility: hidden` element is none on the web.
            live: (visible && !self.live_regions.is_empty())
                .then(|| self.live(id))
                .flatten(),
            numeric_minimum: state.numeric_minimum,
            numeric_maximum: state.numeric_maximum,
            numeric_step: state.numeric_step,
            numeric_value: state.numeric_value,
            scroll_x: scroll_metrics.and_then(|metrics| {
                let min = metrics.min_offset().x;
                let max = metrics.max_offset().x;
                let offset = scroll_offset?;
                (max > min).then_some(crate::AccessibilityScrollAxis {
                    value: offset.x as f64,
                    minimum: min as f64,
                    maximum: max as f64,
                })
            }),
            scroll_y: scroll_metrics.and_then(|metrics| {
                let min = metrics.min_offset().y;
                let max = metrics.max_offset().y;
                let offset = scroll_offset?;
                (max > min).then_some(crate::AccessibilityScrollAxis {
                    value: offset.y as f64,
                    minimum: min as f64,
                    maximum: max as f64,
                })
            }),
            focused: visible && self.input.focused.get(&document) == Some(&id),
            bounds,
            writing,
        })
    }
}

impl UiWorld {
    fn project_virtual_menu_items(&self, menu: &AccessibilityNode) -> Vec<AccessibilityNode> {
        let Some(StandardVisual::MenuSurface {
            kind: crate::MenuSurfaceKind::ContextMenu,
            open: true,
            rows,
            highlighted,
            ..
        }) = self.nodes.visual(menu.id)
        else {
            return Vec::new();
        };
        let options = match self.component_geometry(menu.id) {
            Some(crate::ComponentGeometry::MenuSurface { options, .. }) => options,
            _ => Vec::new(),
        };
        let count = rows.len().max(1) as f32;
        let slots = crate::menu_row_slots(rows);
        rows.iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let id = crate::virtual_menu_item_id(menu.id, slots[index])?;
                let bounds = options
                    .get(index)
                    .map(|option| option.bounds)
                    .unwrap_or_else(|| LayoutBox {
                        x: menu.bounds.x,
                        y: menu.bounds.y + menu.bounds.height * index as f32 / count,
                        width: menu.bounds.width,
                        height: menu.bounds.height / count,
                    });
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
                    focused: *highlighted == Some(index),
                    bounds,
                    writing: menu.writing,
                })
            })
            .collect()
    }

    /// The visible rows of a tree as `TreeItem` nodes under it, each followed
    /// by its toggles as `Checkbox` nodes under the row (#353). The row the
    /// tree is on is focused while the tree has the focus.
    fn project_virtual_tree_items(&self, tree: &AccessibilityNode) -> Vec<AccessibilityNode> {
        let Some(StandardVisual::TreeView {
            rows,
            active,
            column_labels,
            ..
        }) = self.nodes.visual(tree.id)
        else {
            return Vec::new();
        };
        let geometry = match self.component_geometry(tree.id) {
            Some(crate::ComponentGeometry::TreeView { rows, .. }) => rows,
            _ => Vec::new(),
        };
        let count = rows.len().max(1) as f32;
        let mut nodes = Vec::with_capacity(rows.len() * (1 + column_labels.len()));
        for (index, row) in rows.iter().enumerate() {
            let Ok(slot) = u16::try_from(index + 1) else {
                break;
            };
            let Some(id) = crate::virtual_tree_item_id(tree.id, slot) else {
                break;
            };
            let row_geometry = geometry.get(index);
            let bounds = row_geometry
                .map(|row| row.bounds)
                .unwrap_or_else(|| LayoutBox {
                    x: tree.bounds.x,
                    y: tree.bounds.y + tree.bounds.height * index as f32 / count,
                    width: tree.bounds.width,
                    height: tree.bounds.height / count,
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
                            let side = bounds.height.min(bounds.width);
                            let from_end = (column_labels.len() - column) as f32;
                            LayoutBox {
                                x: bounds.x + bounds.width - side * from_end,
                                y: bounds.y,
                                width: side,
                                height: side,
                            }
                        });
                    Some((toggle, column, label, icon))
                })
                .collect::<Vec<_>>();
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
                writing: tree.writing,
            });
            nodes.extend(toggles.into_iter().map(|(toggle, column, label, icon)| {
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
    fn project_select_menu(
        &self,
        field: &mut AccessibilityNode,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(StandardVisual::Select {
            opened,
            options,
            highlighted,
            checkable,
            ..
        }) = self.nodes.visual(field.id)
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
            Some(crate::ComponentGeometry::Select {
                menu: Some(menu), ..
            }),
            Some(listbox),
        ) = (
            self.component_geometry(field.id),
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
        let Some(surface) = seen(menu.surface)
            .and_then(|local| self.accessibility_viewport_bounds(field.id, local, memo))
        else {
            return Vec::new();
        };
        let set_size = u32::try_from(options.len()).ok();
        let slots = crate::menu_row_slots(options);
        let mut rows = Vec::with_capacity(options.len());
        for (index, (option, row)) in options.iter().zip(&menu.options).enumerate() {
            // Only the part of a row inside the menu's viewport is drawn and
            // takes the pointer.
            let Some(bounds) = intersect_layout_boxes(row.bounds, menu.viewport)
                .and_then(seen)
                .and_then(|local| self.accessibility_viewport_bounds(field.id, local, memo))
            else {
                continue;
            };
            let Some(id) = crate::virtual_menu_item_id(field.id, slots[index]) else {
                continue;
            };
            if *highlighted == Some(index) {
                field.active_descendant = Some(id);
            }
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
                writing: field.writing,
            });
        }
        field.children.push(listbox);
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
            writing: field.writing,
        });
        nodes.extend(rows);
        nodes
    }

    fn project_accessibility_entries(
        &self,
        id: StableNodeId,
        plain: bool,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(mut node) = self.project_accessibility_node(id, plain, memo) else {
            return Vec::new();
        };
        if self.nodes.has_visuals()
            && matches!(self.nodes.visual(id), Some(StandardVisual::Select { .. }))
        {
            let menu = self.project_select_menu(&mut node, memo);
            let mut entries = vec![node];
            entries.extend(menu);
            return entries;
        }
        if self.nodes.has_visuals()
            && matches!(self.nodes.visual(id), Some(StandardVisual::TreeView { .. }))
        {
            let rows = self.project_virtual_tree_items(&node);
            let mut entries = vec![node];
            entries.extend(rows);
            return entries;
        }
        let open_context_menu = self.nodes.has_visuals()
            && matches!(
                self.nodes.visual(id),
                Some(StandardVisual::MenuSurface {
                    kind: crate::MenuSurfaceKind::ContextMenu,
                    open: true,
                    ..
                })
            );
        if !open_context_menu {
            return vec![node];
        }
        let mut entries = vec![node];
        let menu = entries[0].clone();
        entries.extend(self.project_virtual_menu_items(&menu));
        entries
    }

    /// Project one complete incremental accessibility transaction, including
    /// tombstones for nodes removed from the retained world.
    pub fn project_accessibility_delta(&self, work: &SystemWork) -> AccessibilityDelta {
        // Scrolling and transforms leave LayoutBox unchanged and may carry no
        // ACCESSIBILITY dirty bit. Their hit-test subtrees nevertheless moved
        // in viewport space, so the native accessibility cache must see them.
        //
        // Scheduled-layout nodes are deliberately NOT seeds, for the same
        // reason `RuntimeDocument::apply_hit_test_work` refuses them: layout
        // invalidation propagates to ancestors, so any leaf resize puts the
        // document root in `work.layout_frontier_seeds`, and expanding a root seed walks the
        // whole document. The nodes whose box actually moved are already here
        // by a different route -- layout writeback commits `WriteLayout`,
        // which marks INPUT | RENDER | ACCESSIBILITY on exactly those nodes,
        // shifted descendants included.
        //
        // These sets exist to produce a sorted, de-duplicated id sequence. A
        // `BTreeSet` pays a tree insert and node allocation per id to do that;
        // sorting a flat vec once yields the identical sequence for far less.
        let mut affected = work.accessibility.clone();
        // A node named by another one's text is projected again with it.
        let named = affected
            .iter()
            .filter_map(|id| self.labels.get(id))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        affected.extend(named);
        let mut pending = work
            .input_hit_test
            .iter()
            .chain(&work.transform)
            .copied()
            .collect::<Vec<_>>();
        // `visited` is membership-only, so it does not need ordering at all.
        let mut visited = hashbrown::HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            affected.push(id);
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.hierarchy.children.iter().copied());
            }
        }
        affected.sort_unstable();
        affected.dedup();
        let mut removed = work.accessibility_removals.clone();
        let mut memo = ProjectionMemo::default();
        let plain = affected.len() >= 64 && self.accessibility_tree_is_plain();
        let mut updated = Vec::new();
        for id in affected {
            let entries = self.project_accessibility_entries(id, plain, &mut memo);
            if entries.is_empty() {
                if self.nodes.contains(id) {
                    removed.push(id);
                }
            } else {
                updated.extend(entries);
            }
        }
        removed.sort_unstable();
        removed.dedup();
        AccessibilityDelta {
            generation: work.generation,
            updated,
            removed,
        }
    }
}

impl UiWorld {
    /// The row of `menu` that holds accessibility `slot` now.
    /// A select field's open menu counts as its menu: its options are rows.
    pub(crate) fn virtual_menu_row_index(&self, menu: StableNodeId, slot: u16) -> Option<usize> {
        match self.nodes.visual(menu) {
            Some(StandardVisual::MenuSurface { rows, .. })
            | Some(StandardVisual::Select {
                opened: true,
                options: rows,
                ..
            }) => crate::menu_row_index(rows, slot),
            _ => None,
        }
    }

    /// The visible row of `tree` that holds accessibility `slot` now.
    pub(crate) fn virtual_tree_row(
        &self,
        tree: StableNodeId,
        slot: u16,
    ) -> Option<crate::tree_view::TreeRowData> {
        let index = usize::from(slot).checked_sub(1)?;
        match self.nodes.visual(tree) {
            Some(StandardVisual::TreeView { rows, .. }) => rows.get(index).cloned(),
            _ => None,
        }
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
        match self.nodes.visual(tree) {
            Some(StandardVisual::TreeView { rows, columns, .. }) if column < columns.len() => {
                rows.get(index).cloned()
            }
            _ => None,
        }
    }

    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        let mut memo = ProjectionMemo::default();
        let plain = ids.len() >= 64 && self.accessibility_tree_is_plain();
        let mut projected = Vec::with_capacity(ids.len());
        for &id in ids {
            projected.extend(self.project_accessibility_entries(id, plain, &mut memo));
        }
        projected
    }
}

impl UiWorld {
    /// Project visible semantics and their neutral structural ancestors from
    /// the same retained authority, preserving a connected accessibility tree.
    pub fn project_accessibility(&self, document: DocumentId) -> Vec<AccessibilityNode> {
        self.project_accessibility_nodes(&self.document_order(document))
    }
}
