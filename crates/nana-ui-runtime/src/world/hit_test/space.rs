//! Each entry's space: offsets, inherited scroll shifts, child seeds, and
//! the transforms between the window and layout coordinates.

use super::*;

/// What [`HitIndex::inherited_shift_skipping`] remembers over one rebuild,
/// while the index stays as it is.
#[derive(Default)]
pub(super) struct ShiftSkips {
    /// Each entry passed: the first entry at or above it the walk stops at.
    first: crate::NodeMap<Option<StableNodeId>>,
    /// The entries one lookup passed, reused.
    path: Vec<StableNodeId>,
}

impl HitIndex {
    /// The scroll the entries above `id` took since they were built: what
    /// a transform built against the live scroll has in it beyond theirs.
    /// Up to the nearest entry that hands its children its own layout
    /// coordinates ([`HitEntry::local_children`]), its own scroll included:
    /// `id`'s transform holds nothing from above it.
    pub(super) fn inherited_shift(&self, id: StableNodeId) -> [f32; 2] {
        let mut shift = [0.0, 0.0];
        if !self.shifted {
            return shift;
        }
        let mut cursor = self.entries.get(&id).and_then(|n| n.parent);
        while let Some(id) = cursor {
            #[cfg(test)]
            crate::world::page_origins::steps::step(
                crate::world::page_origins::steps::Walk::HitShift,
            );
            let Some(node) = self.entries.get(&id) else {
                break;
            };
            shift[0] += node.shift[0];
            shift[1] += node.shift[1];
            if node.entry.local_children {
                break;
            }
            cursor = node.parent;
        }
        shift
    }

    /// [`Self::inherited_shift`], passing over the entries that hold no
    /// scroll shift and hand their children no coordinates of their own:
    /// adding their zeros leaves the sum as it is (it starts at +0 and an
    /// addition gives -0 only from two of them), so only the entries the
    /// walk adds something at or stops at are visited, in the same order.
    /// `skips` remembers, for each entry passed, the first such entry at or
    /// above it: a deep chain is walked once for all the children under it.
    pub(super) fn inherited_shift_skipping(
        &self,
        id: StableNodeId,
        skips: &mut ShiftSkips,
    ) -> [f32; 2] {
        let mut shift = [0.0, 0.0];
        if !self.shifted {
            return shift;
        }
        let mut cursor = self.entries.get(&id).and_then(|n| n.parent);
        while let Some(at) = cursor {
            let Some(node) = self
                .notable_at_or_above(at, skips)
                .and_then(|notable| self.entries.get(&notable))
            else {
                break;
            };
            shift[0] += node.shift[0];
            shift[1] += node.shift[1];
            if node.entry.local_children {
                break;
            }
            cursor = node.parent;
        }
        #[cfg(any(test, feature = "layout-verify"))]
        {
            #[cfg(test)]
            let walked = crate::world::page_origins::steps::uncounted(|| self.inherited_shift(id));
            #[cfg(not(test))]
            let walked = self.inherited_shift(id);
            assert!(
                shift[0].to_bits() == walked[0].to_bits()
                    && shift[1].to_bits() == walked[1].to_bits(),
                "{id:?}: the shift past the unshifted entries {shift:?} is the walked one \
                 {walked:?}"
            );
        }
        shift
    }

    /// The first entry from `id` up (`id` included) that holds a scroll
    /// shift or hands its children its own coordinates; `None` where the
    /// entries end first.
    fn notable_at_or_above(
        &self,
        id: StableNodeId,
        skips: &mut ShiftSkips,
    ) -> Option<StableNodeId> {
        let ShiftSkips { first, path } = skips;
        path.clear();
        let mut cursor = Some(id);
        let found = loop {
            let Some(at) = cursor else {
                break None;
            };
            #[cfg(test)]
            crate::world::page_origins::steps::step(
                crate::world::page_origins::steps::Walk::HitShift,
            );
            if let Some(&known) = first.get(&at) {
                break known;
            }
            let Some(node) = self.entries.get(&at) else {
                break None;
            };
            if node.shift != [0.0, 0.0] || node.entry.local_children {
                break Some(at);
            }
            path.push(at);
            cursor = node.parent;
        };
        for &passed in path.iter() {
            first.insert(passed, found);
        }
        found
    }

    /// How far the space the walk reaches `id` in is from the space `id`'s
    /// transform is held against: the origins and scroll of the entries
    /// above it, the walk's own order (from `id`'s parent up) aside, up to
    /// the nearest entry that hands its children its own layout coordinates
    /// (its scroll alone counts). That entry is returned with it: `None`
    /// when there is none and the offset is from the window.
    fn inherited_offset(&self, id: StableNodeId) -> ([f32; 2], Option<StableNodeId>) {
        let mut offset = [0.0, 0.0];
        let mut cursor = self.entries.get(&id).and_then(|n| n.parent);
        while let Some(id) = cursor {
            let Some(node) = self.entries.get(&id) else {
                break;
            };
            if node.entry.local_children {
                offset[0] += node.shift[0];
                offset[1] += node.shift[1];
                return (offset, Some(id));
            }
            offset[0] += node.entry.origin[0] + node.shift[0];
            offset[1] += node.entry.origin[1] + node.shift[1];
            cursor = node.parent;
        }
        (offset, None)
    }

    /// How far `id`'s own coordinates are from the space its transform is
    /// held against: what a point there takes off before `id`'s transform
    /// maps it. That space is the window's unless [`Self::entry_space`]
    /// says otherwise.
    #[cfg(test)]
    pub(in crate::world) fn entry_offset(&self, id: StableNodeId) -> [f32; 2] {
        self.entry_space(id).0
    }

    /// [`Self::entry_offset`], and the map from the space it is measured in
    /// to the window: the layout coordinates of the nearest entry above
    /// that hands them to its children, through that entry's transform and
    /// everything above it. `None`: the window's own space.
    fn entry_space(&self, id: StableNodeId) -> ([f32; 2], Option<[f32; 6]>) {
        self.space_of(id, true)
    }

    /// [`Self::entry_space`] for the space the walk reaches `id` in (where
    /// its bounds are kept): `id`'s own origin left out, not taken back.
    fn space_of(&self, id: StableNodeId, own: bool) -> ([f32; 2], Option<[f32; 6]>) {
        let (mut offset, mut above) = self.inherited_offset(id);
        if own && let Some(node) = self.entries.get(&id) {
            offset[0] += node.entry.origin[0];
            offset[1] += node.entry.origin[1];
        }
        let Some(first) = above else {
            return (offset, None);
        };
        // The entries handing down their own coordinates, innermost first,
        // each with where it stands in the one above's; composed outermost
        // first, without recursion: they nest as deep as the tree does.
        let mut chain = Vec::new();
        above = Some(first);
        while let Some(at) = above {
            let (mut placed, next) = self.inherited_offset(at);
            let entry = &self.entries[&at].entry;
            placed[0] += entry.origin[0];
            placed[1] += entry.origin[1];
            chain.push(then_affine(
                [1.0, 0.0, 0.0, 1.0, placed[0], placed[1]],
                entry.transform,
            ));
            above = next;
        }
        let map = chain
            .into_iter()
            .rev()
            .reduce(then_affine)
            .expect("one entry at least");
        (offset, Some(map))
    }

    /// The window point `(x, y)` in `id`'s own coordinates, before its
    /// transform: `None` where a turn above it cannot be undone.
    pub(super) fn own_point(&self, id: StableNodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        self.point_in(self.entry_space(id), x, y)
    }

    fn point_in(
        &self,
        (offset, map): ([f32; 2], Option<[f32; 6]>),
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        let (x, y) = match map {
            Some(map) => transformed_point(map, [0.0, 0.0], x, y)?,
            None => (x, y),
        };
        Some((x - offset[0], y - offset[1]))
    }

    /// The window point `(x, y)` in the space the walk reaches `id` in:
    /// where its bounds are kept.
    pub(super) fn reach_point(&self, id: StableNodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        self.point_in(self.space_of(id, false), x, y)
    }

    /// A point of `id`'s own coordinates, past its transform, in the window.
    pub(super) fn own_to_window(&self, id: StableNodeId, x: f32, y: f32) -> (f32, f32) {
        let (offset, map) = self.entry_space(id);
        let (x, y) = (x + offset[0], y + offset[1]);
        match map {
            Some([a, b, c, d, e, f]) => (a * x + c * y + e, b * x + d * y + f),
            None => (x, y),
        }
    }

    /// The shift the index holds `parent`'s children at: what a transform
    /// built against the live scroll for one of them takes off.
    pub(super) fn child_shift(&self, parent: Option<StableNodeId>) -> [f32; 2] {
        if !self.shifted {
            // Every shift is +0 ([`Self::shifted`]), and so is their sum.
            return [0.0, 0.0];
        }
        parent.map_or([0.0, 0.0], |id| {
            let Some(node) = self.entries.get(&id) else {
                return self.inherited_shift(id);
            };
            let mut shift = if node.entry.local_children {
                [0.0, 0.0]
            } else {
                self.inherited_shift(id)
            };
            shift[0] += node.shift[0];
            shift[1] += node.shift[1];
            shift
        })
    }

    /// What `parent` hands a child built now, against the live `scroll`, in
    /// the space the index holds the parent's transform in (scroll shifts
    /// above it put back), with the shift the index holds the child at
    /// ([`Self::child_shift`]) for the caller to take off again.
    ///
    /// With `skips`, the walk up passes over the entries that hold no shift
    /// ([`Self::inherited_shift_skipping`]): the same sum, the entries
    /// between remembered once for every child seeded under them.
    pub(super) fn live_child_seed(
        &self,
        parent: StableNodeId,
        scroll: crate::ScrollOffset,
        skips: Option<&mut ShiftSkips>,
    ) -> Option<([f32; 6], [f32; 2])> {
        let node = self.entries.get(&parent)?;
        if node.entry.local_children {
            return Some((scroll_seed([scroll.x, scroll.y]), node.shift));
        }
        // `find_hit_transform` and `child_shift` from one walk up: the
        // shift above the parent, and the parent's own on top for its
        // children.
        let above = match skips {
            Some(skips) => self.inherited_shift_skipping(parent, skips),
            None => self.inherited_shift(parent),
        };
        let mut transform = node.entry.transform;
        transform[4] += above[0];
        transform[5] += above[1];
        Some((
            then_affine(transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]),
            [above[0] + node.shift[0], above[1] + node.shift[1]],
        ))
    }
}

pub(in crate::world) fn then_affine([a, b, c, d, e, f]: [f32; 6], rhs: [f32; 6]) -> [f32; 6] {
    then_hit(([a, b, c, d, e, f], [0.0, 0.0]), (rhs, [0.0, 0.0])).0
}

pub(in crate::world) fn then_hit(
    (left, [lg, lh]): ([f32; 6], [f32; 2]),
    (right, [rg, rh]): ([f32; 6], [f32; 2]),
) -> ([f32; 6], [f32; 2]) {
    let [a, b, c, d, e, f] = left;
    let [ra, rb, rc, rd, re, rf] = right;
    let na = a * ra + c * rb + e * rg;
    let nb = b * ra + d * rb + f * rg;
    let nc = a * rc + c * rd + e * rh;
    let nd = b * rc + d * rd + f * rh;
    let ne = a * re + c * rf + e;
    let nf = b * re + d * rf + f;
    let ng = lg * ra + lh * rb + rg;
    let nh = lg * rc + lh * rd + rh;
    let ni = lg * re + lh * rf + 1.0;
    // Match scene composition and WGSL f32/FMA rounding at the singular edge.
    if !ni.is_finite() || ni.abs() < 1e-6 {
        return (IDENTITY_AFFINE, [0.0, 0.0]);
    }
    let inv = 1.0 / ni;
    (
        [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
        [ng * inv, nh * inv],
    )
}

pub(in crate::world) fn transformed_contains(
    bounds: LayoutBox,
    transform: [f32; 6],
    persp: [f32; 2],
    x: f32,
    y: f32,
) -> bool {
    transformed_point(transform, persp, x, y).is_some_and(|(x, y)| bounds.contains(x, y))
}

pub(super) fn transformed_point(
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    let det = a * (d - f * h) - c * (b - f * g) + e * (b * h - d * g);
    // A small but finite determinant is still a valid presentation transform.
    // Reject only a true singularity; using an absolute epsilon here made
    // visible nodes with large logical extents and tiny scales unhittable.
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let ia = (d - f * h) * inv;
    let ic = (-c + e * h) * inv;
    let ie = (c * f - e * d) * inv;
    let ib = (-b + f * g) * inv;
    let id = (a - e * g) * inv;
    let if_ = (e * b - a * f) * inv;
    let ig = (b * h - d * g) * inv;
    let ih = (c * g - a * h) * inv;
    let ii = (a * d - c * b) * inv;
    // For an affine transform `ii` is exactly the 2x2 determinant.  A small
    // non-zero determinant remains invertible; rejecting it with an absolute
    // epsilon makes large logical surfaces with tiny presentation scales
    // visibly hittable but logically unreachable.  Perspective transforms
    // still reject a zero projective denominator below.
    if !ii.is_finite() || ii == 0.0 {
        return None;
    }
    let w = ig * x + ih * y + ii;
    if !w.is_finite() || w.abs() < 1e-8 {
        return None;
    }
    let local_x = (ia * x + ic * y + ie) / w;
    let local_y = (ib * x + id * y + if_) / w;
    (local_x.is_finite() && local_y.is_finite()).then_some((local_x, local_y))
}

pub(in crate::world) fn project_transformed_box(
    bounds: LayoutBox,
    [a, by, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
) -> Option<LayoutBox> {
    let mut x = f32::INFINITY;
    let mut y = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for (px, py) in [
        (bounds.x, bounds.y),
        (bounds.x + bounds.width, bounds.y),
        (bounds.x, bounds.y + bounds.height),
        (bounds.x + bounds.width, bounds.y + bounds.height),
    ] {
        let w = g * px + h * py + 1.0;
        if !w.is_finite() || w.abs() < 1e-8 {
            return None;
        }
        let tx = (a * px + c * py + e) / w;
        let ty = (by * px + d * py + f) / w;
        if !tx.is_finite() || !ty.is_finite() {
            return None;
        }
        x = x.min(tx);
        y = y.min(ty);
        right = right.max(tx);
        bottom = bottom.max(ty);
    }
    Some(LayoutBox {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

impl UiWorld {
    pub(in crate::world) fn layout_projection_transform(
        &self,
        target: StableNodeId,
    ) -> Option<([f32; 6], [f32; 2])> {
        self.layout_projection_transform_from(target, true)
    }

    /// [`Self::layout_projection_transform`], or with `own` false only what
    /// lies above `target`: its ancestors' transforms and scroll offsets,
    /// not its own transform. A box placed beside `target` (a popover's
    /// hanging surface) follows where `target` sits, not how it animates.
    pub(in crate::world) fn layout_projection_transform_from(
        &self,
        target: StableNodeId,
        own: bool,
    ) -> Option<([f32; 6], [f32; 2])> {
        if !self.is_mounted(target) {
            return None;
        }
        // Each ancestor, and whether its scroll reaches `target`: not when
        // `target` (or a box on the way) escapes past it (#201).
        let mut chain = Vec::new();
        let mut escaping = None;
        let mut cursor = if own {
            Some(target)
        } else {
            escaping = self.clip_escape(target);
            self.nodes.get(target)?.hierarchy.parent
        };
        let mut transform = (IDENTITY_AFFINE, [0.0, 0.0]);
        while let Some(id) = cursor {
            let node = self.nodes.get(id)?;
            let passed_over = escaping.is_some_and(|anchor| anchor != Some(id));
            if !passed_over {
                escaping = self.clip_escape(id);
            }
            chain.push((id, passed_over));
            if self.hit_motion_layout(id).position == PositionSpec::Fixed {
                transform = self.hanging_branch_transform(id);
                break;
            }
            cursor = node.hierarchy.parent;
        }
        let mut blocks_3d = false;
        let mut parent_persp = None;
        for (id, passed_over) in chain.into_iter().rev() {
            if !self.nodes.contains(id) {
                return None;
            }
            let style = self.hit_motion_layout(id);
            let b = self.component_layout_box(id)?;
            let local = self.input_local_scene_transform(id, &style, b, blocks_3d, parent_persp);
            transform = then_hit(transform, self.with_sticky_translation(id, local));
            blocks_3d |= style.fails_closed_3d_context();
            if id != target && !passed_over {
                let scroll = self.nodes.scroll_offset(id);
                parent_persp = crate::world::presentation::child_parent_perspective(
                    &style, b, scroll, blocks_3d,
                );
                transform = then_hit(
                    transform,
                    ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
                );
            } else {
                parent_persp = None;
            }
        }
        Some(transform)
    }

    /// Current viewport geometry for layout-time anchors. The hit index is
    /// published only after the frame settles, so it cannot serve this query.
    /// Compositor transform overlays follow presentation; layout boxes stay
    /// logical.
    pub(crate) fn viewport_layout_box(&self, target: StableNodeId) -> Option<LayoutBox> {
        self.presentation_input_bounds(target)
    }

    /// Viewport-space bounding box of `target`, including ancestor scroll
    /// offsets, sticky offsets, and presentation/compositor transforms.
    ///
    /// Returns `None` when `target` is unmounted or has not been laid out.
    pub fn node_viewport_bounds(&self, target: StableNodeId) -> Option<LayoutBox> {
        self.viewport_layout_box(target)
    }

    /// Map a window point into this node's untransformed layout coordinates.
    /// Uses the current hit projection when it exists, otherwise the same
    /// ancestor scroll/transform walk as [`Self::viewport_layout_box`].
    pub fn pointer_layout_position(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        if !self.is_mounted(target) {
            return None;
        }
        if let Some(document) = self.document_of(target)
            && !self.has_compositor_transform_overlay_for(document)
            && let Some(index) = self.hit_index.hit_test_index.get(&document)
            && let Some(indexed) = index.entries.get(&target)
        {
            // The entry maps its own box's coordinates; the answer is the
            // page's, where the box stands.
            let (x, y) = index.own_point(target, x, y)?;
            let [origin_x, origin_y] = self.layout_origin(target)?;
            return transformed_point(indexed.entry.transform, indexed.entry.persp, x, y)
                .map(|(x, y)| (x + origin_x, y + origin_y));
        }
        let (transform, persp) = self.layout_projection_transform(target)?;
        transformed_point(transform, persp, x, y)
    }

    /// Map layout geometry to window coordinates through the current hit projection.
    pub fn layout_pointer_position(
        &self,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(f32, f32)> {
        if !self.is_mounted(target) {
            return None;
        }
        let document = self.document_of(target)?;
        if self.has_compositor_transform_overlay_for(document) {
            let ([a, b, c, d, e, f], [g, h]) = self.layout_projection_transform(target)?;
            let w = g * x + h * y + 1.0;
            if !w.is_finite() || w.abs() < 1e-8 {
                return None;
            }
            let result = ((a * x + c * y + e) / w, (b * x + d * y + f) / w);
            return (result.0.is_finite() && result.1.is_finite()).then_some(result);
        }
        let index = self.hit_index.hit_test_index.get(&document)?;
        let entry = &index.entries.get(&target)?.entry;
        let [origin_x, origin_y] = self.layout_origin(target)?;
        let (x, y) = (x - origin_x, y - origin_y);
        let [a, b, c, d, e, f] = entry.transform;
        let [g, h] = entry.persp;
        let w = g * x + h * y + 1.0;
        if !w.is_finite() || w.abs() < 1e-8 {
            return None;
        }
        let result = index.own_to_window(target, (a * x + c * y + e) / w, (b * x + d * y + f) / w);
        (result.0.is_finite() && result.1.is_finite()).then_some(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::{AppContext, DocumentId, LayoutViewport, NodeStyle, Stack};
    use nana_ui_core::{LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};
    use std::sync::Arc;

    #[test]
    fn node_viewport_bounds_accounts_for_ancestor_scroll() {
        let doc = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let scroller = cx
            .create_component(
                doc,
                Stack::column(0.0).style(NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Px(200.0)),
                        height: Some(LengthSpec::Px(100.0)),
                        overflow_y: OverflowSpec::Scroll,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        let item = cx
            .create_component(
                doc,
                Stack::column(0.0).style(NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        position: PositionSpec::Relative,
                        width: Some(LengthSpec::Px(120.0)),
                        height: Some(LengthSpec::Px(240.0)),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            )
            .unwrap();
        cx.append_child(scroller, item).unwrap();
        cx.layout_document(doc, LayoutViewport::new(400.0, 300.0))
            .unwrap();

        let before = cx.world().node_viewport_bounds(item.stable_id()).unwrap();
        assert_eq!(before.y, 0.0);
        assert_eq!(before.width, 120.0);
        assert_eq!(before.height, 240.0);

        let mut queue = crate::MutationQueue::new();
        queue.set_scroll_offset(
            scroller.stable_id(),
            crate::ScrollOffset { x: 0.0, y: 40.0 },
        );
        cx.commit_mutations(queue).unwrap();

        let after = cx.world().node_viewport_bounds(item.stable_id()).unwrap();
        assert_eq!(after.y, -40.0);
        assert_eq!(after.width, 120.0);
    }
}
