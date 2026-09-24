//! AppContext value input operations.

use super::*;

impl AppContext {
    /// Publish a numeric value using the field's bounds and discrete or
    /// continuous policy; hosts do not reimplement numeric normalization.
    ///
    /// An application write, not the user's edit: whenever it changes the
    /// field, including a new number under a draft that already reads it, the
    /// undo history starts afresh, so undo cannot bring back a draft over it.
    pub fn set_number_value(
        &mut self,
        entity: Entity<NumberInput>,
        value: f64,
    ) -> Result<bool, FrameworkError> {
        if !value.is_finite() {
            return Err(FrameworkError::InvalidComponentValue(entity.id));
        }
        self.write_number(entity, TextEditOrigin::Program, |input| input.assign(value))
    }

    /// Commit a complete value given as text, as assistive technology sets
    /// one: parsed by the field's own policy and published through
    /// [`Self::set_number_value`]. Unparseable text and fields that refuse
    /// input change nothing.
    ///
    /// Reports whether the field took the value: it moved (clamped or snapped
    /// as its policy requires), or it already held exactly that number. A
    /// request the bounds refuse outright (150 on a field already at its
    /// maximum of 10) reports failure.
    pub(super) fn set_number_text(
        &mut self,
        entity: Entity<NumberInput>,
        text: &str,
    ) -> Result<bool, FrameworkError> {
        let parsed = self.read(entity, |input| {
            input
                .accepts_input()
                .then(|| input.parse_text(text))
                .flatten()
        })?;
        let Some(parsed) = parsed else {
            return Ok(false);
        };
        if self.set_number_value(entity, parsed)? {
            return Ok(true);
        }
        let requested = text.trim().parse::<f64>().ok();
        Ok(requested == Some(self.read(entity, NumberInput::value)?))
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
        if !self.read(entity, NumberInput::accepts_input)? {
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
        let changed = self.write_number(
            entity,
            TextEditOrigin::Structural,
            NumberInput::commit_draft,
        )?;
        // A commit ends a run of steps even when the draft already read the
        // number: steps before and after it are two undo steps.
        self.seal_editor_history(entity.stable_id());
        Ok(changed)
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
        self.journal_editor_edit(entity, origin, |input, cx| {
            write_number_field(input, cx, write)
        })
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

    /// Discard the focused numeric field's draft and show the committed value
    /// again. Nothing is emitted: the value never moved.
    pub fn revert_focused_number_input(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(entity) = self.focused_number_input(document) else {
            return Ok(false);
        };
        let changed = self.write_number(
            entity,
            TextEditOrigin::Structural,
            NumberInput::revert_draft,
        )?;
        // Like a commit, Escape ends a run of steps even when the draft
        // already showed the committed value.
        self.seal_editor_history(entity.stable_id());
        Ok(changed)
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
        match self.world.component_geometry(id) {
            Some(crate::ComponentGeometry::TextInput {
                steppers: Some(steppers),
                ..
            }) => Some(steppers),
            _ => None,
        }
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
        if !self.is_range_field(target) {
            return Ok(false);
        }
        self.adjust_range(Entity::from_stable_id(target), adjustment)
    }

    pub fn begin_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        if !self.is_range_field(target) {
            return Ok(false);
        }
        if self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.disabled
        })? {
            return Ok(false);
        }
        let initial_value = self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.value
        })?;
        self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
            range.dragging = Some(crate::RangeDragState {
                pointer_id,
                initial_value,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        self.update_range_drag(document, pointer_id, x)
    }

    pub fn update_range_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        x: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world.pointer_capture(document, pointer_id) else {
            return Ok(false);
        };
        if !self.is_range_field(target) {
            return Ok(false);
        }
        let track = match self.world.component_geometry(target) {
            Some(crate::ComponentGeometry::Range { track, .. }) => track,
            _ => return Ok(false),
        };
        if track.width <= 0.0 {
            return Ok(false);
        }
        let value = self.read(Entity::<RangeField>::from_stable_id(target), |range| {
            range.minimum
                + f64::from(((x - track.x) / track.width).clamp(0.0, 1.0))
                    * (range.maximum - range.minimum)
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
        if !self.is_range_field(target) {
            return Ok(false);
        }
        self.update_component(Entity::<RangeField>::from_stable_id(target), |range, cx| {
            cx.mutations().release_pointer(pointer_id, target);
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
            true
        })
    }

    pub fn begin_xy_pad_drag(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
        if self.read(Entity::<XYPad>::from_stable_id(target), XYPad::inactive)? {
            return Ok(false);
        }
        let Some(bounds) = self.world.layout_box(target) else {
            return Ok(false);
        };
        self.update_component(Entity::<XYPad>::from_stable_id(target), |pad, cx| {
            pad.dragging = Some(XYPadDragState {
                pointer_id,
                origin_x: x - bounds.x,
                origin_y: y - bounds.y,
                axis_lock: None,
                initial: pad.value,
            });
            cx.mutations().capture_pointer(pointer_id, target);
        })?;
        self.update_xy_pad_drag(document, pointer_id, x, y, false)
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
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
        let Some(bounds) = self.world.layout_box(target) else {
            return Ok(false);
        };
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
        if !self.is_xy_pad(target) {
            return Ok(false);
        }
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
            cx.mutations().release_pointer(pointer_id, target);
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
