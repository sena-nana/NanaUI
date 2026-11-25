//! Building hit entries and forests from the retained tree, and the full
//! rebuild.

use super::*;

/// Empty strip between trigger and overlay surface on the axis that separates them.
fn overlay_connector_box(trigger: LayoutBox, surface: LayoutBox) -> Option<LayoutBox> {
    const MIN: f32 = 0.5;
    let x = trigger.x.min(surface.x);
    let y = trigger.y.min(surface.y);
    let right = (trigger.x + trigger.width).max(surface.x + surface.width);
    let bottom = (trigger.y + trigger.height).max(surface.y + surface.height);
    let trigger_right = trigger.x + trigger.width;
    let trigger_bottom = trigger.y + trigger.height;
    let surface_right = surface.x + surface.width;
    let surface_bottom = surface.y + surface.height;
    let below = surface.y - trigger_bottom;
    let above = trigger.y - surface_bottom;
    let rightward = surface.x - trigger_right;
    let leftward = trigger.x - surface_right;
    [
        (
            below,
            LayoutBox {
                x,
                y: trigger_bottom,
                width: right - x,
                height: below,
            },
        ),
        (
            above,
            LayoutBox {
                x,
                y: surface_bottom,
                width: right - x,
                height: above,
            },
        ),
        (
            rightward,
            LayoutBox {
                x: trigger_right,
                y,
                width: rightward,
                height: bottom - y,
            },
        ),
        (
            leftward,
            LayoutBox {
                x: surface_right,
                y,
                width: leftward,
                height: bottom - y,
            },
        ),
    ]
    .into_iter()
    .filter(|(gap, _)| *gap > MIN)
    .reduce(|best, next| if next.0 > best.0 { next } else { best })
    .map(|(_, box_)| box_)
}

impl UiWorld {
    /// Overlay children receive `position: fixed` only from [`Self::effective_layout_style`].
    pub(in crate::world) fn hit_motion_layout(
        &self,
        id: StableNodeId,
    ) -> Arc<nana_ui_core::LayoutStyle> {
        if !self.has_layout_length_overlays()
            && self.overlays_a11y.overlay_host_nodes.is_empty()
            && self.invalidation.z_index_nodes == 0
            && self.tree.detached_mounted.is_empty()
            && !self.nodes.has_widget_visuals()
            && self.overlays_a11y.triggered_overlays.is_empty()
            && self.invalidation.menu_surfaces == 0
            && self.invalidation.modal_frames == 0
            && self.invalidation.list_leading_visuals == 0
        {
            let record = self.record(id);
            if record.mount == crate::MountState::Mounted
                && !record.resolved_layout.omits_box()
                && !record.resolved_layout.has_logical_box_edges()
            {
                return Arc::clone(&record.resolved_layout);
            }
        }
        self.motion_layout(id, &self.effective_layout_style(id))
    }

    /// Gap between a HoverCard trigger and its overlay; that strip is outside the subtree.
    fn overlay_connector_hit(&self, id: StableNodeId) -> Option<LayoutBox> {
        let parent = self.parent_id(id)?;
        let visual = self.widget_visual_as::<crate::MenuSurfaceVisual>(parent)?;
        if visual.kind != crate::MenuSurfaceKind::HoverCard || !visual.open {
            return None;
        }
        let overlay = visual.overlay?;
        if self.record(parent).hierarchy.children.first().copied() != Some(id) {
            return None;
        }
        // The card is viewport-fixed; the trigger is wherever the
        // scroll offsets above it put its layout box.
        let trigger = self.component_layout_box(parent)?;
        overlay_connector_box(
            self.project_input_bounds(parent, trigger)
                .unwrap_or(trigger),
            crate::popover::overlay_surface_from_items(self, parent, Some(&overlay)),
        )
    }

    /// Build hit entries for `seeds` and their visible descendants. Each seed
    /// carries the accumulated transform of its parent, so a scoped patch can
    /// resume from an existing entry instead of walking from the document root.
    ///
    /// `order` is assigned per sibling group from `Hierarchy` position. It is
    /// only ever compared between siblings, so a spliced subtree stays sortable
    /// against untouched siblings without renumbering the document.
    pub(super) fn build_hit_entries(&self, seeds: Vec<HitSeed>) -> Vec<BuiltHit> {
        let (mut built, mut deferred) = self.build_hit_entries_deferring(seeds);
        // An escaped box whose clip parent is no node hangs as a root of its
        // own, from the transforms above it but none of their scroll.
        while let Some(id) = deferred.pop() {
            if self.layout_clip_parent(id).is_some() {
                continue;
            }
            let (transform, _) =
                self.clip_bridge_transform(id, None, (IDENTITY_AFFINE, [0.0, 0.0]), false);
            let (more, later) = self.build_hit_entries_deferring(vec![(id, transform, None)]);
            let base = built.len();
            built.extend(more.into_iter().map(|mut node| {
                node.parent = node.parent.map(|parent| parent + base);
                node
            }));
            if let Some(root) = built.get_mut(base) {
                root.entry.escaped = true;
                root.entry.pinned = true;
                root.entry.order = self
                    .parent_id(id)
                    .and_then(|parent| self.child_index(parent, id))
                    .unwrap_or_default();
            }
            deferred.extend(later);
        }
        built
    }

    /// [`Self::build_hit_entries`], leaving out the escaped boxes whose clip
    /// parent this build does not reach (it was seeded below it): those are
    /// returned for the caller to hang where they belong.
    fn build_hit_entries_deferring(
        &self,
        seeds: Vec<HitSeed>,
    ) -> (Vec<BuiltHit>, Vec<StableNodeId>) {
        self.build_hit_entries_reaching(seeds, true)
    }

    /// [`Self::build_hit_entries_deferring`]; with `descend` false, the
    /// seeds' own entries alone.
    pub(super) fn build_hit_entries_reaching(
        &self,
        seeds: Vec<HitSeed>,
        descend: bool,
    ) -> (Vec<BuiltHit>, Vec<StableNodeId>) {
        let mut stack = seeds
            .into_iter()
            .enumerate()
            .rev()
            .map(|(position, (id, transform, hit_parent))| {
                (
                    id,
                    (transform, [0.0f32, 0.0]),
                    None::<usize>,
                    position,
                    self.parent_used_pointer_events(id),
                    false,
                    None::<nana_ui_core::ParentPerspective>,
                    false,
                    hit_parent,
                )
            })
            .collect::<Vec<_>>();
        let mut built: Vec<BuiltHit> = Vec::new();
        // What each built entry hands its children: an escaped box resumes
        // from its clip parent's.
        let mut child_states: Vec<(
            ([f32; 6], [f32; 2]),
            bool,
            Option<nana_ui_core::ParentPerspective>,
            bool,
        )> = Vec::new();
        let mut deferred = Vec::new();
        let mut memo = AncestorMemo::default();
        let visuals = self.nodes.has_widget_visuals();
        let plain_layout = !self.has_layout_length_overlays()
            && self.overlays_a11y.overlay_host_nodes.is_empty()
            && self.invalidation.z_index_nodes == 0
            && self.tree.detached_mounted.is_empty()
            && self.invalidation.clip_visuals == 0
            && self.overlays_a11y.select_menus.is_empty()
            && self.overlays_a11y.triggered_overlays.is_empty()
            && !visuals;
        while let Some((
            id,
            parent_hit,
            parent,
            position,
            parent_used_pe,
            parent_blocks_3d,
            parent_persp,
            parent_backface_hidden,
            hit_parent,
        )) = stack.pop()
        {
            if self.motion_blocks_input(id) || self.inline_box_omitted(id) {
                continue;
            }
            let (
                visible,
                specified_pointer,
                computed_pointer,
                local_layout,
                (frame, tree_parent),
                resolved_layout,
                default_box,
                scroll,
                children,
                interaction,
                painted,
                (space, resolved_at),
            ) = {
                let record = self.record(id);
                let computed = record.resolved.0.as_ref();
                let has_box = computed.box_visible
                    && (computed.visible || !record.hierarchy.children.is_empty());
                if !has_box {
                    continue;
                }
                let default_box = plain_layout
                    && record.mount == crate::MountState::Mounted
                    && crate::components::layout_style_is_default(&record.resolved_layout);
                let plain_style = !default_box
                    && plain_layout
                    && record.mount == crate::MountState::Mounted
                    && !record.resolved_layout.omits_box()
                    && !record.resolved_layout.has_logical_box_edges();
                (
                    computed.visible,
                    record.style.layout.pointer_events,
                    computed.pointer_events,
                    self.nodes.local_layout(id),
                    (self.nodes.layout_frame(id), record.hierarchy.parent),
                    plain_style.then(|| Arc::clone(&record.resolved_layout)),
                    default_box,
                    self.nodes.scroll_offset(id),
                    Arc::clone(&record.hierarchy.children),
                    record.interaction,
                    record.style.painter.is_some()
                        || !self.invalidation.painter_overrides.is_empty(),
                    (record.space, Arc::as_ptr(&record.resolved_layout)),
                )
            };
            // Every entry is in its own box's coordinates, which stand an
            // offset from its hit parent's: the local box itself where that
            // is what layout measures it from.
            let layout = LayoutBox {
                x: 0.0,
                y: 0.0,
                ..local_layout
            };
            let offset_from = |hit_parent: Option<StableNodeId>| match hit_parent {
                Some(owner)
                    if frame == crate::LayoutFrame::Parent && tree_parent == Some(owner) =>
                {
                    ([local_layout.x, local_layout.y], false)
                }
                Some(owner) => (
                    self.layout_box_from(owner, id)
                        .map_or([local_layout.x, local_layout.y], |b| [b.x, b.y]),
                    true,
                ),
                None if frame == crate::LayoutFrame::Page => {
                    ([local_layout.x, local_layout.y], false)
                }
                None => (self.layout_origin(id).unwrap_or_default(), true),
            };
            // A mounted default box under an identity ancestor has no transform,
            // clip, scroll, or overlay. Skip the style arc and the geometry
            // helpers; descendant seeds stay on the same identity transform.
            if default_box
                && scroll == crate::ScrollOffset::default()
                && parent_hit.0 == IDENTITY_AFFINE
                && parent_hit.1 == [0.0, 0.0]
                && !parent_blocks_3d
                && parent_persp.is_none()
                && !parent_backface_hidden
                && !painted
                && self.invalidation.confirm_modals == 0
            {
                let used_pe = PointerEventsSpec::inherit_from(specified_pointer, parent_used_pe);
                let hittable = visible
                    && interaction.pointer_events
                    && used_pe.hittable()
                    && computed_pointer.hittable();
                let index = built.len();
                let (origin, pinned) = offset_from(hit_parent);
                built.push(BuiltHit {
                    entry: HitEntry {
                        id,
                        source_children: Arc::clone(&children),
                        layout,
                        origin,
                        pinned,
                        transform: IDENTITY_AFFINE,
                        persp: [0.0, 0.0],
                        local_children: false,
                        scroll: [0.0, 0.0],
                        self_clips: Vec::new(),
                        child_clips: Vec::new(),
                        path_clips: Vec::new(),
                        z_index: 0,
                        order: position,
                        hittable,
                        menu: None,
                        painter: None,
                        escaped: false,
                        children: Vec::new(),
                    },
                    parent,
                });
                child_states.push((parent_hit, false, None, false));
                if descend {
                    stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                        (
                            *child,
                            parent_hit,
                            Some(index),
                            position,
                            used_pe,
                            false,
                            None,
                            false,
                            Some(id),
                        )
                    }));
                }
                continue;
            }
            let motion_layout = if let Some(style) = resolved_layout {
                style
            } else {
                self.hit_motion_layout(id)
            };
            let node_style = motion_layout.as_ref();
            // The space of the style the entry is built from: the node's own
            // where that is its resolved style, else that of the effective
            // style a motion overlay or an overlay branch made of it.
            let space = if std::ptr::eq(node_style, resolved_at) {
                space
            } else {
                crate::node_space::NodeSpace::of_style(node_style)
            };
            // This is a projection root, not a Runtime reparent. Fixed layout
            // is viewport-relative; ancestors still control lifecycle and
            // inherited pointer-events, but cannot clip or scroll this branch.
            //
            // An absolute box that escapes overflow containers hangs from its
            // clip parent instead (#201): their clips and scroll do not reach
            // it, the transforms between still do.
            let (
                parent_hit,
                parent,
                parent_blocks_3d,
                parent_persp,
                parent_backface_hidden,
                escaped,
                hit_parent,
            ) = if node_style.position == PositionSpec::Fixed {
                (
                    (IDENTITY_AFFINE, [0.0, 0.0]),
                    None,
                    false,
                    None,
                    false,
                    false,
                    None,
                )
            } else if let Some(tree_parent) = parent
                && node_style.position == PositionSpec::Absolute
                && let Some(anchor) = self.clip_escape(id)
            {
                let mut cursor = Some(tree_parent);
                while let Some(at) = cursor
                    && Some(built[at].entry.id) != anchor
                {
                    cursor = built[at].parent;
                }
                let Some(at) = cursor else {
                    deferred.push(id);
                    continue;
                };
                let (base, blocks_3d, base_persp, backface_hidden) = child_states[at];
                let anchor_id = built[at].entry.id;
                let escaped_persp = if Some(anchor_id) == self.parent_id(id) {
                    base_persp
                } else {
                    None
                };
                (
                    self.clip_bridge_transform_from(
                        id,
                        anchor,
                        base,
                        false,
                        self.layout_origin(anchor_id).unwrap_or_default(),
                    ),
                    Some(at),
                    blocks_3d,
                    escaped_persp,
                    backface_hidden,
                    true,
                    Some(anchor_id),
                )
            } else {
                (
                    parent_hit,
                    parent,
                    parent_blocks_3d,
                    parent_persp,
                    parent_backface_hidden,
                    false,
                    hit_parent,
                )
            };
            let (offset, offset_pinned) = offset_from(hit_parent);
            // A sticky box's offset is a translation before its own
            // transform, as its layout offset is: without a perspective it
            // is part of the origin, so sticking moves this entry alone and
            // what it holds keeps theirs. Under a perspective the entry
            // takes it into its transform below.
            let flat_parent = parent_hit.1 == [0.0, 0.0];
            let stuck = self.sticky_translation(id);
            let sticky = (flat_parent && stuck != [0.0, 0.0]).then_some(stuck);
            // The offset is a translation of what the parent hands down, so
            // it moves the box as a shift of the space it lands in, which
            // the children then keep. Under a perspective it does not
            // commute: the entry takes it into its transform.
            let (parent_hit, origin) = if flat_parent {
                let offset = match sticky {
                    Some([x, y]) => [offset[0] + x, offset[1] + y],
                    None => offset,
                };
                (parent_hit, linear_offset(parent_hit.0, offset))
            } else {
                (
                    then_hit(
                        parent_hit,
                        ([1.0, 0.0, 0.0, 1.0, offset[0], offset[1]], [0.0, 0.0]),
                    ),
                    [0.0, 0.0],
                )
            };
            // Component geometry is on the page; the entry's own box's
            // coordinates start where the box stands.
            let page_origin = || self.layout_origin(id).unwrap_or_default();
            let own_space = |rect: LayoutBox, [x, y]: [f32; 2]| LayoutBox {
                x: rect.x - x,
                y: rect.y - y,
                ..rect
            };
            let mut reads_page =
                offset_pinned || escaped || space.contains(crate::node_space::NodeSpace::STICKY);
            let shifted_persp = parent_persp.map(|pp| nana_ui_core::ParentPerspective {
                distance: pp.distance,
                origin: [
                    pp.origin[0] - offset[0] - stuck[0],
                    pp.origin[1] - offset[1] - stuck[1],
                ],
            });
            let local = if parent_blocks_3d && node_style.transforms.transform_3d.is_some() {
                (IDENTITY_AFFINE, [0.0, 0.0])
            } else {
                node_style
                    .world_scene_transform_with_perspective(
                        layout.x,
                        layout.y,
                        layout.width,
                        layout.height,
                        shifted_persp,
                    )
                    .unwrap_or((IDENTITY_AFFINE, [0.0, 0.0]))
            };
            let local = if flat_parent || stuck == [0.0, 0.0] {
                local
            } else {
                then_hit(([1.0, 0.0, 0.0, 1.0, stuck[0], stuck[1]], [0.0; 2]), local)
            };
            let (transform, persp) = then_hit(parent_hit, local);
            let child_blocks_3d = parent_blocks_3d || space.fails_closed_3d();
            let child_persp = crate::world::presentation::child_parent_perspective(
                node_style,
                layout,
                scroll,
                child_blocks_3d,
            );
            let self_backface_hidden = parent_backface_hidden
                || (node_style.transforms.backface_visibility
                    == nana_ui_core::BackfaceVisibility::Hidden
                    && nana_ui_core::is_projective_backface(
                        transform,
                        persp,
                        layout.width * 0.5,
                        layout.height * 0.5,
                    ));
            // A box that scales, rotates, skews or flips in 2D hands its
            // children its own layout coordinates: nothing of its transform
            // or of what is above it reaches their entries. Under a
            // perspective or in a 3D context they keep the composed one.
            let frames_itself = space.hit_frames_itself(&local);
            #[cfg(any(test, feature = "layout-verify"))]
            assert_eq!(
                frames_itself,
                local.0[..4] != [1.0, 0.0, 0.0, 1.0] && local.1 == [0.0, 0.0],
                "{id:?}: its space {space:?} frames itself unlike its transform {local:?}"
            );
            let local_children = frames_itself
                && persp == [0.0, 0.0]
                && !child_blocks_3d
                && child_persp.is_none()
                && !self_backface_hidden;
            let child_transform = if local_children {
                (scroll_seed([scroll.x, scroll.y]), [0.0, 0.0])
            } else {
                then_hit(
                    (transform, persp),
                    ([1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y], [0.0, 0.0]),
                )
            };
            // The committed value, as pointer dispatch validates it: a commit
            // that turns it `none` withdraws entries before styles resolve.
            let used_pe = PointerEventsSpec::inherit_from(specified_pointer, parent_used_pe);
            let mut self_clips = Vec::new();
            let mut child_clips = Vec::new();
            let path_clips = path_clips_for(node_style, layout);
            if let Some((x, y, w, h)) =
                node_style.overflow_clip_box(layout.x, layout.y, layout.width, layout.height)
            {
                let overlay_menu = !self.overlays_a11y.triggered_overlays.is_empty()
                    && self
                        .widget_visual_as::<crate::MenuSurfaceVisual>(id)
                        .is_some_and(|v| v.open && v.overlay.is_some());
                if !overlay_menu {
                    child_clips.push((
                        LayoutBox {
                            x,
                            y,
                            width: w,
                            height: h,
                        },
                        transform,
                    ));
                }
            }
            if self.invalidation.clip_visuals != 0 {
                if self
                    .widget_visual_ref(id)
                    .is_some_and(crate::WidgetVisualHandle::clips_descendants)
                {
                    child_clips.push((layout, transform));
                }
                let modal_frames = self.invalidation.modal_frames != 0;
                if modal_frames
                    && let Some(crate::ModalFrameGeometry { surface, .. }) =
                        self.modal_frame_geometry(id)
                {
                    reads_page = true;
                    child_clips.push((own_space(surface, page_origin()), transform));
                }
                if modal_frames
                    && let Some(parent_id) = self.parent_id(id)
                    && let Some(modal) = self.widget_visual_as::<crate::ModalFrameVisual>(parent_id)
                    && modal.slots.body == Some(id)
                    && let Some(crate::ModalFrameGeometry { body, .. }) =
                        self.modal_frame_geometry(parent_id)
                {
                    reads_page = true;
                    // Tested past the origin, which holds the sticky offset
                    // the parent's space does not: put it back.
                    let mut to_parent = parent_hit.0;
                    if let Some(sticky) = sticky {
                        let [x, y] = linear_offset(parent_hit.0, sticky);
                        to_parent[4] -= x;
                        to_parent[5] -= y;
                    }
                    self_clips.push((own_space(body, page_origin()), to_parent));
                }
            }
            let confirm_busy = self
                .confirm_action_effect(id)
                .is_some_and(|effect| effect.0);
            let hittable = visible
                && interaction.pointer_events
                && used_pe.hittable()
                && computed_pointer.hittable()
                && !confirm_busy
                && !self_backface_hidden;
            let menu = (hittable
                && (visuals
                    || !self.overlays_a11y.select_menus.is_empty()
                    || !self.overlays_a11y.triggered_overlays.is_empty()))
            .then(|| {
                if self.overlays_a11y.select_menus.contains(&id) {
                    self.select_geometry(id)
                        .and_then(|geometry| geometry.menu.map(|menu| menu.surface))
                } else if visuals || !self.overlays_a11y.triggered_overlays.is_empty() {
                    self.overlay_connector_hit(id)
                } else {
                    None
                }
            })
            .flatten()
            .map(|menu| own_space(menu, page_origin()));
            reads_page |= menu.is_some();
            let index = built.len();
            built.push(BuiltHit {
                entry: HitEntry {
                    id,
                    source_children: Arc::clone(&children),
                    layout,
                    origin,
                    pinned: reads_page,
                    transform,
                    persp,
                    local_children,
                    scroll: [scroll.x, scroll.y],
                    self_clips,
                    child_clips,
                    path_clips,
                    z_index: self.stacking_z_index_memo(id, &mut memo),
                    order: position,
                    hittable,
                    menu,
                    painter: if painted {
                        let [x, y] = page_origin();
                        self.painter_hit(id, LayoutBox { x, y, ..layout })
                    } else {
                        None
                    },
                    escaped,
                    children: Vec::new(),
                },
                parent,
            });
            child_states.push((
                child_transform,
                child_blocks_3d,
                child_persp,
                self_backface_hidden,
            ));
            // Sibling position is the sort key. Invisible siblings are skipped
            // and leave gaps, which is harmless because only relative order
            // between surviving siblings is ever compared.
            if !descend {
                continue;
            }
            stack.extend(children.iter().enumerate().rev().map(|(position, child)| {
                (
                    *child,
                    child_transform,
                    Some(index),
                    position,
                    used_pe,
                    child_blocks_3d,
                    child_persp,
                    self_backface_hidden,
                    Some(id),
                )
            }));
        }
        (built, deferred)
    }

    #[cfg(test)]
    pub(in crate::world) fn build_hit_forest(
        &self,
        seeds: Vec<(StableNodeId, [f32; 6])>,
    ) -> Vec<HitEntry> {
        let seeds = seeds
            .into_iter()
            .map(|(id, transform)| (id, transform, None))
            .collect();
        self.build_hit_forest_deferring(seeds).0
    }

    /// [`Self::build_hit_forest`], and the escaped boxes it left for the
    /// caller to hang from a clip parent outside the seeded branch.
    pub(super) fn build_hit_forest_deferring(
        &self,
        seeds: Vec<HitSeed>,
    ) -> (Vec<HitEntry>, Vec<StableNodeId>) {
        let (built, deferred) = self.build_hit_entries_deferring(seeds);
        let n = built.len();
        let mut parent_of = Vec::with_capacity(n);
        let mut entries = Vec::with_capacity(n);
        for node in built {
            parent_of.push(node.parent);
            entries.push(Some(node.entry));
        }
        for i in (0..n).rev() {
            if let Some(parent) = parent_of[i] {
                let child = entries[i].take().expect("child hit node");
                entries[parent]
                    .as_mut()
                    .expect("parent hit node")
                    .children
                    .push(child);
            }
        }
        (entries.into_iter().flatten().collect::<Vec<_>>(), deferred)
    }
}

impl UiWorld {
    /// Every build path funnels through here, so recording once keeps the
    /// sentinel honest for both a scoped patch and a full rebuild.
    pub(in crate::world) fn note_hit_nodes_built(&mut self, forest: &[HitEntry]) {
        let built = forest.iter().map(count_hit_entries).sum::<usize>();
        self.record_hit_test_nodes_built(built);
    }

    pub(in crate::world) fn record_hit_test_nodes_built(&mut self, built: usize) {
        if built > 0 {
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::HIT_TEST_NODES_REBUILT,
                built
            );
        }
        self.bump_last_counters(|counters| counters.record_hit_test_rebuild(built));
    }
}

impl UiWorld {
    /// Hidden containers keep structural entries for descendant clip, scroll and
    /// sibling order. Hidden leaves and omitted layout subtrees need no entry.
    pub(in crate::world) fn node_has_hit_box(&self, id: StableNodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| {
            node.resolved.0.box_visible
                && (node.resolved.0.visible || !node.hierarchy.children.is_empty())
        })
    }
}

impl UiWorld {
    /// Rebuild one document's event-time hit-test tree after scheduled input
    /// or layout work. Pointer dispatch walks that tree in z/order with clip
    /// early-out instead of flattening and sorting every query.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let roots = self.document_roots(document);
        let seeds = roots
            .iter()
            .copied()
            .map(|id| (id, IDENTITY_AFFINE, None))
            .collect::<Vec<_>>();
        let entries = self.build_hit_entries(seeds);
        let viewport_roots = entries
            .iter()
            .filter(|node| node.parent.is_none() && self.parent_id(node.entry.id).is_some())
            .map(|node| node.entry.id)
            .collect();
        self.record_hit_test_nodes_built(entries.len());
        let mut index = HitIndex::from_entries(entries);
        index.viewport_roots = viewport_roots;
        self.hit_index.hit_test_index.insert(document, index);
        self.hit_index.region_epoch = self.hit_index.region_epoch.wrapping_add(1);
        // Every entry of the document is built from current geometry.
        if !self.hit_index.non_scroll_hit_dirty.is_empty() {
            let mut held = std::mem::take(&mut self.hit_index.non_scroll_hit_dirty);
            held.retain(|&id| {
                self.nodes
                    .get(id)
                    .is_some_and(|record| record.document != document)
                    || self.input_pending(id)
            });
            self.hit_index.non_scroll_hit_dirty = held;
        }
        #[cfg(debug_assertions)]
        self.audit_hit_eligibility("rebuild");
    }
}

#[cfg(test)]
mod connector_tests {
    use super::{overlay_connector_box, transformed_point};
    use crate::LayoutBox;

    fn box_at(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn connector_picks_the_separating_axis() {
        let below =
            overlay_connector_box(box_at(10.0, 0.0, 20.0, 10.0), box_at(0.0, 16.0, 40.0, 8.0))
                .unwrap();
        assert_eq!(
            (below.x, below.y, below.width, below.height),
            (0.0, 10.0, 40.0, 6.0)
        );
        let above =
            overlay_connector_box(box_at(10.0, 20.0, 20.0, 10.0), box_at(0.0, 0.0, 40.0, 14.0))
                .unwrap();
        assert_eq!((above.y, above.height), (14.0, 6.0));
        let right =
            overlay_connector_box(box_at(0.0, 0.0, 20.0, 10.0), box_at(26.0, 0.0, 40.0, 10.0))
                .unwrap();
        assert_eq!((right.x, right.width), (20.0, 6.0));
        assert_eq!(
            overlay_connector_box(box_at(0.0, 0.0, 20.0, 10.0), box_at(4.0, 2.0, 40.0, 20.0)),
            None
        );
        let diagonal =
            overlay_connector_box(box_at(0.0, 0.0, 10.0, 10.0), box_at(14.0, 20.0, 10.0, 10.0))
                .unwrap();
        assert_eq!((diagonal.y, diagonal.height), (10.0, 10.0));
        let tied =
            overlay_connector_box(box_at(0.0, 0.0, 10.0, 10.0), box_at(20.0, 20.0, 10.0, 10.0))
                .unwrap();
        assert_eq!((tied.y, tied.height), (10.0, 10.0));
    }

    #[test]
    fn tiny_affine_scale_remains_hittable() {
        let Some((x, y)) = transformed_point(
            [1.0e-4, 0.0, 0.0, 1.0e-4, 0.0, 0.0],
            [0.0, 0.0],
            1.0e-4,
            1.0e-4,
        ) else {
            panic!("tiny affine transforms must remain invertible");
        };
        assert!((x - 1.0).abs() < 1.0e-5);
        assert!((y - 1.0).abs() < 1.0e-5);
    }
}
