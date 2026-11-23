//! How each node's accessible space stands: placed by its offset, upright
//! or turned, and the transforms it hands its children.

use super::*;

/// How a node's accessible space -- what its children's bounds are offsets
/// in ([`AccessibilityNode::transform`]) -- stands to its own box and to the
/// viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AccessibleSpace {
    /// Placed by its local offset and its own 2D transform in its parent's
    /// space ([`UiWorld::offset_placed`]), not from its viewport box.
    pub(super) local: bool,
    /// The positive per-axis scale from its own layout px to its space,
    /// where its box stands upright in it: its children placed by their
    /// offsets stand at those offsets less its scroll, scaled by this.
    /// `None` where its box is projected in the viewport through a 3D
    /// context, a modal frame or a motion overlay, or turned below one: its
    /// children are placed from their viewport boxes.
    pub(super) unit: Option<[f32; 2]>,
    /// Its space is not the viewport's moved: a box at or above it hands
    /// its own frame -- a rotation, a skew, a flip or a scale other than one
    /// of its own transform ([`UiWorld::local_placement`]), or a turn or
    /// scale of the chain above a box placed from the viewport
    /// ([`UiWorld::frames_from_viewport`]) -- so the space is that box's
    /// layout px, turned or scaled from the viewport's. A box placed from
    /// its viewport box under it hands a frame of its own, or is handed the
    /// inverse map.
    pub(super) framed: bool,
}

/// Whether the linear part `(a, b, c, d)` is the identity: a transform with
/// it only moves.
fn moves_only(a: f32, b: f32, c: f32, d: f32) -> bool {
    a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
}

#[derive(Clone, Copy)]
pub(super) struct AccessibleTransform {
    pub(super) own: ([f32; 6], [f32; 2]),
    pub(super) children: ([f32; 6], [f32; 2]),
    pub(super) blocks_3d: bool,
    pub(super) parent_persp: Option<nana_ui_core::ParentPerspective>,
    /// The node's layout origin on the page ([`UiWorld::layout_origin`]):
    /// a child measured from it stands its local offset away, by the same
    /// addition [`UiWorld::record_layout_origin`] makes.
    pub(super) origin: [f32; 2],
    /// Whether that addition is the child's page origin: the node is no
    /// document (whose children's origins are their offsets alone) and no
    /// component's geometry is being derived in its own coordinates.
    pub(super) stacks: bool,
}

/// What a parent hands the children placed by their offsets in it
/// ([`UiWorld::has_accessibility_bounds_under`]'s fast path), read once for
/// them all: its affine map for its children (no perspective row), and its
/// page origin, which a child measured from it stands its offset away from.
#[derive(Clone, Copy)]
pub(super) struct HandedMap {
    pub(super) map: [f32; 6],
    pub(super) origin: [f32; 2],
}

impl HandedMap {
    pub(super) fn of(transform: AccessibleTransform) -> Option<Self> {
        let (map, [g, h]) = transform.children;
        (g == 0.0 && h == 0.0 && transform.stacks).then_some(Self {
            map,
            origin: transform.origin,
        })
    }

    /// Whether `record`, a child of `parent` placed by its offset with no
    /// transform of its own, has a viewport box: its page box's corners are
    /// finite through the map. `None` for any other child, which takes the
    /// full test.
    #[inline]
    pub(super) fn finite_under(&self, record: &NodeRecord, parent: StableNodeId) -> Option<bool> {
        if record.hierarchy.parent != Some(parent)
            || record.space.transformed()
            || !record.space.places_by_offset()
            || record.space.contains(NodeSpace::DOCUMENT)
        {
            return None;
        }
        let [a, b, c, d, e, f] = self.map;
        let local = record.local_layout;
        let (x, y) = (self.origin[0] + local.x, self.origin[1] + local.y);
        let corner =
            |x: f32, y: f32| (a * x + c * y + e).is_finite() && (b * x + d * y + f).is_finite();
        Some(corner(x, y) && corner(x + local.width, y + local.height))
    }
}

/// What a projected node's chain answers for every child it holds, asked
/// once for them all: whether a box above them is hidden, is not
/// axis-aligned ([`UiWorld::axis_aligned`]) or does not hand offsets
/// ([`UiWorld::offset_placed`]).
#[derive(Clone, Copy)]
pub(super) struct ChildChain {
    pub(super) hidden: bool,
    pub(super) aligned: bool,
    pub(super) hands_offsets: bool,
}

/// Whether the linear part `(a, b, c, d)` only scales, by a positive factor
/// on each axis.
pub(super) fn upright(a: f32, b: f32, c: f32, d: f32) -> bool {
    b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0
}

/// `map` after the move by `-scroll`: the map from the space a node's
/// children are placed in, given its own space's.
pub(super) fn scrolled_map(map: [f32; 6], [sx, sy]: [f32; 2]) -> [f32; 6] {
    if sx == 0.0 && sy == 0.0 {
        return map;
    }
    let [a, b, c, d, e, f] = map;
    [a, b, c, d, e - (a * sx + c * sy), f - (b * sx + d * sy)]
}

/// The inverse of the affine `[a, b, c, d, e, f]` (`x' = a·x + c·y + e`),
/// unless it is singular.
pub(super) fn invert_affine([a, b, c, d, e, f]: [f32; 6]) -> Option<[f32; 6]> {
    let det = a * d - b * c;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    Some([
        d / det,
        -b / det,
        -c / det,
        a / det,
        (c * f - d * e) / det,
        (b * e - a * f) / det,
    ])
}

impl UiWorld {
    /// Layout box and viewport box are the same when no ancestor scrolls or
    /// transforms, and the box's accessible box is its layout box (not a
    /// modal frame's surface). The chain's answer is memoized; a skewed node
    /// forces the normal transform walk for itself and its descendants.
    ///
    /// The transform walk ([`Self::accessibility_ancestor_transform`]) reads
    /// each box's effective style, which visuals, overlays and stacking can
    /// make differ from its resolved one; only an open triggered menu's
    /// placement changes what the walk makes of it (its content is fixed,
    /// hanging from the menu). Motion overlays transform any box, and clip
    /// visuals cut it, so those leave the walk; every live modal frame is a
    /// clip visual, so none is left whose surface is its accessible box.
    pub(super) fn axis_aligned(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let above = self.motion_free_unclipped()
            && record
                .hierarchy
                .parent
                .is_none_or(|parent| self.chain_axis_aligned(parent, memo));
        self.axis_aligned_under(id, record, above, memo)
    }

    /// [`Self::axis_aligned`] for `record`, `id`'s, with the chain above it
    /// already answered (`above`).
    pub(super) fn axis_aligned_under(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        above: bool,
        memo: &mut ProjectionMemo,
    ) -> bool {
        // The box itself is not memoized, only the chains above it: a
        // projection asks after each of its children once, and their own
        // boxes are rarely asked after again, where every ancestor is.
        let aligned = above && self.box_axis_aligned(id, record);
        #[cfg(any(test, feature = "layout-verify"))]
        if aligned {
            assert_eq!(
                self.visible_accessibility_bounds(id, memo),
                self.component_layout_box(id),
                "{id:?}: an axis-aligned box's accessible box is not its layout box"
            );
        }
        #[cfg(not(any(test, feature = "layout-verify")))]
        let _ = memo;
        aligned
    }

    /// What `id`'s chain answers for the children it holds ([`ChildChain`]).
    pub(super) fn child_chain(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> ChildChain {
        let free = self.motion_free_unclipped();
        ChildChain {
            hidden: self.ancestor_hidden(id, memo),
            aligned: free && self.chain_axis_aligned(id, memo),
            hands_offsets: free && self.chain_hands_offsets(id, memo),
        }
    }

    /// No motion overlay and no clip visual anywhere: what decides whether
    /// a box is axis-aligned is on its chain.
    pub(super) fn motion_free_unclipped(&self) -> bool {
        self.layout_length_tracks.is_empty()
            && self.presentation.is_empty()
            && self.clip_visuals == 0
    }

    /// Whether `record`, `id`'s, neither scrolls, transforms nor places
    /// itself off its parent's coordinates.
    fn box_axis_aligned(&self, id: StableNodeId, record: &NodeRecord) -> bool {
        !(self.nodes.scroll_offset(id) != ScrollOffset::default()
            || !record.space.aligned_by_style()
            || (!self.triggered_overlays.is_empty() && self.parent_triggered_overlay(id).is_some()))
    }

    /// [`Self::axis_aligned`] for `id` and every box above it, memoized.
    fn chain_axis_aligned(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        if let Some(aligned) = memo.aligned.get(&id) {
            return *aligned;
        }
        if !self.motion_free_unclipped() {
            memo.aligned.insert(id, false);
            return false;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut aligned = true;
        while let Some(node) = current {
            if node != id
                && let Some(&known) = memo.aligned.get(&node)
            {
                aligned = known;
                break;
            }
            memo.chain.push(node);
            let Some(record) = self.nodes.get(node) else {
                aligned = false;
                break;
            };
            if !self.box_axis_aligned(node, record) {
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

    /// Whether `id` and every box above it hand their children offsets
    /// ([`NodeSpace::hands_offsets`]), with no motion overlay
    /// or clip visual anywhere: then every transform above a child of `id`
    /// is a translation and a positive scale. Memoized as
    /// [`Self::chain_axis_aligned`] is.
    pub(super) fn chain_hands_offsets(&self, id: StableNodeId, memo: &mut ProjectionMemo) -> bool {
        if let Some(offsets) = memo.offsets.get(&id) {
            return *offsets;
        }
        if !self.motion_free_unclipped() {
            memo.offsets.insert(id, false);
            return false;
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut offsets = true;
        while let Some(node) = current {
            if node != id
                && let Some(&known) = memo.offsets.get(&node)
            {
                offsets = known;
                break;
            }
            memo.chain.push(node);
            let Some(record) = self.nodes.get(node) else {
                offsets = false;
                break;
            };
            if !record.space.hands_offsets() {
                offsets = false;
                break;
            }
            current = record.hierarchy.parent;
        }
        for node in memo.chain.drain(..) {
            memo.offsets.insert(node, offsets);
        }
        offsets
    }

    /// Whether `id`'s accessible bounds and transform in its parent's space
    /// come from its local layout box and its own 2D transform: `record`,
    /// `id`'s, is placed by its offset
    /// ([`NodeSpace::places_by_offset`]) and its parent's
    /// space holds its own box upright at a positive scale `(a, d)`
    /// ([`AccessibleSpace::unit`]). Then its bounds are `(a·x, d·y, a·w,
    /// d·h)` -- its local box, scaled, in the space its parent's scroll
    /// ([`AccessibilityNode::scroll`]) moves -- with its own transform, if any, folded in or handed on
    /// ([`Self::local_placement`]), whatever the boxes above stand at, and
    /// however they turn. Nothing in that reads a page position, so a box
    /// that moved with its parent keeps them to the bit.
    pub(super) fn offset_placed(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        memo: &mut ProjectionMemo,
    ) -> bool {
        // [`Self::space_of`]'s test, from the parent's space alone: the box
        // itself is asked once and need not be kept.
        let placed = record.hierarchy.parent.is_some_and(|parent| {
            self.motion_free_unclipped()
                && self.geometry_frame.get().is_none()
                && self
                    .accessible_space(parent, memo)
                    .is_some_and(|space| space.unit.is_some())
        }) && self.offset_placed_under(id, record, true);
        #[cfg(any(test, feature = "layout-verify"))]
        assert_eq!(
            Some(placed),
            self.accessible_space(id, memo).map(|space| space.local),
            "{id:?}: placed by its offset, from its parent's space"
        );
        placed
    }

    /// `id`'s accessible space ([`AccessibleSpace`]), memoized with every
    /// box above it.
    pub(super) fn accessible_space(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<AccessibleSpace> {
        if let Some(space) = memo.spaces.get(&id) {
            return Some(*space);
        }
        let mut chain = std::mem::take(&mut memo.space_chain);
        chain.clear();
        let mut at = Some(id);
        while let Some(node) = at {
            if memo.spaces.contains_key(&node) {
                break;
            }
            let Some(record) = self.nodes.get(node) else {
                break;
            };
            chain.push(node);
            at = record.hierarchy.parent;
        }
        while let Some(node) = chain.pop() {
            let Some(record) = self.nodes.get(node) else {
                continue;
            };
            let parent = record
                .hierarchy
                .parent
                .and_then(|parent| memo.spaces.get(&parent).copied());
            let space = self.space_of(node, record, parent, memo);
            memo.spaces.insert(node, space);
        }
        memo.space_chain = chain;
        memo.spaces.get(&id).copied()
    }

    /// [`AccessibleSpace`] of `record`, `id`'s, under a parent whose space is
    /// `parent`.
    fn space_of(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        parent: Option<AccessibleSpace>,
        memo: &mut ProjectionMemo,
    ) -> AccessibleSpace {
        let free = self.motion_free_unclipped();
        let above = parent.filter(|parent| parent.unit.is_some());
        if let Some(above) = above
            && free
            && self.geometry_frame.get().is_none()
            && self.offset_placed_under(id, record, true)
        {
            return if record.space.frames_itself() {
                AccessibleSpace {
                    local: true,
                    unit: Some([1.0, 1.0]),
                    framed: true,
                }
            } else {
                // None, or a move: folded into the offset.
                AccessibleSpace {
                    local: true,
                    unit: above.unit,
                    framed: above.framed,
                }
            };
        }
        // Placed from its viewport box: its space is the viewport's, moved
        // to that box; its own box stands upright in it only where every
        // transform above scales it by a positive factor per axis.
        let unit =
            (free && record.space.hands_offsets() && !record.space.contains(NodeSpace::MODAL))
                .then(|| {
                    self.accessibility_ancestor_transform(id, memo)?;
                    let ([a, b, c, d, ..], [g, h]) = memo.transforms.get(&id)?.own;
                    (g == 0.0 && h == 0.0 && upright(a, b, c, d)).then_some([a, d])
                })
                .flatten();
        // A root is placed in the viewport, which is no frame.
        let above = match record.hierarchy.parent {
            None => Some(AccessibleSpace {
                local: false,
                unit: Some([1.0, 1.0]),
                framed: false,
            }),
            Some(_) => above,
        };
        let framed = self.frames_from_viewport(id, record, above, memo);
        AccessibleSpace {
            local: false,
            unit: if framed { Some([1.0, 1.0]) } else { unit },
            framed,
        }
    }

    /// Whether `record`, `id`'s, placed from its viewport box under a
    /// parent whose space is `above` (the viewport for a root; `None` where
    /// the parent's map to the viewport is not known), hands its own frame:
    /// its space is its own layout px, its bounds its layout box placed in
    /// the parent's space by the map its whole chain takes it to the
    /// viewport by ([`UiWorld::viewport_frame_in_parent`]). So it is where a 2D chain
    /// with no 3D context, modal frame or motion overlay turns or scales
    /// it: its own transform does more than move it, its box is not
    /// upright in the viewport, or the parent's space is framed. A `fixed`
    /// or `sticky` box, or an absolute one escaping its clip, still takes
    /// its move from the viewport; what turns and scales it is handed on,
    /// and what it holds is placed by offsets inside it, whatever page
    /// position the chain gives it.
    pub(super) fn frames_from_viewport(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        above: Option<AccessibleSpace>,
        memo: &mut ProjectionMemo,
    ) -> bool {
        let Some(above) = above else {
            return false;
        };
        if !(self.motion_free_unclipped()
            && self.geometry_frame.get().is_none()
            && record.space.hands_offsets()
            && !record.space.contains(NodeSpace::DOCUMENT)
            && !record.space.contains(NodeSpace::MODAL)
            && (self.triggered_overlays.is_empty() || self.parent_triggered_overlay(id).is_none()))
        {
            return false;
        }
        if self.accessibility_ancestor_transform(id, memo).is_none() {
            return false;
        }
        let Some(transform) = memo.transforms.get(&id) else {
            return false;
        };
        let ([a, b, c, d, ..], [g, h]) = transform.own;
        if g != 0.0 || h != 0.0 {
            return false;
        }
        let framed = record.space.frames_itself() || !upright(a, b, c, d) || above.framed;
        // The parent's map must be one the box can be placed in.
        framed
            && record.hierarchy.parent.is_none_or(|parent| {
                !above.framed
                    || self
                        .accessible_space_to_viewport(parent, memo)
                        .and_then(invert_affine)
                        .is_some()
            })
    }

    /// `record`'s own 2D transform, `id`'s, in its own layout px (its box at
    /// the origin, about its transform origin): `None` without one.
    fn own_accessible_transform(&self, _id: StableNodeId, record: &NodeRecord) -> Option<[f32; 6]> {
        let style = record.resolved_layout.as_ref();
        style.transforms.transform?;
        let local = record.local_layout;
        style
            .world_scene_transform(0.0, 0.0, local.width, local.height)
            .map(|(matrix, _)| matrix)
    }

    /// How far `id` scrolls what it holds in its accessible space
    /// ([`AccessibilityNode::scroll`]): its scroll offset scaled by its
    /// space's unit, where its box stands upright there; nothing where it
    /// does not scroll or its children are placed from their viewport boxes.
    pub(super) fn accessible_scroll(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> [f32; 2] {
        if !self.nodes.contains(id) {
            return [0.0, 0.0];
        }
        let scroll = self.nodes.scroll_offset(id);
        if scroll == ScrollOffset::default() {
            return [0.0, 0.0];
        }
        match self.accessible_space(id, memo).and_then(|space| space.unit) {
            Some([ua, ud]) => [ua * scroll.x, ud * scroll.y],
            None => [0.0, 0.0],
        }
    }

    /// `id`'s bounds and transform in `parent`'s children's space, from its
    /// local box and its own transform, where `id` is placed by its offset
    /// ([`Self::offset_placed`]) and `unit` is its parent's
    /// ([`AccessibleSpace::unit`]). The parent's scroll is its own
    /// ([`AccessibilityNode::scroll`]): what it scrolls keeps its bounds. A
    /// transform of its own that only moves
    /// it is folded into the offset; any other -- a scale, a rotation, a
    /// skew, a flip -- is handed on, scaled by `unit`, with the box's own
    /// size, so that what it holds is placed by offsets inside it. An
    /// upright scale's move is folded into the offset, and it hands
    /// `diag(ua·a, ud·d)`.
    pub(super) fn local_placement(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        [ua, ud]: [f32; 2],
    ) -> (LayoutBox, Option<nana_ui_core::PaintTransform>) {
        let local = record.local_layout;
        let (x, y) = (local.x, local.y);
        match self.own_accessible_transform(id, record) {
            None => (
                LayoutBox {
                    x: ua * x,
                    y: ud * y,
                    width: ua * local.width,
                    height: ud * local.height,
                },
                None,
            ),
            Some([a, b, c, d, e, f]) if moves_only(a, b, c, d) => (
                LayoutBox {
                    x: ua * (x + e),
                    y: ud * (y + f),
                    width: ua * local.width,
                    height: ud * local.height,
                },
                None,
            ),
            Some([a, b, c, d, e, f]) if upright(a, b, c, d) => (
                LayoutBox {
                    x: ua * (x + e),
                    y: ud * (y + f),
                    width: local.width,
                    height: local.height,
                },
                Some(nana_ui_core::PaintTransform {
                    a: ua * a,
                    b: 0.0,
                    c: 0.0,
                    d: ud * d,
                    e: 0.0,
                    f: 0.0,
                }),
            ),
            Some([a, b, c, d, e, f]) => (
                LayoutBox {
                    x: ua * x,
                    y: ud * y,
                    width: local.width,
                    height: local.height,
                },
                Some(nana_ui_core::PaintTransform {
                    a: ua * a,
                    b: ud * b,
                    c: ua * c,
                    d: ud * d,
                    e: ua * e,
                    f: ud * f,
                }),
            ),
        }
    }

    /// The map from the space `id`'s children are placed in to the
    /// viewport: [`Self::accessible_space_to_viewport`] after the move by
    /// its scroll ([`Self::accessible_scroll`]).
    pub(super) fn children_space_to_viewport(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        let map = self.accessible_space_to_viewport(id, memo)?;
        Some(scrolled_map(map, self.accessible_scroll(id, memo)))
    }

    /// The map from `id`'s accessible space to the viewport, where its box
    /// stands upright in it ([`AccessibleSpace::unit`]): its box's map to
    /// the viewport after the inverse of that scale.
    pub(super) fn accessible_space_to_viewport(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        let [ua, ud] = self.accessible_space(id, memo)?.unit?;
        self.accessibility_ancestor_transform(id, memo)?;
        let transform = memo.transforms.get(&id)?;
        let ([a, b, c, d, e, f], [g, h]) = transform.own;
        if g != 0.0 || h != 0.0 {
            return None;
        }
        let [ox, oy] = transform.origin;
        Some([
            a / ua,
            b / ua,
            c / ud,
            d / ud,
            a * ox + c * oy + e,
            b * ox + d * oy + f,
        ])
    }

    /// [`Self::offset_placed`] with the chain above already answered.
    pub(super) fn offset_placed_under(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        above: bool,
    ) -> bool {
        above
            && record.space.places_by_offset()
            && !record.space.contains(NodeSpace::DOCUMENT)
            && (self.triggered_overlays.is_empty() || self.parent_triggered_overlay(id).is_none())
    }
}
