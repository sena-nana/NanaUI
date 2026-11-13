//! Paint order: group prefixes, order keys and the layers that sort above
//! the document.

use super::*;

/// Whether `primitive` paints in the root stacking context rather than inside
/// the groups around its node: a triggered menu's hanging surface, or an open
/// select menu's surface, rows and scrollbar. Inside those groups, a later
/// sibling or a scroll view's edge fade would paint over or dim the popup.
pub(super) fn paints_at_root(nodes: &SceneNodes, primitive: &ScenePrimitive) -> bool {
    is_triggered_overlay_surface(primitive.id)
        || (is_select_menu_slot(primitive.id.slot)
            && nodes.get(&primitive.node).is_some_and(|node| {
                matches!(node.standard_visual, Some(StandardVisual::Select { .. }))
            }))
}

/// The paint-order key a primitive takes at the root, out of the groups
/// around its node, if it is one of the layers that do.
pub(super) fn root_layer_key(
    nodes: &SceneNodes,
    primitive: &ScenePrimitive,
) -> Option<SceneOrderKey> {
    paints_at_root(nodes, primitive).then(|| {
        SceneOrderKey::at(
            Arc::from([(primitive.z_index, primitive.document_order)]),
            primitive,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SceneOrderKey {
    /// `(z_index, document_order)` for each isolating stacking group from
    /// outermost to innermost, then this primitive's node. Opacity groups,
    /// `isolation: isolate`, and positioned + `z-index` keep a subtree
    /// contiguous against siblings, except `position: fixed` surfaces which
    /// paint in the root stacking context so Popover/menu chrome is not trapped
    /// in a parent card. Not full CSS Appendix E.
    pub(super) stack: GroupPrefix,
    /// Collection identity must not lift scrolling text above sticky bands,
    /// minimaps or popup surfaces owned by the same component.
    paint_layer: u64,
    pub(super) slot: u64,
    pub(super) node: StableNodeId,
}

fn is_stacking_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    // A painted node keeps its children between its two paint phases, so it
    // has to own their order the way a stacking context does.
    node.custom_paint.is_some()
        || is_opacity_group(nodes, node)
        || (has_extracted_child(nodes, node)
            && (node.source_style.layout.creates_paint_stacking_context()
                // Its bars paint over everything it holds: see
                // `SCROLLBAR_CHROME`. Whether they show right now does not
                // matter, so revealing them never re-sorts the subtree.
                || has_scrollbar_chrome(node)))
}

/// One `(z-index, document order)` per stacking group above a node, outermost
/// first.
pub(super) type GroupPrefix = Arc<[(i32, usize)]>;

/// `(z_index, document_order)` of every isolating stacking group above (and
/// including) `node`, outermost first. Opacity / filter / mix-blend dest groups
/// plus `isolation` and positioned + `z-index`.
pub(super) fn group_prefix(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    node: StableNodeId,
) -> Vec<(i32, usize)> {
    let mut stack = Vec::new();
    for (id, candidate) in ancestor_nodes(nodes, node) {
        if is_stacking_group(nodes, candidate) {
            let z_index = candidate.z_index;
            let order = node_order.get(&id).copied().unwrap_or(0);
            stack.push((z_index, order));
        }
        if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
            // Triggered menus and overlay surfaces are viewport-fixed. Their
            // paint order must not stay inside a parent's isolation group, or
            // a later sibling card would cover an open Popover.
            break;
        }
    }
    stack.reverse();
    stack
}

/// [`group_prefix`], remembered per node in `memo`: a node's prefix is its
/// parent's plus its own entry, so a chain of only children pays one step a
/// node rather than one walk a node. The walk up to the nearest remembered
/// prefix is a loop, not recursion.
pub(super) fn group_prefix_memo(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    memo: &mut NodeMap<GroupPrefix>,
    node: StableNodeId,
) -> GroupPrefix {
    if let Some(held) = memo.get(&node) {
        return Arc::clone(held);
    }
    // Most asks are for a node whose parent the pass has already answered,
    // a sibling's parent: one step, with no chain to collect.
    if let Some(candidate) = nodes.get(&node)
        && candidate.source_style.layout.position != nana_ui_core::PositionSpec::Fixed
        && let Some(parent) = candidate.parent
        && let Some(inherited) = memo.get(&parent)
    {
        let prefix = if is_stacking_group(nodes, candidate) {
            let entry = (
                candidate.z_index,
                node_order.get(&node).copied().unwrap_or(0),
            );
            extended_prefix(inherited, entry, None)
        } else {
            Arc::clone(inherited)
        };
        memo.insert(node, Arc::clone(&prefix));
        return prefix;
    }
    // The nodes still to answer, innermost first.
    let mut chain = Vec::new();
    let mut cursor = Some(node);
    let mut prefix: GroupPrefix = loop {
        let Some(id) = cursor else {
            break Arc::from(Vec::new());
        };
        if let Some(held) = memo.get(&id) {
            break Arc::clone(held);
        }
        let Some(candidate) = nodes.get(&id) else {
            break Arc::from(Vec::new());
        };
        chain.push((id, candidate.as_ref()));
        // Triggered menus and overlay surfaces are viewport-fixed and start
        // their own chain; see `group_prefix`.
        if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
            break Arc::from(Vec::new());
        }
        cursor = candidate.parent;
    };
    for (id, candidate) in chain.into_iter().rev() {
        if is_stacking_group(nodes, candidate) {
            let entry = (candidate.z_index, node_order.get(&id).copied().unwrap_or(0));
            prefix = extended_prefix(&prefix, entry, None);
        }
        memo.insert(id, Arc::clone(&prefix));
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
    if let Some(key) = root_layer_key(nodes, primitive) {
        return key;
    }
    let prefix: GroupPrefix = group_prefix(nodes, node_order, primitive.node).into();
    SceneOrderKey::at(order_stack(nodes, &prefix, primitive), primitive)
}

/// The stacking entries a node's primitives paint under, outermost first.
pub(super) fn order_stack(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    primitive: &ScenePrimitive,
) -> GroupPrefix {
    order_stack_reusing(nodes, prefix, primitive, None)
}

/// [`order_stack`], handing back `retained` itself when it already holds
/// the same entries: a rebuild usually keys a primitive where it was.
fn order_stack_reusing(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    primitive: &ScenePrimitive,
    retained: Option<&GroupPrefix>,
) -> GroupPrefix {
    // A group's own paint is the prefix, before its descendants. Repeating its
    // outer z-index here incorrectly puts lower-z children behind its backplate.
    if let Some(node) = nodes.get(&primitive.node)
        && is_stacking_group(nodes, node)
    {
        // A scroll view's bars sit past every child of its group.
        if is_scrollbar_chrome_slot(primitive.id.slot) {
            return extended_prefix(prefix, (i32::MAX, usize::MAX), retained);
        }
        // A painted node splits its own paint around its children.
        if let Some(entry) = node
            .custom_paint
            .as_deref()
            .and_then(|recording| custom_paint::custom_paint_stack(recording, primitive.id.slot))
        {
            return extended_prefix(prefix, entry, retained);
        }
        return Arc::clone(prefix);
    }
    extended_prefix(
        prefix,
        (primitive.z_index, primitive.document_order),
        retained,
    )
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
        if let Some(key) = root_layer_key(&self.nodes, primitive) {
            return key;
        }
        if let Ok(mut scratch) = self.rebuild_scratch.lock()
            && scratch.active
        {
            if let Some((node, stack)) = scratch.order_stack.as_ref()
                && *node == primitive.node
            {
                return SceneOrderKey::at(Arc::clone(stack), primitive);
            }
            let prefix = self.group_prefix_of(&mut scratch, primitive.node);
            let stack = order_stack_reusing(&self.nodes, &prefix, primitive, retained);
            // A painted node's primitives do not share one stack, nor do a
            // scroll view's own paint and its bars.
            if !self.splits_paint(primitive.node) {
                scratch.order_stack = Some((primitive.node, Arc::clone(&stack)));
            }
            return SceneOrderKey::at(stack, primitive);
        }
        order_key(&self.nodes, &self.node_order, primitive)
    }

    /// Whether `node`'s primitives sort under more than one stack: a painted
    /// node splits its paint around its children, and a scroll view puts its
    /// bars over them. A per-node stack cache must not hand one of those
    /// primitives' stack to another.
    pub(super) fn splits_paint(&self, node: StableNodeId) -> bool {
        (!self.custom_paint.is_empty() && has_custom_paint(&self.nodes, node))
            || self
                .nodes
                .get(&node)
                .is_some_and(|node| has_scrollbar_chrome(node))
    }

    /// A node's paint-order prefix: what it inherits from the chain above it,
    /// plus its own entry when it opens a stacking group.
    ///
    /// The inherited half is what siblings share, so it is the half worth
    /// remembering. A node that opens no group of its own **shares** that
    /// `Arc` rather than copying it.
    pub(super) fn group_prefix_of(
        &self,
        scratch: &mut RebuildScratch,
        node: StableNodeId,
    ) -> GroupPrefix {
        group_prefix_memo(
            &self.nodes,
            &self.node_order,
            &mut scratch.group_prefixes,
            node,
        )
    }
}
