//! A range's assembled children: the reset button its
//! [`RangeField::default_value`] puts at the end of the row, and the number
//! field a [`RangeField::value_thumb`] types in. Both lie over the range's
//! own parts (#384) as absolutely placed children, so the row's layout stays
//! the range's: its parts reserve the button's square and the capsule's
//! place, and the children are hung there.

use std::sync::Arc;

use nana_ui_core::{
    ControlPadding, Icon, LengthSpec, NumberFieldSpec, PositionSpec, ThemeExpr, ThemeToken,
};

use crate::view_components::{
    Activate, IconButton, NumberChanged, NumberInput, RangeParts, format_range_value,
};
use crate::{
    AppContext, DocumentId, Entity, FocusWithinChanged, FrameworkError, RangeChanged, RangeField,
    RangeInput,
};

impl AppContext {
    /// Create or refresh `range`'s reset button and number field. A range
    /// with neither a default value nor a value thumb assembles nothing.
    pub fn assemble_range_field(
        &mut self,
        range: Entity<RangeField>,
    ) -> Result<bool, FrameworkError> {
        let wanted = self.read(range, |range| {
            range.parts.is_some() || range.default_value.is_some() || range.draws_value_thumb()
        })?;
        if !wanted {
            return Ok(false);
        }
        let document = self.document_of_node(range.stable_id())?;
        let snapshot = self.read(range, Clone::clone)?;
        let alive = snapshot.parts.filter(|parts| {
            [parts.reset, parts.input]
                .iter()
                .all(|id| self.world().contains(*id))
        });
        let created = alive.is_none();
        let reset_label = Arc::clone(&self.world().framework_strings().range_field_reset);
        let parts = match alive {
            Some(parts) => parts,
            None => {
                let reset = self.create_detached_component(
                    document,
                    IconButton::tool(Icon::Reset, Arc::clone(&reset_label)),
                )?;
                self.observe(reset, range, |range, _: &Activate, cx| {
                    let Some(default) = range.default_value else {
                        return;
                    };
                    let value = range.quantize(default);
                    if range.disabled || range.value == value {
                        return;
                    }
                    range.value = value;
                    if let Some(drag) = range.dragging.as_mut() {
                        drag.initial_value = value;
                    }
                    cx.emit(RangeInput { value });
                    cx.emit(RangeChanged { value });
                    // the button is now at the default: disabled
                    cx.reassemble();
                })?;
                let input =
                    self.create_detached_component(document, NumberInput::continuous(0.0))?;
                self.observe(input, range, |range, event: &NumberChanged, cx| {
                    if !range.editing || range.disabled {
                        return;
                    }
                    let value = range.quantize(event.value);
                    if value != range.value {
                        range.value = value;
                        cx.emit(RangeInput { value });
                        cx.emit(RangeChanged { value });
                        cx.reassemble();
                    }
                })?;
                // Typing ends when the focus leaves the range.
                self.on(range, |range, event: &FocusWithinChanged, cx| {
                    if !event.focused && range.editing {
                        range.editing = false;
                        cx.reassemble();
                    }
                })?;
                RangeParts {
                    reset: reset.stable_id(),
                    input: input.stable_id(),
                }
            }
        };
        let reset_shown = snapshot.default_value.is_some() && snapshot.rail.is_none();
        let reset_disabled = snapshot.disabled || snapshot.at_default();
        let half_square = self.world().theme().spacing().page / 2.0;
        self.append_children(range.stable_id(), &[parts.reset, parts.input])?;
        if created {
            self.update_component(range, |range, _| range.parts = Some(parts))?;
        }
        self.update_component(
            Entity::<IconButton>::from_stable_id(parts.reset),
            |button, _| {
                let layout = Arc::make_mut(&mut button.style.layout);
                button.disabled = reset_disabled;
                button.label = Arc::clone(&reset_label);
                layout.hidden = !reset_shown;
                layout.position = PositionSpec::Absolute;
                // The range is the containing block: the square sits in the
                // trailing part its row reserves, the field's end padding in
                // from its edge, centred down the row.
                layout.theme.offset_right = Some(ThemeExpr::token(ThemeToken::ControlPadding(
                    ControlPadding::Field,
                )));
                layout.offset_top = Some(LengthSpec::percent_offset_inline(50.0, -half_square));
            },
        )?;
        let editing = snapshot.editing && snapshot.draws_value_thumb();
        let field_height = snapshot.size.height_in(self.world().theme().metrics());
        let placement = editing.then(|| self.range_edit_placement(range)).flatten();
        let spec = NumberFieldSpec {
            minimum: Some(snapshot.minimum),
            maximum: Some(snapshot.maximum),
            step: snapshot.step,
            precision: range_precision(snapshot.step),
        };
        self.update_component(
            Entity::<NumberInput>::from_stable_id(parts.input),
            |input, _| {
                input.spec = spec;
                input.size = snapshot.size;
                input.steppers = false;
                input.disabled = snapshot.disabled;
                input.label = snapshot
                    .accessible_name
                    .clone()
                    .or_else(|| snapshot.label.clone());
                let layout = Arc::make_mut(&mut input.style.layout);
                layout.hidden = !editing;
                layout.position = PositionSpec::Absolute;
                layout.offset_top =
                    Some(LengthSpec::percent_offset_inline(50.0, -field_height / 2.0));
                if let Some((left, width)) = placement {
                    layout.offset_left = Some(LengthSpec::Px(left));
                    layout.width = Some(LengthSpec::Px(width));
                }
            },
        )?;
        Ok(created)
    }

    /// Where the number field lies while `range` types: over the capsule,
    /// a little wider, in the range's padding box. `None` before the range
    /// has been laid out.
    fn range_edit_placement(&self, range: Entity<RangeField>) -> Option<(f32, f32)> {
        let id = range.stable_id();
        let thumb = self
            .world()
            .range_geometry(id)
            .and_then(|g| g.value_thumb)?;
        let bounds = self.world().component_layout_box(id)?;
        let border = self
            .world()
            .node_style(id)
            .map_or(0.0, |style| style.layout.resolved_border_width());
        let spacing = self.world().theme().spacing();
        let width = thumb.capsule.width + spacing.xl + spacing.xl;
        let centre = thumb.capsule.x + thumb.capsule.width / 2.0 - bounds.x - border;
        let room = (bounds.width - border - border).max(width);
        Some(((centre - width / 2.0).clamp(0.0, room - width), width))
    }

    /// Start typing a value into `range`: its value thumb makes way for the
    /// number field, which shows the value, selected, with the focus.
    /// Refused unless the range draws a value thumb and is enabled.
    pub fn begin_range_edit(&mut self, range: Entity<RangeField>) -> Result<bool, FrameworkError> {
        self.begin_range_edit_focus(range, true)
    }

    /// Like [`Self::begin_range_edit`], without scrolling ancestor scrollports:
    /// used when a pointer click on the already-visible value thumb starts typing.
    pub(crate) fn begin_range_edit_in_place(
        &mut self,
        range: Entity<RangeField>,
    ) -> Result<bool, FrameworkError> {
        self.begin_range_edit_focus(range, false)
    }

    fn begin_range_edit_focus(
        &mut self,
        range: Entity<RangeField>,
        reveal: bool,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(range.stable_id())?;
        let Some(value) = self.update_component(range, |range, _| {
            if range.disabled || range.editing || !range.draws_value_thumb() {
                return None;
            }
            range.editing = true;
            Some(range.value)
        })?
        else {
            return Ok(false);
        };
        let Some(parts) = self.read(range, |range| range.parts)? else {
            return Ok(false);
        };
        let input = Entity::<NumberInput>::from_stable_id(parts.input);
        self.update_component(input, |input, _| {
            input.assign(value);
        })?;
        if reveal {
            self.focus_node(document, parts.input)?;
        } else {
            self.focus_node_in_place(document, parts.input)?;
        }
        self.select_all_focused_text(document)?;
        Ok(true)
    }

    /// Leave typing on the range whose number field has the focus (Enter
    /// committed it, Escape reverted it): the range takes the focus back.
    pub(crate) fn end_focused_range_edit(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(range) = self
            .world()
            .focused(document)
            .and_then(|input| self.world().parent_id(input))
            .and_then(|parent| self.view_entity::<RangeField>(parent))
        else {
            return Ok(false);
        };
        let editing = self.update_component(range, |range, _| {
            std::mem::replace(&mut range.editing, false)
        })?;
        if editing {
            self.focus_node_in_place(document, range.stable_id())?;
        }
        Ok(editing)
    }

    /// Enter (or Space, or an accessibility press) on a value thumb types a
    /// value; any other range has no activation.
    pub(crate) fn activate_range_field(
        &mut self,
        range: Entity<RangeField>,
    ) -> Result<bool, FrameworkError> {
        self.begin_range_edit(range)
    }
}

/// The decimals a range's values are written with: its step's.
fn range_precision(step: f64) -> u8 {
    format_range_value(step, step)
        .split_once('.')
        .map_or(0, |(_, decimals)| decimals.len() as u8)
}

#[cfg(test)]
mod tests;
