//! AppContext scroll operations.

use super::*;

impl AppContext {
    /// Change follow mode and immediately move to the current end when enabled.
    /// Later layout passes also follow any newly measured content extent.
    pub fn set_scroll_follow_end(
        &mut self,
        scroll: Entity<ScrollView>,
        enabled: bool,
    ) -> Result<bool, FrameworkError> {
        self.update_scroll_view_visual(scroll, |view, _| {
            view.follow_end = enabled;
            if enabled {
                // An explicit return-to-latest command supersedes a deferred
                // reading anchor, even if no subsequent layout is necessary.
                view.pending_anchor = None;
            }
        })?;
        if enabled {
            self.apply_scroll_retention(scroll)
        } else {
            Ok(false)
        }
    }

    /// Capture a retained descendant row's position before changing list data.
    pub fn capture_scroll_anchor(
        &self,
        scroll: Entity<ScrollView>,
        row: StableNodeId,
    ) -> Result<Option<crate::ScrollAnchor>, FrameworkError> {
        self.read(scroll, |_| ())?;
        if !self.scroll_contains_row(scroll.id, row) {
            return Ok(None);
        }
        let Some(viewport) = self.world.component_layout_box(scroll.id) else {
            return Ok(None);
        };
        let Some(bounds) = self.world.component_layout_box(row) else {
            return Ok(None);
        };
        let offset = self.world.scroll_offset(scroll.id).unwrap_or_default();
        Ok(Some(crate::ScrollAnchor {
            row,
            viewport_y: bounds.y - viewport.y - offset.y,
        }))
    }

    /// Restore after the next layout. Removed rows retain the clamped current
    /// offset. An explicit anchor takes precedence over following the end once.
    pub fn restore_scroll_anchor(
        &mut self,
        scroll: Entity<ScrollView>,
        anchor: crate::ScrollAnchor,
    ) -> Result<(), FrameworkError> {
        if !anchor.viewport_y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        self.stop_scroll_motion(scroll.id);
        self.update_scroll_view_visual(scroll, |view, _| view.pending_anchor = Some(anchor))?;
        // Anchor restoration depends on measured geometry. Include this
        // container in the next scoped layout even when its style is unchanged.
        self.world.mark_layout(scroll.id);
        Ok(())
    }

    fn scroll_contains_row(&self, scroll: StableNodeId, row: StableNodeId) -> bool {
        let mut parent = self.world.parent_id(row);
        while let Some(id) = parent {
            if id == scroll {
                return true;
            }
            parent = self.world.parent_id(id);
        }
        false
    }

    pub(super) fn apply_scroll_retention(
        &mut self,
        scroll: Entity<ScrollView>,
    ) -> Result<bool, FrameworkError> {
        let (follow, anchor, dragging) = self.read(scroll, |view| {
            (
                view.follow_end,
                view.pending_anchor,
                view.dragging.is_some(),
            )
        })?;
        if dragging {
            return Ok(false);
        }
        let current = self.world.scroll_offset(scroll.id).unwrap_or_default();
        if let Some(anchor) = anchor {
            self.update_scroll_view_visual(scroll, |view, _| view.pending_anchor = None)?;
            if self.scroll_contains_row(scroll.id, anchor.row)
                && let (Some(viewport), Some(row)) = (
                    self.world.component_layout_box(scroll.id),
                    self.world.component_layout_box(anchor.row),
                )
            {
                return self.scroll_to(
                    scroll,
                    ScrollOffset {
                        x: current.x,
                        y: row.y - viewport.y - anchor.viewport_y,
                    },
                );
            }
            return Ok(false);
        }
        if follow && let Some(metrics) = self.world.scroll_metrics(scroll.id) {
            return self.scroll_to(
                scroll,
                ScrollOffset {
                    x: current.x,
                    y: metrics.max_offset().y,
                },
            );
        }
        Ok(false)
    }

    fn emit_user_scroll(&mut self, scroll: Entity<ScrollView>) -> Result<(), FrameworkError> {
        // A user scroll reveals auto-hiding bars even without subscribers.
        self.hold_scrollbars(scroll.id)?;
        if !self.has_scroll_listener::<crate::UserScroll>(scroll.id) {
            return Ok(());
        }
        let offset = self.world.scroll_offset(scroll.id).unwrap_or_default();
        let at_end = self
            .world
            .scroll_metrics(scroll.id)
            .is_some_and(|metrics| offset.y >= metrics.max_offset().y - 2.0);
        self.update(scroll, |_, cx| {
            cx.emit(crate::UserScroll { offset, at_end })
        })
    }

    fn has_scroll_listener<E: 'static>(&self, id: StableNodeId) -> bool {
        self.event_handlers
            .get(&(id, TypeId::of::<E>()))
            .is_some_and(|handlers| !handlers.is_empty())
    }

    // Hover/drag/retention own scrollbar state, not the style of a scrollport
    // borrowed by a Workspace region. Reuse the normal event/commit transaction.
    fn update_scroll_view_visual<R>(
        &mut self,
        entity: Entity<ScrollView>,
        update: impl FnOnce(&mut ScrollView, &mut ViewContext<'_, ScrollView>) -> R,
    ) -> Result<R, FrameworkError> {
        self.update_inner(entity, update, |scroll, world, mutations| {
            scroll.project_scrollbar_visual(entity.stable_id(), world, mutations);
        })
    }

    /// Send `scroll` a [`ScrollLaidOut`] after the next layout pass of its
    /// document.
    pub(crate) fn notify_laid_out(&mut self, scroll: Entity<ScrollView>) {
        self.laid_out_notices.insert(scroll.id);
    }

    pub fn scroll_to(
        &mut self,
        entity: Entity<ScrollView>,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.read(entity, |_| ())?;
        self.stop_scroll_motion(entity.id);
        self.apply_scroll_to(entity, offset)
    }

    // Wheel samples use the same write/event path without cancelling themselves.
    pub(super) fn apply_scroll_to(
        &mut self,
        entity: Entity<ScrollView>,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        let axes = self.read(entity, |scroll| scroll.axes)?;
        let offset = ScrollOffset {
            x: if matches!(axes, ScrollAxes::Horizontal | ScrollAxes::Both) {
                offset.x
            } else {
                0.0
            },
            y: if matches!(axes, ScrollAxes::Vertical | ScrollAxes::Both) {
                offset.y
            } else {
                0.0
            },
        };
        let offset = self.world.clamp_scroll_offset(entity.id, offset);
        if self.world.scroll_offset(entity.id) == Some(offset) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(entity.id, offset);
        self.world.commit(mutations)?;
        // No subscriber means no callback or observer work. The offset commit
        // already invalidates paint, hit testing and accessibility; skip the
        // empty event transaction. Subscribed callbacks still commit before
        // UserScroll reads the effective offset.
        if self.has_scroll_listener::<ScrollChanged>(entity.id) {
            self.update(entity, |_scroll, cx| {
                cx.emit(ScrollChanged { offset });
            })?;
        }
        Ok(true)
    }

    /// Scrolls `target` into view inside `scroll`, moving the minimum distance.
    ///
    /// Uses the layout boxes published by the last layout pass, so call it
    /// after layout has run for the frame that created or moved `target`.
    /// A target already fully visible does not move the container.
    ///
    /// This does not materialize virtualized rows: for an off-screen row in a
    /// `materialize_virtual_*` list, query the target offset and materialize
    /// first, then call this once the row has a layout box.
    ///
    /// `margin` keeps that many logical pixels of context on the leading and
    /// trailing edge where the container has room for it.
    ///
    /// Focusing a node also reveals it: [`AppContext::focus_node`] scrolls each
    /// ancestor scrollport by the minimum distance, with no extra margin.
    pub fn scroll_into_view(
        &mut self,
        scroll: Entity<ScrollView>,
        target: StableNodeId,
        margin: f32,
    ) -> Result<bool, FrameworkError> {
        let next = self.scroll_offset_for_target(scroll.id, target, margin)?;
        self.scroll_to(scroll, next)
    }

    /// After focus lands on `target`, bring it inside every ancestor scrollport.
    ///
    /// A scrollport between the target and an outer one contributes its own
    /// viewport. The outer content holds that viewport, not the unscrolled
    /// layout position of a descendant clipped inside it. Text editors keep
    /// their own caret reveal and are revealed as a whole.
    pub(super) fn reveal_focused_target(
        &mut self,
        target: StableNodeId,
    ) -> Result<(), FrameworkError> {
        let mut revealed = target;
        let mut current = self.world.parent_id(target);
        while let Some(id) = current {
            let parent = self.world.parent_id(id);
            if self.owns_text_scroll(id) {
                revealed = id;
            } else if self.is_scroll_view(id) || self.overflow_scrolls(id) {
                match self.scroll_node_into_view(id, revealed, 0.0) {
                    Ok(_) => {}
                    Err(FrameworkError::MissingView(_)) => {}
                    Err(error) => return Err(error),
                }
                revealed = id;
            }
            current = parent;
        }
        Ok(())
    }

    fn owns_text_scroll(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<TextArea>() || view.is::<TextInput>())
    }

    fn scroll_node_into_view(
        &mut self,
        scroll: StableNodeId,
        target: StableNodeId,
        margin: f32,
    ) -> Result<bool, FrameworkError> {
        let next = self.scroll_offset_for_target(scroll, target, margin)?;
        self.stop_scroll_motion(scroll);
        if self.is_scroll_view(scroll) {
            let moved = self.scroll_to(Entity::from_stable_id(scroll), next)?;
            if moved {
                self.hold_scrollbars(scroll)?;
            }
            return Ok(moved);
        }
        let Some((scrolls_x, scrolls_y)) = self.overflow_axes(scroll) else {
            return Ok(false);
        };
        let current = self.world.scroll_offset(scroll).unwrap_or_default();
        let next = self.world.clamp_scroll_offset(
            scroll,
            ScrollOffset {
                x: if scrolls_x { next.x } else { current.x },
                y: if scrolls_y { next.y } else { current.y },
            },
        );
        if self.world.scroll_offset(scroll) == Some(next) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(scroll, next);
        self.world.commit(mutations)?;
        Ok(true)
    }

    fn scroll_offset_for_target(
        &self,
        scroll: StableNodeId,
        target: StableNodeId,
        margin: f32,
    ) -> Result<ScrollOffset, FrameworkError> {
        let Some(target_box) = self.world.component_layout_box(target) else {
            return Err(FrameworkError::MissingView(target));
        };
        let Some(view_box) = self.world.component_layout_box(scroll) else {
            return Err(FrameworkError::MissingView(scroll));
        };
        let offset = self.world.scroll_offset(scroll).unwrap_or_default();
        let margin = if margin.is_finite() {
            margin.max(0.0)
        } else {
            0.0
        };

        // Scrolling does not write back into `LayoutBox`, so a child's box is
        // its position within the content, independent of the current offset.
        // The offset is physical, so this reads the same on an axis whose
        // origin is the right / bottom; `scroll_to` clamps to the range.
        let axis = |target_start: f32,
                    target_extent: f32,
                    view_start: f32,
                    view_extent: f32,
                    current: f32| {
            let leading = target_start - view_start;
            let trailing = leading + target_extent;
            if leading - margin < current {
                leading - margin
            } else if trailing + margin > current + view_extent {
                // Never scroll so far that the leading edge leaves the viewport.
                (trailing + margin - view_extent).min(leading)
            } else {
                current
            }
        };

        Ok(ScrollOffset {
            x: axis(
                target_box.x,
                target_box.width,
                view_box.x,
                view_box.width,
                offset.x,
            ),
            y: axis(
                target_box.y,
                target_box.height,
                view_box.y,
                view_box.height,
                offset.y,
            ),
        })
    }

    /// Publish measured scroll geometry and clamp an existing offset when the
    /// content or viewport shrinks. Metrics are Runtime-derived state, not a
    /// duplicate field on [`ScrollView`].
    pub fn set_scroll_metrics(
        &mut self,
        entity: Entity<ScrollView>,
        metrics: ScrollMetrics,
    ) -> Result<bool, FrameworkError> {
        self.read(entity, |_| ())?;
        if self.world.scroll_metrics(entity.id) == Some(metrics) {
            return Ok(false);
        }
        let previous = self.world.scroll_offset(entity.id).unwrap_or_default();
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_metrics(entity.id, Some(metrics));
        self.world.commit(mutations)?;
        let offset = self.world.scroll_offset(entity.id).unwrap_or_default();
        if offset != previous {
            self.update(entity, |_scroll, cx| {
                cx.emit(ScrollChanged { offset });
            })?;
        }
        Ok(true)
    }

    /// Move one scroll container by logical-pixel content offsets.
    pub fn scroll_by(
        &mut self,
        entity: Entity<ScrollView>,
        delta: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.read(entity, |_| ())?;
        let current = self.world.scroll_offset(entity.id).unwrap_or_default();
        self.scroll_to(
            entity,
            ScrollOffset {
                x: current.x + delta.x,
                y: current.y + delta.y,
            },
        )
    }

    /// Route a logical-pixel scroll delta to the nearest hit scroll container.
    ///
    /// [`ScrollView`] and CSS `overflow: auto|scroll` share [`ScrollOffset`].
    /// At a clamped edge the event bubbles to an enclosing container.
    /// Scrollbar chrome stays on [`ScrollView`] only.
    pub fn scroll_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
        delta: ScrollOffset,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !x.is_finite() || !y.is_finite() || !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        let hit = self.world.hit_test(document, x, y);
        self.scroll_from_hit(hit, delta)
    }

    /// [`Self::scroll_at`] with the point already hit-tested: scroll the
    /// nearest container at or above `hit` that can still move.
    pub fn scroll_from_hit(
        &mut self,
        hit: Option<StableNodeId>,
        delta: ScrollOffset,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        let mut current = hit;
        while let Some(id) = current {
            // The parent is read before scrolling: a scroll mutates the
            // container, never the hierarchy above it.
            let parent = self.world.parent_id(id);
            if self.scroll_node_by(id, delta)? {
                return Ok(Some(id));
            }
            current = parent;
        }
        Ok(None)
    }

    /// Whether a node is a [`ScrollView`]. Scrollbar drag and hover chrome
    /// key off this; wheel also routes to CSS `overflow: auto|scroll` boxes.
    pub fn is_scroll_view(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<ScrollView>())
    }

    /// Whether CSS `overflow: auto|scroll` applies on either axis.
    pub fn overflow_scrolls(&self, id: StableNodeId) -> bool {
        self.world.node_style(id).is_some_and(|style| {
            style.layout.overflow_x.scrolls() || style.layout.overflow_y.scrolls()
        })
    }

    pub(super) fn overflow_axes(&self, id: StableNodeId) -> Option<(bool, bool)> {
        let style = self.world.node_style(id)?;
        let x = style.layout.overflow_x.scrolls();
        let y = style.layout.overflow_y.scrolls();
        (x || y).then_some((x, y))
    }

    /// Move a [`ScrollView`] or CSS overflow scroller by `delta`. Returns
    /// `false` at a clamped edge so the caller can bubble.
    pub(crate) fn scroll_node_by(
        &mut self,
        id: StableNodeId,
        delta: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        self.stop_scroll_motion(id);
        let current = self.world.scroll_offset(id).unwrap_or_default();
        self.apply_scroll_node_to(
            id,
            ScrollOffset {
                x: current.x + delta.x,
                y: current.y + delta.y,
            },
        )
    }

    // Immediate input and wheel samples share one absolute-offset commit path.
    pub(super) fn apply_scroll_node_to(
        &mut self,
        id: StableNodeId,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if self.is_scroll_view(id) {
            let scroll = Entity::from_stable_id(id);
            let changed = self.apply_scroll_to(scroll, offset)?;
            if changed {
                self.emit_user_scroll(scroll)?;
            }
            return Ok(changed);
        }
        let editor = self
            .views
            .get(&id)
            .is_some_and(|view| view.is::<TextArea>() || view.is::<crate::RichTextEditor>());
        let (x, y) = if editor {
            // An editor scrolls once it has shaped a value to scroll over.
            if self.world.text_scroll_metrics(id).is_none() {
                return Ok(false);
            }
            (true, true)
        } else if let Some(axes) = self.overflow_axes(id) {
            axes
        } else {
            return Ok(false);
        };
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let next = self.world.clamp_scroll_offset(
            id,
            ScrollOffset {
                x: if x { offset.x } else { current.x },
                y: if y { offset.y } else { current.y },
            },
        );
        if next == current {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(id, next);
        self.world.commit(mutations)?;
        if editor {
            // Pinned to the request, which the caret reveal then yields to.
            let applied = self.world.scroll_request(id).unwrap_or(next);
            self.world.set_text_viewport_pin(id, Some(applied));
            if self.view_entity::<TextArea>(id).is_some() {
                self.update_component(Entity::<TextArea>::from_stable_id(id), |area, _| {
                    area.scroll_offset = applied;
                })?;
            } else {
                self.update_component(
                    Entity::<crate::RichTextEditor>::from_stable_id(id),
                    |area, _| area.scroll_offset = applied,
                )?;
            }
        }
        Ok(true)
    }

    /// An explicit position, a pixel wheel or a grabbed bar ends any line
    /// wheel glide and touch fling on `id`.
    pub(super) fn stop_scroll_motion(&mut self, id: StableNodeId) {
        self.component_lifecycle.wheel_scrolls.remove(&id);
        self.stop_touch_fling(id);
    }

    /// Touch drags and their flings write here. Like
    /// [`Self::apply_scroll_node_to`] on a scroll view or overflow box, except
    /// that an offset past the scrolling area is kept for the rubber band and
    /// the bounce instead of clamped. Only the scroll translation moves; no
    /// layout box is written.
    pub(super) fn apply_touch_scroll_to(
        &mut self,
        id: StableNodeId,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        if self.world.scroll_offset(id) == Some(offset) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_overscroll(id, offset);
        self.world.commit(mutations)?;
        if self.is_scroll_view(id) {
            let scroll = Entity::<ScrollView>::from_stable_id(id);
            if self.has_scroll_listener::<ScrollChanged>(id) {
                self.update(scroll, |_scroll, cx| {
                    cx.emit(ScrollChanged { offset });
                })?;
            }
            self.emit_user_scroll(scroll)?;
        }
        Ok(true)
    }

    pub(super) fn scrollbar_bar(
        &self,
        id: StableNodeId,
        axis: nana_ui_core::ScrollbarAxis,
    ) -> Option<crate::ScrollbarBar> {
        match self.world.component_geometry(id) {
            Some(crate::ComponentGeometry::Scrollbar {
                horizontal,
                vertical,
            }) => match axis {
                nana_ui_core::ScrollbarAxis::Horizontal => horizontal,
                nana_ui_core::ScrollbarAxis::Vertical => vertical,
            },
            _ => None,
        }
    }

    /// Which scrollbar lane of a scroll container a viewport point lands on.
    ///
    /// The vertical bar wins an overlap, matching its drawn order (it paints
    /// last). A bar that is not shown has no lane, and is skipped before any
    /// geometry is derived: this runs on every pointer move.
    pub fn scrollbar_axis_at(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<nana_ui_core::ScrollbarAxis> {
        let shown = match self.world.standard_visual(id) {
            Some(crate::StandardVisual::Scrollbar {
                visibility,
                presence,
                ..
            }) => match visibility {
                nana_ui_core::ScrollbarVisibility::AutoHide => presence > 0.0,
                nana_ui_core::ScrollbarVisibility::Always => true,
                nana_ui_core::ScrollbarVisibility::Hidden => false,
            },
            _ => false,
        };
        if !shown {
            return None;
        }
        let Some(crate::ComponentGeometry::Scrollbar {
            horizontal,
            vertical,
        }) = self.world.component_geometry(id)
        else {
            return None;
        };
        if vertical.is_some_and(|bar| bar.contains(x, y)) {
            Some(nana_ui_core::ScrollbarAxis::Vertical)
        } else if horizontal.is_some_and(|bar| bar.contains(x, y)) {
            Some(nana_ui_core::ScrollbarAxis::Horizontal)
        } else {
            None
        }
    }

    /// Find the innermost scroll container whose scrollbar lane is under a
    /// point.
    ///
    /// Scrollbars overlay content, so the hit-test target is usually a child of
    /// the container that owns the bar. A bar that has faded out has no lane.
    pub fn scrollbar_target_near(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)> {
        self.scrollbar_target_near_hit(x, y, self.world.hit_test(document, x, y))
    }

    /// [`Self::scrollbar_target_near`] with the point already hit-tested.
    ///
    /// Walks the scroll views enclosing `hit`; a viewport-fixed surface (a
    /// menu, a popover) ends the walk, since it does not paint under the
    /// bars of the scroll views it is mounted in.
    pub fn scrollbar_target_near_hit(
        &self,
        x: f32,
        y: f32,
        hit: Option<StableNodeId>,
    ) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)> {
        hit.into_iter()
            .flat_map(|hit| self.enclosing_scroll_views(hit))
            .find_map(|id| self.scrollbar_axis_at(id, x, y).map(|axis| (id, axis)))
    }

    /// Grab a scrollbar. A press on bare track pages toward the point first, so
    /// the thumb is under the pointer when the drag starts.
    pub fn begin_scrollbar_drag(
        &mut self,
        pointer_id: u64,
        target: StableNodeId,
        axis: nana_ui_core::ScrollbarAxis,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(bar) = self.scrollbar_bar(target, axis) else {
            return Ok(false);
        };
        let entity = Entity::<ScrollView>::from_stable_id(target);
        self.read(entity, |_| ())?;
        self.stop_scroll_motion(target);
        let position = bar.axis_position(axis, x, y);
        let track = bar.track_geometry(axis);
        // Cancel restores what the press started from, including any track jump.
        let initial_offset = self.world.scroll_offset(target).unwrap_or_default();
        let grab_offset = if track.thumb_contains(position) {
            position - track.thumb_origin
        } else {
            // Centre the thumb on the press, then keep dragging from there.
            let hold = self.axis_hold(target, axis);
            let offset = track.offset_for_position(position);
            if self.scroll_to(entity, scroll_offset_on(axis, offset, hold))? {
                self.emit_user_scroll(entity)?;
            }
            track.thumb_length / 2.0
        };
        self.update_scroll_view_visual(entity, |scroll, cx| {
            scroll.dragging = Some(crate::ScrollbarDragState {
                pointer_id,
                axis,
                grab_offset,
                initial_offset,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        Ok(true)
    }

    /// The offset on the axis a drag is not touching, so it stays put.
    pub(super) fn axis_hold(&self, id: StableNodeId, axis: nana_ui_core::ScrollbarAxis) -> f32 {
        let offset = self.world.scroll_offset(id).unwrap_or_default();
        match axis {
            nana_ui_core::ScrollbarAxis::Horizontal => offset.y,
            nana_ui_core::ScrollbarAxis::Vertical => offset.x,
        }
    }

    pub fn update_scrollbar_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_scroll_view(target) {
            return Ok(false);
        }
        let entity = Entity::<ScrollView>::from_stable_id(target);
        let Some(drag) = self.read(entity, |scroll| scroll.dragging)? else {
            return Ok(false);
        };
        if drag.pointer_id != pointer_id {
            return Ok(false);
        }
        let Some(bar) = self.scrollbar_bar(target, drag.axis) else {
            return Ok(false);
        };
        let track = bar.track_geometry(drag.axis);
        let offset =
            track.offset_for_thumb_origin(bar.axis_position(drag.axis, x, y) - drag.grab_offset);
        let hold = self.axis_hold(target, drag.axis);
        let changed = self.scroll_to(entity, scroll_offset_on(drag.axis, offset, hold))?;
        if changed {
            self.emit_user_scroll(entity)?;
        }
        Ok(changed)
    }

    pub fn end_scrollbar_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_scroll_view(target) {
            return Ok(false);
        }
        let entity = Entity::<ScrollView>::from_stable_id(target);
        let Some(drag) = self.read(entity, |scroll| scroll.dragging)? else {
            return Ok(false);
        };
        if drag.pointer_id != pointer_id {
            return Ok(false);
        }
        if cancel {
            self.scroll_to(entity, drag.initial_offset)?;
        }
        self.update_scroll_view_visual(entity, |scroll, cx| {
            scroll.dragging = None;
            cx.mutations().release_pointer(pointer_id, target);
        })?;
        self.hold_scrollbars(target)?;
        Ok(true)
    }

    /// Reveal auto-hiding scrollbars for every container under the pointer,
    /// the outer ones as well as the innermost. A container the pointer
    /// leaves keeps its bars for the theme's hide delay before they fade.
    pub(super) fn sync_scroll_view_hover(
        &mut self,
        previous: Option<StableNodeId>,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let entered = target.map_or_else(Vec::new, |id| self.enclosing_scroll_views(id));
        let left = previous.map_or_else(Vec::new, |id| self.enclosing_scroll_views(id));
        if left == entered {
            return Ok(());
        }
        for &id in left.iter().filter(|id| !entered.contains(id)) {
            self.set_scroll_view_hover(id, false)?;
            self.hold_scrollbars(id)?;
        }
        for &id in entered.iter().filter(|id| !left.contains(id)) {
            self.set_scroll_view_hover(id, true)?;
        }
        Ok(())
    }

    pub(super) fn set_scroll_view_hover(
        &mut self,
        id: StableNodeId,
        hovered: bool,
    ) -> Result<(), FrameworkError> {
        let entity = Entity::<ScrollView>::from_stable_id(id);
        if self.read(entity, |scroll| scroll.hovered)? == hovered {
            return Ok(());
        }
        self.update_scroll_view_visual(entity, |scroll, _| {
            scroll.hovered = hovered;
        })?;
        Ok(())
    }

    /// The scroll views at or above `id`, innermost first, up to the first
    /// viewport-fixed surface: a menu or popover mounted inside a scroll view
    /// paints outside it, so pointing at it is not pointing at the view.
    pub(super) fn enclosing_scroll_views(&self, id: StableNodeId) -> Vec<StableNodeId> {
        let mut views = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            if self.is_scroll_view(id) {
                views.push(id);
            }
            if self
                .world
                .node_style(id)
                .is_some_and(|style| style.layout.position == nana_ui_core::PositionSpec::Fixed)
            {
                break;
            }
            current = self.world.parent_id(id);
        }
        views
    }

    /// Keep `id`'s auto-hiding bars shown for the theme's hide delay: after a
    /// scroll, a drag, or the pointer leaving. Resident and hidden bars and a
    /// zero delay take no hold.
    pub(super) fn hold_scrollbars(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
        let entity = Entity::<ScrollView>::from_stable_id(id);
        let Ok(visibility) = self.read(entity, |scroll| scroll.scrollbars) else {
            return Ok(());
        };
        let recipe = *self.world.theme().recipes().scrollbar();
        let delay = recipe.motion.hide_delay();
        if visibility.unwrap_or(recipe.visibility) != nana_ui_core::ScrollbarVisibility::AutoHide
            || delay.is_zero()
        {
            return Ok(());
        }
        let until = self.component_lifecycle.now.saturating_add(delay);
        self.component_lifecycle.scrollbar_holds.insert(id, until);
        if !self.read(entity, |scroll| scroll.held)? {
            self.update_scroll_view_visual(entity, |scroll, _| scroll.held = true)?;
        }
        Ok(())
    }

    /// The earliest moment a scroll view's hold on its bars runs out.
    pub(super) fn scrollbar_hold_deadline(&self) -> Option<Duration> {
        self.component_lifecycle
            .scrollbar_holds
            .iter()
            .filter(|(id, _)| self.world.is_mounted(**id))
            .map(|(_, until)| *until)
            .min()
    }

    /// Let go of the holds that ran out by `now`, so those bars fade out
    /// unless something else still shows them. Returns the views released.
    pub(super) fn release_scrollbar_holds(&mut self, now: Duration) -> Vec<StableNodeId> {
        let due: Vec<_> = self
            .component_lifecycle
            .scrollbar_holds
            .iter()
            .filter(|(id, until)| **until <= now || !self.world.is_mounted(**id))
            .map(|(id, _)| *id)
            .collect();
        let mut released = Vec::with_capacity(due.len());
        for id in due {
            self.component_lifecycle.scrollbar_holds.remove(&id);
            if !self.world.is_mounted(id) || !self.is_scroll_view(id) {
                continue;
            }
            let entity = Entity::<ScrollView>::from_stable_id(id);
            if self
                .update_scroll_view_visual(entity, |scroll, _| scroll.held = false)
                .is_ok()
            {
                released.push(id);
            }
        }
        released
    }

    /// Note which lane, if any, a pointer is on: that axis's thumb grows and
    /// its track shows, and moving off it lets them settle back.
    pub(super) fn sync_scrollbar_hot(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        lane: Option<(StableNodeId, nana_ui_core::ScrollbarAxis)>,
    ) -> Result<(), FrameworkError> {
        let key = (document, pointer_id);
        let previous = self.component_lifecycle.scrollbar_hot.get(&key).copied();
        if previous == lane {
            return Ok(());
        }
        match lane {
            Some(lane) => self.component_lifecycle.scrollbar_hot.insert(key, lane),
            None => self.component_lifecycle.scrollbar_hot.remove(&key),
        };
        if let Some((view, _)) = previous
            && lane.is_none_or(|(next, _)| next != view)
        {
            self.set_scroll_view_hot(view, None)?;
        }
        if let Some((view, axis)) = lane {
            self.set_scroll_view_hot(view, Some(axis))?;
        }
        Ok(())
    }

    fn set_scroll_view_hot(
        &mut self,
        id: StableNodeId,
        hot: Option<nana_ui_core::ScrollbarAxis>,
    ) -> Result<(), FrameworkError> {
        if !self.world.is_mounted(id) || !self.is_scroll_view(id) {
            return Ok(());
        }
        let entity = Entity::<ScrollView>::from_stable_id(id);
        if self.read(entity, |scroll| scroll.hot)? == hot {
            return Ok(());
        }
        self.update_scroll_view_visual(entity, |scroll, _| scroll.hot = hot)?;
        Ok(())
    }

    /// Bring the component clock up to an input event's time, so a hold a
    /// wheel or a key starts runs from when it happened rather than from the
    /// last frame the host drew.
    pub(super) fn note_input_time(&mut self, now: Duration) {
        if now > self.component_lifecycle.now {
            self.component_lifecycle.now = now;
        }
        if now > self.world.animation_now {
            self.world.animation_now = now;
        }
    }

    /// Whether `scroll`'s bars are drawn now: present on at least one axis
    /// that can scroll. Auto-hiding bars count while they fade.
    pub fn scrollbars_visible(&self, scroll: Entity<ScrollView>) -> bool {
        matches!(
            self.world.component_geometry(scroll.stable_id()),
            Some(crate::ComponentGeometry::Scrollbar { .. })
        )
    }
}
