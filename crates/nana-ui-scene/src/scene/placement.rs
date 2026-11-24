//! Where a retained record stands now.
//!
//! A record's primitives, its component geometry and the clips it cuts its
//! own content with are in the node's own coordinates: its border box at
//! the origin ([`UiScene::record_box`]). They are drawn at the node's page
//! origin ([`UiScene::paint_origin`]), the same sum down the frame chain the
//! runtime takes. When layout only moves a subtree, the runtime sends its
//! root's new local box ([`ExtractedMove`]) and nothing for what it holds:
//! every record under it is drawn at its new origin, and nothing is rebuilt.
//!
//! A widget visual whose geometry reads the page is the exception: its record
//! is on the page, and the runtime extracts it again whenever it moves.
use super::*;

impl UiScene {
    /// Where `node`'s border box stands on the page now: its frame's origin
    /// plus its local box, summed down from the top one axis at a time, as
    /// the runtime sums them (`UiWorld::layout_box`). For a record extracted
    /// since the last move above it this is its [`ExtractedNode::layout`]
    /// origin to the bit. A frame the scene does not hold leaves the record
    /// where it was extracted.
    pub(super) fn placed_origin(&self, node: &ExtractedNode) -> [f32; 2] {
        if !self.placement_live {
            return [node.layout.x, node.layout.y];
        }
        self.derived_projection()
            .placed_origin(node.id)
            .unwrap_or_else(|| placed_origin_uncached(&self.nodes, node))
    }

    /// [`Self::placed_origin`] of `id`, for a caller that holds the scene:
    /// no lock.
    pub(super) fn placed_origin_mut(&mut self, id: StableNodeId) -> Option<[f32; 2]> {
        let node = self.nodes.get(&id)?;
        if !self.placement_live {
            return Some([node.layout.x, node.layout.y]);
        }
        let key = self.epochs.derived_key();
        if let Some(origin) = self
            .derived
            .get_mut()
            .expect("scene derived")
            .as_ref()
            .and_then(|d| d.projection.as_ref())
            .filter(|t| t.key == key)
            .and_then(|t| t.placed_origin(id))
        {
            return Some(origin);
        }
        Some(placed_origin_uncached(&self.nodes, node))
    }

    /// `node`'s box where it stands now, at its record's size.
    pub(super) fn placed_layout(&self, node: &ExtractedNode) -> LayoutBox {
        let [x, y] = self.placed_origin(node);
        LayoutBox {
            x,
            y,
            ..node.layout
        }
    }

    /// Where `node`'s primitives, component geometry and own clips are drawn
    /// from: its page origin, or the page's for a record that is on the page
    /// ([`record_on_page`]).
    pub(super) fn paint_origin(&self, node: &ExtractedNode) -> [f32; 2] {
        if record_on_page(node) {
            [0.0, 0.0]
        } else {
            self.placed_origin(node)
        }
    }

    /// [`Self::paint_origin_of`] for a caller that holds the scene: no lock.
    pub(super) fn paint_origin_mut(&mut self, id: StableNodeId) -> [f32; 2] {
        match self.nodes.get(&id) {
            Some(node) if record_on_page(node) => [0.0, 0.0],
            Some(_) => self.placed_origin_mut(id).unwrap_or([0.0, 0.0]),
            None => [0.0, 0.0],
        }
    }

    /// [`Self::paint_origin`] by id; the page's for a node the scene does not
    /// hold.
    pub(super) fn paint_origin_of(&self, id: StableNodeId) -> [f32; 2] {
        self.nodes
            .get(&id)
            .map_or([0.0, 0.0], |node| self.paint_origin(node))
    }

    /// `node`'s border box in the space its record is in: at the origin, or
    /// where it was extracted for a record on the page.
    pub(super) fn record_box(node: &ExtractedNode) -> LayoutBox {
        if record_on_page(node) {
            node.layout
        } else {
            LayoutBox {
                x: 0.0,
                y: 0.0,
                ..node.layout
            }
        }
    }

    /// Where `id`'s box stands now, before any scroll or transform.
    pub fn node_layout_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        let node = self.nodes.get(&id)?;
        Some(scene_rect(self.placed_layout(node)))
    }

    /// `rect`, given in the space `primitive`'s node's record is in (any of
    /// its own geometry), where it stands on the page now.
    pub fn place_rect(&self, node: StableNodeId, rect: SceneRect) -> SceneRect {
        let [x, y] = self.paint_origin_of(node);
        offset_rect(rect, [x, y])
    }

    /// `primitive`'s bounds where they stand on the page now, before any
    /// scroll or transform: its [`ScenePrimitive::bounds`] moved to its
    /// node's origin.
    pub fn primitive_layout_bounds(&self, primitive: &ScenePrimitive) -> SceneRect {
        self.place_rect(primitive.node, primitive.bounds)
    }
}

/// `rect` moved by `origin`, one add per axis: the one way a record's
/// geometry is put on the page, so every reader puts it at the same place
/// to the bit.
pub(super) fn offset_rect(rect: SceneRect, [x, y]: [f32; 2]) -> SceneRect {
    SceneRect {
        x: x + rect.x,
        y: y + rect.y,
        ..rect
    }
}

/// Whether `node`'s record -- its component geometry and so its primitives
/// -- is on the page rather than in its own coordinates.
pub(super) fn record_on_page(node: &ExtractedNode) -> bool {
    node.widget_paint
        .as_deref()
        .is_some_and(|w| w.geometry_on_page)
}

/// Uncached `placed_origin` respecting `placement_live`: returns the
/// extracted `node.layout` origin until the first `ExtractedMove`.
pub(super) fn placed_origin_live(
    nodes: &SceneNodes,
    placement_live: bool,
    node: &ExtractedNode,
) -> [f32; 2] {
    if !placement_live {
        [node.layout.x, node.layout.y]
    } else {
        placed_origin_uncached(nodes, node)
    }
}

/// Iterative [`UiScene::placed_origin`] fallback without shared cache.
pub(super) fn placed_origin_uncached(nodes: &SceneNodes, node: &ExtractedNode) -> [f32; 2] {
    if is_document(node) {
        return [0.0, 0.0];
    }
    let mut chain: Vec<&ExtractedNode> = Vec::new();
    let mut cursor = node;
    let mut depth = 0usize;
    let mut origin = loop {
        if depth >= MAX_ANCESTOR_DEPTH {
            break [cursor.local_layout.x, cursor.local_layout.y];
        }
        depth += 1;
        let frame = match cursor.layout_frame {
            nana_ui_runtime::LayoutFrame::Parent => cursor.parent,
            nana_ui_runtime::LayoutFrame::Block(block) => Some(block),
            nana_ui_runtime::LayoutFrame::Page => None,
        };
        match frame {
            None => break [cursor.local_layout.x, cursor.local_layout.y],
            Some(frame) => match nodes.get(&frame) {
                Some(frame) if is_document(frame) => {
                    break [cursor.local_layout.x, cursor.local_layout.y];
                }
                Some(frame) => {
                    chain.push(cursor);
                    cursor = frame;
                }
                None => break [cursor.layout.x, cursor.layout.y],
            },
        }
    };
    for node in chain.into_iter().rev() {
        origin = [
            origin[0] + node.local_layout.x,
            origin[1] + node.local_layout.y,
        ];
    }
    origin
}

pub(super) fn is_document(node: &ExtractedNode) -> bool {
    matches!(node.kind.as_ref(), nana_ui_runtime::NodeKind::Document)
}
