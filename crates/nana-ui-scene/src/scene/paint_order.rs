//! Paint order: group prefixes, order keys and the layers that sort above
//! the document.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SceneOrderKey {
    /// `(z_index, document_order)` for each isolating stacking group from
    /// outermost to innermost in visual parent order, then this primitive's
    /// visual node. Opacity groups, `isolation: isolate`, modals, positioned +
    /// `z-index`, and floating overlays keep their subtrees contiguous, while
    /// overlays mount into their resolved `VisualParent` (`Root`, `TopLayer`,
    /// `Modal`, `Node`, or `Overlay`).
    pub(super) stack: GroupPrefix,
    /// Collection identity must not lift scrolling text above sticky bands,
    /// minimaps or popup surfaces owned by the same component.
    paint_layer: u64,
    pub(super) slot: u64,
    pub(super) node: StableNodeId,
}

fn is_stacking_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    // A painted node keeps its children between its two paint phases, so it
    // has to own their order the way a stacking context does. Modal frames
    // establish a stacking boundary for overlays mounted with
    // `OverlayMount::NearestModal`.
    node.custom_paint.is_some()
        || is_opacity_group(nodes, node)
        || matches!(
            node.standard_visual,
            Some(StandardVisual::ModalFrame { .. })
        )
        || matches!(
            node.component_geometry.as_deref(),
            Some(ComponentGeometry::ModalFrame { .. })
        )
        || (has_extracted_child(nodes, node)
            && (node.source_style.layout.creates_paint_stacking_context()
                // Its bars paint over everything it holds: see
                // `SCROLLBAR_CHROME`. Whether they show right now does not
                // matter, so revealing them never re-sorts the subtree.
                || has_scrollbar_chrome(node)))
}

fn is_visual_stacking_group(nodes: &SceneNodes, vn: VisualNode) -> bool {
    match vn {
        VisualNode::Overlay(_) => true,
        VisualNode::Node(id) => nodes
            .get(&id)
            .is_some_and(|node| is_stacking_group(nodes, node)),
    }
}

fn visual_node_z_and_stops(nodes: &SceneNodes, vn: VisualNode) -> (i32, bool) {
    match vn {
        VisualNode::Overlay(id) => {
            let Some(node) = nodes.get(&id) else {
                return (0, true);
            };
            let base_z = if matches!(node.standard_visual, Some(StandardVisual::Select { .. })) {
                node.z_index.max(nana_ui_runtime::MENU_OVERLAY_Z_INDEX)
            } else {
                nana_ui_runtime::MENU_OVERLAY_Z_INDEX
            };
            match effective_overlay_visual_parent(nodes, node) {
                nana_ui_runtime::VisualParent::Root => (base_z, true),
                nana_ui_runtime::VisualParent::TopLayer => {
                    (base_z.max(nana_ui_runtime::TOP_LAYER_Z_INDEX), true)
                }
                nana_ui_runtime::VisualParent::Modal(modal_id) => {
                    let modal_z = nodes.get(&modal_id).map_or(0, |modal| modal.z_index);
                    (modal_z.saturating_add(base_z), false)
                }
                nana_ui_runtime::VisualParent::Node(_)
                | nana_ui_runtime::VisualParent::Overlay(_) => (base_z, false),
            }
        }
        VisualNode::Node(id) => {
            let Some(node) = nodes.get(&id) else {
                return (0, true);
            };
            match effective_visual_parent(nodes, node) {
                Some(nana_ui_runtime::VisualParent::Root) => (
                    node.z_index.max(nana_ui_runtime::MENU_OVERLAY_Z_INDEX),
                    true,
                ),
                Some(nana_ui_runtime::VisualParent::TopLayer) => {
                    (node.z_index.max(nana_ui_runtime::TOP_LAYER_Z_INDEX), true)
                }
                Some(nana_ui_runtime::VisualParent::Modal(modal_id)) => {
                    let modal_z = nodes.get(&modal_id).map_or(0, |modal| modal.z_index);
                    let base_z = node.z_index.max(nana_ui_runtime::MENU_OVERLAY_Z_INDEX);
                    (modal_z.saturating_add(base_z), false)
                }
                Some(
                    nana_ui_runtime::VisualParent::Node(_)
                    | nana_ui_runtime::VisualParent::Overlay(_),
                ) => (node.z_index, false),
                None => (
                    node.z_index,
                    node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed,
                ),
            }
        }
    }
}

/// One `(z-index, document order)` per stacking group above a visual node,
/// outermost first.
pub(super) type GroupPrefix = Arc<[(i32, usize)]>;

/// `(z_index, document_order)` of every isolating stacking group above (and
/// including) `vn`, outermost first.
pub(super) fn group_prefix(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    vn: VisualNode,
) -> Vec<(i32, usize)> {
    let mut stack = Vec::new();
    let mut cursor = Some(vn);
    let mut depth = 0usize;
    while let Some(cur) = cursor {
        if depth >= MAX_ANCESTOR_DEPTH || !nodes.contains_key(&cur.node_id()) {
            break;
        }
        depth += 1;
        let (z_index, stops) = visual_node_z_and_stops(nodes, cur);
        if is_visual_stacking_group(nodes, cur) {
            let order = node_order.get(&cur.node_id()).copied().unwrap_or(0);
            stack.push((z_index, order));
        }
        if stops {
            break;
        }
        cursor = visual_parent_of(nodes, cur);
    }
    stack.reverse();
    stack
}

/// [`group_prefix`], remembered per visual node in `memo`.
pub(super) fn group_prefix_memo(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    memo: &mut HashMap<VisualNode, GroupPrefix>,
    vn: VisualNode,
) -> GroupPrefix {
    if let Some(held) = memo.get(&vn) {
        return Arc::clone(held);
    }
    if nodes.contains_key(&vn.node_id()) {
        let (z_index, stops) = visual_node_z_and_stops(nodes, vn);
        if !stops
            && let Some(parent_vn) = visual_parent_of(nodes, vn)
            && let Some(inherited) = memo.get(&parent_vn)
        {
            let prefix = if is_visual_stacking_group(nodes, vn) {
                let entry = (z_index, node_order.get(&vn.node_id()).copied().unwrap_or(0));
                extended_prefix(inherited, entry, None)
            } else {
                Arc::clone(inherited)
            };
            memo.insert(vn, Arc::clone(&prefix));
            return prefix;
        }
    }
    let mut chain = Vec::new();
    let mut cursor = Some(vn);
    let mut depth = 0usize;
    let mut prefix: GroupPrefix = loop {
        let Some(cur) = cursor else {
            break Arc::from(Vec::new());
        };
        if depth >= MAX_ANCESTOR_DEPTH {
            break Arc::from(Vec::new());
        }
        depth += 1;
        if let Some(held) = memo.get(&cur) {
            break Arc::clone(held);
        }
        if !nodes.contains_key(&cur.node_id()) {
            break Arc::from(Vec::new());
        }
        let (z_index, stops) = visual_node_z_and_stops(nodes, cur);
        let stacking = is_visual_stacking_group(nodes, cur);
        chain.push((cur, stacking, z_index));
        if stops {
            break Arc::from(Vec::new());
        }
        cursor = visual_parent_of(nodes, cur);
    };
    for (cur, stacking, z_index) in chain.into_iter().rev() {
        if stacking {
            let entry = (
                z_index,
                node_order.get(&cur.node_id()).copied().unwrap_or(0),
            );
            prefix = extended_prefix(&prefix, entry, None);
        }
        memo.insert(cur, Arc::clone(&prefix));
    }
    prefix
}

pub(super) fn has_custom_paint(nodes: &SceneNodes, node: StableNodeId) -> bool {
    nodes
        .get(&node)
        .is_some_and(|node| node.custom_paint.is_some())
}

pub(super) fn primitive_paint_layer(slot: u64) -> u64 {
    match slot >> 32 {
        // Behind the glyphs (layer 2), above a custom-rendered backdrop
        // (layer 1, broken by the much larger slot).
        namespace if namespace == u64::from(DOCUMENT_TEXT_SELECTION) => 1,
        namespace if namespace == u64::from(TEXT_LINE_LABELS) => 40,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_MARKERS) => 20,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_LABELS) => 58,
        namespace if namespace == u64::from(TEXT_ATOM_ICONS) => 27,
        namespace if namespace == u64::from(TEXT_ATOM_LABELS) => 32,
        // A dialog's own card, painted by its surface painter: the card
        // quad's layer, over the scrim (10), under the title (12).
        namespace if namespace == u64::from(custom_paint::MODAL_SURFACE_PAINT) => 11,
        _ => slot,
    }
}

pub(super) fn order_key(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    primitive: &ScenePrimitive,
) -> SceneOrderKey {
    let vn = primitive_visual_node(nodes, primitive);
    let prefix: GroupPrefix = group_prefix(nodes, node_order, vn).into();
    SceneOrderKey::at(order_stack(nodes, &prefix, vn, primitive), primitive)
}

/// The stacking entries a primitive paints under, outermost first.
pub(super) fn order_stack(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    vn: VisualNode,
    primitive: &ScenePrimitive,
) -> GroupPrefix {
    order_stack_reusing(nodes, prefix, vn, primitive, None)
}

/// [`order_stack`], handing back `retained` itself when it already holds
/// the same entries: a rebuild usually keys a primitive where it was.
fn order_stack_reusing(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    vn: VisualNode,
    primitive: &ScenePrimitive,
    retained: Option<&GroupPrefix>,
) -> GroupPrefix {
    // A group's own paint is the prefix, before its descendants. Repeating its
    // outer z-index here incorrectly puts lower-z children behind its backplate.
    if is_visual_stacking_group(nodes, vn) {
        if matches!(vn, VisualNode::Overlay(_)) {
            return Arc::clone(prefix);
        }
        if let Some(node) = nodes.get(&vn.node_id()) {
            // A scroll view's bars sit past every child of its group.
            if is_scrollbar_chrome_slot(primitive.id.slot) {
                return extended_prefix(prefix, (i32::MAX, usize::MAX), retained);
            }
            // A painted node splits its own paint around its children.
            if let Some(entry) = node.custom_paint.as_deref().and_then(|recording| {
                custom_paint::custom_paint_stack(recording, primitive.id.slot)
            }) {
                return extended_prefix(prefix, entry, retained);
            }
        }
        return Arc::clone(prefix);
    }
    let z_index = if let Some(node) = nodes.get(&vn.node_id())
        && effective_visual_parent(nodes, node).is_some()
    {
        visual_node_z_and_stops(nodes, vn).0
    } else {
        primitive.z_index
    };
    extended_prefix(prefix, (z_index, primitive.document_order), retained)
}

/// `prefix` followed by `entry`: `retained` when it holds exactly that,
/// else a new list built in one allocation.
fn extended_prefix(
    prefix: &[(i32, usize)],
    entry: (i32, usize),
    retained: Option<&GroupPrefix>,
) -> GroupPrefix {
    if let Some(retained) = retained
        && let Some((last, head)) = retained.split_last()
        && *last == entry
        && head == prefix
    {
        return Arc::clone(retained);
    }
    prefix
        .iter()
        .copied()
        .chain(std::iter::once(entry))
        .collect()
}

impl SceneOrderKey {
    pub(super) fn at(stack: GroupPrefix, primitive: &ScenePrimitive) -> Self {
        Self {
            stack,
            paint_layer: primitive_paint_layer(primitive.id.slot),
            slot: primitive.id.slot,
            node: primitive.node,
        }
    }
}

impl UiScene {
    /// The paint-order key.
    ///
    /// Every primitive of a node sits under the same stack — the z-index and
    /// document order in it are the node's — so while the rebuild pass runs it
    /// is computed once per node and every primitive shares that `Arc`.
    pub(super) fn scene_order_key(
        &self,
        primitive: &ScenePrimitive,
        retained: Option<&GroupPrefix>,
    ) -> SceneOrderKey {
        let vn = primitive_visual_node(&self.nodes, primitive);
        if let Ok(mut scratch) = self.rebuild_scratch.lock()
            && scratch.active
        {
            if let Some((node, stack)) = scratch.order_stack.as_ref()
                && *node == primitive.node
            {
                return SceneOrderKey::at(Arc::clone(stack), primitive);
            }
            let prefix = self.group_prefix_of(&mut scratch, vn);
            let stack = order_stack_reusing(&self.nodes, &prefix, vn, primitive, retained);
            // A painted node's primitives do not share one stack, nor do a
            // scroll view's own paint and its bars, or a menu/select with
            // separate trigger and overlay branches.
            if !self.splits_paint(primitive.node) {
                scratch.order_stack = Some((primitive.node, Arc::clone(&stack)));
            }
            return SceneOrderKey::at(stack, primitive);
        }
        order_key(&self.nodes, &self.node_order, primitive)
    }

    /// Whether `node`'s primitives sort under more than one stack: a painted
    /// node splits its paint around its children, a scroll view puts its
    /// bars over them, and a triggered menu or open select splits trigger from
    /// floating overlay.
    pub(super) fn splits_paint(&self, node: StableNodeId) -> bool {
        (!self.custom_paint.is_empty() && has_custom_paint(&self.nodes, node))
            || self.nodes.get(&node).is_some_and(|node| {
                has_scrollbar_chrome(node)
                    || is_triggered_menu(node)
                    || matches!(node.standard_visual, Some(StandardVisual::Select { .. }))
            })
    }

    /// A visual node's paint-order prefix: what it inherits from the chain above
    /// it, plus its own entry when it opens a stacking group.
    pub(super) fn group_prefix_of(
        &self,
        scratch: &mut RebuildScratch,
        vn: VisualNode,
    ) -> GroupPrefix {
        group_prefix_memo(
            &self.nodes,
            &self.node_order,
            &mut scratch.group_prefixes,
            vn,
        )
    }
}
