//! Drag-to-scrub number field (#338): the dense value field of professional
//! tools' property panels. Pressing on it and dragging sideways changes the
//! value — Ctrl (Cmd) finer, Shift coarser — with [`ScrubInput`] while the
//! drag lasts and one [`ScrubChanged`] when it ends, so an application keeps
//! only the commit in its undo history. A press that does not move, Enter or
//! Space switch the field to typing: an assembled [`NumberInput`] takes the
//! focus with its text selected; Enter or leaving commits, Escape reverts.
//! Arrow keys step a focused field. A field showing values that differ across
//! several objects is [`ScrubField::mixed`]: it shows a dash until a drag,
//! a step or a typed number sets them all.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, CursorSpec, DisplaySpec, FlexDirection, JustifySpec, LengthSpec,
    NumberFieldSpec, SemanticColorRole,
};

use crate::view_components::{NumberChanged, NumberInput, Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FocusWithinChanged, FrameworkError, InteractionState, MutationQueue, NodeKind, NodePointer,
    NodePointerHooks, NodeStyle, NodeWheel, RangeAdjustment, StableNodeId, TextContent, UiWorld,
};

/// A press that moves less than this is a click, px.
const CLICK_SLOP: f32 = nana_ui_core::space::XXS;
/// Ctrl (Cmd) drags this much finer, Shift this much coarser.
const FINE: f64 = 0.1;
const COARSE: f64 = 10.0;
/// What a mixed field shows.
const MIXED: &str = "—";

/// The value while a drag is in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrubInput {
    pub value: f64,
}

/// A committed value: the end of a drag, a step, or a typed number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrubChanged {
    pub value: f64,
}

/// A drag started (`true`) or ended (`false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrubDragging {
    pub dragging: bool,
}

/// A press held on the field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrubDrag {
    origin: f32,
    last: f32,
    /// The unrounded value the drag has reached.
    raw: f64,
    start: f64,
    moved: bool,
}

/// The assembled parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrubParts {
    pub(crate) label: StableNodeId,
    pub(crate) value: StableNodeId,
    pub(crate) input: StableNodeId,
}

/// A number field changed by dragging across it; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrubField {
    pub value: f64,
    /// Showing several differing values: a dash until the next change.
    pub mixed: bool,
    /// Bounds and display precision; `step` is what an arrow key moves.
    pub spec: NumberFieldSpec,
    /// Value change per pixel dragged; `None` moves one `step` per pixel.
    pub drag_rate: Option<f64>,
    /// Text before the value, e.g. `X`.
    pub label: Option<Arc<str>>,
    /// Text after the value, e.g. `px` or `°`.
    pub unit: Option<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    /// Typing a value: the assembled number input is shown and focused.
    pub editing: bool,
    pub style: NodeStyle,
    pub(crate) drag: Option<ScrubDrag>,
    pub(crate) parts: Option<ScrubParts>,
}

impl ScrubField {
    pub fn new(value: f64) -> Self {
        Self {
            value,
            mixed: false,
            spec: NumberFieldSpec::default(),
            drag_rate: None,
            label: None,
            unit: None,
            size: ControlSize::Medium,
            disabled: false,
            editing: false,
            style: field_style(ControlSize::Medium),
            drag: None,
            parts: None,
        }
    }

    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.spec.minimum = Some(minimum);
        self.spec.maximum = Some(maximum);
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.spec.step = step;
        self
    }

    pub fn precision(mut self, precision: u8) -> Self {
        self.spec.precision = precision;
        self
    }

    pub fn drag_rate(mut self, per_pixel: f64) -> Self {
        self.drag_rate = Some(per_pixel);
        self
    }

    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn unit(mut self, unit: impl Into<Arc<str>>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self.style = field_style(size);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `value` within the bounds at the display precision.
    pub fn settle(&self, value: f64) -> f64 {
        let unit = 10f64.powi(i32::from(self.spec.precision));
        self.spec.clamp((value * unit).round() / unit)
    }

    /// The value as shown: a dash when mixed, else the number and the unit.
    pub fn display_value(&self) -> String {
        if self.mixed {
            return MIXED.to_owned();
        }
        let number = self.spec.format(self.value);
        match &self.unit {
            Some(unit) => format!("{number} {unit}"),
            None => number,
        }
    }

    fn rate(&self, pointer: &NodePointer) -> f64 {
        let base = self.drag_rate.unwrap_or(self.spec.step);
        if pointer.modifiers.control || pointer.modifiers.meta {
            base * FINE
        } else if pointer.modifiers.shift {
            base * COARSE
        } else {
            base
        }
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Row);
            layout.align_items = AlignSpec::Center;
            layout.justify_content = JustifySpec::SpaceBetween;
            layout.gap = Some(LengthSpec::Px(nana_ui_core::space::SM));
            if self.editing {
                // the number input brings its own frame and fills the field
                layout.padding_left = Some(LengthSpec::Px(0.0));
                layout.padding_right = Some(LengthSpec::Px(0.0));
                layout.border_width = Some(0.0);
            }
            layout.cursor = Some(if self.disabled {
                CursorSpec::NotAllowed
            } else if self.editing {
                CursorSpec::Text
            } else {
                CursorSpec::EwResize
            });
        }
        if self.drag.is_some_and(|d| d.moved) {
            style.background = Some(SemanticColorRole::Hover);
        }
        style.control_height = Some(nana_ui_core::ControlHeight::Min(self.size));
        style
    }
}

impl ComponentView for ScrubField {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        hooks: Some(|hooks| {
            crate::framework::routes::scrub_field(hooks);
            crate::framework::routes::number_input(hooks);
        }),
        activation: Some(activate_scrub_field),
        assembler: Some(AppContext::assemble_scrub_field),
        pointer: Some(&SCRUB_POINTER),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(&mut self, share: &mut dyn FnMut(&mut Arc<nana_ui_core::LayoutStyle>)) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // A drag or typing in progress is the field's own; the parts are the
        // runtime's.
        next.drag = self.drag;
        next.editing = self.editing && !next.disabled;
        next.parts = self.parts;
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "scrub-field".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some("") {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled && !self.editing,
            },
            AccessibilityState {
                role: AccessibilityRole::Slider,
                label: self.label.clone(),
                value: Some(Arc::from(self.display_value())),
                disabled: self.disabled,
                numeric_minimum: self.spec.minimum,
                numeric_maximum: self.spec.maximum,
                numeric_step: Some(self.spec.step),
                numeric_value: (!self.mixed).then_some(self.value),
                ..AccessibilityState::default()
            },
        );
    }
}

fn field_style(size: ControlSize) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Subtle);
    style.border = Some(SemanticColorRole::Border);
    {
        let layout = Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(LengthSpec::Px(nana_ui_core::space::SM));
        layout.padding_right = Some(LengthSpec::Px(nana_ui_core::space::SM));
        layout.border_width = Some(nana_ui_core::HAIRLINE);
        layout.width = Some(LengthSpec::Fill);
    }
    style.shape = Some(nana_ui_core::ShapeRole::Field);
    style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
    // the text fields' states (hover border, keyboard focus border, disabled)
    style.interaction = crate::InteractionStyle {
        hovered: crate::SemanticPaint {
            border: Some(SemanticColorRole::BorderStrong),
            ..crate::SemanticPaint::default()
        },
        focused: crate::SemanticPaint::FOCUS_BORDER,
        disabled: crate::SemanticPaint {
            foreground: Some(SemanticColorRole::Faint),
            background: Some(SemanticColorRole::Subtle),
            border: Some(SemanticColorRole::Border),
            ..crate::SemanticPaint::default()
        },
        ..crate::InteractionStyle::default()
    }
    .into();
    style
}

fn shown(display: bool) -> Option<DisplaySpec> {
    (!display).then_some(DisplaySpec::None)
}

pub(crate) const SCRUB_POINTER: NodePointerHooks<ScrubField> = NodePointerHooks {
    moved: scrub_moved,
    left: scrub_left,
    pressed: scrub_pressed,
    released: scrub_released,
    wheel: scrub_wheel,
};

fn scrub_pressed(
    cx: &mut AppContext,
    field: Entity<ScrubField>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if pointer.button != 0 {
        return Ok(false);
    }
    cx.update_component(field, |field, _| {
        if field.disabled || field.editing {
            return false;
        }
        field.drag = Some(ScrubDrag {
            origin: pointer.x,
            last: pointer.x,
            raw: field.value,
            start: field.value,
            moved: false,
        });
        true
    })
}

fn scrub_moved(
    cx: &mut AppContext,
    field: Entity<ScrubField>,
    pointer: NodePointer,
) -> Result<bool, FrameworkError> {
    if !pointer.captured {
        return Ok(false);
    }
    cx.update_component(field, |field, view| {
        let Some(mut drag) = field.drag else {
            return false;
        };
        if !drag.moved && (pointer.x - drag.origin).abs() < CLICK_SLOP {
            return true;
        }
        if !drag.moved {
            drag.moved = true;
            // the slop is not lost: the drag measures from the press
            drag.last = drag.origin;
            view.emit(ScrubDragging { dragging: true });
        }
        drag.raw += f64::from(pointer.x - drag.last) * field.rate(&pointer);
        drag.last = pointer.x;
        field.drag = Some(drag);
        let value = field.settle(drag.raw);
        if value != field.value || field.mixed {
            field.value = value;
            field.mixed = false;
            view.emit(ScrubInput { value });
        }
        true
    })
}

fn scrub_released(
    cx: &mut AppContext,
    field: Entity<ScrubField>,
    _: NodePointer,
) -> Result<bool, FrameworkError> {
    let Some(drag) = cx.update_component(field, |field, view| {
        let drag = field.drag.take()?;
        if drag.moved {
            view.emit(ScrubDragging { dragging: false });
            if field.value != drag.start {
                view.emit(ScrubChanged { value: field.value });
            }
        }
        Some(drag)
    })?
    else {
        return Ok(false);
    };
    if !drag.moved {
        cx.begin_scrub_edit(field)?;
    }
    Ok(true)
}

fn scrub_left(cx: &mut AppContext, field: Entity<ScrubField>) -> Result<(), FrameworkError> {
    // a cancelled press: back to where the drag started
    cx.update_component(field, |field, view| {
        if let Some(drag) = field.drag.take()
            && drag.moved
        {
            field.value = drag.start;
            view.emit(ScrubInput { value: drag.start });
            view.emit(ScrubDragging { dragging: false });
        }
    })
}

fn scrub_wheel(
    _: &mut AppContext,
    _: Entity<ScrubField>,
    _: NodeWheel,
) -> Result<bool, FrameworkError> {
    // the wheel scrolls the panel the field sits in
    Ok(false)
}

fn activate_scrub_field(
    cx: &mut AppContext,
    field: Entity<ScrubField>,
) -> Result<bool, FrameworkError> {
    cx.begin_scrub_edit(field)
}

impl AppContext {
    /// Create or refresh `field`'s label, value text and number input.
    pub fn assemble_scrub_field(
        &mut self,
        field: Entity<ScrubField>,
    ) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(field.stable_id())?;
        let snapshot = self.read(field, Clone::clone)?;
        let alive = snapshot.parts.filter(|p| {
            [p.label, p.value, p.input]
                .iter()
                .all(|id| self.world().contains(*id))
        });
        let created = alive.is_none();
        let parts = match alive {
            Some(parts) => parts,
            None => {
                let label = self.create_detached_component(document, Text::new(""))?;
                let value = self.create_detached_component(document, Text::new(""))?;
                let input =
                    self.create_detached_component(document, NumberInput::continuous(0.0))?;
                self.observe(input, field, |field, event: &NumberChanged, cx| {
                    let value = field.settle(event.value);
                    if field.editing && (value != field.value || field.mixed) {
                        field.value = value;
                        field.mixed = false;
                        cx.emit(ScrubChanged { value });
                    }
                })?;
                // typing ends when the focus leaves the field
                self.on(field, |field, event: &FocusWithinChanged, _| {
                    if !event.focused {
                        field.editing = false;
                    }
                })?;
                ScrubParts {
                    label: label.stable_id(),
                    value: value.stable_id(),
                    input: input.stable_id(),
                }
            }
        };
        let label = snapshot.label.clone();
        self.update_component(Entity::<Text>::from_stable_id(parts.label), |text, _| {
            text.value = label.as_deref().unwrap_or("").to_owned();
            Arc::make_mut(&mut text.style.layout).display =
                shown(label.is_some() && !snapshot.editing);
            text.style.foreground = Some(if snapshot.disabled {
                SemanticColorRole::Faint
            } else {
                SemanticColorRole::Muted
            });
        })?;
        let shown_value = snapshot.display_value();
        self.update_component(Entity::<Text>::from_stable_id(parts.value), |text, _| {
            text.value = shown_value;
            Arc::make_mut(&mut text.style.layout).display = shown(!snapshot.editing);
            text.style.foreground = snapshot.disabled.then_some(SemanticColorRole::Faint);
        })?;
        let spec = snapshot.spec;
        self.update_component(
            Entity::<NumberInput>::from_stable_id(parts.input),
            |input, _| {
                input.spec = spec;
                input.size = snapshot.size;
                input.disabled = snapshot.disabled;
                let layout = Arc::make_mut(&mut input.style.layout);
                layout.display = shown(snapshot.editing);
                layout.width = Some(LengthSpec::Fill);
            },
        )?;
        if created {
            self.update_component(field, |field, _| field.parts = Some(parts))?;
        }
        self.append_children(field.stable_id(), &[parts.label, parts.value, parts.input])?;
        Ok(created)
    }

    /// Switch `field` to typing: the input shows the value, selected, with
    /// the focus.
    pub fn begin_scrub_edit(&mut self, field: Entity<ScrubField>) -> Result<bool, FrameworkError> {
        let document = self.document_of_node(field.stable_id())?;
        let Some((value, mixed)) = self.update_component(field, |field, _| {
            if field.disabled || field.editing {
                return None;
            }
            field.editing = true;
            Some((field.value, field.mixed))
        })?
        else {
            return Ok(false);
        };
        let Some(parts) = self.read(field, |field| field.parts)? else {
            return Ok(false);
        };
        let input = Entity::<NumberInput>::from_stable_id(parts.input);
        self.update_component(input, |input, _| {
            input.assign(value);
            if mixed {
                input.state.replace_value(String::new());
            }
        })?;
        self.focus_node(document, parts.input)?;
        self.select_all_focused_text(document)?;
        Ok(true)
    }

    /// One step up or down by an assistive action, announced as a drag's
    /// change is.
    pub(crate) fn step_scrub_field(
        &mut self,
        entity: Entity<ScrubField>,
        steps: i32,
    ) -> Result<bool, FrameworkError> {
        let step = self.read(entity, |field| field.spec.step)?;
        let value = self.read(entity, |field| field.value)?;
        let next = value + step * f64::from(steps.signum());
        if self.set_scrub_field_value(entity, next, false)? {
            let value = self.read(entity, |field| field.value)?;
            self.update_component(entity, |_, view| view.emit(crate::ScrubChanged { value }))?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Leave typing on the field whose input has the focus (Enter committed
    /// or Escape reverted it): the field takes the focus back.
    pub(crate) fn end_focused_scrub_edit(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(field) = self
            .world()
            .focused(document)
            .and_then(|input| self.world().parent_id(input))
            .and_then(|parent| self.view_entity::<ScrubField>(parent))
        else {
            return Ok(false);
        };
        let editing = self.update_component(field, |field, _| {
            std::mem::replace(&mut field.editing, false)
        })?;
        if editing {
            self.focus_node_in_place(document, field.stable_id())?;
        }
        Ok(editing)
    }

    /// Step the focused field (arrow keys): a commit.
    pub(crate) fn adjust_focused_scrub_field(
        &mut self,
        document: DocumentId,
        adjustment: RangeAdjustment,
    ) -> Result<bool, FrameworkError> {
        let Some(field) = self
            .world()
            .focused(document)
            .and_then(|id| self.view_entity::<ScrubField>(id))
        else {
            return Ok(false);
        };
        self.update_component(field, |field, view| {
            if field.disabled || field.editing {
                return false;
            }
            let step = field.spec.step;
            let target = match adjustment {
                RangeAdjustment::Increment => field.value + step,
                RangeAdjustment::Decrement => field.value - step,
                RangeAdjustment::PageIncrement => field.value + step * COARSE,
                RangeAdjustment::PageDecrement => field.value - step * COARSE,
                RangeAdjustment::Minimum => match field.spec.minimum {
                    Some(minimum) => minimum,
                    None => return false,
                },
                RangeAdjustment::Maximum => match field.spec.maximum {
                    Some(maximum) => maximum,
                    None => return false,
                },
            };
            let value = field.settle(target);
            if value == field.value && !field.mixed {
                return true;
            }
            field.value = value;
            field.mixed = false;
            view.emit(ScrubChanged { value });
            true
        })
    }

    /// Set the committed value from the application.
    pub fn set_scrub_field_value(
        &mut self,
        field: Entity<ScrubField>,
        value: f64,
        mixed: bool,
    ) -> Result<bool, FrameworkError> {
        self.update_component(field, |field, _| {
            let value = field.settle(value);
            let changed = field.value != value || field.mixed != mixed;
            field.value = value;
            field.mixed = mixed;
            changed
        })
    }

    pub(crate) fn document_of_node(&self, id: StableNodeId) -> Result<DocumentId, FrameworkError> {
        Ok(self
            .world()
            .node(id)
            .ok_or(FrameworkError::MissingView(id))?
            .document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field built by a view gets its parts without the view assembling it.
    #[test]
    fn a_view_field_assembles_itself() {
        use crate::view::widget;
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let root = context
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        context
            .mount_view(root.stable_id(), || {
                widget(ScrubField::new(2.5).precision(1).unit("°"))
            })
            .unwrap();
        let field = context.world().node(root.stable_id()).unwrap().children[0];
        let parts = context
            .read(Entity::<ScrubField>::from_stable_id(field), |f| f.parts)
            .unwrap()
            .expect("assembled");
        assert_eq!(context.world().text(parts.value), Some("2.5 °"));
    }

    #[test]
    fn settle_rounds_to_the_precision_within_the_bounds() {
        let field = ScrubField::new(0.0).range(-1.0, 1.0).precision(2);
        assert_eq!(field.settle(0.123_4), 0.12);
        assert_eq!(field.settle(7.0), 1.0);
        assert_eq!(field.settle(f64::NAN), -1.0);
        assert_eq!(ScrubField::new(0.0).mixed(true).display_value(), "—");
    }
}
