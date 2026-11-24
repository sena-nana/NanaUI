//! Viewport bounds of a projected box: the ancestor transform chain, the
//! visible part and the clips above it.

use super::*;

impl UiWorld {
    /// Whether `id`, a child of `parent`, has a viewport box
    /// ([`Self::visible_accessibility_bounds`] is `Some`). A box placed by its
    /// offset ([`Self::offset_placed`]) under a parent whose transform is
    /// known stands where that transform for its children puts its layout
    /// box: it has one when those corners are finite, which is answered
    /// without computing and keeping its own transform and box.
    #[cfg(any(test, feature = "layout-verify"))]
    pub(super) fn has_accessibility_bounds(
        &self,
        id: StableNodeId,
        parent: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return self.visible_accessibility_bounds(id, memo).is_some();
        };
        let above = self.motion_free_unclipped() && self.chain_hands_offsets(parent, memo);
        let handed = self.handed_transform(parent, above, memo);
        self.has_accessibility_bounds_under(id, record, parent, above, handed, memo)
    }

    /// What `parent` hands the children that stand at their offsets in it
    /// ([`Self::has_accessibility_bounds_under`]), when its transform is
    /// known and its chain hands offsets (`above`): looked up once for all
    /// of its children.
    pub(super) fn handed_transform(
        &self,
        parent: StableNodeId,
        above: bool,
        memo: &ProjectionMemo,
    ) -> Option<AccessibleTransform> {
        if !above || self.invalidation.geometry_frame.get().is_some() {
            return None;
        }
        memo.transforms.get(&parent).copied()
    }

    /// [`Self::has_accessibility_bounds`] for `record`, `id`'s, with whether
    /// the chain above hands offsets already answered (`above`), and
    /// `parent`'s transform when that is known (`handed`,
    /// [`Self::handed_transform`]). The child's page box is its local offset
    /// from `parent`'s origin, which the parent's transform already holds.
    pub(super) fn has_accessibility_bounds_under(
        &self,
        id: StableNodeId,
        record: &NodeRecord,
        parent: StableNodeId,
        above: bool,
        handed: Option<AccessibleTransform>,
        memo: &mut ProjectionMemo,
    ) -> bool {
        if let Some(transform) = handed
            && record.hierarchy.parent == Some(parent)
            && record.resolved_layout.transforms.transform.is_none()
            && self.offset_placed_under(id, record, above)
        {
            let ([a, b, c, d, e, f], [g, h]) = transform.children;
            if g == 0.0 && h == 0.0 {
                let local = self.nodes.local_layout(id);
                let bounds = LayoutBox {
                    x: transform.origin[0] + local.x,
                    y: transform.origin[1] + local.y,
                    ..local
                };
                let corner = |x: f32, y: f32| {
                    (a * x + c * y + e).is_finite() && (b * x + d * y + f).is_finite()
                };
                let finite = corner(bounds.x, bounds.y)
                    && corner(bounds.x + bounds.width, bounds.y + bounds.height);
                #[cfg(any(test, feature = "layout-verify"))]
                {
                    assert_eq!(
                        bounds,
                        self.record_layout_box(id, record),
                        "{id:?}: its page box is not its parent's origin and its offset"
                    );
                    assert_eq!(
                        finite,
                        self.visible_accessibility_bounds(id, memo).is_some(),
                        "{id:?}: whether it has a viewport box"
                    );
                }
                return finite;
            }
        }
        self.visible_accessibility_bounds(id, memo).is_some()
    }

    pub(super) fn visible_accessibility_bounds(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        if let Some(bounds) = memo.bounds.get(&id) {
            return *bounds;
        }
        let bounds = self.compute_accessibility_bounds(id, memo);
        memo.bounds.insert(id, bounds);
        bounds
    }

    fn compute_accessibility_bounds(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        let local = if self.invalidation.modal_frames != 0
            && self
                .widget_visual_as::<crate::ModalFrameVisual>(id)
                .is_some()
        {
            match self.modal_frame_geometry(id) {
                Some(crate::ModalFrameGeometry { surface, .. }) => surface,
                None => self.component_layout_box(id)?,
            }
        } else {
            self.component_layout_box(id)?
        };
        let mut bounds = self.accessibility_viewport_bounds(id, local, memo)?;
        if self.invalidation.clip_visuals == 0 {
            return Some(bounds);
        }
        // Only the clip chain clips `id` (#201): an escaped box passes over
        // the ancestors between it and its clip parent, and a `fixed` box,
        // `id` or one above it, ends the chain, as in paint and hit testing.
        let mut below = id;
        for ancestor in self.clip_ancestors(id) {
            if self
                .widget_visual_ref(ancestor)
                .is_some_and(crate::WidgetVisualHandle::clips_descendants)
            {
                bounds = intersect_layout_boxes(
                    bounds,
                    self.accessibility_viewport_bounds(
                        ancestor,
                        self.component_layout_box(ancestor)?,
                        memo,
                    )?,
                )?;
            }
            if let Some(modal) = self.widget_visual_as::<crate::ModalFrameVisual>(ancestor)
                && let Some(crate::ModalFrameGeometry { surface, body, .. }) =
                    self.modal_frame_geometry(ancestor)
            {
                bounds = intersect_layout_boxes(
                    bounds,
                    self.accessibility_viewport_bounds(ancestor, surface, memo)?,
                )?;
                if modal.slots.body == Some(below) {
                    bounds = intersect_layout_boxes(
                        bounds,
                        self.accessibility_viewport_bounds(ancestor, body, memo)?,
                    )?;
                }
            }
            below = ancestor;
        }
        Some(bounds)
    }
}

impl UiWorld {
    // Match build_hit_forest's composition from the current retained state.
    // A committed transform may precede the hit projection; that projection
    // is not an authority for accessibility coordinates. Memoizing ancestors
    // also avoids repeating hit-index ancestor walks for every projected node.
    pub(super) fn accessibility_ancestor_transform(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<[f32; 6]> {
        if let Some(transform) = memo.transforms.get(&id) {
            return Some(transform.own.0);
        }
        memo.chain.clear();
        let mut current = Some(id);
        let mut cumulative = (IDENTITY_AFFINE, [0.0, 0.0]);
        let mut blocks_3d = false;
        let mut parent_persp = None;
        // Where the box above the next one stands on the page, when a box
        // measured from it stands at that origin and its own offset.
        let mut above: Option<[f32; 2]> = None;
        while let Some(node) = current {
            if let Some(transform) = memo.transforms.get(&node) {
                cumulative = transform.children;
                blocks_3d = transform.blocks_3d;
                parent_persp = transform.parent_persp;
                above = transform.stacks.then_some(transform.origin);
                break;
            }
            memo.chain.push(node);
            current = self.nodes.get(node)?.hierarchy.parent;
        }
        // Nothing on the page but layout styles and records: a box in flow
        // that neither transforms nor places itself is its parent's map,
        // read without its page box's frame walk or the overlay, escape,
        // sticky and transform lookups, which all answer "none" for it.
        let plain_page = !self.has_layout_length_overlays()
            && self.overlays_a11y.overlay_host_nodes.is_empty()
            && self.invalidation.z_index_nodes == 0
            && self.tree.detached_mounted.is_empty()
            && !self.nodes.has_widget_visuals()
            && self.invalidation.modal_frames == 0
            && self.invalidation.menu_surfaces == 0
            && self.overlays_a11y.triggered_overlays.is_empty()
            && self.overlays_a11y.select_menus.is_empty()
            && self.motion.presentation.is_empty()
            && self.invalidation.geometry_frame.get().is_none();
        while let Some(node) = memo.chain.pop() {
            let record = self.nodes.get(node)?;
            let stacks = self.invalidation.geometry_frame.get().is_none()
                && !matches!(record.kind.as_ref(), NodeKind::Document);
            if plain_page
                && let Some([ox, oy]) = above
                && self.plain_transform_step(node, record)
            {
                let local = self.nodes.local_layout(node);
                let origin = [ox + local.x, oy + local.y];
                #[cfg(any(test, feature = "layout-verify"))]
                {
                    let bounds = self.component_layout_box(node)?;
                    assert_eq!(
                        origin.map(f32::to_bits),
                        [bounds.x, bounds.y].map(f32::to_bits),
                        "{node:?}: its page origin is not its parent's and its offset"
                    );
                    let layout = self.hit_motion_layout(node);
                    assert_eq!(
                        self.input_local_scene_transform(
                            node,
                            &layout,
                            bounds,
                            blocks_3d,
                            parent_persp,
                        ),
                        (IDENTITY_AFFINE, [0.0, 0.0]),
                        "{node:?}: a plain box hands its parent's map on"
                    );
                    assert!(self.clip_escape(node).is_none());
                    assert_eq!(self.sticky_translation(node), [0.0, 0.0]);
                }
                cumulative = then_hit(cumulative, (IDENTITY_AFFINE, [0.0, 0.0]));
                let own = cumulative;
                let scroll = self.nodes.scroll_offset(node);
                cumulative = then_hit(
                    cumulative,
                    ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
                );
                blocks_3d |= record.space.fails_closed_3d();
                parent_persp = None;
                memo.transforms.insert(
                    node,
                    AccessibleTransform {
                        own,
                        children: cumulative,
                        blocks_3d,
                        parent_persp,
                        origin,
                        stacks,
                    },
                );
                above = stacks.then_some(origin);
                continue;
            }
            let bounds = self.component_layout_box(node)?;
            let layout = self.hit_motion_layout(node);
            if layout.position == PositionSpec::Fixed {
                // Viewport-relative; ancestors cannot scroll or clip this branch.
                cumulative = self.hanging_branch_transform(node);
                blocks_3d = false;
                parent_persp = None;
            } else if let Some(anchor) = self.clip_escape(node) {
                // An escaped box resumes from its clip parent, through the
                // transforms it passes over but none of their scroll (#201).
                // The clip parent is an ancestor, so it is memoized by now.
                let base = match anchor.and_then(|anchor| memo.transforms.get(&anchor)) {
                    Some(transform) => {
                        blocks_3d = transform.blocks_3d;
                        parent_persp = if anchor == record.hierarchy.parent {
                            transform.parent_persp
                        } else {
                            None
                        };
                        transform.children
                    }
                    None => {
                        blocks_3d = false;
                        parent_persp = None;
                        (IDENTITY_AFFINE, [0.0, 0.0])
                    }
                };
                cumulative = self.clip_bridge_transform(node, anchor, base, true);
            }
            let local =
                self.input_local_scene_transform(node, &layout, bounds, blocks_3d, parent_persp);
            cumulative = then_hit(cumulative, self.with_sticky_translation(node, local));
            let own = cumulative;
            let scroll = self.nodes.scroll_offset(node);
            cumulative = then_hit(
                cumulative,
                ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
            );
            blocks_3d |= layout.fails_closed_3d_context();
            parent_persp = crate::world::presentation::child_parent_perspective(
                &layout, bounds, scroll, blocks_3d,
            );
            memo.transforms.insert(
                node,
                AccessibleTransform {
                    own,
                    children: cumulative,
                    blocks_3d,
                    parent_persp,
                    origin: [bounds.x, bounds.y],
                    stacks,
                },
            );
            above = stacks.then_some([bounds.x, bounds.y]);
        }
        memo.transforms.get(&id).map(|transform| transform.own.0)
    }

    /// Whether `record`'s box, on a page with nothing but layout styles and
    /// records ([`Self::accessibility_ancestor_transform`]'s `plain_page`),
    /// takes its parent's map for its own: measured from its parent, in
    /// flow or relative (not fixed, sticky or absolute: no hanging branch,
    /// sticky offset or clip escape), shown, with physical box edges (its
    /// resolved style is its effective one) and no transform of its own.
    fn plain_transform_step(&self, id: StableNodeId, record: &NodeRecord) -> bool {
        let style = record.resolved_layout.as_ref();
        self.nodes.layout_frame(id) == crate::LayoutFrame::Parent
            && record.mount == crate::MountState::Mounted
            && matches!(
                style.position,
                PositionSpec::Static | PositionSpec::Relative
            )
            && style.transforms.transform.is_none()
            && style.transforms.transform_3d.is_none()
            && !style.omits_box()
            && !style.has_logical_box_edges()
    }

    /// AccessKit consumes document/viewport logical coordinates. Layout boxes
    /// deliberately stay unscrolled; compose current ancestor transforms so
    /// pointer and accessibility consumers agree after nested scrolling.
    pub(super) fn accessibility_viewport_bounds(
        &self,
        id: StableNodeId,
        bounds: LayoutBox,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        let transform = self.accessibility_ancestor_transform(id, memo)?;
        let [a, b, c, d, e, f] = transform;
        if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && e == 0.0 && f == 0.0 {
            return Some(bounds);
        }
        let corners = [
            (bounds.x, bounds.y),
            (bounds.x + bounds.width, bounds.y),
            (bounds.x, bounds.y + bounds.height),
            (bounds.x + bounds.width, bounds.y + bounds.height),
        ]
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
        if corners
            .iter()
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return None;
        }
        let x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let right = corners
            .iter()
            .map(|p| p.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = corners
            .iter()
            .map(|p| p.1)
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
    /// What `id`'s clip chain leaves of the viewport, in the coordinates of
    /// its accessible bounds: the overflow containers that reach it, each
    /// where it sits after the scroll above it. `None` when none does. Paint
    /// and hit testing cut the box by the same chain.
    pub(super) fn accessibility_clip(
        &self,
        id: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        let clip = self.layout_clip_chain(id).next()?;
        self.accessibility_clip_region(clip, memo)
    }

    fn accessibility_clip_region(
        &self,
        clip: StableNodeId,
        memo: &mut ProjectionMemo,
    ) -> Option<LayoutBox> {
        if let Some(region) = memo.clips.get(&clip) {
            return Some(*region);
        }
        let layout = self.component_layout_box(clip)?;
        let (x, y, width, height) = self.hit_motion_layout(clip).overflow_clip_box(
            layout.x,
            layout.y,
            layout.width,
            layout.height,
        )?;
        let own = self.accessibility_viewport_bounds(
            clip,
            LayoutBox {
                x,
                y,
                width,
                height,
            },
            memo,
        )?;
        let region = match self
            .layout_clip_chain(clip)
            .next()
            .and_then(|outer| self.accessibility_clip_region(outer, memo))
        {
            Some(outer) => {
                let left = own.x.max(outer.x);
                let top = own.y.max(outer.y);
                LayoutBox {
                    x: left,
                    y: top,
                    width: ((own.x + own.width).min(outer.x + outer.width) - left).max(0.0),
                    height: ((own.y + own.height).min(outer.y + outer.height) - top).max(0.0),
                }
            }
            None => own,
        };
        memo.clips.insert(clip, region);
        Some(region)
    }
}
