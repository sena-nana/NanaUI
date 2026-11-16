//! Projecting one retained node, and the virtual nodes it holds, into
//! accessibility nodes.

use super::*;

impl UiWorld {
    /// Whether a node or one of its ancestors is explicitly hidden from the
    /// accessibility tree. The retained hierarchy is still used for paint,
    /// layout, and input; this is only the semantic projection boundary.
    fn accessibility_hidden(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        // As in `axis_aligned`, only the boxes above are memoized.
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if self.nodes.accessibility(id).hidden || self.inline_box_omitted(id) {
            return true;
        }
        node.hierarchy
            .parent
            .is_some_and(|parent| self.ancestor_hidden(parent, memo))
    }

    /// [`Self::accessibility_hidden`], memoized.
    pub(super) fn ancestor_hidden(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        if let Some(hidden) = memo.hidden.get(&id) {
            return *hidden;
        }
        let Some(node) = self.nodes.get(id) else {
            memo.hidden.insert(id, false);
            return false;
        };
        if self.nodes.accessibility(id).hidden || self.inline_box_omitted(id) {
            memo.hidden.insert(id, true);
            return true;
        }
        let parent = node.hierarchy.parent;
        let hidden = parent.is_some_and(|parent| self.ancestor_hidden(parent, memo));
        memo.hidden.insert(id, hidden);
        hidden
    }

    /// What `id` says to assistive technology as the name of a node it
    /// labels: its own label, or its text.
    fn accessible_text(&self, id: StableNodeId) -> Option<Arc<str>> {
        let node = self.nodes.get(id)?;
        self.nodes
            .accessibility(id)
            .label
            .clone()
            .filter(|label| !label.is_empty())
            .or_else(|| {
                node.has_text
                    .then(|| self.inline_text(id, true).unwrap_or_default().into())
            })
    }

    /// Every retained node is a mounted, visible, untransformed box. Viewport
    /// bounds are the layout box, and every structural child stays in the
    /// accessibility tree. One scan, then the per-node walk does not re-check
    /// ancestors. A single scrolled or transformed node keeps the normal path.
    pub(super) fn accessibility_tree_is_plain(&self) -> bool {
        if !self.layout_length_tracks.is_empty()
            || !self.presentation.is_empty()
            || self.z_index_nodes != 0
            || self.nodes.has_widget_visuals()
            || self.modal_frames != 0
            || self.menu_surfaces != 0
            || !self.select_menus.is_empty()
            || !self.triggered_overlays.is_empty()
            || !self.overlay_host_nodes.is_empty()
            || self.clip_visuals != 0
            || self.nodes.has_inline_content()
            || !self.detached_mounted.is_empty()
            || self.nodes.has_scroll_offsets()
            || self.nodes.any_hidden_accessibility()
        {
            return false;
        }
        self.nodes.records().all(|node| {
            node.mount == crate::MountState::Mounted
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
                    self.nodes.accessibility(id).into_owned()
                } else {
                    AccessibilityState::default()
                },
                if node.resolved.0.visible {
                    self.nodes.text_content(id).value.clone()
                } else {
                    crate::TextValue::default()
                },
                node.document,
                node.resolved.0.visible,
                node.resolved.0.box_visible,
                node.resolved.0.writing_context(),
                node.style.layout.overflow_x.scrolls() || node.style.layout.overflow_y.scrolls(),
                self.record_layout_box(node),
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
        let secure = self.text_input_is_secure(id);
        // Scroll positions are part of the semantic node, rather than a
        // second visual-only state. Publish an axis only when the retained
        // geometry says it can actually move; this keeps ordinary containers
        // from advertising meaningless scroll actions.
        let scroll_metrics = (visible
            && (scrolls || self.nodes.has_widget_visuals() || self.modal_frames != 0))
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
        } else {
            self.accessible_viewport_bounds(id, plain, memo)?
        };
        let mut children = if plain {
            children.as_ref().clone()
        } else {
            let mut kept = Vec::with_capacity(children.len());
            if !children.is_empty() {
                let chain = self.child_chain(id, memo);
                let mut handed = self.handed_transform(id, chain.hands_offsets, memo);
                // The map and origin a child placed by its offset stands
                // under, read once for every child: the per-child test is
                // then its record and two corners
                // ([`HandedMap::finite_under`]).
                let offsets_free = chain.hands_offsets && self.triggered_overlays.is_empty();
                let mut map = handed.filter(|_| offsets_free).and_then(HandedMap::of);
                kept.extend(children.iter().copied().filter(|&child| {
                    let Some(node) = self.nodes.get(child) else {
                        return false;
                    };
                    let kept = node.resolved.0.box_visible
                        && (node.resolved.0.visible || !node.hierarchy.children.is_empty())
                        && !matches!(node.kind.as_ref(), NodeKind::Comment)
                        && !(chain.hidden
                            || self.nodes.accessibility(child).hidden
                            || self.inline_box_omitted(child))
                        && (self.axis_aligned_under(child, node, chain.aligned, memo) || {
                            match map.as_ref().and_then(|map| map.finite_under(node, id)) {
                                Some(finite) => {
                                    #[cfg(any(test, feature = "layout-verify"))]
                                    assert_eq!(
                                        finite,
                                        self.has_accessibility_bounds_under(
                                            child,
                                            node,
                                            id,
                                            chain.hands_offsets,
                                            handed,
                                            memo,
                                        ),
                                        "{child:?}: its corners under its parent's map"
                                    );
                                    finite
                                }
                                None => {
                                    // A child off the offset path memoizes
                                    // the transform on its way: the next
                                    // one uses it.
                                    if handed.is_none() {
                                        handed =
                                            self.handed_transform(id, chain.hands_offsets, memo);
                                        map =
                                            handed.filter(|_| offsets_free).and_then(HandedMap::of);
                                    }
                                    self.has_accessibility_bounds_under(
                                        child,
                                        node,
                                        id,
                                        chain.hands_offsets,
                                        handed,
                                        memo,
                                    )
                                }
                            }
                        });
                    #[cfg(any(test, feature = "layout-verify"))]
                    assert_eq!(
                        kept,
                        node.resolved.0.box_visible
                            && (node.resolved.0.visible || !node.hierarchy.children.is_empty())
                            && !matches!(node.kind.as_ref(), NodeKind::Comment)
                            && !self.accessibility_hidden(child, memo)
                            && (self.axis_aligned(child, memo)
                                || self.has_accessibility_bounds(child, id, memo)),
                        "{child:?}: kept by its parent's chain, not by its own"
                    );
                    kept
                }));
            }
            kept
        };
        if self.menu_surfaces != 0
            && let Some(menu) = self.widget_visual_as::<crate::MenuSurfaceVisual>(id)
            && menu.kind == crate::MenuSurfaceKind::ContextMenu
            && menu.open
        {
            children.extend(
                crate::menu_row_slots(&menu.rows)
                    .into_iter()
                    .filter_map(|slot| crate::virtual_menu_item_id(id, slot)),
            );
        }
        if self.nodes.has_widget_visuals()
            && let Some(tree) = self.widget_visual_as::<crate::TreeViewVisual>(id)
        {
            children.extend(
                (0..tree.rows.len())
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
            expanded: state.expanded,
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
            transform: None,
            scroll: [0.0, 0.0],
            clip: if plain {
                None
            } else {
                self.accessibility_clip(id, memo)
            },
            writing,
        })
    }
}

impl UiWorld {
    /// `id` and the virtual nodes it holds, their bounds relative to their
    /// parents' ([`AccessibilityNode::bounds`]) when `relative`, in viewport
    /// coordinates otherwise.
    pub(super) fn project_accessibility_entries(
        &self,
        id: StableNodeId,
        plain: bool,
        relative: bool,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let mut pages = Vec::new();
        let mut entries = self.project_viewport_entries(id, plain, &mut pages, memo);
        if relative && !entries.is_empty() {
            self.make_bounds_relative(&mut entries, &pages, plain, memo);
        }
        entries
    }

    /// `id`'s projection, and the virtual nodes it holds after it, all with
    /// their bounds in the viewport; `pages` takes the page boxes of the
    /// virtual ones, in order.
    fn project_viewport_entries(
        &self,
        id: StableNodeId,
        plain: bool,
        pages: &mut Vec<LayoutBox>,
        memo: &mut ProjectionMemo,
    ) -> Vec<AccessibilityNode> {
        let Some(mut node) = self.project_accessibility_node(id, plain, memo) else {
            return Vec::new();
        };
        if self.nodes.has_widget_visuals()
            && self.widget_visual_as::<crate::SelectVisual>(id).is_some()
        {
            let menu = self.project_select_menu(&mut node, pages, memo);
            let mut entries = vec![node];
            entries.extend(menu);
            return entries;
        }
        if self.nodes.has_widget_visuals()
            && self.widget_visual_as::<crate::TreeViewVisual>(id).is_some()
        {
            let rows = self.project_virtual_tree_items(&node, pages, memo);
            let mut entries = vec![node];
            entries.extend(rows);
            return entries;
        }
        let open_context_menu = self.menu_surfaces != 0
            && self
                .widget_visual_as::<crate::MenuSurfaceVisual>(id)
                .is_some_and(|menu| menu.kind == crate::MenuSurfaceKind::ContextMenu && menu.open);
        if !open_context_menu {
            return vec![node];
        }
        let items = self.project_virtual_menu_items(&node, pages, memo);
        let mut entries = vec![node];
        entries.extend(items);
        entries
    }
}
