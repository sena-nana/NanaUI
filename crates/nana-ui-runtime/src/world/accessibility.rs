//! Accessible projection from retained nodes.

use super::*;
use crate::node_space::NodeSpace;

mod bounds;
mod delta;
mod frames;
mod pins;
mod project;
mod space;
#[cfg(any(test, feature = "layout-verify"))]
mod verify;
mod virtual_nodes;

use space::*;
#[cfg(any(test, feature = "layout-verify"))]
pub(crate) use verify::ShippedAccessibility;

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
    /// Nodes whose chain hands their children offsets
    /// ([`NodeSpace::hands_offsets`]).
    offsets: hashbrown::HashMap<StableNodeId, bool>,
    /// What each overflow container of a clip chain leaves its descendants,
    /// with the containers above it, in viewport coordinates (#201).
    clips: hashbrown::HashMap<StableNodeId, LayoutBox>,
    /// How each node's accessible space stands ([`AccessibleSpace`]).
    spaces: hashbrown::HashMap<StableNodeId, AccessibleSpace>,
    chain: Vec<StableNodeId>,
    space_chain: Vec<StableNodeId>,
    /// Deep nodes whose record and every one above hand their children
    /// offsets, motion and clips aside ([`UiWorld::records_hand_offsets`]).
    records_offsets: hashbrown::HashMap<StableNodeId, bool>,
    /// Deep nodes whose record and every one above hand their children
    /// offsets and stand upright ([`UiWorld::records_upright`]).
    records_upright: hashbrown::HashMap<StableNodeId, bool>,
}

impl ProjectionMemo {
    /// Room for `nodes` more projected nodes in the tables each one fills.
    fn reserve(&mut self, nodes: usize) {
        self.transforms.reserve(nodes);
        self.bounds.reserve(nodes);
        self.hidden.reserve(nodes);
        self.aligned.reserve(nodes);
        self.offsets.reserve(nodes);
    }
}

impl UiWorld {
    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        let projected = self.project_nodes(ids, true);
        #[cfg(any(test, feature = "layout-verify"))]
        self.overlays_a11y
            .accessibility_shipped
            .record(&projected, &[]);
        projected
    }

    fn project_nodes(&self, ids: &[StableNodeId], relative: bool) -> Vec<AccessibilityNode> {
        let _origins = self.remember_page_origins();
        let mut memo = ProjectionMemo::default();
        let plain = ids.len() >= 64 && self.accessibility_tree_is_plain();
        let mut projected = Vec::with_capacity(ids.len());
        for &id in ids {
            projected.extend(self.project_accessibility_entries(id, plain, relative, &mut memo));
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

    /// [`Self::project_accessibility`] with every node's `bounds` where it
    /// stands in the viewport rather than relative to its parent: for
    /// automation and tests that find a node on screen. A platform tree is
    /// built from [`Self::project_accessibility`] and its deltas, never
    /// from this.
    pub fn project_accessibility_in_viewport(
        &self,
        document: DocumentId,
    ) -> Vec<AccessibilityNode> {
        self.project_nodes(&self.document_order(document), false)
    }

    /// [`Self::project_accessibility_nodes`] with viewport bounds, as
    /// [`Self::project_accessibility_in_viewport`].
    pub fn project_accessibility_nodes_in_viewport(
        &self,
        ids: &[StableNodeId],
    ) -> Vec<AccessibilityNode> {
        self.project_nodes(ids, false)
    }
}

#[cfg(test)]
mod viewport_tests;

#[cfg(test)]
mod relative_tests;
