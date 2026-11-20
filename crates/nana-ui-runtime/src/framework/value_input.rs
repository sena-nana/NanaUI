//! AppContext value input operations.

use super::*;

impl AppContext {
    /// Publish a numeric value using the field's bounds and discrete or
    /// continuous policy; hosts do not reimplement numeric normalization.
    ///
    /// A number the field already holds changes nothing, so a controlled
    /// field echoing `NumberChanged` back keeps the user's draft and undo
    /// history. A new number is an application write, not the user's edit:
    /// the draft shows it and the undo history starts afresh, so undo cannot
    /// bring back a draft over it.
    pub fn set_number_value(
        &mut self,
        entity: Entity<NumberInput>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.write_number(entity, TextEditOrigin::Program, |input| {
            input.publish(value)
        })
    }

    /// Commit a complete value given as text, as assistive technology sets
    /// one: committed as the field's value and shown in place of any draft,
    /// with the field's grid applied. Returns whether the field took it.
    ///
    /// Unparseable text, a number outside the field's bounds, and a field
    /// that refuses input are refused outright: nothing changes, the pending
    /// draft and its undo history included. A number within bounds is taken,
    /// snapped to the grid, even when the field already holds it.
    pub(super) fn set_number_text(
        &mut self,
        entity: Entity<NumberInput>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(requested) = nana_ui_core::NumberFieldSpec::parse_unsnapped(text) else {
            return Ok(false);
        };
        // A composition owns the draft until it commits or cancels.
        if self.node_composing(entity.stable_id())
            || !self.read(entity, |input| {
                input.accepts_input() && input.within_bounds(requested)
            })?
        {
            return Ok(false);
        }
        // Unlike `set_number_value`, the field shows what was set even when it
        // already held the number: the user asked for this value.
        self.write_number(entity, TextEditOrigin::Program, |input| {
            input.assign(requested)
        })?;
        Ok(true)
    }

    /// Move a numeric field by step increments, from the typed draft when it
    /// parses and from the committed value otherwise. Disabled and read-only
    /// fields refuse, so a stepper press cannot bypass either flag. A run of
    /// steps is one undo step of its own: typing after it does not merge into
    /// the typing before it, and a held arrow key does not flood the history.
    pub fn step_number_input(
        &mut self,
        entity: Entity<NumberInput>,
        steps: i32,
    ) -> Result<bool, FrameworkError> {
        // A composition owns the draft until it commits or cancels.
        if !self.read(entity, NumberInput::accepts_input)?
            || self.node_composing(entity.stable_id())
        {
            return Ok(false);
        }
        self.write_number(entity, TextEditOrigin::Step, |input| {
            input.step_value(steps)
        })
    }

    /// Parse the in-progress draft into the committed value. An unparseable
    /// draft restores the last committed value and reports no change. The
    /// normalized draft is its own undo step.
    pub fn commit_number_input(
        &mut self,
        entity: Entity<NumberInput>,
    ) -> Result<bool, FrameworkError> {
        // A composition owns the draft until it commits or cancels.
        if self.node_composing(entity.stable_id()) {
            return Ok(false);
        }
        self.write_number(
            entity,
            TextEditOrigin::Structural,
            NumberInput::commit_draft,
        )
    }

    /// Rewrite a numeric field's draft through the undo journal, emitting
    /// [`NumberChanged`] when the committed number moved. Returns whether the
    /// field changed at all (a reformatted draft counts).
    fn write_number(
        &mut self,
        entity: Entity<NumberInput>,
        origin: TextEditOrigin,
        write: impl FnOnce(&mut NumberInput) -> bool,
    ) -> Result<bool, FrameworkError> {
        let changed = self.journal_editor_edit(entity, origin, |input, cx| {
            write_number_field(input, cx, write)
        })?;
        // A commit or a revert ends a run of steps even when it rewrote
        // nothing: steps before and after it are two undo steps.
        if origin == TextEditOrigin::Structural {
            self.seal_editor_history(entity.stable_id());
        }
        Ok(changed)
    }

    /// Step the focused numeric field, if any. Returns whether it moved.
    pub fn step_focused_number_input(
        &mut self,
        document: DocumentId,
        steps: i32,
    ) -> Result<bool, FrameworkError> {
        match self.focused_number_input(document) {
            Some(entity) => self.step_number_input(entity, steps),
            None => Ok(false),
        }
    }

    /// Commit the focused numeric field's draft, if any.
    pub fn commit_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        match self.focused_number_input(document) {
            Some(entity) => self.commit_number_input(entity),
            None => Ok(false),
        }
    }

    /// Submit an editable focused [`NumberInput`], committing any pending draft
    /// (or reverting an unparseable one) and emitting [`crate::NumberSubmitted`].
    /// IME composition never submits. Returns whether the draft changed or a
    /// submission listener observed the confirmation.
    pub fn submit_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        if self.world.ime(entity.id).is_some() || !self.read(entity, NumberInput::accepts_input)? {
            return Ok(false);
        }
        let committed = self.commit_number_input(entity)?;
        let observed = self.has_event_listeners::<crate::NumberSubmitted>(entity.stable_id());
        if observed && self.world.contains(entity.id) {
            self.update(entity, |input, cx| {
                cx.emit(crate::NumberSubmitted {
                    value: input.value(),
                });
            })?;
            if self.world.focused(document) != Some(entity.id) {
                self.sync_interactive_style_states();
            }
        }
        Ok(committed || observed)
    }

    /// Cancel editing on an editable focused [`NumberInput`], reverting any
    /// pending draft and emitting [`crate::NumberCancelled`]. IME composition
    /// is left alone. Returns whether the draft reverted or a cancellation
    /// listener observed the event.
    pub(super) fn cancel_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        if self.world.ime(entity.id).is_some() || !self.read(entity, NumberInput::accepts_input)? {
            return Ok(false);
        }
        let reverted = self.revert_focused_number_input(document)?;
        let observed = self.has_event_listeners::<crate::NumberCancelled>(entity.stable_id());
        if observed && self.world.contains(entity.id) {
            self.update(entity, |_, cx| {
                cx.emit(crate::NumberCancelled);
            })?;
            if self.world.focused(document) != Some(entity.id) {
                self.sync_interactive_style_states();
            }
        }
        Ok(reverted || observed)
    }

    /// Select all text when focus enters a [`NumberInput`] on pointer entry
    /// (excluding stepper hits) or sequential Tab navigation. Returns `true`
    /// when `node` is the focused [`NumberInput`] and accepts selection.
    pub(super) fn select_focused_number_on_entry(
        &mut self,
        document: DocumentId,
        node: StableNodeId,
        point: Option<(f32, f32)>,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        if entity.stable_id() != node {
            return Ok(false);
        }
        if let Some((x, y)) = point {
            let (lx, ly) = self.layout_point(node, x, y);
            if self.on_number_stepper(node, lx, ly) {
                return Ok(false);
            }
        }
        if !self.read(entity, EditableText::accepts_selection)? {
            return Ok(false);
        }
        self.select_all_editable(entity)?;
        Ok(true)
    }

    /// Discard the focused numeric field's draft and show the committed value
    /// again. Nothing is emitted: the value never moved.
    pub fn revert_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        // Escape during a composition belongs to the IME, not the draft.
        if self.node_composing(entity.stable_id()) {
            return Ok(false);
        }
        self.write_number(
            entity,
            TextEditOrigin::Structural,
            NumberInput::revert_draft,
        )
    }

    /// Settle the focused numeric draft before focus leaves the field.
    /// Leaving focus cancels an unfinished composition (the world drops it on
    /// blur), so it is cancelled first and the draft beneath it committed,
    /// rather than committed under a live preedit.
    pub(super) fn settle_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        if self.node_composing(entity.stable_id()) {
            let mut mutations = MutationQueue::new();
            mutations.set_ime(entity.stable_id(), None);
            self.world.commit(mutations)?;
        }
        self.commit_number_input(entity)
    }

    pub(super) fn focused_number_input(&self, document: DocumentId) -> Option<Entity<NumberInput>> {
        let target = self.world.focused(document)?;
        self.view_entity(target)
    }

    /// Whether a point is on a numeric field's spinner, whichever halves are
    /// enabled. Coordinates are viewport-local, matching hit testing.
    pub(super) fn on_number_stepper(&self, id: StableNodeId, x: f32, y: f32) -> bool {
        self.number_steppers(id)
            .is_some_and(|steppers| steppers.contains(x, y))
    }

    fn number_steppers(&self, id: StableNodeId) -> Option<crate::NumberSteppers> {
        self.world.text_input_geometry(id)?.steppers
    }

    /// Resolve a stepper press inside a numeric field to a signed step count.
    ///
    /// Coordinates are viewport-local, matching hit testing. Returns `None`
    /// when the point is on the editable text instead of the spinner, so the
    /// caller can fall through to caret placement.
    pub fn number_stepper_at(&self, id: StableNodeId, x: f32, y: f32) -> Option<i32> {
        self.number_steppers(id)?.step_at(x, y)
    }

    /// Route a pointer press on a numeric field's spinner. Returns whether the
    /// press was consumed by a stepper.
    pub fn press_number_stepper(
        &mut self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(steps) = self.number_stepper_at(id, x, y) else {
            return Ok(false);
        };
        let Some(entity) = self.view_entity::<NumberInput>(id) else {
            return Ok(false);
        };
        self.step_number_input(entity, steps)?;
        Ok(true)
    }

    /// Commits `value`: emits [`RangeInput`] then [`RangeChanged`]. During a
    /// drag the committed value also becomes the value a cancel restores.
    pub fn set_range_value(
        &mut self,
        entity: Entity<RangeField>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        self.write_range_value(entity, value, true)
    }

    fn write_range_value(
        &mut self,
        entity: Entity<RangeField>,
        value: f64,
        commit: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, |range| range.disabled)? {
            return Ok(false);
        }
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.update_component(entity, |range, cx| {
            let value = range.quantize(value);
            if range.value == value {
                return false;
            }
            range.value = value;
            cx.emit(RangeInput { value });
            if commit {
                if let Some(drag) = range.dragging.as_mut() {
                    drag.initial_value = value;
                }
                cx.emit(RangeChanged { value });
            }
            true
        })
    }

    pub fn adjust_range(
        &mut self,
        entity: Entity<RangeField>,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let value = self.read(entity, |range| match adjustment {
            RangeAdjustment::Decrement => range.value - range.step,
            RangeAdjustment::Increment => range.value + range.step,
            RangeAdjustment::PageDecrement => range.value - range.page_step,
            RangeAdjustment::PageIncrement => range.value + range.page_step,
            RangeAdjustment::Minimum => range.minimum,
            RangeAdjustment::Maximum => range.maximum,
        })?;
        self.set_range_value(entity, value)
    }

    pub fn adjust_focused_range(
        &mut self,
        document: DocumentId,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.focused(document) else {
            return Ok(false);
        };
        if !via!(self.is_range_field(target)) {
            return Ok(false);
        }
        self.adjust_range(Entity::from_stable_id(target), adjustment)
    }

    /// [`Self::begin_range_drag`] at a window point: through the scroll
    /// offsets and transforms above the field, as hit testing saw the press.
    pub(crate) fn begin_range_drag_at(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let (x, _) = self.layout_point(target, x, y);
        self.begin_range_drag(document, pointer_id, target, x)
    }

    /// [`Self::update_range_drag_fine`] at a window point.
    pub(crate) fn update_range_drag_at(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        fine: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        let (x, _) = self.layout_point(target, x, y);
        self.update_range_drag_fine(document, pointer_id, x, fine)
    }

    /// A window point in `target`'s layout space.
    fn layout_point(&self, target: StableNodeId, x: f32, y: f32) -> (f32, f32) {
        self.world
            .pointer_layout_position(target, x, y)
            .unwrap_or((x, y))
    }

    /// Starts dragging the range at `x`, in the field's layout space (the
    /// space of its [`crate::RangeGeometry`] track).
    ///
    /// The press picks the value under it, except on a value thumb's
    /// capsule, where the drag starts from the value the range holds. A
    /// value thumb's drag is relative from then on: the value moves by the
    /// pointer's travel.
    pub fn begin_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        if !via!(self.is_range_field(target)) {
            return Ok(false);
        }
        let (disabled, initial_value, value_thumb) = self
            .read(Entity::<RangeField>::from_stable_id(target), |range| {
                (range.disabled, range.value, range.draws_value_thumb())
            })?;
        if disabled {
            return Ok(false);
        }
        let capsule = self
            .world
            .range_geometry(target)
            .filter(|_| value_thumb)
            .and_then(|g| g.value_thumb)
            .map(|thumb| thumb.capsule);
        let on_thumb =
            capsule.is_some_and(|capsule| x >= capsule.x && x <= capsule.x + capsule.width);
        self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
            let mut drag = crate::RangeDragState::new(pointer_id, initial_value);
            drag.press_x = x;
            drag.last_x = x;
            drag.on_thumb = on_thumb;
            drag.anchor = on_thumb.then_some((x, initial_value));
            range.dragging = Some(drag);
            cx.mutations().capture_pointer(pointer_id, target);
            cx.emit(RangeDragging { dragging: true });
        })?;
        let moved = self.update_range_drag(document, pointer_id, x)?;
        if value_thumb && !on_thumb {
            // A press beside the capsule jumps there, then drags from it.
            self.update_component(Entity::<RangeField>::from_stable_id(target), |range, _| {
                if let Some(drag) = range.dragging.as_mut() {
                    drag.anchor = Some((x, drag.last_raw));
                }
            })?;
        }
        Ok(moved)
    }

    /// [`Self::update_range_drag_fine`] at the drag's ordinary pace.
    pub fn update_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        self.update_range_drag_fine(document, pointer_id, x, false)
    }

    /// Moves the drag `pointer_id` holds to `x`. With `fine` (Alt held) the
    /// value moves a tenth as far per pixel. Changing `fine` mid-drag
    /// re-anchors at the last pointer position and value, so the value
    /// never jumps, and the drag stays relative from then on.
    pub fn update_range_drag_fine(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        fine: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !via!(self.is_range_field(target)) {
            return Ok(false);
        }
        let Some(geometry) = self.world.range_geometry(target) else {
            return Ok(false);
        };
        let track = geometry.track;
        if track.width <= 0.0 {
            return Ok(false);
        }
        let value =
            self.update_component(Entity::<RangeField>::from_stable_id(target), |range, _| {
                let span = range.maximum - range.minimum;
                let Some(mut drag) = range.dragging else {
                    return range.value_at(track.x, track.width, x);
                };
                if drag.fine != fine {
                    drag.anchor = Some((drag.last_x, drag.last_raw));
                    drag.fine = fine;
                }
                if (x - drag.press_x).abs() >= nana_ui_core::space::XXS {
                    drag.moved = true;
                }
                let raw = match drag.anchor {
                    Some((anchor_x, anchor_value)) => {
                        // Alt: a tenth of the pace.
                        let pace = if fine { 0.1 } else { 1.0 };
                        let per_pixel = span / f64::from(track.width) * pace;
                        (anchor_value + f64::from(x - anchor_x) * per_pixel)
                            .clamp(range.minimum, range.maximum)
                    }
                    None => range.value_at(track.x, track.width, x),
                };
                drag.last_x = x;
                drag.last_raw = raw;
                range.dragging = Some(drag);
                raw
            })?;
        self.write_range_value(Entity::from_stable_id(target), value, false)
    }

    /// Ends the drag `pointer_id` holds. Release commits the dragged value
    /// with [`RangeChanged`] when it moved; cancel, or a field disabled
    /// mid-drag, restores the value the drag started from and commits nothing.
    pub fn end_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        via!(self.end_range_drag_on(target, pointer_id, cancel))
    }

    /// A commit released `pointer_id`'s capture of `target` without the
    /// drag ending (the field was hidden or parked): end a range or XY pad
    /// drag it held as a cancel, so its dragging and seek events close.
    pub(crate) fn end_drags_that_lost_their_capture(
        &mut self,
        released: &[(u64, StableNodeId)],
    ) -> Result<(), FrameworkError> {
        for &(pointer_id, target) in released {
            if !self.world.contains(target) {
                continue;
            }
            if via!(self.is_range_field(target))
                && self.read(Entity::<RangeField>::from_stable_id(target), |range| {
                    range
                        .dragging
                        .is_some_and(|drag| drag.pointer_id == pointer_id)
                })?
            {
                via!(self.end_range_drag_on(target, pointer_id, true))?;
            } else if self.range_span_drags(target, pointer_id) {
                via!(self.end_range_span_drag_on(target, pointer_id, true))?;
            } else if self.curve_drags(target, pointer_id) {
                via!(self.end_curve_drag_on(target, pointer_id, true))?;
            } else if via!(self.is_xy_pad(target))
                && self.read(Entity::<XYPad>::from_stable_id(target), |pad| {
                    pad.dragging.is_some()
                })?
            {
                via!(self.end_xy_pad_drag_on(target, pointer_id, true))?;
            }
        }
        Ok(())
    }

    fn holds_capture(&self, target: StableNodeId, pointer_id: u64) -> bool {
        self.world
            .document_of(target)
            .and_then(|document| self.world.pointer_capture(document, pointer_id))
            == Some(target)
    }

    pub(crate) fn end_range_drag_on(
        &mut self,
        target: StableNodeId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        if !via!(self.is_range_field(target)) {
            return Ok(false);
        }
        let held = self.holds_capture(target, pointer_id);
        let mut clicked_thumb = false;
        let ended =
            self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
                if held {
                    cx.mutations().release_pointer(pointer_id, target);
                }
                let Some(drag) = range.dragging.take() else {
                    return false;
                };
                if range.value != drag.initial_value {
                    if cancel || range.disabled {
                        range.value = drag.initial_value;
                        cx.emit(RangeInput { value: range.value });
                    } else {
                        cx.emit(RangeChanged { value: range.value });
                    }
                }
                cx.emit(RangeDragging { dragging: false });
                clicked_thumb = !cancel && drag.on_thumb && !drag.moved;
                true
            })?;
        if clicked_thumb {
            // A click on the value thumb types a value.
            self.begin_range_edit_in_place(Entity::from_stable_id(target))?;
        }
        Ok(ended)
    }

    pub fn begin_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if !via!(self.is_xy_pad(target)) {
            return Ok(false);
        }
        if self.read(Entity::<XYPad>::from_stable_id(target), XYPad::inactive)? {
            return Ok(false);
        }
        let Some(bounds) = self.world.component_layout_box(target) else {
            return Ok(false);
        };
        let (local_x, local_y) = self.layout_point(target, x, y);
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            pad.dragging = Some(XYPadDragState {
                pointer_id,
                origin_x: local_x - bounds.x,
                origin_y: local_y - bounds.y,
                axis_lock: None,
                initial: pad.value,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        via!(self.update_xy_pad_drag(document, pointer_id, x, y, false))
    }

    pub fn update_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !via!(self.is_xy_pad(target)) {
            return Ok(false);
        }
        let Some(bounds) = self.world.component_layout_box(target) else {
            return Ok(false);
        };
        let (x, y) = self.layout_point(target, x, y);
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            if pad.inactive() {
                return false;
            }
            if let Some(drag) = pad.dragging.as_mut() {
                if shift && drag.axis_lock.is_none() {
                    let local_x = x - bounds.x;
                    let local_y = y - bounds.y;
                    let dx = (local_x - drag.origin_x).abs() / bounds.width.max(1.0);
                    let dy = (local_y - drag.origin_y).abs() / bounds.height.max(1.0);
                    drag.axis_lock = Some(if dx >= dy {
                        crate::XYPadAxisLock::Horizontal
                    } else {
                        crate::XYPadAxisLock::Vertical
                    });
                } else if !shift {
                    drag.axis_lock = None;
                }
            } else {
                return false;
            }
            let locked = pad
                .dragging
                .and_then(|drag| drag.axis_lock.map(|axis| (axis, pad.value)));
            let value = pad.value_from_point(x, y, bounds, locked);
            pad.value = value;
            cx.emit(XYPadEvent::Input(value));
            true
        })
    }

    pub fn end_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        via!(self.end_xy_pad_drag_on(target, pointer_id, cancel))
    }

    pub(crate) fn end_xy_pad_drag_on(
        &mut self,
        target: StableNodeId,
        pointer_id: u64,
        cancel: bool,
    ) -> Result<bool, FrameworkError> {
        if !via!(self.is_xy_pad(target)) {
            return Ok(false);
        }
        let held = self.holds_capture(target, pointer_id);
        let initial = self.read(Entity::<XYPad>::from_stable_id(target), |pad| {
            pad.dragging.map(|drag| drag.initial)
        })?;
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            if cancel {
                if let Some(value) = initial {
                    pad.value = value;
                }
            } else if initial.is_some() {
                cx.emit(XYPadEvent::Change(pad.value));
            }
            pad.dragging = None;
            if held {
                cx.mutations().release_pointer(pointer_id, target);
            }
        })?;
        Ok(initial.is_some())
    }
}

/// Apply a write to a numeric field and emit [`NumberChanged`] when the
/// committed number moved. Returns whether the field changed at all (a
/// reformatted draft counts).
fn write_number_field(
    input: &mut NumberInput,
    cx: &mut ViewContext<'_, NumberInput>,
    write: impl FnOnce(&mut NumberInput) -> bool,
) -> bool {
    let before = input.value();
    if !write(input) {
        return false;
    }
    if input.value() != before {
        cx.emit(NumberChanged {
            value: input.value(),
        });
    }
    true
}
