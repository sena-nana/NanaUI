//! Primitive identity: the slot namespaces a node's primitives live in, and
//! the predicates that read them.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveId {
    pub node: StableNodeId,
    /// Fixed component slots occupy 0..=255. Unbounded component collections
    /// use a separate namespace and a checked collection index.
    pub slot: u64,
}

pub(super) fn collection_slot(namespace: u32, index: usize) -> u64 {
    debug_assert!(namespace != 0);
    (u64::from(namespace) << 32)
        | u64::from(u32::try_from(index).expect("primitive collection exceeds u32::MAX items"))
}

/// `::selection` fills share the custom-render slot's paint layer so they stay
/// behind the glyphs, but live in their own namespace so the two cannot
/// overwrite each other's key.
pub(super) const DOCUMENT_TEXT_SELECTION: u32 = 6;

pub(super) const TEXT_LINE_LABELS: u32 = 1;

pub(super) const TEXT_DIAGNOSTIC_MARKERS: u32 = 2;

pub(super) const TEXT_DIAGNOSTIC_LABELS: u32 = 3;

pub(super) const TEXT_ATOM_ICONS: u32 = 4;

pub(super) const TEXT_ATOM_LABELS: u32 = 5;

/// Inline objects of a rich text node, one slot per object in text order.
pub(super) const TEXT_INLINE_OBJECTS: u32 = 7;

/// A rich text editor's caret, over its glyphs.
pub(super) const TEXT_EDITOR_CARET: u32 = 8;

/// Rich text span backgrounds share the document selection's paint layer
/// below every selection: one batch per span, indices up to the document
/// selection's.
pub(super) const DOCUMENT_SELECTION_INDEX: usize = 1 << 15;

/// A rich text editor's selection rectangles share the document selection's
/// paint layer (under the glyphs) from this index on.
pub(super) const RICH_EDITOR_SELECTION_BASE: usize = 1 << 16;

/// Rich text tag backgrounds share that paint layer too (under the glyphs,
/// over a selection), from this index on.
pub(crate) const RICH_TAG_BASE: usize = 1 << 17;

/// The surface of an open triggered menu (Popover, ActionMenu, HoverCard).
/// It is the trigger's primitive, but it wraps content Runtime lays out
/// viewport-fixed above the page, so it paints the way that content does: in
/// the root stacking context, untransformed, and cut by none of the trigger's
/// clips. It keeps no projection of the trigger's to rebase on either, so a
/// scroll under the trigger leaves it where its content is.
const TRIGGERED_OVERLAY_SURFACE: u32 = 0xFFFF_FFE0;

pub(crate) const TRIGGERED_OVERLAY_SURFACE_SLOT: u64 = (TRIGGERED_OVERLAY_SURFACE as u64) << 32;

/// The slot namespace a scroll view's bars live in. A scroll view whose bars
/// can show is a stacking group, and these slots sort after everything it
/// holds, whatever its descendants' z-index ([`order_stack`]): content never
/// covers its bars, while the view's later siblings and viewport-fixed
/// surfaces still paint over them. Indices 0 and 1 are the horizontal track
/// and thumb, 2 and 3 the vertical: the vertical bar paints last, as it wins
/// the pointer where the two meet.
pub(super) const SCROLLBAR_CHROME: u32 = 0xFFFF_FFF8;

pub(super) fn is_scrollbar_chrome_slot(slot: u64) -> bool {
    slot >> 32 == SCROLLBAR_CHROME as u64
}

/// A scroll view whose bars can show; it paints them over its descendants.
pub(super) fn has_scrollbar_chrome(node: &ExtractedNode) -> bool {
    node.widget_paint
        .as_deref()
        .is_some_and(|paint| paint.has_scrollbar_chrome)
}

pub(super) fn is_triggered_overlay_surface(id: PrimitiveId) -> bool {
    id.slot == TRIGGERED_OVERLAY_SURFACE_SLOT
}

/// Whether `node` is a triggered menu. Its node is the trigger, and its
/// presentation layers are the hanging surface's: they fade the surface and
/// the items it places, and move them about the surface's pivot
/// ([`UiScene::hanging_transform`]); the trigger and its trigger content keep
/// their logical opacity and transform.
pub(super) fn is_triggered_menu(node: &ExtractedNode) -> bool {
    node.widget_paint
        .as_deref()
        .and_then(|w| w.floating_overlay.as_ref())
        .is_some_and(|overlay| {
            overlay.kind
                == nana_ui_runtime::scene_contract::WidgetFloatingOverlayKind::TriggeredSurface
        })
}

/// Whether `child` draws the trigger of the triggered menu `menu`.
pub(super) fn is_trigger_content(menu: &ExtractedNode, child: StableNodeId) -> bool {
    menu.widget_paint
        .as_deref()
        .and_then(|w| w.floating_overlay.as_ref())
        .is_some_and(|overlay| {
            overlay.kind
                == nana_ui_runtime::scene_contract::WidgetFloatingOverlayKind::TriggeredSurface
                && overlay.trigger_content == Some(child)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VisualNode {
    Node(StableNodeId),
    Overlay(StableNodeId),
}

impl VisualNode {
    pub(super) const fn node_id(self) -> StableNodeId {
        match self {
            Self::Node(id) | Self::Overlay(id) => id,
        }
    }
}

pub(super) fn primitive_visual_node(nodes: &SceneNodes, primitive: &ScenePrimitive) -> VisualNode {
    let is_overlay = nodes.get(&primitive.node).is_some_and(|node| {
        if let Some(paint) = node.widget_paint.as_deref()
            && nana_ui_runtime::scene_contract::is_floating_overlay_slot(paint, primitive.id.slot)
        {
            return true;
        }
        is_triggered_overlay_surface(primitive.id)
    }) || is_triggered_overlay_surface(primitive.id);
    if is_overlay {
        VisualNode::Overlay(primitive.node)
    } else {
        VisualNode::Node(primitive.node)
    }
}

fn is_modal_frame_node(node: &ExtractedNode) -> bool {
    node.widget_paint
        .as_deref()
        .is_some_and(|w| w.modal_frame.is_some())
}

fn resolve_scene_overlay_mount(
    nodes: &SceneNodes,
    node: &ExtractedNode,
    mount: nana_ui_core::OverlayMount,
) -> nana_ui_runtime::scene_contract::VisualParent {
    match mount {
        nana_ui_core::OverlayMount::TopLayer => {
            nana_ui_runtime::scene_contract::VisualParent::TopLayer
        }
        nana_ui_core::OverlayMount::NearestModal => {
            let mut cursor = node.parent;
            let mut depth = 0usize;
            while let Some(ancestor_id) = cursor {
                if depth >= MAX_ANCESTOR_DEPTH {
                    break;
                }
                depth += 1;
                let Some(ancestor) = nodes.get(&ancestor_id) else {
                    break;
                };
                if is_modal_frame_node(ancestor) {
                    return nana_ui_runtime::scene_contract::VisualParent::Modal(ancestor_id);
                }
                cursor = ancestor.parent;
            }
            nana_ui_runtime::scene_contract::VisualParent::Root
        }
        nana_ui_core::OverlayMount::Inline => {
            let Some(parent_id) = node.parent else {
                return nana_ui_runtime::scene_contract::VisualParent::Root;
            };
            let Some(parent) = nodes.get(&parent_id) else {
                return nana_ui_runtime::scene_contract::VisualParent::Root;
            };
            if is_triggered_menu(parent) && !is_trigger_content(parent, node.id) {
                nana_ui_runtime::scene_contract::VisualParent::Overlay(parent_id)
            } else {
                nana_ui_runtime::scene_contract::VisualParent::Node(parent_id)
            }
        }
    }
}

pub(super) fn effective_visual_parent(
    nodes: &SceneNodes,
    node: &ExtractedNode,
) -> Option<nana_ui_runtime::scene_contract::VisualParent> {
    node.visual_parent.or_else(|| {
        let parent_id = node.parent?;
        let parent = nodes.get(&parent_id)?;
        (is_triggered_menu(parent) && !is_trigger_content(parent, node.id)).then_some(
            nana_ui_runtime::scene_contract::VisualParent::Overlay(parent_id),
        )
    })
}

pub(super) fn effective_overlay_visual_parent(
    nodes: &SceneNodes,
    node: &ExtractedNode,
) -> nana_ui_runtime::scene_contract::VisualParent {
    node.overlay_visual_parent.unwrap_or_else(|| {
        if let Some(overlay) = node
            .widget_paint
            .as_deref()
            .and_then(|w| w.floating_overlay.as_ref())
        {
            return resolve_scene_overlay_mount(nodes, node, overlay.overlay_mount);
        }
        resolve_scene_overlay_mount(nodes, node, nana_ui_core::OverlayMount::NearestModal)
    })
}

pub(super) const fn visual_parent_to_node(
    vp: nana_ui_runtime::scene_contract::VisualParent,
) -> Option<VisualNode> {
    match vp {
        nana_ui_runtime::scene_contract::VisualParent::Root
        | nana_ui_runtime::scene_contract::VisualParent::TopLayer => None,
        nana_ui_runtime::scene_contract::VisualParent::Modal(id)
        | nana_ui_runtime::scene_contract::VisualParent::Node(id) => Some(VisualNode::Node(id)),
        nana_ui_runtime::scene_contract::VisualParent::Overlay(id) => Some(VisualNode::Overlay(id)),
    }
}

pub(super) fn visual_parent_of(nodes: &SceneNodes, vn: VisualNode) -> Option<VisualNode> {
    match vn {
        VisualNode::Overlay(id) => {
            let node = nodes.get(&id)?;
            visual_parent_to_node(effective_overlay_visual_parent(nodes, node))
                .filter(|parent| nodes.contains_key(&parent.node_id()))
        }
        VisualNode::Node(id) => {
            let node = nodes.get(&id)?;
            match effective_visual_parent(nodes, node) {
                Some(vp) => {
                    visual_parent_to_node(vp).filter(|parent| nodes.contains_key(&parent.node_id()))
                }
                None => node
                    .parent
                    .filter(|parent| nodes.contains_key(parent))
                    .map(VisualNode::Node),
            }
        }
    }
}

pub(super) fn visual_ancestor_nodes(
    nodes: &SceneNodes,
    start: VisualNode,
) -> impl Iterator<Item = (VisualNode, &ExtractedNode)> {
    let mut current = Some(start);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let vn = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        let entry = nodes.get(&vn.node_id())?;
        current = visual_parent_of(nodes, vn);
        Some((vn, entry.as_ref()))
    })
}
