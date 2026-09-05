//! Accessible projection from retained nodes.

use super::*;
use nana_ui_core::PaintMat4;

/// One snapshot per projection batch. The retained hit index already owns
/// cumulative transforms, including ancestor scrolling and CSS perspective.
/// Walk each involved document once instead of searching its tree per node.
struct AccessibilityTransforms(HashMap<StableNodeId, PaintMat4>);

impl AccessibilityTransforms {
    fn new(world: &UiWorld, documents: impl IntoIterator<Item = DocumentId>) -> Self {
        let mut transforms = HashMap::new();
        let mut visited = HashSet::new();
        for document in documents {
            if !visited.insert(document) {
                continue;
            }
            let Some(forest) = world.hit_test_index.get(&document) else {
                continue;
            };
            let mut pending = forest.iter().collect::<Vec<_>>();
            while let Some(entry) = pending.pop() {
                let [a, b, c, d, e, f] = entry.transform;
                let [g, h] = entry.persp;
                transforms.insert(
                    entry.id,
                    PaintMat4 {
                        m: [
                            a, b, 0.0, g, c, d, 0.0, h, 0.0, 0.0, 1.0, 0.0, e, f, 0.0, 1.0,
                        ],
                    },
                );
                pending.extend(&entry.children);
            }
        }
        Self(transforms)
    }

    fn bounds(&self, id: StableNodeId, bounds: LayoutBox) -> Option<LayoutBox> {
        let corners = self
            .0
            .get(&id)
            .copied()
            .unwrap_or(PaintMat4::IDENTITY)
            .projected_corners(bounds.x, bounds.y, bounds.width, bounds.height)?;
        let x = corners.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let y = corners.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let right = corners
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = corners
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max);
        Some(LayoutBox {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

impl UiWorld {
    fn visible_accessibility_bounds(
        &self,
        id: StableNodeId,
        transforms: &AccessibilityTransforms,
    ) -> Option<LayoutBox> {
        let local = match self.component_geometry(id) {
            Some(crate::ComponentGeometry::ModalFrame { surface, .. }) => surface,
            _ => self.nodes.get(id)?.layout,
        };
        let mut bounds = transforms.bounds(id, local)?;
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
                    transforms.bounds(ancestor, self.nodes.get(ancestor)?.layout)?,
                )?;
            }
            if let Some(crate::ComponentGeometry::ModalFrame { surface, body, .. }) =
                self.component_geometry(ancestor)
            {
                bounds = intersect_layout_boxes(bounds, transforms.bounds(ancestor, surface)?)?;
                if let Some(StandardVisual::ModalFrame { slots, .. }) = self.nodes.visual(ancestor)
                    && slots
                        .body
                        .is_some_and(|body_root| self.is_descendant_or_self(id, body_root))
                {
                    bounds = intersect_layout_boxes(bounds, transforms.bounds(ancestor, body)?)?;
                }
            }
            parent = self.nodes.get(ancestor)?.hierarchy.parent;
        }
        Some(bounds)
    }
}

#[cfg(test)]
#[path = "accessibility_viewport_tests.rs"]
mod viewport_tests;

impl UiWorld {
    fn project_accessibility_node(
        &self,
        id: StableNodeId,
        transforms: &AccessibilityTransforms,
    ) -> Option<AccessibilityNode> {
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
        let bounds = self.visible_accessibility_bounds(id, transforms)?;
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
                    }) && self
                        .visible_accessibility_bounds(child_id, transforms)
                        .is_some()
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
        // Scrolling and transforms leave LayoutBox unchanged and may carry no
        // ACCESSIBILITY dirty bit. Their hit-test subtrees nevertheless moved
        // in viewport space, so the native accessibility cache must see them.
        let mut affected = work.accessibility.iter().copied().collect::<BTreeSet<_>>();
        let mut pending = work
            .input_hit_test
            .iter()
            .chain(&work.transform)
            .chain(&work.layout)
            .copied()
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            affected.insert(id);
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.hierarchy.children.iter().copied());
            }
        }
        let affected = affected.into_iter().collect::<Vec<_>>();
        let mut removed = work
            .accessibility_removals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let transforms = AccessibilityTransforms::new(
            self,
            affected
                .iter()
                .filter_map(|id| self.nodes.get(*id).map(|node| node.document)),
        );
        let mut updated = Vec::new();
        for id in affected {
            if let Some(node) = self.project_accessibility_node(id, &transforms) {
                updated.push(node);
            } else if self.nodes.contains(id) {
                removed.insert(id);
            }
        }
        AccessibilityDelta {
            generation: work.generation,
            updated,
            removed: removed.into_iter().collect(),
        }
    }
}

impl UiWorld {
    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        let transforms = AccessibilityTransforms::new(
            self,
            ids.iter()
                .filter_map(|id| self.nodes.get(*id).map(|node| node.document)),
        );
        ids.iter()
            .filter_map(|&id| self.project_accessibility_node(id, &transforms))
            .collect()
    }
}

impl UiWorld {
    /// Project the visible accessibility tree from the same retained authority.
    pub fn project_accessibility(&self, document: DocumentId) -> Vec<AccessibilityNode> {
        let transforms = AccessibilityTransforms::new(self, [document]);
        self.document_order(document)
            .into_iter()
            .filter_map(|id| self.project_accessibility_node(id, &transforms))
            .collect()
    }
}
