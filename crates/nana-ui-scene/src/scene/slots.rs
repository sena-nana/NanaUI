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

/// An open select menu's rows, one slot per option in each namespace. A
/// long menu outgrew fixed slots: the highlight of its 31st row took the
/// first row's label slot. The namespaces paint in this order, so a row's
/// highlight stays under its texts.
pub(super) const SELECT_MENU_BACKGROUNDS: u32 = 9;

pub(super) const SELECT_MENU_MARKS: u32 = 10;

pub(super) const SELECT_MENU_LABELS: u32 = 11;

pub(super) const SELECT_MENU_HINTS: u32 = 12;

/// An open select menu's scrollbar over its rows: the track (0), the thumb
/// (1).
pub(super) const SELECT_MENU_SCROLLBAR: u32 = 13;

/// A context menu row's hint at its end, one slot per row. Fixed row slots
/// below 256 are already shared by backgrounds, labels and icons.
pub(super) const CONTEXT_MENU_HINTS: u32 = 14;

/// A context menu row's picture: its placeholder circle, then the host
/// texture over it, one slot per row in each.
pub(super) const CONTEXT_MENU_IMAGE_BACKDROPS: u32 = 15;

pub(super) const CONTEXT_MENU_IMAGES: u32 = 16;

/// An open select menu's row pictures, as a context menu's. They belong to
/// the menu's layer with its other namespaces (`is_select_menu_slot`).
pub(super) const SELECT_MENU_IMAGE_BACKDROPS: u32 = 17;

pub(super) const SELECT_MENU_IMAGES: u32 = 18;

/// An open select menu's surface, under its rows. Slots 3..=6 are the
/// field's arrow and 7 its hint: the surface used to take slot 4 and hid an
/// arrow stroke for as long as the menu was open.
pub(super) const SELECT_MENU_SURFACE_SLOT: u64 = 8;

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

/// The primitive slot a modal frame paints its scrim in. The scrim fades on
/// its own clock: its node's own opacity layer is the card's.
pub(crate) const MODAL_SCRIM_SLOT: u64 = 10;

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

pub(super) const fn scrollbar_chrome_slot(index: u64) -> u64 {
    ((SCROLLBAR_CHROME as u64) << 32) | index
}

pub(super) fn is_scrollbar_chrome_slot(slot: u64) -> bool {
    slot >> 32 == SCROLLBAR_CHROME as u64
}

/// A scroll view whose bars can show; it paints them over its descendants.
pub(super) fn has_scrollbar_chrome(node: &ExtractedNode) -> bool {
    matches!(
        node.standard_visual,
        Some(StandardVisual::Scrollbar { visibility, .. })
            if visibility != nana_ui_core::ScrollbarVisibility::Hidden
    )
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
    node.standard_visual
        .as_ref()
        .is_some_and(|visual| visual.triggered_menu_overlay().is_some())
}

/// Whether `child` draws the trigger of the triggered menu `menu`.
pub(super) fn is_trigger_content(menu: &ExtractedNode, child: StableNodeId) -> bool {
    menu.standard_visual
        .as_ref()
        .and_then(StandardVisual::triggered_menu_overlay)
        .is_some_and(|overlay| overlay.trigger_content == Some(child))
}

/// An open select menu's surface, rows and bar: the surface slot and the
/// menu's namespaces, `SELECT_MENU_BACKGROUNDS` to `SELECT_MENU_SCROLLBAR`
/// and its row pictures'.
pub(super) fn is_select_menu_slot(slot: u64) -> bool {
    slot == SELECT_MENU_SURFACE_SLOT
        || (u64::from(SELECT_MENU_BACKGROUNDS)..=u64::from(SELECT_MENU_SCROLLBAR))
            .contains(&(slot >> 32))
        || (u64::from(SELECT_MENU_IMAGE_BACKDROPS)..=u64::from(SELECT_MENU_IMAGES))
            .contains(&(slot >> 32))
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
    let is_overlay = is_triggered_overlay_surface(primitive.id)
        || (is_select_menu_slot(primitive.id.slot)
            && nodes.get(&primitive.node).is_some_and(|node| {
                matches!(node.standard_visual, Some(StandardVisual::Select { .. }))
            }));
    if is_overlay {
        VisualNode::Overlay(primitive.node)
    } else {
        VisualNode::Node(primitive.node)
    }
}

fn is_modal_frame_node(node: &ExtractedNode) -> bool {
    matches!(
        node.standard_visual,
        Some(StandardVisual::ModalFrame { .. })
    ) || matches!(
        node.component_geometry.as_deref(),
        Some(nana_ui_runtime::ComponentGeometry::ModalFrame { .. })
    )
}

fn resolve_scene_overlay_mount(
    nodes: &SceneNodes,
    node: &ExtractedNode,
    mount: nana_ui_core::OverlayMount,
) -> nana_ui_runtime::VisualParent {
    match mount {
        nana_ui_core::OverlayMount::TopLayer => nana_ui_runtime::VisualParent::TopLayer,
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
                    return nana_ui_runtime::VisualParent::Modal(ancestor_id);
                }
                cursor = ancestor.parent;
            }
            nana_ui_runtime::VisualParent::Root
        }
        nana_ui_core::OverlayMount::Inline => {
            let Some(parent_id) = node.parent else {
                return nana_ui_runtime::VisualParent::Root;
            };
            let Some(parent) = nodes.get(&parent_id) else {
                return nana_ui_runtime::VisualParent::Root;
            };
            if is_triggered_menu(parent) && !is_trigger_content(parent, node.id) {
                nana_ui_runtime::VisualParent::Overlay(parent_id)
            } else {
                nana_ui_runtime::VisualParent::Node(parent_id)
            }
        }
    }
}

pub(super) fn effective_visual_parent(
    nodes: &SceneNodes,
    node: &ExtractedNode,
) -> Option<nana_ui_runtime::VisualParent> {
    node.visual_parent.or_else(|| {
        let parent_id = node.parent?;
        let parent = nodes.get(&parent_id)?;
        (is_triggered_menu(parent) && !is_trigger_content(parent, node.id))
            .then_some(nana_ui_runtime::VisualParent::Overlay(parent_id))
    })
}

pub(super) fn effective_overlay_visual_parent(
    nodes: &SceneNodes,
    node: &ExtractedNode,
) -> nana_ui_runtime::VisualParent {
    node.overlay_visual_parent.unwrap_or_else(|| {
        let mount = match node.standard_visual.as_ref() {
            Some(StandardVisual::MenuSurface {
                overlay: Some(overlay),
                ..
            }) => overlay.overlay_mount,
            Some(StandardVisual::Select { overlay_mount, .. }) => *overlay_mount,
            _ => nana_ui_core::OverlayMount::NearestModal,
        };
        resolve_scene_overlay_mount(nodes, node, mount)
    })
}

pub(super) const fn visual_parent_to_node(vp: nana_ui_runtime::VisualParent) -> Option<VisualNode> {
    match vp {
        nana_ui_runtime::VisualParent::Root | nana_ui_runtime::VisualParent::TopLayer => None,
        nana_ui_runtime::VisualParent::Modal(id) | nana_ui_runtime::VisualParent::Node(id) => {
            Some(VisualNode::Node(id))
        }
        nana_ui_runtime::VisualParent::Overlay(id) => Some(VisualNode::Overlay(id)),
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
