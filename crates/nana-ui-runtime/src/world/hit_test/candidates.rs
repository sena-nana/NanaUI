//! World hit queries: paint-order keys, presentation and menu surfaces, the
//! candidate list and the topmost hit.

use super::*;

/// Paint order of one hit candidate: its stacking groups from the root down,
/// each a z-index and the length of the sibling-order path that reaches it.
///
/// Every group's path is a prefix of the one path, so the key is that path
/// plus the group cut points. It is compared as the list of `(z, path
/// prefix)` pairs it stands for, lexicographically. Kept as reusable buffers
/// so picking the topmost of several candidates allocates nothing once warm.
#[derive(Debug, Default, Clone)]
pub(in crate::world) struct HitPaintKey {
    groups: Vec<(i32, usize)>,
    pub(super) path: Vec<usize>,
    pub(super) chain: Vec<StableNodeId>,
}

impl HitPaintKey {
    pub(super) fn clear(&mut self) {
        self.groups.clear();
        self.path.clear();
        self.chain.clear();
    }
}

impl PartialEq for HitPaintKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for HitPaintKey {}

impl PartialOrd for HitPaintKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HitPaintKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        for (&(z, len), &(other_z, other_len)) in self.groups.iter().zip(&other.groups) {
            let order = z
                .cmp(&other_z)
                .then_with(|| self.path[..len].cmp(&other.path[..other_len]));
            if order.is_ne() {
                return order;
            }
        }
        self.groups.len().cmp(&other.groups.len())
    }
}

/// Scratch for [`UiWorld::hit_test`]: the candidates at the point and the
/// best and current paint keys, reused across queries.
#[derive(Debug, Default)]
pub(in crate::world) struct HitScratch {
    pub(super) candidates: Vec<StableNodeId>,
    pub(super) best: HitPaintKey,
    pub(super) current: HitPaintKey,
}

impl UiWorld {
    fn overlay_mount_visual_parent(
        &self,
        id: StableNodeId,
        overlay_surface: bool,
    ) -> Option<crate::VisualParent> {
        if overlay_surface {
            match self.nodes.visual(id) {
                Some(StandardVisual::MenuSurface {
                    overlay: Some(overlay),
                    ..
                }) => Some(self.resolve_overlay_mount(id, overlay.overlay_mount)),
                Some(StandardVisual::Select { overlay_mount, .. }) => {
                    Some(self.resolve_overlay_mount(id, *overlay_mount))
                }
                _ => None,
            }
        } else if let Some(mount) = self.overlay_mount(id) {
            Some(self.resolve_overlay_mount(id, mount))
        } else {
            self.parent_triggered_overlay_id(id)
                .map(crate::VisualParent::Overlay)
        }
    }

    // Fixed branches are separate spatial roots unless `OverlayMount` mounts
    // them into an enclosing modal, `TopLayer`, or inline under their parent.
    // Hit order uses the same visual parent chain as `UiScene`, so floating
    // overlays match paint stacking inside and outside modals.
    fn hit_paint_key(
        &self,
        index: &HitIndex,
        target: StableNodeId,
        x: f32,
        y: f32,
        key: &mut HitPaintKey,
    ) {
        key.clear();
        let mut dom = Some(target);
        while let Some(id) = dom {
            key.chain.push(id);
            dom = self.parent_id(id);
        }
        for &id in key.chain.iter().rev() {
            key.path
                .push(index.entries.get(&id).map_or(0, |entry| entry.entry.order));
        }
        let dom_len = |id: StableNodeId| -> usize {
            key.chain
                .iter()
                .position(|&dom_id| dom_id == id)
                .map_or(0, |pos| key.chain.len() - pos)
        };
        let triggered_surface = self.triggered_overlays.contains(&target)
            && self.hanging_surface_contains(target, x, y);
        let menu_hit = !triggered_surface && self.menu_region_contains(index, target, x, y);
        let select_menu = menu_hit && self.select_menus.contains(&target);
        let mut steps: Vec<(StableNodeId, bool, Option<i32>)> = Vec::new();
        let mut cursor = Some((target, triggered_surface || select_menu));
        let mut remaining = self.nodes.len().saturating_add(1);
        while let Some((id, overlay_surface)) = cursor {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            if overlay_surface {
                let base_z = if select_menu && id == target {
                    self.hit_motion_layout(id)
                        .z_index
                        .unwrap_or(0)
                        .max(crate::popover::MENU_OVERLAY_Z_INDEX)
                } else {
                    crate::popover::MENU_OVERLAY_Z_INDEX
                };
                match self
                    .overlay_mount_visual_parent(id, true)
                    .unwrap_or(crate::VisualParent::Root)
                {
                    crate::VisualParent::Root => {
                        steps.push((id, true, Some(base_z)));
                        break;
                    }
                    crate::VisualParent::TopLayer => {
                        steps.push((
                            id,
                            true,
                            Some(base_z.max(crate::popover::TOP_LAYER_Z_INDEX)),
                        ));
                        break;
                    }
                    crate::VisualParent::Modal(modal_id) => {
                        let modal_z = self.hit_motion_layout(modal_id).z_index.unwrap_or(0);
                        steps.push((id, true, Some(modal_z.saturating_add(base_z))));
                        cursor = Some((modal_id, false));
                    }
                    crate::VisualParent::Node(parent_id) => {
                        steps.push((id, true, Some(base_z)));
                        cursor = Some((parent_id, false));
                    }
                    crate::VisualParent::Overlay(menu_id) => {
                        steps.push((id, true, Some(base_z)));
                        cursor = Some((menu_id, true));
                    }
                }
                continue;
            }
            let explicit_vp = self.overlay_mount_visual_parent(id, false);
            let style_z = self.hit_motion_layout(id).z_index;
            match explicit_vp {
                Some(crate::VisualParent::Root) => {
                    steps.push((
                        id,
                        false,
                        Some(
                            style_z
                                .unwrap_or(0)
                                .max(crate::popover::MENU_OVERLAY_Z_INDEX),
                        ),
                    ));
                    break;
                }
                Some(crate::VisualParent::TopLayer) => {
                    steps.push((
                        id,
                        false,
                        Some(style_z.unwrap_or(0).max(crate::popover::TOP_LAYER_Z_INDEX)),
                    ));
                    break;
                }
                Some(crate::VisualParent::Modal(modal_id)) => {
                    let modal_z = self.hit_motion_layout(modal_id).z_index.unwrap_or(0);
                    let z = modal_z.saturating_add(
                        style_z
                            .unwrap_or(0)
                            .max(crate::popover::MENU_OVERLAY_Z_INDEX),
                    );
                    steps.push((id, false, Some(z)));
                    cursor = Some((modal_id, false));
                }
                Some(crate::VisualParent::Node(parent_id)) => {
                    steps.push((id, false, None));
                    cursor = Some((parent_id, false));
                }
                Some(crate::VisualParent::Overlay(menu_id)) => {
                    steps.push((id, false, None));
                    cursor = Some((menu_id, true));
                }
                None => {
                    steps.push((id, false, None));
                    if self.hit_motion_layout(id).position == PositionSpec::Fixed {
                        break;
                    }
                    cursor = self.parent_id(id).map(|parent| (parent, false));
                }
            }
        }
        let mut inherited_z = 0;
        for &(id, overlay_surface, z_override) in steps.iter().rev() {
            let path_len = dom_len(id);
            if overlay_surface {
                inherited_z = z_override
                    .unwrap_or(crate::popover::MENU_OVERLAY_Z_INDEX)
                    .max(inherited_z);
                key.groups.push((inherited_z, path_len));
                continue;
            }
            let node = self.record(id);
            let style = self.hit_motion_layout(id);
            inherited_z = z_override.unwrap_or_else(|| style.z_index.unwrap_or(inherited_z));
            let children = node
                .hierarchy
                .children
                .iter()
                .any(|child| self.is_mounted(*child));
            let opacity = self
                .sampled_compositor_opacity(id)
                .unwrap_or_else(|| style.opacity.unwrap_or(1.0))
                .clamp(0.0, 1.0);
            let filter_group = style
                .paint
                .filter
                .filter(|filter| !filter.is_identity())
                .is_some_and(|filter| {
                    filter.blur_radius > 0.0
                        || filter.drop_shadow.is_some()
                        || children
                        || !node.text.value.is_empty()
                        || self.nodes.custom_render(id).is_some()
                });
            // A scroll view whose bars can show groups what it holds, so its
            // bars paint over it: hit order cuts there too.
            let scrollbar_chrome = matches!(
                self.nodes.visual(id),
                Some(StandardVisual::Scrollbar { visibility, .. })
                    if *visibility != nana_ui_core::ScrollbarVisibility::Hidden
            );
            let modal_frame = matches!(
                self.nodes.visual(id),
                Some(StandardVisual::ModalFrame { .. })
            );
            let group = modal_frame
                || (children
                    && (style.creates_paint_stacking_context()
                        || (opacity > 0.0 && opacity < 1.0)
                        || scrollbar_chrome))
                || filter_group
                || !style.paint.mix_blend.is_normal();
            if group || id == target {
                let z = if id == target && !group && menu_hit {
                    inherited_z.max(crate::popover::MENU_OVERLAY_Z_INDEX)
                } else {
                    inherited_z
                };
                key.groups.push((z, path_len));
            }
        }
    }

    /// Whether `target`'s menu region (an open select menu, a hover card's
    /// connector strip) is at the window point `(x, y)`.
    fn menu_region_contains(&self, index: &HitIndex, target: StableNodeId, x: f32, y: f32) -> bool {
        let has_overlay = self
            .document_of(target)
            .is_some_and(|document| self.has_compositor_transform_overlay_for(document));
        index.entries.get(&target).is_some_and(|node| {
            node.entry.menu.is_some_and(|menu| {
                if has_overlay {
                    // The projection maps page coordinates; the menu is in
                    // the entry's own.
                    let at = self.layout_origin(target).unwrap_or_default();
                    let menu = LayoutBox {
                        x: menu.x + at[0],
                        y: menu.y + at[1],
                        ..menu
                    };
                    self.layout_projection_transform(target)
                        .is_some_and(|(transform, persp)| {
                            transformed_contains(menu, transform, persp, x, y)
                        })
                } else {
                    index.own_point(target, x, y).is_some_and(|(x, y)| {
                        transformed_contains(menu, node.entry.transform, node.entry.persp, x, y)
                    })
                }
            })
        })
    }

    /// Whether an open select menu of `index`'s document is at `(x, y)`, so
    /// the hit there is ranked by paint order rather than taken as the tree
    /// walk first meets one.
    fn select_menu_at(&self, index: &HitIndex, document: DocumentId, x: f32, y: f32) -> bool {
        !self.select_menus.is_empty()
            && self.select_menus.iter().any(|id| {
                self.document_of(*id) == Some(document)
                    && self.menu_region_contains(index, *id, x, y)
            })
    }

    pub(super) fn visit_presentation_roots(
        &self,
        index: &HitIndex,
        x: f32,
        y: f32,
        emit: &mut impl FnMut(StableNodeId) -> bool,
    ) -> bool {
        for slot in (0..index.roots.len()).rev() {
            let Some(id) = index.roots[slot] else {
                continue;
            };
            let base = (IDENTITY_AFFINE, [0.0, 0.0]);
            let base = if index
                .entries
                .get(&id)
                .is_some_and(|root| root.entry.escaped)
            {
                self.clip_bridge_transform(id, None, base, true)
            } else {
                base
            };
            if self.visit_presentation_hits(index, id, base, false, x, y, emit) {
                return true;
            }
        }
        false
    }

    fn visit_presentation_hits(
        &self,
        index: &HitIndex,
        id: StableNodeId,
        parent_hit: ([f32; 6], [f32; 2]),
        parent_blocks_3d: bool,
        x: f32,
        y: f32,
        emit: &mut impl FnMut(StableNodeId) -> bool,
    ) -> bool {
        let Some(indexed) = index.entries.get(&id) else {
            return false;
        };
        if self.motion_blocks_input(id) || self.inline_box_omitted(id) {
            return false;
        }
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let style = self.hit_motion_layout(id);
        let (parent_hit, parent_blocks_3d) = if style.position == PositionSpec::Fixed {
            (self.hanging_branch_transform(id), false)
        } else {
            (parent_hit, parent_blocks_3d)
        };
        let Some(layout) = self.component_layout_box(id) else {
            return false;
        };
        let local = self.input_local_scene_transform(id, &style, layout, parent_blocks_3d);
        let (transform, persp) = then_hit(parent_hit, self.with_sticky_translation(id, local));
        // The entry keeps its clips, menu and box in its own coordinates;
        // these transforms map the page's.
        let to_page = ([1.0, 0.0, 0.0, 1.0, layout.x, layout.y], [0.0, 0.0]);
        let own = then_hit((transform, persp), to_page);
        let parent_own = then_hit(parent_hit, to_page);
        let node = &indexed.entry;
        if !node
            .self_clips
            .iter()
            .all(|(bounds, _)| transformed_contains(*bounds, parent_own.0, parent_own.1, x, y))
        {
            return false;
        }
        if !path_clips_contain(&node.path_clips, own.0, own.1, x, y) {
            return false;
        }
        let menu_hit = node
            .menu
            .is_some_and(|menu| transformed_contains(menu, own.0, own.1, x, y));
        let children_ok = node
            .child_clips
            .iter()
            .all(|(bounds, _)| transformed_contains(*bounds, own.0, own.1, x, y));
        let child_transform = then_hit(
            (transform, persp),
            (
                [
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    -record.scroll_offset.x,
                    -record.scroll_offset.y,
                ],
                [0.0, 0.0],
            ),
        );
        let child_blocks_3d = parent_blocks_3d || style.fails_closed_3d_context();
        let menu_z = node.z_index.max(1000);
        let mut emitted_menu = !menu_hit;
        if children_ok {
            for slot in (0..indexed.children.len()).rev() {
                let Some(child) = indexed.children[slot] else {
                    continue;
                };
                if !emitted_menu
                    && index
                        .entries
                        .get(&child)
                        .is_some_and(|child| child.entry.z_index <= menu_z)
                {
                    emitted_menu = true;
                    if emit(id) {
                        return true;
                    }
                }
                // An escaped child takes the transforms of the ancestors it
                // passes over, but none of their scroll.
                let parent_hit = if index
                    .entries
                    .get(&child)
                    .is_some_and(|child| child.entry.escaped)
                {
                    self.clip_bridge_transform(child, Some(id), child_transform, true)
                } else {
                    child_transform
                };
                if self.visit_presentation_hits(
                    index,
                    child,
                    parent_hit,
                    child_blocks_3d,
                    x,
                    y,
                    emit,
                ) {
                    return true;
                }
            }
        }
        if !emitted_menu && emit(id) {
            return true;
        }
        if node.hittable
            && transformed_contains(node.layout, own.0, own.1, x, y)
            && painter_accepts(node, node.layout, own.0, own.1, x, y)
        {
            return emit(id);
        }
        false
    }

    /// Every hit at `(x, y)`, topmost first.
    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        self.hit_test_queries.set(self.hit_test_queries.get() + 1);
        let mut candidates = Vec::new();
        let Some(forest) = self.collect_hit_candidates(document, x, y, &mut candidates) else {
            return candidates;
        };
        if forest.viewport_hit_at(x, y)
            || self.select_menu_at(forest, document, x, y)
            || self.has_compositor_transform_overlay_for(document)
            || self.hanging_surfaces_at(document, x, y).next().is_some()
        {
            candidates.sort_by_cached_key(|id| {
                let mut key = HitPaintKey::default();
                self.hit_paint_key(forest, *id, x, y, &mut key);
                std::cmp::Reverse(key)
            });
        }
        candidates
    }

    /// Hits at `(x, y)` in visit order, motion-blocked ones dropped. `None`
    /// when the document has no hit index.
    fn collect_hit_candidates(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
        candidates: &mut Vec<StableNodeId>,
    ) -> Option<&HitIndex> {
        candidates.clear();
        let forest = self.hit_test_index.get(&document)?;
        if self.has_compositor_transform_overlay_for(document) {
            self.visit_presentation_roots(forest, x, y, &mut |id| {
                candidates.push(id);
                false
            });
        } else {
            forest.visit_roots(x, y, &mut |id| {
                candidates.push(id);
                false
            });
        }
        // An open menu's surface is not in its trigger's branch of the
        // index: it hangs above the page, out of the trigger's clips.
        for id in self.hanging_surfaces_at(document, x, y) {
            if !candidates.contains(&id) {
                candidates.push(id);
            }
        }
        candidates.retain(|id| !self.motion_blocks_input(*id));
        Some(forest)
    }

    /// Open triggered menus (Popover, ActionMenu, HoverCard) of `document`
    /// whose surface is at `(x, y)`. The surface paints in the root stacking
    /// context at its content's level, so it takes the pointer there too:
    /// over the page, under its own items.
    fn hanging_surfaces_at(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> impl Iterator<Item = StableNodeId> + '_ {
        self.triggered_overlays.iter().copied().filter(move |id| {
            self.document_of(*id) == Some(document)
                && self.record(*id).interaction.pointer_events
                && self.hanging_surface_contains(*id, x, y)
        })
    }

    pub(crate) fn hanging_surface_contains(&self, id: StableNodeId, x: f32, y: f32) -> bool {
        crate::popover::hanging_surface(self, id).is_some_and(|surface| {
            match self.hanging_surface_transform(id) {
                Some((transform, persp)) => transformed_contains(surface, transform, persp, x, y),
                None => surface.contains(x, y),
            }
        })
    }

    /// Hit-test queries answered since the world was created: one per
    /// [`Self::hit_test`] or [`Self::hit_test_candidates`] call. Input routing
    /// is budgeted against it (at most one per uncaptured pointer event).
    pub fn hit_test_queries(&self) -> u64 {
        self.hit_test_queries.get()
    }
}

impl UiWorld {
    /// Topmost hit at `(x, y)`.
    ///
    /// Walks the same order as [`Self::hit_test_candidates`] but returns at the
    /// first hit, so pointer dispatch on every move does not collect and then
    /// discard the full candidate list.
    pub fn hit_test(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        self.hit_test_queries.set(self.hit_test_queries.get() + 1);
        if !self.closing_surfaces.is_empty()
            || self.hit_test_index.get(&document).is_some_and(|index| {
                index.viewport_hit_at(x, y) || self.select_menu_at(index, document, x, y)
            })
            || self.hanging_surfaces_at(document, x, y).next().is_some()
        {
            return self.topmost_candidate(document, x, y);
        }
        let forest = self.hit_test_index.get(&document)?;
        let mut found = None;
        if self.has_compositor_transform_overlay_for(document) {
            self.visit_presentation_roots(forest, x, y, &mut |id| {
                found = Some(id);
                true
            });
        } else {
            forest.visit_roots(x, y, &mut |id| {
                found = Some(id);
                true
            });
        }
        found
    }

    /// The first of [`Self::hit_test_candidates`] without collecting or
    /// sorting them: the highest paint key wins, the earliest visited on a
    /// tie (the stable sort's first). Reuses the world's scratch, so a pointer
    /// moving inside a scroll viewport allocates nothing once warm.
    fn topmost_candidate(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        let Ok(mut scratch) = self.hit_scratch.try_borrow_mut() else {
            // Only a hit query made from inside another one lands here.
            return self.hit_test_candidates(document, x, y).into_iter().next();
        };
        let HitScratch {
            candidates,
            best,
            current,
        } = &mut *scratch;
        let forest = self.collect_hit_candidates(document, x, y, candidates)?;
        if !forest.viewport_hit_at(x, y)
            && !self.select_menu_at(forest, document, x, y)
            && !self.has_compositor_transform_overlay_for(document)
            && self.hanging_surfaces_at(document, x, y).next().is_none()
        {
            return candidates.first().copied();
        }
        let mut top = None;
        for &id in candidates.iter() {
            self.hit_paint_key(forest, id, x, y, current);
            if top.is_none() || *current > *best {
                std::mem::swap(best, current);
                top = Some(id);
            }
        }
        top
    }
}
