//! AppContext scroll operations.

use super::*;

impl AppContext {
    pub fn scroll_to(
        &mut self,
        entity: Entity<ScrollView>,
        offset: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !offset.x.is_finite() || !offset.y.is_finite() || offset.x < 0.0 || offset.y < 0.0 {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
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
        self.update(entity, |_scroll, cx| {
            cx.emit(ScrollChanged { offset });
        })?;
        Ok(true)
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
                x: (current.x + delta.x).max(0.0),
                y: (current.y + delta.y).max(0.0),
            },
        )
    }

    /// Route a logical-pixel scroll delta to the nearest hit scroll container.
    ///
    /// L2 [`ScrollView`] and L1 `overflow: auto|scroll` share [`ScrollOffset`].
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
        let Some(target) = self.world.hit_test(document, x, y) else {
            return Ok(None);
        };
        let mut ancestors = Vec::new();
        let mut current = Some(target);
        while let Some(id) = current {
            ancestors.push(id);
            current = self.world.node(id).and_then(|node| node.parent);
        }
        for id in ancestors {
            if self.scroll_node_by(id, delta)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Whether a node is an L2 [`ScrollView`]. Scrollbar drag and hover chrome
    /// key off this; wheel also routes to L1 `overflow: auto|scroll` boxes.
    pub fn is_scroll_view(&self, id: StableNodeId) -> bool {
        self.views
            .get(&id)
            .is_some_and(|view| view.is::<ScrollView>())
    }

    /// Whether L1 `overflow: auto|scroll` applies on either axis.
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

    pub(super) fn write_scroll_metrics(
        &mut self,
        id: StableNodeId,
        metrics: ScrollMetrics,
    ) -> Result<bool, FrameworkError> {
        if self.world.scroll_metrics(id) == Some(metrics) {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_metrics(id, Some(metrics));
        self.world.commit(mutations)?;
        Ok(true)
    }

    pub(super) fn ensure_scroll_metrics(&mut self, id: StableNodeId) -> Result<(), FrameworkError> {
        let Some(metrics) = self.scroll_metrics_from_layout(id) else {
            return Ok(());
        };
        self.write_scroll_metrics(id, metrics)?;
        Ok(())
    }

    /// Move a [`ScrollView`] or L1 overflow scroller by `delta`. Returns
    /// `false` at a clamped edge so the caller can bubble.
    pub(crate) fn scroll_node_by(
        &mut self,
        id: StableNodeId,
        delta: ScrollOffset,
    ) -> Result<bool, FrameworkError> {
        if !delta.x.is_finite() || !delta.y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        if self.is_scroll_view(id) {
            return self.scroll_by(Entity::from_stable_id(id), delta);
        }
        let Some((scrolls_x, scrolls_y)) = self.overflow_axes(id) else {
            return Ok(false);
        };
        self.ensure_scroll_metrics(id)?;
        let current = self.world.scroll_offset(id).unwrap_or_default();
        let next = self.world.clamp_scroll_offset(
            id,
            ScrollOffset {
                x: if scrolls_x {
                    (current.x + delta.x).max(0.0)
                } else {
                    current.x
                },
                y: if scrolls_y {
                    (current.y + delta.y).max(0.0)
                } else {
                    current.y
                },
            },
        );
        if next == current {
            return Ok(false);
        }
        let mut mutations = MutationQueue::new();
        mutations.set_scroll_offset(id, next);
        self.world.commit(mutations)?;
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

    /// Which scrollbar axis of a scroll container a viewport point lands on.
    ///
    /// The vertical bar wins an overlap, matching its drawn order.
    pub fn scrollbar_axis_at(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<nana_ui_core::ScrollbarAxis> {
        [
            nana_ui_core::ScrollbarAxis::Vertical,
            nana_ui_core::ScrollbarAxis::Horizontal,
        ]
        .into_iter()
        .find(|axis| {
            self.scrollbar_bar(id, *axis)
                .is_some_and(|bar| bar.contains(x, y))
        })
    }

    /// Find the innermost scroll container whose scrollbar is under a point.
    ///
    /// Scrollbars overlay content, so the hit-test target is usually a child of
    /// the container that owns the bar.
    pub fn scrollbar_target_near(
        &self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Option<(StableNodeId, nana_ui_core::ScrollbarAxis)> {
        let mut current = self.world.hit_test(document, x, y);
        while let Some(id) = current {
            if let Some(axis) = self.scrollbar_axis_at(id, x, y) {
                return Some((id, axis));
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
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
            self.scroll_to(entity, scroll_offset_on(axis, offset, hold))?;
            track.thumb_length / 2.0
        };
        self.update_component(entity, |scroll, cx| {
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
        self.scroll_to(entity, scroll_offset_on(drag.axis, offset, hold))
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
        self.update_component(entity, |scroll, cx| {
            scroll.dragging = None;
            cx.mutations().release_pointer(pointer_id, target);
        })?;
        Ok(true)
    }

    /// Reveal auto-hiding scrollbars for the container under the pointer.
    pub(super) fn sync_scroll_view_hover(
        &mut self,
        previous: Option<StableNodeId>,
        target: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let entered = target.and_then(|id| self.enclosing_scroll_view(id));
        let left = previous.and_then(|id| self.enclosing_scroll_view(id));
        if left == entered {
            return Ok(());
        }
        if let Some(id) = left {
            self.set_scroll_view_hover(id, false)?;
        }
        if let Some(id) = entered {
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
        self.update_component(entity, |scroll, _| {
            scroll.hovered = hovered;
        })?;
        Ok(())
    }

    pub(super) fn enclosing_scroll_view(&self, id: StableNodeId) -> Option<StableNodeId> {
        let mut current = Some(id);
        while let Some(id) = current {
            if self.is_scroll_view(id) {
                return Some(id);
            }
            current = self.world.node(id).and_then(|node| node.parent);
        }
        None
    }
}
