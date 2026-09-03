//! Accessible projection from retained nodes.

use super::*;

impl UiWorld {
    pub(super) fn visible_accessibility_bounds(&self, id: StableNodeId) -> Option<LayoutBox> {
        let mut bounds = self.nodes.get(id)?.layout;
        if self.clip_visuals == 0 {
            return Some(bounds);
        }
        let mut parent = self.nodes.get(id)?.hierarchy.parent;
        while let Some(ancestor) = parent {
            if matches!(
                self.nodes.visual(ancestor),
                Some(StandardVisual::EmptyState { .. })
            ) {
                bounds = intersect_layout_boxes(bounds, self.nodes.get(ancestor)?.layout)?;
            }
            if let Some(crate::ComponentGeometry::ModalFrame { surface, body, .. }) =
                self.component_geometry(ancestor)
            {
                bounds = intersect_layout_boxes(bounds, surface)?;
                if let Some(StandardVisual::ModalFrame { slots, .. }) = self.nodes.visual(ancestor)
                    && slots
                        .body
                        .is_some_and(|body_root| self.is_descendant_or_self(id, body_root))
                {
                    bounds = intersect_layout_boxes(bounds, body)?;
                }
            }
            parent = self.nodes.get(ancestor)?.hierarchy.parent;
        }
        Some(bounds)
    }
}

impl UiWorld {
    pub(super) fn project_accessibility_node(&self, id: StableNodeId) -> Option<AccessibilityNode> {
        if !self.is_mounted(id) {
            return None;
        }
        let (parent, children, kind, state, text_value, document, visible) = {
            let node = self.nodes.get(id)?;
            (
                node.hierarchy.parent,
                Arc::clone(&node.hierarchy.children),
                Arc::clone(&node.kind),
                node.accessibility.clone(),
                node.text.value.clone(),
                node.document,
                node.resolved.0.visible,
            )
        };
        if !visible {
            return None;
        }
        if matches!(kind.as_ref(), NodeKind::Comment) {
            return None;
        }
        let role = match (state.role, kind.as_ref()) {
            (AccessibilityRole::Generic, NodeKind::Document) => AccessibilityRole::Document,
            (AccessibilityRole::Generic, NodeKind::Text) => AccessibilityRole::Text,
            (role, _) => role,
        };
        let label = state
            .label
            .clone()
            .or_else(|| (!text_value.is_empty()).then(|| Arc::<str>::from(text_value.as_str())));
        let bounds = match self.component_geometry(id) {
            Some(crate::ComponentGeometry::ModalFrame { surface, .. }) => surface,
            _ => self.visible_accessibility_bounds(id)?,
        };
        Some(AccessibilityNode {
            id,
            parent,
            children: children
                .iter()
                .copied()
                .filter(|child| {
                    let child_id = *child;
                    self.nodes.get(child_id).is_some_and(|node| {
                        node.resolved.0.visible && !matches!(node.kind.as_ref(), NodeKind::Comment)
                    }) && self.visible_accessibility_bounds(child_id).is_some()
                })
                .collect(),
            role,
            label,
            description: state.description.clone(),
            value: if matches!(
                self.nodes.visual(id),
                Some(StandardVisual::TextInput { secure: true, .. })
            ) {
                None
            } else {
                self.nodes
                    .text_input(id)
                    .map(|input| Arc::<str>::from(input.value.as_str()))
                    .or_else(|| state.value.clone())
            },
            disabled: state.disabled
                || self
                    .confirm_action_effect(id)
                    .is_some_and(|effect| effect.0),
            checked: state.checked,
            mixed: state.mixed,
            orientation: state.orientation,
            selected: state.selected,
            multiline: state.multiline,
            editable: state.editable,
            selection: self.nodes.text_input(id).map(|input| input.selection),
            modal: state.modal,
            busy: state.busy,
            invalid: state.invalid,
            numeric_minimum: state.numeric_minimum,
            numeric_maximum: state.numeric_maximum,
            numeric_step: state.numeric_step,
            numeric_value: state.numeric_value,
            focused: self.input.focused.get(&document) == Some(&id),
            bounds,
        })
    }
}

impl UiWorld {
    /// Project one complete incremental accessibility transaction, including
    /// tombstones for nodes removed from the retained world.
    pub fn project_accessibility_delta(&self, work: &SystemWork) -> AccessibilityDelta {
        let mut removed = work
            .accessibility_removals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        removed.extend(work.accessibility.iter().copied().filter(|id| {
            self.nodes.contains(*id) && self.project_accessibility_node(*id).is_none()
        }));
        AccessibilityDelta {
            generation: work.generation,
            updated: self.project_accessibility_nodes(&work.accessibility),
            removed: removed.into_iter().collect(),
        }
    }
}

impl UiWorld {
    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        ids.iter()
            .filter_map(|&id| self.project_accessibility_node(id))
            .collect()
    }
}

impl UiWorld {
    /// Project the visible accessibility tree from the same retained authority.
    pub fn project_accessibility(&self, document: DocumentId) -> Vec<AccessibilityNode> {
        self.document_order(document)
            .into_iter()
            .filter_map(|id| self.project_accessibility_node(id))
            .collect()
    }
}
